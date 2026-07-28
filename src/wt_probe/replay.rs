//! --replay: проигрывает ранее записанный session_*.jsonl с исходным
//! таймингом вместо обращения к игре. Гоняет каждую строку через тот же
//! `writer::handle_outcome`, что и живой опрос, — так схема/оружие
//! считаются идентично в обоих режимах. Позволяет отлаживать логику
//! эффектов без запущенной игры и на другой машине, включая CI на Linux.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::wt_probe::model::{Endpoint, PollOutcome, RawRecord};
use crate::wt_probe::schema::SchemaTracker;
use crate::wt_probe::shared::SharedHandle;
use crate::wt_probe::weapons::WeaponDetector;
use crate::wt_probe::writer::{self, Outputs, WriterResult};

#[derive(Deserialize)]
struct RawLine {
    t: f64,
    endpoint: String,
    body: Value,
}

pub fn run(
    path: &Path,
    speed: f64,
    shared: SharedHandle,
    shutdown: Arc<AtomicBool>,
) -> Result<WriterResult, String> {
    let file =
        File::open(path).map_err(|e| format!("не удалось открыть {}: {e}", path.display()))?;
    let reader = BufReader::new(file);

    let mut schema = SchemaTracker::new();
    let mut weapons = WeaponDetector::new();
    let mut outputs = Outputs {
        session: None,
        events: None,
    };
    let mut prev_t: Option<f64> = None;
    let speed = if speed > 0.0 { speed } else { 1.0 };

    for line in reader.lines() {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        let line = match line {
            Ok(l) => l,
            // Повреждённая строка не должна ронять всё воспроизведение —
            // пропускаем и идём дальше, как и при сетевых ошибках вживую.
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let parsed: RawLine = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(endpoint) = Endpoint::parse_name(&parsed.endpoint) else {
            continue;
        };

        if let Some(prev) = prev_t {
            let delay = ((parsed.t - prev) / speed).max(0.0);
            sleep_interruptible(Duration::from_secs_f64(delay), &shutdown);
        }
        prev_t = Some(parsed.t);

        let record = RawRecord {
            t: parsed.t,
            endpoint,
            parsed: parsed.body,
        };
        writer::handle_outcome(
            PollOutcome::Ok(record),
            &mut outputs,
            &mut schema,
            &mut weapons,
            &shared,
        );
    }
    schema.finalize();
    Ok(WriterResult { schema, weapons })
}

fn sleep_interruptible(total: Duration, shutdown: &AtomicBool) {
    let mut remaining = total;
    while remaining > Duration::ZERO {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        let chunk = remaining.min(Duration::from_millis(50));
        thread::sleep(chunk);
        remaining = remaining.saturating_sub(chunk);
    }
}
