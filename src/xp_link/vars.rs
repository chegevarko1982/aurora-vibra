//! Конверсия сырых значений RREF (X-Plane datarefs, см. `super::datarefs`) в
//! единый `crate::FlightVars` — точный аналог `src/sim/parse.rs` у
//! MSFS-конвейера, только источник данных другой. Собранный `FlightVars`
//! дальше идёт в тот же `crate::rumble::RumbleEngine`, что и у MSFS/аддонов —
//! именно поэтому все MSFS-специфичные поля (Fenix, MD-11, PMDG, Learjet) тут
//! просто остаются на дефолте (см. `..Default::default()` ниже): движок
//! эффектов их не потребует, пока соответствующий эффект/профиль не
//! активирован явно для конкретного борта, а `is_fenix`/`spoilers_md11_*`/
//! `fenix_gear_nose_raw`/`overspeed_lear_horn`/`eng{1,2}_pmdg_starter_ext`
//! на X-Plane попросту не имеют смысла (эти аддоны существуют только в
//! MSFS) — то есть самонейтрализуются нулём/false, как и на любом другом
//! MSFS-борту без этих аддонов.

use super::datarefs::{ACF_DESCRIP_RANGE, DrIdx};
use crate::FlightVars;

/// Порог Overspeed по умолчанию, когда `acf_Vne` ещё не пришёл от X-Plane
/// (0.0/невалидное значение) — тот же дефолт, что использует MSFS-конвейер,
/// см. `DEFAULT_OVERSPEED_BARBER_POLE_KN` в `src/sim/parse.rs`.
const DEFAULT_OVERSPEED_BARBER_POLE_KN: f64 = 350.0;

/// Единственная константа во всём модуле, требующая живой калибровки в
/// самом симе (см. план фичи): "полное" обжатие стойки в метрах, при
/// котором синтетическая величина `gear_comp_*` (см. ниже) достигает 100.
/// Слишком большая — эффект Touchdown бьёт слабо на реальных посадках,
/// слишком маленькая — насыщается (клипует в 100) на любом, даже мягком,
/// касании. Стартовое значение подобрано «на глаз», не проверено вживую.
pub const XP_FULL_COMP_M: f64 = 0.20;

/// Собирает синтетическое "обжатие стойки" в ТОЙ ЖЕ шкале 0..100, которую
/// ожидает `crate::rumble` (см. `GEAR_COMP_TOUCHDOWN_THRESHOLD = 50.1` и
/// headroom `peak/55` там же).
///
/// Расхождение семантики между симами тут ключевое, поэтому подробно:
/// - MSFS отдаёт `GEAR ANIMATION POSITION:0..2` — ОДНУ шкалу 0..100,
///   покрывающую сразу и выпуск стойки, и её обжатие после касания земли
///   (0 = убрано, ~50 = выпущено/разгружено, 50..100 = выпущено и обжато
///   пропорционально нагрузке). `rumble.rs` целиком построен вокруг этой
///   единой шкалы.
/// - X-Plane разносит ровно то же самое на ДВА независимых массива
///   datarefs: `gear/deploy_ratio[]` (0.0 = убрано, 1.0 = выпущено — только
///   положение стойки, без учёта нагрузки) и
///   `gear/tire_vertical_deflection_mtr[]` (обжатие пневматика под
///   нагрузкой, в МЕТРАХ, а не в проценте от чего-то).
///
/// Вместо переписывания `rumble.rs` под две raw-величины X-Plane, здесь
/// собирается синтетическая величина в ТОЙ ЖЕ шкале 0..100, что ждёт
/// движок эффектов — 0..50 отражает выпуск (deploy_ratio), 50..100 отражает
/// обжатие (deflection, нормированное на `XP_FULL_COMP_M` и зажатое в 0..1).
/// Итог: движок эффектов не меняется ни на строку, вся разница симов
/// инкапсулирована в этой одной функции.
fn gear_comp(deploy_ratio: f64, deflection_m: f64) -> f64 {
    50.0 * deploy_ratio.clamp(0.0, 1.0) + 50.0 * (deflection_m / XP_FULL_COMP_M).clamp(0.0, 1.0)
}

