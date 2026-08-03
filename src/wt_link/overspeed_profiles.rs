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
/// требованию пользователя: вибрация начинается за 10 км/ч до Vne и растёт
/// линейно к полной мощности ровно в момент достижения порога.
const OVERSPEED_WARNING_WINDOW_KMH: f64 = 10.0;

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
/// `vne_kmh - OVERSPEED_WARNING_WINDOW_KMH`, линейный рост к `ceiling`
/// внутри окна, плато на `ceiling` на/за порогом (без дальнейшего роста —
/// в отличие от stall v2, пользователь явно попросил держать на полной
/// мощности, а не продолжать наращивать за красной чертой).
pub fn intensity(ias_kmh: f64, vne_kmh: f64, ceiling: f64) -> f64 {
    let onset = vne_kmh - OVERSPEED_WARNING_WINDOW_KMH;
    let frac = ((ias_kmh - onset) / OVERSPEED_WARNING_WINDOW_KMH).clamp(0.0, 1.0);
    frac * ceiling
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
        assert_eq!(intensity(779.0, 790.0, 80.0), 0.0);
    }

    #[test]
    fn intensity_grows_monotonically_to_ceiling_at_threshold() {
        let ceiling = 80.0;
        let vne = 790.0;
        let low = intensity(781.0, vne, ceiling);
        let mid = intensity(785.0, vne, ceiling);
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
}
