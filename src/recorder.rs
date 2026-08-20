//! Общий (не только War Thunder) писатель сессий в JSONL, встроенный прямо в
//! воркеры телеметрии. Раньше жил в `wt_link::recorder` и умел только WT —
//! перенесён сюда без изменения формата/поведения WT-пути, потому что
//! записывать сессии теперь умеют все три конвейера (`wt_link::worker`,
//! `sim::worker`, `xp_link::worker`), а не только War Thunder.
//!
//! Пишет тот же JSONL-формат, что и `wt_probe::writer`
//! (`{"t":..,"endpoint":"state"|"indicators"|"flightvars","body":..}`, одна
//! строка на объект):
//! - `tick_wt` — ДВЕ строки на тик (`state`+`indicators`), БАЙТ В БАЙТ как
//!   раньше — на это завязаны существующие интеграционные replay-тесты
//!   (`tests/wt_*_replay.rs`) и утилита `wt_probe`, которые читают файлы из
//!   той же `wt_probe_sessions/` напрямую по имени `session_<timestamp>.jsonl`
//!   (см. `start()` ниже — у WT `game_marker` всегда `None`, имя не трогали).
//! - `tick_flightvars` — новый путь для MSFS/X-Plane: ОДНА строка `flightvars`
//!   на тик (вся телеметрия уже в одной структуре, дробить незачем), имя
//!   файла получает короткий маркер игры (`session_msfs_...`/
//!   `session_xplane_...`), чтобы на диске было видно, какая это запись, не
//!   открывая файл. `custom_fx::player::load_session` уже умеет читать этот
//!   endpoint.
//!
//! В отличие от `wt_probe` (многопоточный, отдельные каналы опроса на
//! эндпоинт) здесь всё проще: каждый воркер и так уже получает свою
//! телеметрию каждый тик в одном месте, поэтому записывающий JSONL-writer —
//! не отдельный поток, а метод, вызываемый прямо из основного цикла воркера.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

use chrono::Local;
use serde::Serialize;
use serde_json::Value;

use crate::{FlightVars, LogBuffer};
// Каталог сессий определён РОВНО в одном месте — у плеера. Рекордер пишет
// туда, плеер оттуда читает; два независимых списка путей молча разъехались
// бы при первой же правке одного из них.
use crate::custom_fx::player::sessions_dir;

/// Тот же ритм принудительного сброса на диск, что у `wt_probe::writer`
/// (`FLUSH_INTERVAL`) — не после каждой строки (лишний syscall на каждый тик
/// 20 Гц), но и не только при закрытии файла, чтобы аварийное завершение
/// процесса не смыло всю сессию целиком.
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

pub struct SessionRecorder {
    writer: Option<BufWriter<File>>,
    last_flush: Instant,
}

impl Default for SessionRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRecorder {
    pub fn new() -> Self {
        Self {
            writer: None,
            last_flush: Instant::now(),
        }
    }

    /// Вызывается каждый тик `wt_worker`, когда WT реально ответил на
    /// `/state`+`/indicators` — сам решает, когда открыть/закрыть файл (по
    /// фронту `enabled`) и когда писать/сбрасывать буфер, вызывающему коду
    /// не нужно хранить никакого дополнительного состояния перехода.
    /// Формат и имя файла — БАЙТ В БАЙТ как раньше (см. doc-комментарий
    /// модуля): `game_marker = None`.
    pub fn tick_wt(
        &mut self,
        enabled: bool,
        t: f64,
        state: &Value,
        indicators: &Value,
        logs: &LogBuffer,
    ) {
        self.sync_enabled(enabled, logs, None);

        let Some(w) = self.writer.as_mut() else {
            return;
        };
        let _ = write_line(w, t, "state", state);
        let _ = write_line(w, t, "indicators", indicators);
        self.flush_if_due();
    }

    /// Вызывается каждый тик `sim_worker`/`xp_worker`, когда конвейер владеет
    /// слотом и есть готовый кадр `FlightVars`. В отличие от WT (два объекта
    /// на тик) тут один объект `flightvars` на тик. `game_marker`
    /// ("msfs"/"xplane") попадает в имя файла — см. doc-комментарий модуля.
    pub fn tick_flightvars(
        &mut self,
        enabled: bool,
        t: f64,
        fv: &FlightVars,
        game_marker: &str,
        logs: &LogBuffer,
    ) {
        self.sync_enabled(enabled, logs, Some(game_marker));

        let Some(w) = self.writer.as_mut() else {
            return;
        };
        let _ = write_line(w, t, "flightvars", fv);
        self.flush_if_due();
    }

    /// Общий переход состояния "надо ли сейчас писать файл" — раньше был
    /// продублирован в начале единственного метода `tick`, теперь общий для
    /// обоих путей записи.
    fn sync_enabled(&mut self, enabled: bool, logs: &LogBuffer, game_marker: Option<&str>) {
        if enabled && self.writer.is_none() {
            self.start(logs, game_marker);
        } else if !enabled && self.writer.is_some() {
            self.stop(logs);
        }
    }

    fn flush_if_due(&mut self) {
        if self.last_flush.elapsed() >= FLUSH_INTERVAL
            && let Some(w) = self.writer.as_mut()
        {
            let _ = w.flush();
            self.last_flush = Instant::now();
        }
    }

