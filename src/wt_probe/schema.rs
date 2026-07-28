//! Автообнаружение схемы: какие ключи вообще отдаёт игра, с каким типом,
//! диапазоном значений и как часто они меняются. Никакого заранее известного
//! списка полей — только то, что реально встретилось в потоке.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde_json::Value;

use crate::wt_probe::model::{Endpoint, FlatPair};

/// Насколько глубоко разворачивать вложенные объекты/массивы в плоские
/// пути вида "a.b.c". War Thunder отдаёт в основном плоские объекты, но
/// глубина ограничена на случай неожиданной вложенности — без предела
/// разворачивание могло бы никогда не кончиться на рекурсивных структурах
/// (мы этого не ожидаем от JSON, но не полагаемся на это).
const MAX_DEPTH: usize = 4;

/// Разворачивает произвольный JSON в список плоских пар (путь, значение).
/// Объекты разворачиваются по ключам ("engine.rpm"), массивы — по первому
/// элементу с суффиксом "[]" плюс отдельная синтетическая запись "[].len"
/// с длиной массива (сама длина — тоже полезный сигнал схемы, а массивы
/// вроде /hudmsg или /map_obj.json иначе вообще не попали бы в отчёт).
pub fn flatten(value: &Value) -> Vec<FlatPair> {
    let mut out = Vec::new();
    flatten_into(value, "", 0, &mut out);
    out
}

fn flatten_into(value: &Value, prefix: &str, depth: usize, out: &mut Vec<FlatPair>) {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                out.push((key_or_root(prefix), value.clone()));
                return;
            }
            for (k, v) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                if depth < MAX_DEPTH {
                    flatten_into(v, &key, depth + 1, out);
                } else {
                    out.push((key, v.clone()));
                }
            }
        }
        Value::Array(arr) => {
            out.push((
                format!("{}[].len", key_or_root(prefix)),
                Value::from(arr.len()),
            ));
            if let Some(first) = arr.first() {
                let elem_key = format!("{}[]", key_or_root(prefix));
                if depth < MAX_DEPTH {
                    flatten_into(first, &elem_key, depth + 1, out);
                } else {
                    out.push((elem_key, first.clone()));
                }
            }
        }
        leaf => out.push((key_or_root(prefix), leaf.clone())),
    }
}

