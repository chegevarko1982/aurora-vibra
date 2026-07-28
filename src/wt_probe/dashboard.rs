//! Простой перерисовываемый TUI-дашборд оператора: ANSI clear+home и
//! обычный текст. Полноценный TUI-фреймворк (ratatui/crossterm) намеренно
//! не подключаем — задача прямо говорит, что это не обязательно, а VT100
//! escape-последовательности работают что в Windows Terminal, что в любом
//! Linux-терминале без дополнительных зависимостей.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::wt_probe::interesting;
use crate::wt_probe::model::{ConnStatus, Endpoint};
use crate::wt_probe::shared::SharedHandle;

const CLEAR_AND_HOME: &str = "\x1B[2J\x1B[H";

struct Snapshot {
    conn: ConnStatus,
    vehicle_type: Option<String>,
    recent_events: Vec<String>,
    lines_written: u64,
    bytes_written: u64,
    last_error: Option<String>,
    elapsed: Duration,
    flat_state: BTreeMap<String, Value>,
    flat_indicators: BTreeMap<String, Value>,
    hz: BTreeMap<Endpoint, f64>,
}

pub struct Dashboard {
    targets: BTreeMap<Endpoint, f64>,
    prev_counts: BTreeMap<Endpoint, (u64, Instant)>,
    recording: bool,
}

impl Dashboard {
    pub fn new(targets: BTreeMap<Endpoint, f64>, recording: bool) -> Self {
        Self {
            targets,
            prev_counts: BTreeMap::new(),
            recording,
        }
    }

    fn snapshot(&mut self, shared: &SharedHandle) -> Snapshot {
        let now = Instant::now();
        let s = shared.lock().unwrap_or_else(|e| e.into_inner());
        let mut hz = BTreeMap::new();
        for ep in Endpoint::all() {
            let total = *s.success_total.get(&ep).unwrap_or(&0);
            let hz_val = match self.prev_counts.get(&ep) {
                Some((prev_total, prev_time)) => {
                    let dt = now.duration_since(*prev_time).as_secs_f64();
                    if dt > 0.01 {
                        (total.saturating_sub(*prev_total)) as f64 / dt
                    } else {
                        0.0
                    }
                }
                None => 0.0,
            };
            hz.insert(ep, hz_val);
            self.prev_counts.insert(ep, (total, now));
        }
        Snapshot {
            conn: s.conn,
            vehicle_type: s.vehicle_type.clone(),
            recent_events: s.recent_events.iter().cloned().collect(),
            lines_written: s.lines_written,
            bytes_written: s.bytes_written,
            last_error: s.last_error.clone(),
            elapsed: s.started_at.elapsed(),
            flat_state: s
                .last_flat
                .get(&Endpoint::State)
                .cloned()
                .unwrap_or_default(),
            flat_indicators: s
                .last_flat
                .get(&Endpoint::Indicators)
                .cloned()
                .unwrap_or_default(),
            hz,
        }
    }

    pub fn render(&mut self, shared: &SharedHandle) {
        let text = self.format_text(shared);
        // Один write_all вместо print!-построчно: меньше мерцания в терминале.
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        let _ = lock.write_all(CLEAR_AND_HOME.as_bytes());
        let _ = lock.write_all(text.as_bytes());
        let _ = lock.flush();
    }

    /// Собирает текущий снимок состояния и форматирует его как обычный
    /// текст — без ANSI-очистки экрана. Общая точка входа для обоих
    /// потребителей: терминальный `render()` (ANSI поверх этого текста) и
    /// `wt_probe_gui`, который кладёт этот же текст в скроллируемое поле
    /// нативного окна вместо консоли.
    pub fn format_text(&mut self, shared: &SharedHandle) -> String {
        let snap = self.snapshot(shared);
        self.format(&snap)
    }

