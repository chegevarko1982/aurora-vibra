//! Таблица порогов Vne (never-exceed speed) для War Thunder и функция
//! интенсивности эффекта overspeed. Аналог `aero_profiles.rs` (см. план
//! `greedy-spinning-panda.md`), но в отличие от `StallProfile` матчинг
//! точный, а не по подстроке: `telemetry_id` в таблице — это ровно та же
//! строка, что WT отдаёт в `/indicators.type` (`WtVars::vehicle_type`), без
//! нормализации регистра/дефисов.
//!
//! Таблица (`data/overspeed_vne.csv`, 1299 бортов) выгружена пользователем
//! из справочника wiki.warthunder.ru — см. reference-память
//! `wt_overspeed_vne_table_source`. Борта, отсутствующие в таблице, получают
//! `None` от `vne_kmh_for` — эффект молча выключен, гадать по умолчанию (как
//! временный fallback у stall на Bf 109 F-4) здесь не нужно: таблица
//! покрывает практически весь ростер игры.

use std::collections::HashMap;
use std::sync::OnceLock;

const OVERSPEED_VNE_CSV: &str = include_str!("data/overspeed_vne.csv");

/// Ширина окна нарастания перед порогом (км/ч по приборной скорости) — по
/// требованию пользователя: вибрация начинается за 50 км/ч до Vne и растёт
/// линейно к полной мощности ровно в момент достижения порога.
const OVERSPEED_WARNING_WINDOW_KMH: f64 = 50.0;

fn table() -> &'static HashMap<&'static str, f32> {
    static TABLE: OnceLock<HashMap<&'static str, f32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut map = HashMap::new();
        for line in OVERSPEED_VNE_CSV.lines().skip(1) {
            let Some((id, vne)) = line.split_once(',') else {
                continue;
            };
            if let Ok(vne) = vne.trim().parse::<f32>() {
                map.insert(id.trim(), vne);
            }
        }
        map
    })
}

/// Точный поиск порога Vne (км/ч) по `WtVars::vehicle_type`. Пустая строка
/// или борт вне таблицы — `None`.
pub fn vne_kmh_for(vehicle_type: &str) -> Option<f32> {
    if vehicle_type.is_empty() {
        return None;
    }
    table().get(vehicle_type).copied()
}

/// Интенсивность overspeed на сырой шкале `0..=ceiling`: тишина ниже
/// `limit_kmh - window_kmh`, линейный рост к `ceiling` внутри окна, плато
/// на `ceiling` на/за порогом (без дальнейшего роста — в отличие от stall
/// v2, пользователь явно попросил держать на полной мощности, а не
/// продолжать наращивать за красной чертой). Общий для Vne-эффекта (окно
/// 10 км/ч) и Gear overspeed (окно 20 км/ч) — ширина окна разная, форма
/// нарастания одна и та же.
fn intensity_with_window(ias_kmh: f64, limit_kmh: f64, window_kmh: f64, ceiling: f64) -> f64 {
    let onset = limit_kmh - window_kmh;
    let frac = ((ias_kmh - onset) / window_kmh).clamp(0.0, 1.0);
    frac * ceiling
}

pub fn intensity(ias_kmh: f64, vne_kmh: f64, ceiling: f64) -> f64 {
    intensity_with_window(ias_kmh, vne_kmh, OVERSPEED_WARNING_WINDOW_KMH, ceiling)
}

