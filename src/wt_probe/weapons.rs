//! Обнаружение "счётчиков боеприпасов" по поведению, а не по имени: любое
//! целочисленное поле /state или /indicators, которое за сессию в основном
//! убывает, — кандидат. Для каждого кандидата копим статистику темпа
//! убывания, чтобы понять, можно ли по нему восстановить темп стрельбы.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;

use crate::wt_probe::model::{Endpoint, FlatPair};

/// Одно зафиксированное убывание значения поля.
#[derive(Debug, Clone, Copy)]
pub struct DecreaseEvent {
    pub t: f64,
    pub delta: f64,
    pub dt: f64,
    /// Мгновенный темп, единиц/сек (delta/dt).
    pub rate: f64,
}

#[derive(Debug, Default)]
struct Track {
    last_value: Option<f64>,
    last_t: Option<f64>,
    decreases: Vec<DecreaseEvent>,
    increase_count: u32,
    decrease_count: u32,
    non_integer_seen: bool,
}

impl Track {
    fn observe(&mut self, n: f64, t: f64) {
        if n.fract().abs() > 1e-6 {
            self.non_integer_seen = true;
        }
        if let (Some(last), Some(last_t)) = (self.last_value, self.last_t) {
            let delta = n - last;
            let dt = (t - last_t).max(1e-6);
            if delta < -0.5 {
                self.decrease_count += 1;
                self.decreases.push(DecreaseEvent {
                    t,
                    delta: -delta,
                    dt,
                    rate: -delta / dt,
                });
            } else if delta > 0.5 {
                self.increase_count += 1;
            }
        }
        self.last_value = Some(n);
        self.last_t = Some(t);
    }

    /// Поле похоже на счётчик боеприпасов: целочисленное и в основном
    /// убывающее (редкие скачки вверх допустимы — респавн/пополнение боекомплекта).
    fn is_ammo_like(&self) -> bool {
        !self.non_integer_seen
            && self.decrease_count > 0
            && (self.increase_count == 0
                || self.decrease_count as f64 >= 2.0 * self.increase_count as f64)
    }
}

/// Максимальный разрыв между последовательными убываниями, ещё считающийся
/// той же очередью, а не отдельной. Ниже typical человеческой "паузы между
/// нажатиями гашетки" (условно, доли секунды).
const BURST_GAP_S: f64 = 0.5;

#[derive(Debug, Clone, Default)]
pub struct BurstStat {
    pub start_t: f64,
    pub end_t: f64,
    pub rounds: f64,
}

impl BurstStat {
    pub fn duration(&self) -> f64 {
        (self.end_t - self.start_t).max(0.0)
    }
}

#[derive(Debug, Clone)]
pub struct WeaponReport {
    pub key: String,
    pub decrease_count: u32,
    pub increase_count: u32,
    pub median_rate: f64,
    pub max_rate: f64,
    /// Гистограмма дельт за тик (округлённых до целого) -> сколько раз встретилась.
    pub delta_histogram: BTreeMap<i64, u32>,
    pub bursts: Vec<BurstStat>,
    pub pauses: Vec<f64>,
    /// Тики, где за один опрос 20 Гц ушло больше одного снаряда — признак,
    /// что дискретный импульс на выстрел здесь принципиально невозможен.
    pub multi_round_ticks: u32,
}

impl WeaponReport {
    fn median_burst_duration(&self) -> f64 {
        median(&mut self.bursts.iter().map(|b| b.duration()).collect::<Vec<_>>())
    }

    fn median_pause(&self) -> f64 {
        median(&mut self.pauses.clone())
    }
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

#[derive(Debug, Default)]
pub struct WeaponDetector {
    tracks: BTreeMap<String, Track>,
}

impl WeaponDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Скармливаем только /state и /indicators — это единственные эндпоинты
    /// с числовой телеметрией техники; /hudmsg/map_obj/mission ей не нужны.
    pub fn process(&mut self, endpoint: Endpoint, pairs: &[FlatPair], t: f64) {
        if !matches!(endpoint, Endpoint::State | Endpoint::Indicators) {
            return;
        }
        for (key, value) in pairs {
            let Value::Number(_) = value else { continue };
            let Some(n) = value.as_f64() else { continue };
            let track_key = format!("{endpoint}:{key}");
            let track = self.tracks.entry(track_key).or_default();
            track.observe(n, t);
        }
    }