/// Собирает `crate::FlightVars` из массива значений RREF — `v[i]`
/// соответствует `DrIdx::DEFS[i]` (см. doc-комментарий `datarefs.rs`:
/// дискриминант `DrIdx` == индекс подписки == индекс слота значения, это
/// один и тот же список).
///
/// Не паникует на короткой/нулевой `v` (например до первого пакета от
/// сима) — недостающие/нулевые значения интерпретируются как честные нули,
/// ровно как `unwrap_or(0.0)` у MSFS-конвейера в `sim/parse.rs`.
pub fn to_flight_vars(v: &[f32]) -> FlightVars {
    // Локальный хелпер: читает значение по DrIdx как f64 (RREF всегда даёт
    // f32, движок эффектов везде работает с f64, как и MSFS-конвейер).
    let g = |i: DrIdx| v.get(i as usize).copied().unwrap_or(0.0) as f64;

    let vne = g(DrIdx::AcfVne);
    // Одна и та же величина уходит сразу в три поля телеметрии
    // (flaps_pct/trailing_edge_flaps_left_percent) и в три поля спойлеров
    // (spoilers_pct/left/right) — X-Plane не разводит их по крыльям в
    // используемых нами datarefs, в отличие от MSFS (см. таблицу в плане).
    let flaps_ratio_pct = g(DrIdx::FlapDeployRatio) * 100.0;
    let spoilers_ratio_pct = g(DrIdx::SpeedbrakeRatio) * 100.0;

    FlightVars {
        sim_time_s: g(DrIdx::SimTime),
        airspeed_indicated: g(DrIdx::Airspeed),
        on_ground: g(DrIdx::OnGround) != 0.0,
        bank_deg: g(DrIdx::BankDeg),
        flaps_pct: flaps_ratio_pct,
        // В X-Plane нет прямого аналога "индекса ступени закрылков" (это
        // MSFS-специфичный FLAPS HANDLE INDEX) — поле только для
        // телеметрии на MSFS-борту, здесь остаётся нулём.
        flaps_index: 0,
        gear_handle: if g(DrIdx::GearHandleDown) != 0.0 {
            1.0
        } else {
            0.0
        },
        stalled: g(DrIdx::StallWarning) != 0.0,
        // м/с → узлы; 1 м/с = 1.943844 узла.
        ground_speed_kt: g(DrIdx::Groundspeed) * 1.943844,
        paused: g(DrIdx::Paused) != 0.0,
        spoilers_pct: spoilers_ratio_pct,
        spoilers_left_pct: spoilers_ratio_pct,
        spoilers_right_pct: spoilers_ratio_pct,
        gear_comp_nose: gear_comp(g(DrIdx::GearDeploy0), g(DrIdx::GearDefl0)),
        gear_comp_left: gear_comp(g(DrIdx::GearDeploy1), g(DrIdx::GearDefl1)),
        gear_comp_right: gear_comp(g(DrIdx::GearDeploy2), g(DrIdx::GearDefl2)),
        trailing_edge_flaps_left_percent: flaps_ratio_pct,
        // Телеметрия запуска двигателей (Engine Spool-up & Ignition).
        eng1_n2_percent: g(DrIdx::Eng1N2),
        eng1_combustion: if g(DrIdx::Eng1Burning) != 0.0 { 1.0 } else { 0.0 },
        eng2_n2_percent: g(DrIdx::Eng2N2),
        eng2_combustion: if g(DrIdx::Eng2Burning) != 0.0 { 1.0 } else { 0.0 },
        eng3_n2_percent: g(DrIdx::Eng3N2),
        eng3_combustion: if g(DrIdx::Eng3Burning) != 0.0 { 1.0 } else { 0.0 },
        eng4_n2_percent: g(DrIdx::Eng4N2),
        eng4_combustion: if g(DrIdx::Eng4Burning) != 0.0 { 1.0 } else { 0.0 },
        eng1_starter: g(DrIdx::Eng1Starter) != 0.0,
        eng2_starter: g(DrIdx::Eng2Starter) != 0.0,
        eng3_starter: g(DrIdx::Eng3Starter) != 0.0,
        eng4_starter: g(DrIdx::Eng4Starter) != 0.0,
        // ENGN_N1_[..] — универсальная поддержка поршневых (см. таблицу
        // конверсий в плане: маппится на eng*_pct_max_rpm, не на n2).
        eng1_pct_max_rpm: g(DrIdx::Eng1N1),
        eng2_pct_max_rpm: g(DrIdx::Eng2N1),
        eng3_pct_max_rpm: g(DrIdx::Eng3N1),
        eng4_pct_max_rpm: g(DrIdx::Eng4N1),
        eng1_rpm: g(DrIdx::Eng1Rpm),
        eng2_rpm: g(DrIdx::Eng2Rpm),
        eng3_rpm: g(DrIdx::Eng3Rpm),
        eng4_rpm: g(DrIdx::Eng4Rpm),
        prop1_rpm: g(DrIdx::Prop1Rpm),
        prop2_rpm: g(DrIdx::Prop2Rpm),
        prop3_rpm: g(DrIdx::Prop3Rpm),
        prop4_rpm: g(DrIdx::Prop4Rpm),
        // acf_Vne == 0.0 значит "ещё не пришло от X-Plane" (симметрично
        // AIRSPEED BARBER POLE == 0.0 у MSFS) — подставляем тот же дефолт.
        overspeed_barber_pole_kn: if vne <= 0.0 {
            DEFAULT_OVERSPEED_BARBER_POLE_KN
        } else {
            vne
        },
        slats_pct: g(DrIdx::SlatDeployRatio) * 100.0,
        overspeed_warning: g(DrIdx::OverspeedWarning) != 0.0,
        // GENERAL ENG STARTER ACTIVE:1/2 у MSFS — отдельный dataref от
        // "стартер крутит". В X-Plane такого второго сигнала нет, поэтому
        // дублируем из eng{1,2}_starter (см. план) — на MSFS-борту это два
        // разных поля для сравнения в UI, здесь они по построению совпадают.
        eng1_starter_active: g(DrIdx::Eng1Starter) != 0.0,
        eng2_starter_active: g(DrIdx::Eng2Starter) != 0.0,
        // Остальное — MSFS-аддонная специфика (Fenix/MD-11/PMDG/Learjet),
        // на X-Plane смысла не имеет, остаётся на дефолте (false/0.0), см.
        // doc-комментарий модуля выше.
        ..Default::default()
    }
}