// --- Разрушение закрылков/шасси (Vfe/Vlo) ---
//
// Отдельная таблица `data/flap_gear_break.csv` (1299 бортов, тот же
// telemetry_id), собрана скрейпом wiki.warthunder.ru/unit/<id> — блок
// "Разрушение закрылков" встречается на вики в 3 разных вёрстках:
//   - "П / В / Б" — 3 значения (посадочные/взлётные/боевые), основной случай;
//   - одно число без слэшей — борт имеет только одну позицию закрылков
//     (например avenger_mk1: "285 км/ч"); во всех трёх колонках CSV
//     дублируется одно и то же значение, поэтому effective_limit_kmh не
//     нуждается в отдельной ветке — какой бы бакет ни выбрался, число одно;
//   - "П / В" с явной пометкой позиций (например gladiator_j8a, Ju-87) — нет
//     боевого положения вообще, третья колонка (Б) остаётся пустой.
// "Разрушение шасси" — отдельно, 1 значение, км/ч, есть не у всех бортов
// (например неубираемое шасси). 1217/1299 бортов имеют данные по закрылкам
// (хоть в каком-то виде), 1199/1299 — по шасси; отсутствие записи = не
// понижаем порог (тот же принцип "нет данных — не гадаем", что и у Vne).
//
// Шасси намеренно НЕ подмешивается в effective_limit_kmh — у него свой
// отдельный эффект Gear overspeed (окно 20 км/ч вместо 10, свой тумблер,
// см. gear_kmh_for/GEAR_OVERSPEED_WARNING_WINDOW_KMH ниже), чтобы не
// получить двойную/накладывающуюся вибрацию на одном и том же превышении
// при выпущенном шасси через два разных эффекта с разными окнами.
//
// `WtVars::flaps_pct` — степень выпуска 0..100%, без явного признака,
// какая из трёх позиций (П/В/Б) выбрана. Калибровано живой сессией
// (борт la-5fn, 2026-08-03, `wt_probe_sessions/session_20260803_185922.jsonl`):
// на retract-фазе flaps_pct останавливался на ~20% (боевые/Б) и ~33%
// (взлётные/В) перед тем как дойти до 0; полный выпуск 100% соответствует
// посадочным/П. Границы бакетов — середины между калиброванными точками
// (см. FLAP_*_BOUNDARY_PCT ниже). Другие борта не проверялись — если для
// конкретного самолёта границы окажутся не такими, править константы, а
// не саму трёхбакетную схему.

const FLAP_GEAR_BREAK_CSV: &str = include_str!("data/flap_gear_break.csv");

/// Клиренс вокруг 0% для «поверхность хоть немного выпущена» — тот же
/// принцип, что GEAR_LOCKED_THRESHOLD_PCT в rumble.rs, чтобы не реагировать
/// на телеметрический шум около нуля.
const SURFACE_EXTENDED_THRESHOLD_PCT: f64 = 0.5;

/// Граница между боевыми (Б, ~20%) и взлётными (В, ~33%) — середина между
/// калиброванными точками la-5fn.
const FLAP_COMBAT_TAKEOFF_BOUNDARY_PCT: f64 = 26.5;

/// Граница между взлётными (В, ~33%) и посадочными (П, 100%).
const FLAP_TAKEOFF_LANDING_BOUNDARY_PCT: f64 = 66.5;

/// Ширина окна нарастания для Gear overspeed — единое значение с общим
/// Vne-эффектом (50 км/ч).
const GEAR_OVERSPEED_WARNING_WINDOW_KMH: f64 = 50.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct FlapGearBreak {
    landing_kmh: Option<f32>,
    takeoff_kmh: Option<f32>,
    combat_kmh: Option<f32>,
    gear_kmh: Option<f32>,
}

fn flap_gear_table() -> &'static HashMap<&'static str, FlapGearBreak> {
    static TABLE: OnceLock<HashMap<&'static str, FlapGearBreak>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut map = HashMap::new();
        for line in FLAP_GEAR_BREAK_CSV.lines().skip(1) {
            let mut parts = line.split(',');
            let (Some(id), Some(p), Some(v), Some(b), Some(g)) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            ) else {
                continue;
            };
            let entry = FlapGearBreak {
                landing_kmh: p.trim().parse::<f32>().ok(),
                takeoff_kmh: v.trim().parse::<f32>().ok(),
                combat_kmh: b.trim().parse::<f32>().ok(),
                gear_kmh: g.trim().parse::<f32>().ok(),
            };
            map.insert(id.trim(), entry);
        }
        map
    })
}