    fn format(&self, snap: &Snapshot) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "wt_probe — разведка телеметрии War Thunder  (Ctrl+C — завершить и сохранить отчёты)"
        );
        let _ = writeln!(out, "{}", "=".repeat(78));
        let _ = writeln!(
            out,
            "Соединение: {}    Прошло: {:>6.1} с    Запись: {}",
            snap.conn,
            snap.elapsed.as_secs_f64(),
            if self.recording {
                "да"
            } else {
                "нет (--dashboard-only)"
            }
        );
        let _ = writeln!(
            out,
            "Техника: {}",
            snap.vehicle_type.as_deref().unwrap_or("не определена")
        );
        if let Some(err) = &snap.last_error {
            let _ = writeln!(out, "Последняя ошибка: {err}");
        }
        let _ = writeln!(out);

        let _ = writeln!(out, "-- Частота опроса (Гц, факт / цель) --");
        for ep in Endpoint::all() {
            let achieved = snap.hz.get(&ep).copied().unwrap_or(0.0);
            let target = self.targets.get(&ep).copied().unwrap_or(0.0);
            let _ = writeln!(
                out,
                "  {:<12} {:>6.1} / {:<6.1}",
                ep.as_str(),
                achieved,
                target
            );
        }
        let _ = writeln!(out);

        let _ = writeln!(
            out,
            "-- /state: все поля (сырой ключ -> расшифровка -> значение) --"
        );
        write_decoded_fields(&mut out, &snap.flat_state);
        let _ = writeln!(out);

        let _ = writeln!(
            out,
            "-- /indicators: все поля (сырой ключ -> расшифровка -> значение) --"
        );
        write_decoded_fields(&mut out, &snap.flat_indicators);
        let _ = writeln!(out);

        let _ = writeln!(out, "-- Последние события /hudmsg --");
        if snap.recent_events.is_empty() {
            let _ = writeln!(out, "  (пока нет)");
        } else {
            for e in &snap.recent_events {
                let _ = writeln!(out, "  {e}");
            }
        }
        let _ = writeln!(out);

        let _ = writeln!(
            out,
            "Записано строк: {}    Размер: {}",
            snap.lines_written,
            human_bytes(snap.bytes_written)
        );
        out
    }
}

/// Печатает КАЖДОЕ поле карты (не только первое совпадение на категорию, как
/// в старом "интересные величины") — сырой ключ, расшифровка через
/// interesting::decode_key (или "—", если категория не распознана) и текущее
/// значение. Это и есть "элементарный UI с расшифровкой параметров": оператор
/// видит вообще всё, что прислала игра, сразу с человекочитаемым названием.
fn write_decoded_fields(out: &mut String, fields: &BTreeMap<String, Value>) {
    use std::fmt::Write as _;
    if fields.is_empty() {
        let _ = writeln!(out, "  (нет данных)");
        return;
    }
    for (key, value) in fields {
        let decoded = interesting::decode_key(key).unwrap_or("—");
        let _ = writeln!(out, "  {key:<32} {decoded:<32} = {value}");
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["Б", "КБ", "МБ", "ГБ"];
    let mut value = bytes as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.2} {}", UNITS[unit_idx])
    }
}

/// Печатает финальную сводку по завершении сессии (без ANSI-очистки — она
/// должна остаться в скроллбэке терминала).
pub fn print_final_summary(
    session_path: Option<&std::path::Path>,
    schema_path: &std::path::Path,
    weapons_path: &std::path::Path,
    events_path: Option<&std::path::Path>,
    lines_written: u64,
    bytes_written: u64,
) {
    println!();
    println!("Сессия завершена.");
    if let Some(p) = session_path {
        println!("  Сырой лог:      {}", p.display());
    }
    if let Some(p) = events_path {
        println!("  События hudmsg: {}", p.display());
    }
    println!("  Схема:          {}", schema_path.display());
    println!("  Оружие:         {}", weapons_path.display());
    println!(
        "  Строк записано: {lines_written}, размер: {}",
        human_bytes(bytes_written)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_formats_units() {
        assert_eq!(human_bytes(500), "500 Б");
        assert_eq!(human_bytes(2048), "2.00 КБ");
    }
}
