//! Встроенная запись сессии War Thunder прямо из `wt_worker`, без отдельной
//! утилиты `wt_probe`. Пишет ровно тот же JSONL-формат, что и
//! `wt_probe::writer` (`{"t":..,"endpoint":"state"|"indicators","body":..}`,
//! одна строка на объект), поэтому файлы, записанные отсюда, кладутся в тот
//! же `wt_probe_sessions/` и автоматически подхватываются существующими
//! replay-тестами (`tests/wt_ammo_fallback_replay.rs`) без каких-либо
//! изменений в них — формат специально не придуман заново.
//!
//! В отличие от `wt_probe` (многопоточный, отдельные каналы опроса на
//! эндпоинт) здесь всё проще: `wt_worker` и так уже получает `state`/
//! `indicators` каждый тик в одном месте, поэтому записывающий JSONL-writer —
//! не отдельный поток, а метод, вызываемый прямо из основного цикла воркера.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::Local;
use serde_json::Value;

use crate::LogBuffer;

/// Тот же ритм принудительного сброса на диск, что у `wt_probe::writer`
/// (`FLUSH_INTERVAL`) — не после каждой строки (лишний syscall на каждый тик
/// 20 Гц), но и не только при закрытии файла, чтобы аварийное завершение
/// процесса не смыло всю сессию целиком.
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

pub struct SessionRecorder {
    writer: Option<BufWriter<File>>,
    last_flush: Instant,
}

impl SessionRecorder {
    pub fn new() -> Self {
        Self {
            writer: None,
            last_flush: Instant::now(),
        }
    }

    /// Вызывается каждый тик воркера независимо от состояния тумблера.
    /// Сам решает, когда открыть/закрыть файл (по фронту `enabled`) и когда
    /// писать/сбрасывать буфер — вызывающему коду (`worker.rs`) не нужно
    /// хранить никакого дополнительного состояния перехода.
    pub fn tick(&mut self, enabled: bool, t: f64, state: &Value, indicators: &Value, logs: &LogBuffer) {
        if enabled && self.writer.is_none() {
            self.start(logs);
        } else if !enabled && self.writer.is_some() {
            self.stop(logs);
        }

        let Some(w) = self.writer.as_mut() else {
            return;
        };
        let _ = write_line(w, t, "state", state);
        let _ = write_line(w, t, "indicators", indicators);

        if self.last_flush.elapsed() >= FLUSH_INTERVAL {
            let _ = w.flush();
            self.last_flush = Instant::now();
        }
    }

    fn start(&mut self, logs: &LogBuffer) {
        let dir = sessions_dir();
        if let Err(e) = fs::create_dir_all(&dir) {
            logs.push(format!("WT recorder: failed to create {}: {e}", dir.display()));
            return;
        }
        let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
        let path = dir.join(format!("session_{timestamp}.jsonl"));
        match File::create(&path) {
            Ok(f) => {
                logs.push(format!("WT recorder: started → {}", path.display()));
                self.writer = Some(BufWriter::new(f));
                self.last_flush = Instant::now();
            }
            Err(e) => logs.push(format!("WT recorder: failed to create {}: {e}", path.display())),
        }
    }

    fn stop(&mut self, logs: &LogBuffer) {
        if let Some(mut w) = self.writer.take() {
            let _ = w.flush();
        }
        logs.push("WT recorder: stopped".to_string());
    }
}

/// То же самое ручное форматирование, что `wt_probe::writer::format_line` —
/// `body` пересериализуется компактно (WT отдаёт `/state`/`/indicators`
/// pretty-printed, со встроенными переводами строк, которые сломали бы JSONL).
fn write_line(w: &mut BufWriter<File>, t: f64, endpoint: &str, body: &Value) -> std::io::Result<()> {
    let compact_body = serde_json::to_string(body).unwrap_or_else(|_| "null".to_string());
    writeln!(w, "{{\"t\":{t:.6},\"endpoint\":\"{endpoint}\",\"body\":{compact_body}}}")
}

/// Тот же приоритет путей, что у `LogBuffer::try_init_file_prefer_exe_dir` —
/// сначала рядом с exe (портативная установка), потом %LOCALAPPDATA%, чтобы
/// файл сессии лежал предсказуемо там же, где `AuroraVibra.log`/`.settings.json`.
fn sessions_dir() -> PathBuf {
    if let Ok(p) = std::env::current_exe()
        && let Some(dir) = p.parent()
    {
        return dir.join("wt_probe_sessions");
    }
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        let mut p = PathBuf::from(base);
        p.push("AuroraVibra");
        p.push("wt_probe_sessions");
        return p;
    }
    std::env::temp_dir().join("wt_probe_sessions")
}
