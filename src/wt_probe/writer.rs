//! Единственный поток-потребитель: получает PollOutcome из каналов опроса,
//! пишет сырой JSONL на диск, кормит SchemaTracker/WeaponDetector и
//! обновляет разделяемое состояние для дашборда. Один писатель — не нужно
//! синхронизировать запись в файл между потоками опроса.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError};
use serde_json::Value;

use crate::wt_probe::hits;
use crate::wt_probe::model::{ConnStatus, Endpoint, FlatPair, PollOutcome};
use crate::wt_probe::schema::{self, SchemaTracker};
use crate::wt_probe::shared::SharedHandle;
use crate::wt_probe::weapons::WeaponDetector;

const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

pub struct Outputs {
    pub session: Option<BufWriter<File>>,
    pub events: Option<BufWriter<File>>,
}

pub struct WriterResult {
    pub schema: SchemaTracker,
    pub weapons: WeaponDetector,
}

pub fn run(
    rx: Receiver<PollOutcome>,
    mut outputs: Outputs,
    shared: SharedHandle,
    shutdown: Arc<AtomicBool>,
) -> WriterResult {
    let mut schema = SchemaTracker::new();
    let mut weapons = WeaponDetector::new();
    let mut last_flush = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(outcome) => {
                handle_outcome(outcome, &mut outputs, &mut schema, &mut weapons, &shared)
            }
            Err(RecvTimeoutError::Timeout) => {
                if shutdown.load(Ordering::Relaxed) {
                    // Даём последним уже отправленным сообщениям дойти, но не
                    // ждём вечно — poller-потоки к этому моменту уже
                    // присоединены вызывающим кодом.
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if last_flush.elapsed() >= FLUSH_INTERVAL {
            flush(&mut outputs);
            last_flush = Instant::now();
        }
    }
    flush(&mut outputs);
    schema.finalize();
    WriterResult { schema, weapons }
}

fn flush(outputs: &mut Outputs) {
    if let Some(w) = outputs.session.as_mut() {
        let _ = w.flush();
    }
    if let Some(w) = outputs.events.as_mut() {
        let _ = w.flush();
    }
}

/// `pub(crate)`, потому что replay.rs прогоняет записанный лог через тот же
/// путь обработки (schema/weapons/shared), что и живой опрос — иначе логика
/// разбора двух режимов расходилась бы незаметно для отчётов.
pub(crate) fn handle_outcome(
    outcome: PollOutcome,
    outputs: &mut Outputs,
    schema: &mut SchemaTracker,
    weapons: &mut WeaponDetector,
    shared: &SharedHandle,
) {
    match outcome {
        PollOutcome::Ok(record) => {
            let line = format_line(record.t, record.endpoint, &record.parsed);
            let mut written_bytes = 0u64;
            if let Some(w) = outputs.session.as_mut() {
                written_bytes = write_line(w, &line);
            }
            if record.endpoint == Endpoint::HudMsg
                && let Some(w) = outputs.events.as_mut()
            {
                write_line(w, &line);
            }

            let pairs = schema::flatten(&record.parsed);
            schema.process_pairs(record.endpoint, &pairs, record.t);
            weapons.process(record.endpoint, &pairs, record.t);

            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.bump_success(record.endpoint);
            s.last_error = None;
            if written_bytes > 0 {
                s.lines_written += 1;
                s.bytes_written += written_bytes;
            }
            if matches!(record.endpoint, Endpoint::State | Endpoint::Indicators) {
                if let Some(v) = find_vehicle_type(&pairs) {
                    s.vehicle_type = Some(v);
                }
                s.last_flat.insert(
                    record.endpoint,
                    pairs
                        .into_iter()
                        .collect::<std::collections::BTreeMap<_, _>>(),
                );
            }
            if record.endpoint == Endpoint::State {
                s.conn = classify_conn(&record.parsed);
            }
            if record.endpoint == Endpoint::HudMsg {
                for line in hudmsg_event_summaries(&record.parsed) {
                    s.push_event(line);
                }
                if !s.own_callsign.is_empty() {
                    let callsign = s.own_callsign.clone();
                    for msg in hudmsg_damage_msgs(&record.parsed) {
                        if let Some(kind) = hits::classify_own_hit(msg, &callsign) {
                            s.push_own_hit(format!("{kind}: {msg}"));
                        }
                    }
                }
            }
        }
        PollOutcome::Err {
            endpoint, message, ..
        } => {
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.last_error = Some(format!("{endpoint}: {message}"));
            if endpoint == Endpoint::State {
                s.conn = ConnStatus::Disconnected;
            }
        }
    }
}

fn write_line(w: &mut BufWriter<File>, line: &str) -> u64 {
    match w.write_all(line.as_bytes()) {
        Ok(()) => line.len() as u64,
        Err(_) => 0,
    }
}

/// Собирает строку session_*.jsonl / events_*.jsonl. Тело сериализуется
/// компактно (`to_string`, не `to_string_pretty`) специально: War Thunder
/// сама отдаёт /state и /hudmsg pretty-printed, с настоящими переводами
/// строк внутри JSON — вставь мы такой текст как есть, одна запись перестала
/// бы умещаться в одну строку и сломала бы сам формат JSON Lines (см.
/// комментарий у `WtClient::handle` в http.rs). Компактная пересериализация
/// не теряет ни одного значения — только незначащие пробелы и порядок ключей.
fn format_line(t: f64, endpoint: Endpoint, body: &Value) -> String {
    let compact_body = serde_json::to_string(body).unwrap_or_else(|_| "null".to_string());
    format!(
        "{{\"t\":{t:.6},\"endpoint\":\"{}\",\"body\":{compact_body}}}\n",
        endpoint.as_str()
    )
}

fn classify_conn(state_body: &Value) -> ConnStatus {
    match state_body.get("valid") {
        Some(Value::Bool(true)) => ConnStatus::InBattle,
        _ => ConnStatus::Menu,
    }
}

/// Ищет поле, последний сегмент пути которого — "type" (без учёта
/// регистра), со строковым значением. В официальном /indicators такое поле
/// действительно есть у боевой техники — это единственное имя, на которое
/// мы полагаемся напрямую (а не через эвристику interesting.rs), потому что
/// без него не показать "тип техники" вообще никак.
fn find_vehicle_type(pairs: &[FlatPair]) -> Option<String> {
    pairs.iter().find_map(|(k, v)| {
        let last_segment = k.rsplit('.').next().unwrap_or(k);
        if last_segment.eq_ignore_ascii_case("type") {
            v.as_str().filter(|s| !s.is_empty()).map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// Короткие человекочитаемые строки для панели "последние события" на
/// дашборде — не идут в файл, только для живого отображения.
fn hudmsg_event_summaries(body: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for array_key in ["events", "damage"] {
        if let Some(arr) = body.get(array_key).and_then(Value::as_array) {
            for e in arr {
                let msg = e
                    .get("msg")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| e.to_string());
                out.push(format!("[{array_key}] {msg}"));
            }
        }
    }
    out
}

/// Тексты `msg` из /hudmsg.damage[] — только этот массив несёт боевые
/// сообщения (в отличие от `events[]`, который во всех живых сессиях пока
/// оставался пустым); используется и для панели "последние события", и для
/// hits::classify_own_hit.
fn hudmsg_damage_msgs(body: &Value) -> impl Iterator<Item = &str> {
    body.get("damage")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("msg").and_then(Value::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_conn_valid_true_is_in_battle() {
        assert_eq!(classify_conn(&json!({"valid": true})), ConnStatus::InBattle);
    }

    #[test]
    fn classify_conn_valid_false_is_menu() {
        assert_eq!(classify_conn(&json!({"valid": false})), ConnStatus::Menu);
    }

    #[test]
    fn classify_conn_missing_field_is_menu() {
        assert_eq!(classify_conn(&json!({})), ConnStatus::Menu);
    }

    #[test]
    fn finds_vehicle_type_case_insensitive() {
        let pairs = vec![("Type".to_string(), json!("fw190d9"))];
        assert_eq!(find_vehicle_type(&pairs).as_deref(), Some("fw190d9"));
    }

    #[test]
    fn format_line_is_single_line_even_for_pretty_printed_source() {
        // War Thunder действительно отдаёт /state и /hudmsg pretty-printed —
        // здесь имитируем это через Value, полученный из многострочного
        // JSON-текста, и проверяем, что результирующая строка не содержит
        // переводов строк внутри тела (иначе session_*.jsonl перестал бы
        // быть валидным JSON Lines).
        let body: Value = serde_json::from_str("{\n  \"z\": 1,\n  \"a\": 2\n}").unwrap();
        let line = format_line(1.5, Endpoint::State, &body);
        assert_eq!(
            line.matches('\n').count(),
            1,
            "ровно один перевод строки — в конце записи"
        );
        assert!(line.ends_with('\n'));
        let expected_body = serde_json::to_string(&body).unwrap();
        assert_eq!(
            line,
            format!("{{\"t\":1.500000,\"endpoint\":\"state\",\"body\":{expected_body}}}\n")
        );
    }

    #[test]
    fn hudmsg_summaries_extracts_events_and_damage() {
        let body = json!({
            "events": [{"id": 1, "msg": "hit enemy"}],
            "damage": [{"id": 1, "msg": "engine on fire"}]
        });
        let out = hudmsg_event_summaries(&body);
        assert_eq!(
            out,
            vec![
                "[events] hit enemy".to_string(),
                "[damage] engine on fire".to_string()
            ]
        );
    }

    #[test]
    fn hudmsg_damage_msgs_extracts_only_damage_array() {
        let body = json!({
            "events": [{"id": 1, "msg": "ignored"}],
            "damage": [{"id": 1, "msg": "shot down enemy"}, {"id": 2, "msg": "set afire"}]
        });
        let out: Vec<&str> = hudmsg_damage_msgs(&body).collect();
        assert_eq!(out, vec!["shot down enemy", "set afire"]);
    }

    #[test]
    fn handle_outcome_populates_own_hits_when_callsign_matches() {
        use crate::wt_probe::model::RawRecord;
        use crate::wt_probe::shared::Shared;
        use std::sync::{Arc, Mutex};

        let shared: SharedHandle = Arc::new(Mutex::new(Shared::new()));
        shared.lock().unwrap().own_callsign = "Lazarev_IJN".to_string();

        let mut outputs = Outputs {
            session: None,
            events: None,
        };
        let mut schema = SchemaTracker::new();
        let mut weapons = WeaponDetector::new();
        let record = RawRecord {
            t: 1.0,
            endpoint: Endpoint::HudMsg,
            parsed: json!({
                "damage": [
                    {"id": 1, "msg": "Krill303 (Fw 190 A) сбил Lazarev_IJN (Bf 109 E)"},
                    {"id": 2, "msg": "Krill303 (Fw 190 A) сбил someone_else (Bf 109 F)"}
                ],
                "events": []
            }),
        };
        handle_outcome(
            PollOutcome::Ok(record),
            &mut outputs,
            &mut schema,
            &mut weapons,
            &shared,
        );

        let s = shared.lock().unwrap();
        assert_eq!(s.own_hits.len(), 1);
        assert!(s.own_hits[0].starts_with("СБИТ:"));
    }
}
