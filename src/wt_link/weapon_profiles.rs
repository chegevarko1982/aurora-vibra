//! Статическая таблица ёмкости боекомплекта по слотам вооружения,
//! перенесённая вручную из датамайн-таблицы War Thunder (Google Sheet
//! `11XiQYTdfoxYUj_iPcncXVd_tX4lB0Dz6j-OG4sdAZFY`, колонки `Telemetry_ID`,
//! `Weapon_1_MG`/`_Ammo`, `Weapon_2_Cannon`/`_Ammo`, `Weapon_3_Cannon`/
//! `_Ammo`), ключ — та же строка, что `WtVars::vehicle_type`/`/indicators.type`
//! (подтверждено: в таблице есть буквально `bf-109f-4`).
//!
//! Используется ТОЛЬКО как необязательная подсказка для
//! `ammo::AmmoTracker::set_weapon_capacity_hint` — помогает выбрать, какой из
//! двух уже самостоятельно выученных по живой телеметрии кластеров назвать
//! weapon1, а какой weapon2, когда у борта вообще нет ключей `weapon1..4` в
//! `/indicators`. Никогда не подменяет и не опережает живой сигнал: борт без
//! единого поля боеприпасов вообще (например, известный пробел телеметрии
//! A6M3 Zero) этой таблицей не чинится — сигнала для неё нет в принципе.
//!
//! **Важно при добавлении новых строк**: названия колонок в исходной таблице
//! (`Weapon_1_MG` vs `Weapon_2_Cannon`) — это ярлык слота у большинства
//! бортов, а не гарантия типа оружия. На части бортов (например, с двумя
//! разными калибрами пулемётов на одном борту, без пушки вообще) датамайн
//! кладёт оба калибра в одну колонку `Weapon_1_MG` через `; ` — суммировать
//! их в `weapon1_ammo_capacity`, не пытаться развести по калибрам. Перед тем
//! как заносить борт в `ALL_WEAPON_PROFILES`, смотреть на текст названия
//! оружия в ячейке, а не считать по умолчанию, что слот 2/3 — обязательно
//! пушка.
//!
//! Транскрибируется вручную при разработке (тот же подход, что
//! `aero_profiles::BF_109_F4`) — без сетевого похода за CSV в рантайме и без
//! новой зависимости на CSV-парсер. Таблица заведомо неполная: начинаем с
//! бортов, уже встречавшихся в записанных сессиях (`wt_probe_sessions/`),
//! расширяем по мере необходимости.

/// Ёмкость боекомплекта по слотам оружия одного борта.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponProfile {
    /// Варианты нормализованного имени `/indicators.type` (см.
    /// `match_weapon_profile`) — та же схема сравнения, что в
    /// `aero_profiles::StallProfile::name_patterns`.
    pub name_patterns: &'static [&'static str],
    /// Сумма `Weapon_1_MG_Ammo` (если в ячейке несколько позиций через `; ` —
    /// сумма всех). `None`, если в этом слоте у борта вообще нет оружия —
    /// специально не `Some(0.0)`, ноль испортил бы сравнение ближайшей
    /// ёмкости в `AmmoTracker`.
    pub weapon1_ammo_capacity: Option<f64>,
    /// Сумма `Weapon_2_Cannon_Ammo` + `Weapon_3_Cannon_Ammo` (обе колонки —
    /// WT-телеметрия всё равно ORит weapon3→weapon1 и weapon4→weapon2 на
    /// уровне игровых индексов, см. `vars::parse`, так что физически это
    /// один и тот же второй канал, а не три разных). `None`, если оба поля
    /// пустые.
    pub weapon2_ammo_capacity: Option<f64>,
}

/// Транскрибировано из строк CSV (см. doc-комментарий модуля). Числа — сумма
/// боезапаса по слоту, не количество стволов.
const ALL_WEAPON_PROFILES: &[WeaponProfile] = &[
    // 2 × MG17 (1000) / 20-мм MG151 (200)
    WeaponProfile {
        name_patterns: &["bf 109f 4", "bf 109 f4", "bf 109 f 4", "bf109f4"],
        weapon1_ammo_capacity: Some(1000.0),
        weapon2_ammo_capacity: Some(200.0),
    },
    // 2 × MG17 (2000) / 20-мм MG FF (120)
    WeaponProfile {
        name_patterns: &["bf 109e 3", "bf109e3"],
        weapon1_ammo_capacity: Some(2000.0),
        weapon2_ammo_capacity: Some(120.0),
    },
    // 2 × MG17 (1800) / 2 × 20-мм MG FF/M (180) + 2 × 20-мм MG151 (500)
    WeaponProfile {
        name_patterns: &["fw 190a 4", "fw190a4"],
        weapon1_ammo_capacity: Some(1800.0),
        weapon2_ammo_capacity: Some(680.0),
    },
    // A6M3 Zero НАРОЧНО не добавлен: живая телеметрия этого борта не шлёт ни
    // одного похожего на боеприпасы поля вообще (см. память проекта) —
    // запись в этой таблице была бы мёртвым грузом, а не фиксом.
];

/// Та же нормализация имени, что и в `aero_profiles::match_profile`
/// (lowercase, `-`/`_` → пробел, `contains`-матч) — но БЕЗ универсального
/// fallback: неизвестный борт должен получить `None`, а не угаданную
/// ёмкость. Ошибочная подсказка активно испортит маркировку кластеров в
/// `AmmoTracker`, в отличие от аэродинамического fallback в
/// `aero_profiles`, который просто приблизительный, а не вредный.
pub fn match_weapon_profile(vehicle_type: &str) -> Option<&'static WeaponProfile> {
    if vehicle_type.is_empty() {
        return None;
    }
    let normalized = vehicle_type.to_lowercase().replace(['-', '_'], " ");
    ALL_WEAPON_PROFILES
        .iter()
        .find(|p| p.name_patterns.iter().any(|pat| normalized.contains(pat)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_name_variants() {
        for name in ["bf-109f-4", "BF-109F-4", "bf_109f_4", "fw-190a-4"] {
            assert!(
                match_weapon_profile(name).is_some(),
                "expected match for {name:?}"
            );
        }
    }

    #[test]
    fn empty_name_has_no_profile() {
        assert!(match_weapon_profile("").is_none());
    }

    #[test]
    fn unknown_aircraft_has_no_weapon_profile() {
        // В отличие от aero_profiles::match_profile — здесь нет
        // универсального fallback: неверная подсказка вредна, поэтому
        // неизвестный борт должен получать None, не первую попавшуюся запись.
        for name in ["a6m3_zero", "spitfire mk ix", "p-51d"] {
            assert_eq!(
                match_weapon_profile(name),
                None,
                "expected no profile for {name:?}"
            );
        }
    }
}