    /// Кандидаты, отобранные по поведению (см. `Track::is_ammo_like`), с
    /// посчитанной статистикой темпа для каждого.
    pub fn reports(&self) -> Vec<WeaponReport> {
        let mut out = Vec::new();
        for (key, track) in &self.tracks {
            if !track.is_ammo_like() {
                continue;
            }
            let mut rates: Vec<f64> = track.decreases.iter().map(|d| d.rate).collect();
            let median_rate = median(&mut rates);
            let max_rate = track
                .decreases
                .iter()
                .map(|d| d.rate)
                .fold(0.0_f64, f64::max);

            let mut delta_histogram: BTreeMap<i64, u32> = BTreeMap::new();
            let mut multi_round_ticks = 0u32;
            for d in &track.decreases {
                *delta_histogram.entry(d.delta.round() as i64).or_insert(0) += 1;
                // dt близок к периоду одного тика 20 Гц (<=75 мс) и за него
                // ушло больше одного снаряда.
                if d.delta >= 2.0 && d.dt <= 0.075 {
                    multi_round_ticks += 1;
                }
            }

            let mut bursts: Vec<BurstStat> = Vec::new();
            let mut pauses: Vec<f64> = Vec::new();
            for d in &track.decreases {
                match bursts.last_mut() {
                    Some(last) if d.t - last.end_t <= BURST_GAP_S => {
                        last.end_t = d.t;
                        last.rounds += d.delta;
                    }
                    Some(last) => {
                        pauses.push(d.t - last.end_t);
                        bursts.push(BurstStat {
                            start_t: d.t,
                            end_t: d.t,
                            rounds: d.delta,
                        });
                    }
                    None => bursts.push(BurstStat {
                        start_t: d.t,
                        end_t: d.t,
                        rounds: d.delta,
                    }),
                }
            }

            out.push(WeaponReport {
                key: key.clone(),
                decrease_count: track.decrease_count,
                increase_count: track.increase_count,
                median_rate,
                max_rate,
                delta_histogram,
                bursts,
                pauses,
                multi_round_ticks,
            });
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    pub fn to_markdown(&self) -> String {
        let reports = self.reports();
        let mut out = String::new();
        let _ = writeln!(out, "# Отчёт по обнаруженным счётчикам боеприпасов\n");
        let _ = writeln!(
            out,
            "Кандидаты найдены по поведению (в основном убывающие целочисленные \
             поля /state и /indicators), а не по имени ключа. \
             Стрельба вычисляется по убыванию счётчика — это единственный способ \
             задетектировать выстрел через официальный HTTP API.\n"
        );
        if reports.is_empty() {
            let _ = writeln!(
                out,
                "_Ни одного поля, похожего на счётчик боеприпасов, за сессию не найдено._\n"
            );
            return out;
        }
        for r in &reports {
            let _ = writeln!(out, "## `{}`\n", r.key);
            let _ = writeln!(out, "- Убываний: {}", r.decrease_count);
            let _ = writeln!(
                out,
                "- Скачков вверх (респавн/пополнение): {}",
                r.increase_count
            );
            let _ = writeln!(out, "- Медианный темп: {:.2} ед/с", r.median_rate);
            let _ = writeln!(out, "- Максимальный темп: {:.2} ед/с", r.max_rate);
            let _ = writeln!(out, "- Очередей (burst): {}", r.bursts.len());
            let _ = writeln!(
                out,
                "- Медианная длительность очереди: {:.3} с",
                r.median_burst_duration()
            );
            let _ = writeln!(
                out,
                "- Медианная пауза между очередями: {:.3} с",
                r.median_pause()
            );
            if r.multi_round_ticks > 0 {
                let _ = writeln!(
                    out,
                    "- **{} тик(ов), где за один опрос 20 Гц ушло больше одного снаряда** \
                     — для этого оружия дискретный импульс на выстрел невозможен, нужен непрерывный режим.",
                    r.multi_round_ticks
                );
            } else {
                let _ = writeln!(
                    out,
                    "- Многозарядных тиков не обнаружено — похоже на оружие с низким темпом, \
                     дискретные импульсы должны работать."
                );
            }
            let _ = write!(out, "- Гистограмма дельт за тик:");
            for (delta, count) in &r.delta_histogram {
                let _ = write!(out, " {delta}×{count}");
            }
            let _ = writeln!(out, "\n");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pairs(v: Value) -> Vec<FlatPair> {
        crate::wt_probe::schema::flatten(&v)
    }

    #[test]
    fn detects_decreasing_integer_field_as_ammo() {
        let mut det = WeaponDetector::new();
        det.process(Endpoint::State, &pairs(json!({"mgun1_ammo": 200})), 0.0);
        det.process(Endpoint::State, &pairs(json!({"mgun1_ammo": 199})), 0.05);
        det.process(Endpoint::State, &pairs(json!({"mgun1_ammo": 198})), 0.10);
        let reports = det.reports();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].key, "state:mgun1_ammo");
        assert_eq!(reports[0].decrease_count, 2);
    }

    #[test]
    fn ignores_monotonically_increasing_field() {
        let mut det = WeaponDetector::new();
        det.process(Endpoint::State, &pairs(json!({"mission_time": 0})), 0.0);
        det.process(Endpoint::State, &pairs(json!({"mission_time": 1})), 1.0);
        det.process(Endpoint::State, &pairs(json!({"mission_time": 2})), 2.0);
        assert!(det.reports().is_empty());
    }

    #[test]
    fn ignores_non_integer_field() {
        let mut det = WeaponDetector::new();
        det.process(Endpoint::State, &pairs(json!({"altitude": 100.25})), 0.0);
        det.process(Endpoint::State, &pairs(json!({"altitude": 90.5})), 0.05);
        assert!(det.reports().is_empty());
    }

    #[test]
    fn tolerates_reload_style_reset() {
        let mut det = WeaponDetector::new();
        det.process(Endpoint::State, &pairs(json!({"ammo": 10})), 0.0);
        det.process(Endpoint::State, &pairs(json!({"ammo": 5})), 0.05);
        det.process(Endpoint::State, &pairs(json!({"ammo": 200})), 5.0); // respawn
        det.process(Endpoint::State, &pairs(json!({"ammo": 199})), 5.05);
        let reports = det.reports();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].increase_count, 1);
        assert_eq!(reports[0].decrease_count, 2);
    }

    #[test]
    fn flags_multi_round_per_tick() {
        let mut det = WeaponDetector::new();
        det.process(Endpoint::State, &pairs(json!({"cannon": 100})), 0.0);
        // 3 снаряда за один тик 20 Гц (dt=0.05с)
        det.process(Endpoint::State, &pairs(json!({"cannon": 97})), 0.05);
        let reports = det.reports();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].multi_round_ticks, 1);
    }
}