/// Собирает строку имени борта (`acf_descrip`) из байтов, пришедших как
/// отдельные float-значения — RREF умеет отдавать только числа, у X-Plane
/// нет способа прислать dataref-строку одним пакетом, поэтому каждый байт
/// имени подписан и читается отдельным слотом (см. `ACF_DESCRIP_RANGE` в
/// `datarefs.rs`).
pub fn acf_descrip(v: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(ACF_DESCRIP_RANGE.len());
    for i in ACF_DESCRIP_RANGE {
        // round(), не as u8 напрямую: RREF присылает float, значение байта
        // может прийти как 65.0000001/64.9999998 из-за округления на
        // стороне сима — прямой `as u8` в таких случаях иногда обрубает
        // вниз и портит символ на единицу.
        let raw = v.get(i).copied().unwrap_or(0.0).round();
        let b = raw.clamp(0.0, 255.0) as u8;
        if b == 0 {
            // NUL-терминатор — конец строки, как в Си.
            break;
        }
        // Отбрасываем непечатаемые/не-ASCII байты (мусор в неиспользуемом
        // хвосте массива на некоторых бортах), но не прерываем сборку —
        // только настоящий 0 означает конец строки.
        if b.is_ascii() && !b.is_ascii_control() {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn zeros() -> Vec<f32> {
        vec![0f32; DrIdx::COUNT]
    }

    #[test]
    fn groundspeed_converts_meters_per_second_to_knots() {
        let mut v = zeros();
        v[DrIdx::Groundspeed as usize] = 100.0;
        let fv = to_flight_vars(&v);
        assert_relative_eq!(fv.ground_speed_kt, 194.3844, epsilon = 1e-6);
    }

    #[test]
    fn ratio_fields_scale_to_percent() {
        let mut v = zeros();
        v[DrIdx::FlapDeployRatio as usize] = 0.5;
        v[DrIdx::SpeedbrakeRatio as usize] = 0.5;
        let fv = to_flight_vars(&v);
        assert_eq!(fv.flaps_pct, 50.0);
        assert_eq!(fv.trailing_edge_flaps_left_percent, 50.0);
        assert_eq!(fv.spoilers_pct, 50.0);
        assert_eq!(fv.spoilers_left_pct, 50.0);
        assert_eq!(fv.spoilers_right_pct, 50.0);
    }

    #[test]
    fn overspeed_defaults_to_350_when_vne_non_positive() {
        let fv = to_flight_vars(&zeros());
        assert_eq!(fv.overspeed_barber_pole_kn, 350.0);
    }

    #[test]
    fn overspeed_uses_acf_vne_when_positive() {
        let mut v = zeros();
        v[DrIdx::AcfVne as usize] = 250.0;
        let fv = to_flight_vars(&v);
        assert_eq!(fv.overspeed_barber_pole_kn, 250.0);
    }

    #[test]
    fn gear_comp_zero_when_retracted() {
        assert_eq!(gear_comp(0.0, 0.0), 0.0);
    }

    #[test]
    fn gear_comp_fifty_when_deployed_unloaded() {
        assert_eq!(gear_comp(1.0, 0.0), 50.0);
    }

    #[test]
    fn gear_comp_hundred_at_full_compression() {
        assert_eq!(gear_comp(1.0, XP_FULL_COMP_M), 100.0);
    }

    #[test]
    fn gear_comp_clamps_beyond_full_compression() {
        assert_eq!(gear_comp(1.0, 2.0 * XP_FULL_COMP_M), 100.0);
    }

    #[test]
    fn gear_comp_scales_linearly_with_partial_deploy() {
        assert_eq!(gear_comp(0.5, 0.0), 25.0);
    }

    #[test]
    fn acf_descrip_reads_name_until_nul() {
        let mut v = zeros();
        let name = b"Cessna 172";
        let base = DrIdx::AcfDescrip00 as usize;
        for (i, &b) in name.iter().enumerate() {
            v[base + i] = b as f32;
        }
        // Хвост массива остаётся нулём по умолчанию — это и есть NUL-терминатор.
        assert_eq!(acf_descrip(&v), "Cessna 172");
    }

    #[test]
    fn acf_descrip_empty_when_all_zero() {
        assert_eq!(acf_descrip(&zeros()), "");
    }

    #[test]
    fn to_flight_vars_on_zero_array_does_not_panic_and_has_sane_defaults() {
        let fv = to_flight_vars(&zeros());
        assert_eq!(fv.overspeed_barber_pole_kn, 350.0);
        assert_eq!(fv.flaps_index, 0);
        assert!(!fv.on_ground);
        assert!(!fv.paused);
        // MSFS-only поля остаются на дефолте на X-Plane.
        assert!(!fv.is_fenix);
        assert_eq!(fv.spoilers_md11_left_avg, 0.0);
    }

    #[test]
    fn to_flight_vars_handles_short_slice_without_panicking() {
        let short = [0f32; 3];
        let fv = to_flight_vars(&short);
        assert_eq!(fv.overspeed_barber_pole_kn, 350.0);
    }
}