    fn start(&mut self, logs: &LogBuffer, game_marker: Option<&str>) {
        let dir = sessions_dir();
        if let Err(e) = fs::create_dir_all(&dir) {
            logs.push(format!("Recorder: failed to create {}: {e}", dir.display()));
            return;
        }
        let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
        // WT — без маркера, имя ровно как раньше (`session_<ts>.jsonl`, см.
        // doc-комментарий модуля); MSFS/X-Plane получают короткий маркер игры.
        let filename = match game_marker {
            Some(marker) => format!("session_{marker}_{timestamp}.jsonl"),
            None => format!("session_{timestamp}.jsonl"),
        };
        let path = dir.join(filename);
        match File::create(&path) {
            Ok(f) => {
                logs.push(format!("Recorder: started → {}", path.display()));
                self.writer = Some(BufWriter::new(f));
                self.last_flush = Instant::now();
            }
            Err(e) => logs.push(format!(
                "Recorder: failed to create {}: {e}",
                path.display()
            )),
        }
    }

    fn stop(&mut self, logs: &LogBuffer) {
        if let Some(mut w) = self.writer.take() {
            let _ = w.flush();
        }
        logs.push("Recorder: stopped".to_string());
    }
}

/// То же самое ручное форматирование, что `wt_probe::writer::format_line` —
/// `body` пересериализуется компактно (WT отдаёт `/state`/`/indicators`
/// pretty-printed, со встроенными переводами строк, которые сломали бы JSONL).
/// Общий для WT (`body: &Value`) и flightvars (`body: &FlightVars`) — оба
/// реализуют `Serialize`, а сам формат строки (порядок полей `t`/`endpoint`/
/// `body`, точность `t`) от типа `body` не зависит.
fn write_line<T: Serialize>(
    w: &mut BufWriter<File>,
    t: f64,
    endpoint: &str,
    body: &T,
) -> std::io::Result<()> {
    let compact_body = serde_json::to_string(body).unwrap_or_else(|_| "null".to_string());
    writeln!(
        w,
        "{{\"t\":{t:.6},\"endpoint\":\"{endpoint}\",\"body\":{compact_body}}}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_fx::{player, sources::TelemetryFrame};

    /// Связка "рекордер -> загрузчик" целиком для нового пути записи: пишем
    /// пару кадров `FlightVars` через реальный `tick_flightvars` (не напрямую
    /// `write_line`) и проверяем, что `custom_fx::player::load_session`
    /// читает их обратно с теми же значениями (см.
    /// `custom_fx::player::load_session`, ветка `endpoint == "flightvars"`).
    #[test]
    fn tick_flightvars_roundtrips_through_load_session() {
        // Уникальный маркер вместо "msfs"/"xplane" — чтобы найти РОВНО файл
        // этого теста в sessions_dir() среди того, что могло остаться от
        // прошлых прогонов/ручных сессий, не гадая по timestamp в имени.
        let marker = "unittestflightvars";
        let dir = sessions_dir();
        let _ = fs::create_dir_all(&dir);
        cleanup_marker_files(&dir, marker);

        let logs = LogBuffer::default();
        let mut recorder = SessionRecorder::new();
        let fv1 = FlightVars {
            airspeed_indicated: 123.5,
            on_ground: false,
            ..Default::default()
        };
        let fv2 = FlightVars {
            airspeed_indicated: 200.0,
            on_ground: true,
            ..Default::default()
        };

        // Включаем запись только на первый тик — `start()` открывает файл по
        // фронту `enabled` (см. `sync_enabled`), дальше файл остаётся
        // открытым до явного `enabled = false`.
        recorder.tick_flightvars(true, 1.0, &fv1, marker, &logs);
        recorder.tick_flightvars(true, 2.5, &fv2, marker, &logs);
        // Явно гасим запись, чтобы файл точно сброшен на диск (flush) к
        // моменту чтения ниже.
        recorder.tick_flightvars(false, 3.0, &fv2, marker, &logs);

        let written = find_marker_file(&dir, marker).expect("recorder wrote a session file");
        let session = player::load_session(&written).expect("load_session on freshly written file");
        let _ = fs::remove_file(&written);

        assert_eq!(session.frames.len(), 2);
        assert_eq!(session.frames[0].t, 1.0);
        assert_eq!(session.frames[1].t, 2.5);

        match &session.frames[0].frame {
            TelemetryFrame::Flight(fv) => {
                assert_eq!(fv.airspeed_indicated, 123.5);
                assert!(!fv.on_ground);
            }
            _ => panic!("Ожидался Flight-кадр"),
        }
        match &session.frames[1].frame {
            TelemetryFrame::Flight(fv) => {
                assert_eq!(fv.airspeed_indicated, 200.0);
                assert!(fv.on_ground);
            }
            _ => panic!("Ожидался Flight-кадр"),
        }
    }

    fn cleanup_marker_files(dir: &std::path::Path, marker: &str) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let prefix = format!("session_{marker}_");
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn find_marker_file(dir: &std::path::Path, marker: &str) -> Option<std::path::PathBuf> {
        let prefix = format!("session_{marker}_");
        fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
            })
    }
}