fn key_or_root(prefix: &str) -> String {
    if prefix.is_empty() {
        "$root".to_string()
    } else {
        prefix.to_string()
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Сколько уникальных строковых представлений значения хранить, прежде чем
/// признать поле "высококардинальным" и перестать копить множество (иначе
/// поле вроде таймстампа накопило бы миллионы записей за сессию).
const DISTINCT_CAP: usize = 32;

#[derive(Debug, Default)]
pub struct KeyStats {
    pub types_seen: BTreeSet<&'static str>,
    pub numeric_min: Option<f64>,
    pub numeric_max: Option<f64>,
    pub numeric_sum: f64,
    pub numeric_count: u64,
    pub sample_count: u64,
    pub change_count: u64,
    pub last_value: Option<Value>,
    pub distinct_values: BTreeSet<String>,
    pub distinct_capped: bool,
    pub first_t: f64,
    pub last_t: f64,
    /// true, если ключ впервые появился не на первом тике сессии — признак
    /// смены техники/режима в середине записи.
    pub appeared_mid_session: bool,
    /// true, если ключ хотя бы раз пропадал после появления и потом
    /// возвращался (или пропал и не вернулся до конца сессии).
    pub episodic: bool,
    pub currently_present: bool,
}

impl KeyStats {
    fn observe(&mut self, v: &Value, t: f64) {
        self.sample_count += 1;
        self.last_t = t;
        self.types_seen.insert(type_name(v));
        if let Some(n) = v.as_f64() {
            self.numeric_min = Some(self.numeric_min.map_or(n, |m| m.min(n)));
            self.numeric_max = Some(self.numeric_max.map_or(n, |m| m.max(n)));
            self.numeric_sum += n;
            self.numeric_count += 1;
        }
        if self.distinct_values.len() < DISTINCT_CAP {
            self.distinct_values.insert(v.to_string());
        } else {
            self.distinct_capped = true;
        }
        let changed = self.last_value.as_ref().is_none_or(|last| last != v);
        if changed && self.sample_count > 1 {
            self.change_count += 1;
        }
        self.last_value = Some(v.clone());
    }

    pub fn mean(&self) -> Option<f64> {
        if self.numeric_count == 0 {
            None
        } else {
            Some(self.numeric_sum / self.numeric_count as f64)
        }
    }

    pub fn classification(&self) -> &'static str {
        if self.episodic || self.appeared_mid_session {
            "эпизодический"
        } else if self.change_count > 0 || self.distinct_capped {
            "динамический"
        } else {
            "статический"
        }
    }
}

#[derive(Debug, Default)]
pub struct EndpointSchema {
    pub keys: BTreeMap<String, KeyStats>,
    present_last_tick: BTreeSet<String>,
    pub tick_count: u64,
    session_start_t: Option<f64>,
}

impl EndpointSchema {
    fn process(&mut self, pairs: &[FlatPair], t: f64) {
        let start_t = *self.session_start_t.get_or_insert(t);
        self.tick_count += 1;
        let mut current: BTreeSet<String> = BTreeSet::new();
        for (k, v) in pairs {
            current.insert(k.clone());
            let is_new = !self.keys.contains_key(k);
            let stats = self.keys.entry(k.clone()).or_default();
            if is_new {
                stats.first_t = t;
                // Небольшой допуск: первые несколько тиков сессии всё ещё
                // "старт", а не смена техники в середине боя.
                if t - start_t > 1.0 {
                    stats.appeared_mid_session = true;
                }
            }
            if !stats.currently_present && stats.sample_count > 0 {
                // Ключ был, пропал, теперь снова появился.
                stats.episodic = true;
            }
            stats.currently_present = true;
            stats.observe(v, t);
        }
        for missing in self.present_last_tick.difference(&current) {
            if let Some(stats) = self.keys.get_mut(missing) {
                stats.currently_present = false;
                stats.episodic = true;
            }
        }
        self.present_last_tick = current;
    }

    pub fn finalize(&mut self) {
        for missing in &self.present_last_tick {
            let _ = missing; // ключи, присутствовавшие в последнем тике, не эпизодические
        }
        for (k, stats) in self.keys.iter_mut() {
            if !self.present_last_tick.contains(k) && stats.currently_present {
                stats.episodic = true;
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct SchemaTracker {
    pub endpoints: BTreeMap<Endpoint, EndpointSchema>,
}

impl SchemaTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Разворачивает `value` и учитывает его в схеме эндпоинта. writer.rs и
    /// replay.rs зовут `process_pairs` напрямую, потому что им и так нужны
    /// развёрнутые пары для weapons::WeaponDetector — не разворачивать один
    /// и тот же JSON дважды.
    #[cfg(test)]
    pub fn process(&mut self, endpoint: Endpoint, value: &Value, t: f64) {
        let pairs = flatten(value);
        self.process_pairs(endpoint, &pairs, t);
    }

    pub fn process_pairs(&mut self, endpoint: Endpoint, pairs: &[FlatPair], t: f64) {
        self.endpoints
            .entry(endpoint)
            .or_default()
            .process(pairs, t);
    }

    pub fn finalize(&mut self) {
        for ep in self.endpoints.values_mut() {
            ep.finalize();
        }
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# Отчёт по схеме телеметрии War Thunder\n");
        let _ = writeln!(
            out,
            "Автоматически обнаруженные ключи по каждому эндпоинту. \
             Колонка «Класс»: динамический — значение реально менялось за сессию; \
             статический — ключ был, но ни разу не изменился; \
             эпизодический — ключ появлялся/пропадал в середине сессии \
             (признак смены техники или режима).\n"
        );
        for (endpoint, schema) in &self.endpoints {
            let _ = writeln!(out, "## /{endpoint} (тиков: {})\n", schema.tick_count);
            if schema.keys.is_empty() {
                let _ = writeln!(out, "_Ни одного ответа не получено._\n");
                continue;
            }
            let _ = writeln!(
                out,
                "| Ключ | Тип(ы) | Min | Max | Mean | Изменений | Класс |"
            );
            let _ = writeln!(out, "|---|---|---|---|---|---|---|");
            for (key, stats) in &schema.keys {
                let types = stats
                    .types_seen
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    .join(",");
                let min = stats
                    .numeric_min
                    .map(|v| format!("{v:.3}"))
                    .unwrap_or_else(|| "-".into());
                let max = stats
                    .numeric_max
                    .map(|v| format!("{v:.3}"))
                    .unwrap_or_else(|| "-".into());
                let mean = stats
                    .mean()
                    .map(|v| format!("{v:.3}"))
                    .unwrap_or_else(|| "-".into());
                let _ = writeln!(
                    out,
                    "| `{key}` | {types} | {min} | {max} | {mean} | {} | {} |",
                    stats.change_count,
                    stats.classification()
                );
            }
            let _ = writeln!(out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flatten_flat_object() {
        let v = json!({"a": 1, "b": "x", "c": true});
        let mut pairs = flatten(&v);
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0].0, "a");
    }

    #[test]
    fn flatten_nested_object() {
        let v = json!({"engine": {"rpm": 2400}});
        let pairs = flatten(&v);
        assert_eq!(pairs, vec![("engine.rpm".to_string(), json!(2400))]);
    }

    #[test]
    fn flatten_array_records_len_and_first_elem() {
        let v = json!({"events": [{"id": 1, "msg": "hit"}, {"id": 2}]});
        let pairs = flatten(&v);
        assert!(
            pairs
                .iter()
                .any(|(k, val)| k == "events[].len" && *val == json!(2))
        );
        assert!(pairs.iter().any(|(k, _)| k == "events[].id"));
    }

    #[test]
    fn schema_tracks_min_max_mean_and_changes() {
        let mut tracker = SchemaTracker::new();
        tracker.process(Endpoint::State, &json!({"rpm": 1000.0}), 0.0);
        tracker.process(Endpoint::State, &json!({"rpm": 2000.0}), 0.05);
        tracker.process(Endpoint::State, &json!({"rpm": 1500.0}), 0.10);
        let stats = &tracker.endpoints[&Endpoint::State].keys["rpm"];
        assert_eq!(stats.numeric_min, Some(1000.0));
        assert_eq!(stats.numeric_max, Some(2000.0));
        assert_eq!(stats.change_count, 2);
        assert_eq!(stats.classification(), "динамический");
    }

    #[test]
    fn schema_flags_constant_key_as_static() {
        let mut tracker = SchemaTracker::new();
        tracker.process(Endpoint::State, &json!({"const": 1}), 0.0);
        tracker.process(Endpoint::State, &json!({"const": 1}), 0.05);
        let stats = &tracker.endpoints[&Endpoint::State].keys["const"];
        assert_eq!(stats.classification(), "статический");
    }

    #[test]
    fn schema_flags_key_appearing_mid_session() {
        let mut tracker = SchemaTracker::new();
        tracker.process(Endpoint::State, &json!({"a": 1}), 0.0);
        tracker.process(Endpoint::State, &json!({"a": 1, "b": 2}), 5.0);
        tracker.finalize();
        let stats = &tracker.endpoints[&Endpoint::State].keys["b"];
        assert!(stats.appeared_mid_session);
        assert_eq!(stats.classification(), "эпизодический");
    }

    #[test]
    fn schema_flags_key_disappearing_before_end() {
        let mut tracker = SchemaTracker::new();
        tracker.process(Endpoint::State, &json!({"a": 1, "b": 2}), 0.0);
        tracker.process(Endpoint::State, &json!({"a": 1}), 0.05);
        tracker.finalize();
        let stats = &tracker.endpoints[&Endpoint::State].keys["b"];
        assert!(stats.episodic);
    }
}