/// Итоговый действующий предел скорости (км/ч) с учётом текущего положения
/// закрылков: минимум из общего Vne и (если закрылки выпущены) порога
/// разрушения для позиции, которой соответствует текущий flaps_pct
/// (боевые/взлётные/посадочные — см. калибровку выше). `None`, только
/// если вообще нет ни одного применимого порога (борт отсутствует во всех
/// таблицах). Шасси сюда не входит — см. Gear overspeed ниже.
pub fn effective_limit_kmh(
    vehicle_type: &str,
    base_vne_kmh: Option<f32>,
    flaps_pct: f64,
) -> Option<f32> {
    let flap_limit = flap_gear_lookup(vehicle_type).and_then(|fg| {
        if flaps_pct <= SURFACE_EXTENDED_THRESHOLD_PCT {
            None
        } else if flaps_pct <= FLAP_COMBAT_TAKEOFF_BOUNDARY_PCT {
            fg.combat_kmh
        } else if flaps_pct <= FLAP_TAKEOFF_LANDING_BOUNDARY_PCT {
            fg.takeoff_kmh
        } else {
            fg.landing_kmh
        }
    });
    [base_vne_kmh, flap_limit]
        .into_iter()
        .flatten()
        .fold(None, |acc, x| Some(acc.map_or(x, |a: f32| a.min(x))))
}

fn flap_gear_lookup(vehicle_type: &str) -> Option<&'static FlapGearBreak> {
    if vehicle_type.is_empty() {
        return None;
    }
    flap_gear_table().get(vehicle_type)
}

/// Порог разрушения шасси (км/ч) по точному совпадению `vehicle_type`.
/// `None`, если борт вне таблицы или для него нет отдельной записи по
/// шасси (например, неубираемое шасси).
pub fn gear_kmh_for(vehicle_type: &str) -> Option<f32> {
    flap_gear_lookup(vehicle_type).and_then(|fg| fg.gear_kmh)
}

/// Интенсивность Gear overspeed: то же плато-нарастание, что у общего
/// Vne-эффекта, но с окном 20 км/ч вместо 10 (см.
/// `GEAR_OVERSPEED_WARNING_WINDOW_KMH`).
pub fn gear_intensity(ias_kmh: f64, gear_limit_kmh: f64, ceiling: f64) -> f64 {
    intensity_with_window(
        ias_kmh,
        gear_limit_kmh,
        GEAR_OVERSPEED_WARNING_WINDOW_KMH,
        ceiling,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_id_resolves() {
        // "bf-109f-4" — тот же борт, что используется в тестах aero_profiles.rs.
        assert_eq!(vne_kmh_for("bf-109f-4"), Some(790.0));
    }

    #[test]
    fn unknown_id_and_empty_are_none() {
        assert_eq!(vne_kmh_for(""), None);
        assert_eq!(vne_kmh_for("not-a-real-vehicle-xyz"), None);
    }

    #[test]
    fn intensity_is_silent_below_window() {
        assert_eq!(intensity(739.0, 790.0, 80.0), 0.0);
    }

    #[test]
    fn intensity_grows_monotonically_to_ceiling_at_threshold() {
        let ceiling = 80.0;
        let vne = 790.0;
        let low = intensity(750.0, vne, ceiling);
        let mid = intensity(770.0, vne, ceiling);
        let at_threshold = intensity(790.0, vne, ceiling);
        assert!(low < mid);
        assert!(mid < at_threshold);
        assert_eq!(at_threshold, ceiling);
    }

    #[test]
    fn intensity_holds_at_ceiling_past_threshold() {
        let ceiling = 80.0;
        let vne = 790.0;
        assert_eq!(intensity(850.0, vne, ceiling), ceiling);
        assert_eq!(intensity(vne, vne, ceiling), intensity(830.0, vne, ceiling));
    }

    #[test]
    fn effective_limit_ignores_flaps_when_retracted() {
        // bf-109f-4: vne=790, П/В/Б=260/409/438.
        assert_eq!(
            effective_limit_kmh("bf-109f-4", Some(790.0), 0.0),
            Some(790.0)
        );
    }

    #[test]
    fn effective_limit_uses_combat_bucket_near_20pct() {
        assert_eq!(
            effective_limit_kmh("bf-109f-4", Some(790.0), 10.0),
            Some(438.0)
        );
    }

    #[test]
    fn effective_limit_uses_takeoff_bucket_near_33pct() {
        assert_eq!(
            effective_limit_kmh("bf-109f-4", Some(790.0), 50.0),
            Some(409.0)
        );
    }

    #[test]
    fn effective_limit_uses_landing_bucket_near_100pct() {
        assert_eq!(
            effective_limit_kmh("bf-109f-4", Some(790.0), 90.0),
            Some(260.0)
        );
    }

    #[test]
    fn effective_limit_single_value_flap_applies_in_any_bucket() {
        // avenger_mk1: wiki lists one flap-break value (285 km/h), no П/В/Б
        // slashes — aircraft has only one flap deployment position, so all
        // three CSV columns are filled with the same number at merge time.
        // Whichever bucket flaps_pct falls into, the limit is the same.
        assert_eq!(
            effective_limit_kmh("avenger_mk1", Some(500.0), 10.0),
            Some(285.0)
        );
        assert_eq!(
            effective_limit_kmh("avenger_mk1", Some(500.0), 50.0),
            Some(285.0)
        );
        assert_eq!(
            effective_limit_kmh("avenger_mk1", Some(500.0), 90.0),
            Some(285.0)
        );
    }

    #[test]
    fn effective_limit_missing_combat_position_falls_back_to_base_vne() {
        // gladiator_j8a: wiki only lists "П / В" (landing/takeoff), no
        // combat position — combat_kmh is None. In the combat bucket
        // (flaps_pct <= 26.5), flap_limit is None too, so only base Vne
        // applies (no silent wrong guess at a limit that doesn't exist).
        assert_eq!(
            effective_limit_kmh("gladiator_j8a", Some(500.0), 10.0),
            Some(500.0)
        );
        assert_eq!(
            effective_limit_kmh("gladiator_j8a", Some(500.0), 50.0),
            Some(469.0)
        );
        assert_eq!(
            effective_limit_kmh("gladiator_j8a", Some(500.0), 90.0),
            Some(320.0)
        );
    }

    #[test]
    fn effective_limit_falls_back_when_no_flap_data() {
        // Aircraft absent from flap_gear_break.csv entirely: base Vne still applies.
        assert_eq!(
            effective_limit_kmh("not-a-real-vehicle-xyz", Some(500.0), 100.0),
            Some(500.0)
        );
    }

    #[test]
    fn effective_limit_none_when_nothing_applies() {
        assert_eq!(
            effective_limit_kmh("not-a-real-vehicle-xyz", None, 100.0),
            None
        );
    }

    #[test]
    fn gear_kmh_for_resolves_known_id() {
        assert_eq!(gear_kmh_for("bf-109f-4"), Some(360.0));
    }

    #[test]
    fn gear_kmh_for_none_when_no_gear_data_or_unknown() {
        assert_eq!(gear_kmh_for("a5m4_hagiri"), None); // has flaps but no gear entry
        assert_eq!(gear_kmh_for("not-a-real-vehicle-xyz"), None);
        assert_eq!(gear_kmh_for(""), None);
    }

    #[test]
    fn gear_intensity_ramps_over_50kmh_window() {
        let ceiling = 80.0;
        let gear_limit = 360.0;
        assert_eq!(gear_intensity(309.0, gear_limit, ceiling), 0.0);
        assert!(gear_intensity(330.0, gear_limit, ceiling) > 0.0);
        assert_eq!(gear_intensity(gear_limit, gear_limit, ceiling), ceiling);
        assert_eq!(gear_intensity(400.0, gear_limit, ceiling), ceiling);
    }
}
