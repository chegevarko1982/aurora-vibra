use std::collections::BTreeMap;

use super::elem_idx::ElemIdx;
// custom_fx (CustomEffect/SourceId) существует только под фичой "app" (см.
// lib.rs) — а sim::parse собирается всегда (recon-бинарники вроде wt_probe
// её не включают). collect_lvar_defs — единственное, что здесь зависит от
// custom_fx, поэтому и импорт, и сама функция под тем же cfg.
#[cfg(feature = "app")]
use crate::custom_fx::model::CustomEffect;
#[cfg(feature = "app")]
use crate::custom_fx::sources::SourceId;
use crate::{FlightVars, SimStatus};

// Порог Overspeed по умолчанию, когда ни AIRSPEED BARBER POLE, ни L:I_PFD_VMAX
// ещё не пришли от SimConnect (0.0/невалидное значение) — см. sanitize_flight_vars.
const DEFAULT_OVERSPEED_BARBER_POLE_KN: f64 = 350.0;

/// Потолок числа одновременно зарегистрированных пользовательских LVAR (см.
/// `collect_lvar_defs`, `sim/worker.rs::DEF_LVAR`). Каждая переменная — это
/// трафик SimConnect на КАЖДОМ кадре сима, поэтому список не безлимитный.
#[cfg(feature = "app")]
pub const MAX_CUSTOM_LVARS: usize = 16;

/// Собирает уникальный список пользовательских LVAR (имя, единица) из
/// активных эффектов — то, что `sim/worker.rs` регистрирует в SimConnect под
/// отдельным `DEF_LVAR` (НЕ подмешивается в `DEF_MAIN`, см. doc-комментарий
/// там: невалидное имя/единица пользователя не должны ронять штатную
/// телеметрию).
///
/// Дедуплицирует по имени: два эффекта могут смотреть на одну и ту же
/// переменную, регистрировать её дважды нельзя. Если для одного имени
/// встретились РАЗНЫЕ единицы — оставляет первую попавшуюся и добавляет
/// предупреждение во второй возвращённый список (молча выбирать нельзя).
/// Ограничивает список `MAX_CUSTOM_LVARS`; переменные сверх лимита тоже
/// попадают в предупреждения вместо тихого отбрасывания.
#[cfg(feature = "app")]
pub fn collect_lvar_defs(effects: &[CustomEffect]) -> (Vec<(String, String)>, Vec<String>) {
    let mut defs: Vec<(String, String)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for effect in effects {
        if effect.source != SourceId::Lvar {
            continue;
        }
        let Some(lvar) = &effect.lvar else {
            continue;
        };
        let name = lvar.name.trim();
        if name.is_empty() {
            continue;
        }

        if let Some((_, existing_unit)) = defs.iter().find(|(n, _)| n == name) {
            if existing_unit != &lvar.unit {
                warnings.push(format!(
                    "'{name}': unit conflict ('{existing_unit}' vs '{}'), keeping the first one",
                    lvar.unit
                ));
            }
            continue;
        }

        if defs.len() >= MAX_CUSTOM_LVARS {
            warnings.push(format!(
                "'{name}': dropped, limit of {MAX_CUSTOM_LVARS} custom LVARs reached"
            ));
            continue;
        }

        defs.push((name.to_string(), lvar.unit.clone()));
    }

    (defs, warnings)
}

/// Разбирает буфер значений `f64`, пришедший от SimConnect для одного пакета
/// `DEF_LVAR`/`REQ_LVAR`, в словарь имя -> значение. `names` обязан быть тем
/// же вектором и в том же порядке, что был передан `SimConnect_AddToDataDefinition`
/// при регистрации (см. `collect_lvar_defs`/`sim/worker.rs`) — SimConnect
/// возвращает значения строго в порядке регистрации, без имён внутри пакета
/// (та же ловушка, из-за которой в `elem_idx.rs` завёлся макрос `elem_idx!`
/// для основного списка; здесь список динамический, поэтому источник истины —
/// вектор `names`, который вызывающий код обязан хранить рядом).
///
/// Буфер `values` короче `names` — читаем сколько есть (`zip` останавливается
/// на более коротком); длиннее — лишний хвост игнорируется.
pub fn parse_lvar_values(names: &[String], values: &[f64]) -> BTreeMap<String, f64> {
    names.iter().cloned().zip(values.iter().copied()).collect()
}

pub fn parse_main_elems(
    elem: &[f64],
    paused_from_events: bool,
    ias_deadband_kn: f64,
    aircraft_title: &str,
) -> FlightVars {
    // Спойлеры: берём МИНИМУМ левой/правой панели (индексы 41/42), а не
    // общий SPOILERS HANDLE POSITION (10) — на части самолётов (напр. TFDI
    // MD-11) часть спойлерных секций работает как roll spoilers и поднимается
    // асимметрично при одном крене, без выпуска рычага спидбрейков. min(L, R)
    // естественно гасит эффект в этом случае (одно крыло ~0%), но не мешает
    // честному симметричному выпуску, даже если поверх него добавляется
    // асимметрия от крена в развороте.
    let spoilers_left = elem
        .get(ElemIdx::SpoilersLeftPosition as usize)
        .copied()
        .unwrap_or(0.0)
        .clamp(0.0, 100.0);
    let spoilers_right = elem
        .get(ElemIdx::SpoilersRightPosition as usize)
        .copied()
        .unwrap_or(0.0)
        .clamp(0.0, 100.0);

    // TFDI MD-11: среднее по 5 секциям на каждое крыло, используется только
    // как доп. проверка симметрии в rumble.rs, единицы измерения этих
    // L-vars не важны — сравнивается только L против R.
    let md11_panel = |i: usize| elem.get(i).copied().unwrap_or(0.0).max(0.0);
    let spoilers_md11_left_avg = (md11_panel(ElemIdx::Md11SpoilerL1 as usize)
        + md11_panel(ElemIdx::Md11SpoilerL2 as usize)
        + md11_panel(ElemIdx::Md11SpoilerL3 as usize)
        + md11_panel(ElemIdx::Md11SpoilerL4 as usize)
        + md11_panel(ElemIdx::Md11SpoilerL5 as usize))
        / 5.0;
    let spoilers_md11_right_avg = (md11_panel(ElemIdx::Md11SpoilerR1 as usize)
        + md11_panel(ElemIdx::Md11SpoilerR2 as usize)
        + md11_panel(ElemIdx::Md11SpoilerR3 as usize)
        + md11_panel(ElemIdx::Md11SpoilerR4 as usize)
        + md11_panel(ElemIdx::Md11SpoilerR5 as usize))
        / 5.0;

    let mut fv = FlightVars {
        airspeed_indicated: elem
            .get(ElemIdx::AirspeedIndicated as usize)
            .copied()
            .unwrap_or(0.0),
        on_ground: elem
            .get(ElemIdx::SimOnGround as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        bank_deg: elem
            .get(ElemIdx::PlaneBankDegrees as usize)
            .copied()
            .unwrap_or(0.0),
        flaps_pct: ((elem
            .get(ElemIdx::FlapsLeftPercent as usize)
            .copied()
            .unwrap_or(0.0)
            + elem
                .get(ElemIdx::FlapsRightPercent as usize)
                .copied()
                .unwrap_or(0.0))
            * 0.5)
            .clamp(0.0, 100.0),
        flaps_index: elem
            .get(ElemIdx::FlapsHandleIndex as usize)
            .copied()
            .unwrap_or(0.0)
            .round() as i32,
        gear_handle: elem
            .get(ElemIdx::GearHandlePosition as usize)
            .copied()
            .unwrap_or(0.0),
        stalled: elem
            .get(ElemIdx::StallWarning as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        sim_time_s: elem
            .get(ElemIdx::AbsoluteTime as usize)
            .copied()
            .unwrap_or(0.0),
        ground_speed_kt: elem
            .get(ElemIdx::GroundVelocity as usize)
            .copied()
            .unwrap_or(0.0)
            .max(0.0),
        paused: paused_from_events,
        spoilers_pct: spoilers_left.min(spoilers_right),
        spoilers_left_pct: spoilers_left,
        spoilers_right_pct: spoilers_right,
        spoilers_md11_left_avg,
        spoilers_md11_right_avg,
        gear_comp_nose: elem
            .get(ElemIdx::GearAnimNose as usize)
            .copied()
            .unwrap_or(0.0),
        gear_comp_left: elem
            .get(ElemIdx::GearAnimLeft as usize)
            .copied()
            .unwrap_or(0.0),
        gear_comp_right: elem
            .get(ElemIdx::GearAnimRight as usize)
            .copied()
            .unwrap_or(0.0),
        trailing_edge_flaps_left_percent: elem
            .get(ElemIdx::FlapsLeftPercent as usize)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 100.0),
        // Телеметрия запуска двигателей (Engine Spool-up & Ignition)
        eng1_n2_percent: elem.get(ElemIdx::Eng1N2 as usize).copied().unwrap_or(0.0),
        eng1_combustion: elem
            .get(ElemIdx::Eng1Combustion as usize)
            .copied()
            .unwrap_or(0.0),
        eng2_n2_percent: elem.get(ElemIdx::Eng2N2 as usize).copied().unwrap_or(0.0),
        eng2_combustion: elem
            .get(ElemIdx::Eng2Combustion as usize)
            .copied()
            .unwrap_or(0.0),
        // Двигатели 3/4 (4-моторные самолёты, см. RumbleConfig::four_engine_mode)
        eng3_n2_percent: elem.get(ElemIdx::Eng3N2 as usize).copied().unwrap_or(0.0),
        eng3_combustion: elem
            .get(ElemIdx::Eng3Combustion as usize)
            .copied()
            .unwrap_or(0.0),
        eng4_n2_percent: elem.get(ElemIdx::Eng4N2 as usize).copied().unwrap_or(0.0),
        eng4_combustion: elem
            .get(ElemIdx::Eng4Combustion as usize)
            .copied()
            .unwrap_or(0.0),
        // GENERAL ENG PCT MAX RPM:1/2/3/4 — универсальная поддержка поршневых
        eng1_pct_max_rpm: elem
            .get(ElemIdx::Eng1PctMaxRpm as usize)
            .copied()
            .unwrap_or(0.0),
        eng2_pct_max_rpm: elem
            .get(ElemIdx::Eng2PctMaxRpm as usize)
            .copied()
            .unwrap_or(0.0),
        eng3_pct_max_rpm: elem
            .get(ElemIdx::Eng3PctMaxRpm as usize)
            .copied()
            .unwrap_or(0.0),
        eng4_pct_max_rpm: elem
            .get(ElemIdx::Eng4PctMaxRpm as usize)
            .copied()
            .unwrap_or(0.0),
        // GENERAL ENG STARTER:1/2/3/4 — универсальная модель запуска
        eng1_starter: elem
            .get(ElemIdx::Eng1Starter as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        eng2_starter: elem
            .get(ElemIdx::Eng2Starter as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        eng3_starter: elem
            .get(ElemIdx::Eng3Starter as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        eng4_starter: elem
            .get(ElemIdx::Eng4Starter as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        // Поршневые двигатели (Piston Engine Telemetry): GENERAL ENG RPM —
        // обороты коленвала, PROP RPM — обороты воздушного винта.
        eng1_rpm: elem.get(ElemIdx::Eng1Rpm as usize).copied().unwrap_or(0.0),
        eng2_rpm: elem.get(ElemIdx::Eng2Rpm as usize).copied().unwrap_or(0.0),
        eng3_rpm: elem.get(ElemIdx::Eng3Rpm as usize).copied().unwrap_or(0.0),
        eng4_rpm: elem.get(ElemIdx::Eng4Rpm as usize).copied().unwrap_or(0.0),
        prop1_rpm: elem.get(ElemIdx::Prop1Rpm as usize).copied().unwrap_or(0.0),
        prop2_rpm: elem.get(ElemIdx::Prop2Rpm as usize).copied().unwrap_or(0.0),
        prop3_rpm: elem.get(ElemIdx::Prop3Rpm as usize).copied().unwrap_or(0.0),
        prop4_rpm: elem.get(ElemIdx::Prop4Rpm as usize).copied().unwrap_or(0.0),
        // Порог Overspeed: Fenix A320 не держит AIRSPEED BARBER POLE синхронной
        // с реальным PFD, поэтому для этого борта (подстрока "Fenix" в title,
        // регистронезависимо) читаем его собственный L:I_PFD_VMAX вместо
        // AIRSPEED BARBER POLE. Финальный fallback на
        // DEFAULT_OVERSPEED_BARBER_POLE_KN, если выбранное значение всё ещё
        // 0.0/невалидно — см. sanitize_flight_vars.
        overspeed_barber_pole_kn: if crate::profiles::is_fenix_aircraft(aircraft_title) {
            elem.get(ElemIdx::FenixOverspeedVmax as usize)
                .copied()
                .unwrap_or(0.0)
        } else {
            elem.get(ElemIdx::AirspeedBarberPole as usize)
                .copied()
                .unwrap_or(0.0)
        },
        // Предкрылки (Slats) — среднее LEADING EDGE FLAPS LEFT/RIGHT PERCENT.
        slats_pct: ((elem
            .get(ElemIdx::SlatsLeftPercent as usize)
            .copied()
            .unwrap_or(0.0)
            + elem
                .get(ElemIdx::SlatsRightPercent as usize)
                .copied()
                .unwrap_or(0.0))
            * 0.5)
            .clamp(0.0, 100.0),
        // OVERSPEED WARNING — булев флаг "клацера" сима, для сравнения в
        // телеметрии с нашим порогом overspeed_barber_pole_kn.
        overspeed_warning: elem
            .get(ElemIdx::OverspeedWarning as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        // L:XMLSND75 — клаксон "overspeed / mach trim" на Learjet 35A
        // (Flysimware); на прочих самолётах L-var не определён, читается 0.0.
        overspeed_lear_horn: elem
            .get(ElemIdx::OverspeedLearHorn as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        // PMDG 737 (NG3): L:EngineStart1b/2b_Ext, см. sim/worker.rs/elem_idx.rs
        // и rumble.rs (pre-spool разгон). На прочих самолётах L-var'ы не
        // определены, читается 0.0/false (самонейтрализуется).
        eng1_pmdg_starter_ext: elem
            .get(ElemIdx::PmdgEngineStart1b as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        eng2_pmdg_starter_ext: elem
            .get(ElemIdx::PmdgEngineStart2b as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        eng1_starter_active: elem
            .get(ElemIdx::Eng1StarterActive as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        eng2_starter_active: elem
            .get(ElemIdx::Eng2StarterActive as usize)
            .copied()
            .unwrap_or(0.0)
            != 0.0,
        // Fenix A320: L:A320_Gear_Nose, сырая позиция носовой стойки
        // (0 = убрано, 1000 = выпущено), см. sim/elem_idx.rs. Используется
        // эффектом Gear Transit в rumble.rs вместо gear_comp_nose/left/right
        // (не отражают движение стоек на этом борте), см. is_fenix ниже.
        // Также остаётся в телеметрии как "F_Gear".
        fenix_gear_nose_raw: elem
            .get(ElemIdx::FenixGearNose as usize)
            .copied()
            .unwrap_or(0.0),
        is_fenix: crate::profiles::is_fenix_aircraft(aircraft_title),
        // Пустой словарь здесь: этот парсер работает только с фиксированным
        // buffer'ом ElemIdx::DEFS (DEF_MAIN) и ничего не знает о динамическом
        // списке пользовательских LVAR. Их регистрация (DEF_LVAR/REQ_LVAR) и
        // приём значений живут в sim/worker.rs — вызывающий код там
        // присваивает `fv.lvars` накопленный словарь ПОСЛЕ этого вызова (см.
        // collect_lvar_defs/parse_lvar_values выше и doc-комментарий на
        // FlightVars::lvars в types.rs).
        lvars: BTreeMap::new(),
    };

    sanitize_flight_vars(&mut fv, ias_deadband_kn);
    fv
}

pub fn sanitize_flight_vars(fv: &mut FlightVars, ias_deadband_kn: f64) {
    if !fv.airspeed_indicated.is_finite()
        || fv.airspeed_indicated < -5.0
        || fv.airspeed_indicated > 1200.0
    {
        fv.airspeed_indicated = 0.0;
    }
    if fv.airspeed_indicated.abs() < ias_deadband_kn {
        fv.airspeed_indicated = 0.0;
    }
    if !fv.bank_deg.is_finite() {
        fv.bank_deg = 0.0;
    }
    // AIRSPEED BARBER POLE / L:I_PFD_VMAX приходят как 0.0, когда SimConnect
    // ещё не отдал значение (не подключились/аддон его не поддерживает) —
    // вместо того, чтобы оставлять эффект Overspeed навсегда выключенным,
    // подставляем безопасный дефолт.
    if !fv.overspeed_barber_pole_kn.is_finite() || fv.overspeed_barber_pole_kn <= 0.0 {
        fv.overspeed_barber_pole_kn = DEFAULT_OVERSPEED_BARBER_POLE_KN;
    }
}

pub fn flight_status(fv: &FlightVars) -> SimStatus {
    if !fv.on_ground && fv.airspeed_indicated > 30.0 {
        SimStatus::InFlight
    } else {
        SimStatus::Connected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_elems() -> [f64; ElemIdx::COUNT] {
        let mut e = [0.0; ElemIdx::COUNT];
        e[ElemIdx::AirspeedIndicated as usize] = 120.0;
        e[ElemIdx::SimOnGround as usize] = 0.0;
        e[ElemIdx::PlaneBankDegrees as usize] = 15.0;
        e[ElemIdx::FlapsLeftPercent as usize] = 50.0;
        e[ElemIdx::FlapsRightPercent as usize] = 70.0;
        e[ElemIdx::FlapsHandleIndex as usize] = 2.0;
        e[ElemIdx::GearHandlePosition as usize] = 1.0;
        e[ElemIdx::StallWarning as usize] = 0.0;
        e[ElemIdx::AbsoluteTime as usize] = 100.0;
        e[ElemIdx::GroundVelocity as usize] = 25.0;
        // SPOILERS HANDLE POSITION — больше не читается, значение не должно влиять.
        e[ElemIdx::SpoilersHandlePosition as usize] = 999.0;
        e[ElemIdx::GearAnimNose as usize] = 0.0;
        e[ElemIdx::GearAnimLeft as usize] = 0.0;
        e[ElemIdx::GearAnimRight as usize] = 0.0;
        e[ElemIdx::SpoilersLeftPosition as usize] = 45.0;
        // Симметрично с левой — честный выпуск спидбрейков.
        e[ElemIdx::SpoilersRightPosition as usize] = 45.0;
        e
    }

    #[test]
    fn parses_all_fields_from_sample_elems() {
        let fv = parse_main_elems(&sample_elems(), false, 1.0, "");
        assert_eq!(fv.airspeed_indicated, 120.0);
        assert!(!fv.on_ground);
        assert_eq!(fv.bank_deg, 15.0);
        assert_eq!(fv.flaps_pct, 60.0);
        assert_eq!(fv.flaps_index, 2);
        assert_eq!(fv.gear_handle, 1.0);
        assert!(!fv.stalled);
        assert_eq!(fv.sim_time_s, 100.0);
        assert_eq!(fv.ground_speed_kt, 25.0);
        assert_eq!(fv.spoilers_pct, 45.0);
    }

    #[test]
    fn spoilers_pct_handles_missing_elements_gracefully() {
        let short_e = &sample_elems()[0..10];
        let fv = parse_main_elems(short_e, false, 1.0, "");
        assert_eq!(fv.spoilers_pct, 0.0);
    }

    #[test]
    fn spoilers_pct_ignores_legacy_handle_position_index() {
        // Индекс 10 (SPOILERS HANDLE POSITION) в sample_elems() выставлен в 999.0,
        // но spoilers_pct теперь считается из индексов 41/42 (L/R), а не из него.
        let fv = parse_main_elems(&sample_elems(), false, 1.0, "");
        assert_eq!(fv.spoilers_pct, 45.0);
    }

    #[test]
    fn spoilers_pct_uses_min_for_symmetric_extension() {
        let mut e = sample_elems();
        e[ElemIdx::SpoilersLeftPosition as usize] = 60.0;
        e[ElemIdx::SpoilersRightPosition as usize] = 60.0;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(fv.spoilers_pct, 60.0);
    }

    #[test]
    fn spoilers_pct_zero_during_pure_roll_asymmetry() {
        // Крен без выпуска спидбрейков: одна плоскость поднята (roll spoiler),
        // другая на нуле — эффект не должен срабатывать.
        let mut e = sample_elems();
        e[ElemIdx::SpoilersLeftPosition as usize] = 15.0;
        e[ElemIdx::SpoilersRightPosition as usize] = 0.0;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(fv.spoilers_pct, 0.0);
    }

    #[test]
    fn spoilers_pct_uses_min_when_roll_adds_on_top_of_real_deployment() {
        // Честный выпуск спидбрейков (симметричная база) + крен в развороте
        // добавляет спойлерон сверху на одном крыле. Эффект не должен резко
        // обнуляться — интенсивность просто следует за менее выпущенным крылом.
        let mut e = sample_elems();
        e[ElemIdx::SpoilersLeftPosition as usize] = 70.0;
        e[ElemIdx::SpoilersRightPosition as usize] = 50.0;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(fv.spoilers_pct, 50.0);
    }

    #[test]
    fn spoilers_md11_panel_averages_default_to_zero_on_other_aircraft() {
        let fv = parse_main_elems(&sample_elems(), false, 1.0, "");
        assert_eq!(fv.spoilers_md11_left_avg, 0.0);
        assert_eq!(fv.spoilers_md11_right_avg, 0.0);
    }

    #[test]
    fn spoilers_md11_panel_averages_computed_from_five_sections_per_wing() {
        let mut e = sample_elems();
        for slot in &mut e[ElemIdx::Md11SpoilerL1 as usize..=ElemIdx::Md11SpoilerL5 as usize] {
            *slot = 29.511; // L1..L5
        }
        for slot in &mut e[ElemIdx::Md11SpoilerR1 as usize..=ElemIdx::Md11SpoilerR5 as usize] {
            *slot = 0.0; // R1..R5 (крен без выпуска рычага, как на скриншоте)
        }
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert!((fv.spoilers_md11_left_avg - 29.511).abs() < 1e-9);
        assert_eq!(fv.spoilers_md11_right_avg, 0.0);
    }

    #[test]
    fn flaps_pct_is_average_of_left_and_right() {
        let mut e = sample_elems();
        e[ElemIdx::FlapsLeftPercent as usize] = 0.0;
        e[ElemIdx::FlapsRightPercent as usize] = 100.0;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(fv.flaps_pct, 50.0);
    }

    #[test]
    fn non_finite_ias_becomes_zero() {
        let mut e = sample_elems();
        e[ElemIdx::AirspeedIndicated as usize] = f64::NAN;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(fv.airspeed_indicated, 0.0);
    }

    #[test]
    fn out_of_range_ias_becomes_zero() {
        let mut e = sample_elems();
        e[ElemIdx::AirspeedIndicated as usize] = 1500.0;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(fv.airspeed_indicated, 0.0);
    }

    #[test]
    fn ias_within_deadband_becomes_zero() {
        let mut e = sample_elems();
        e[ElemIdx::AirspeedIndicated as usize] = 0.5;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(fv.airspeed_indicated, 0.0);
    }

    #[test]
    fn non_finite_bank_becomes_zero() {
        let mut e = sample_elems();
        e[ElemIdx::PlaneBankDegrees as usize] = f64::INFINITY;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(fv.bank_deg, 0.0);
    }

    #[test]
    fn ground_speed_is_clamped_to_non_negative() {
        let mut e = sample_elems();
        e[ElemIdx::GroundVelocity as usize] = -5.0;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(fv.ground_speed_kt, 0.0);
    }

    #[test]
    fn flight_status_in_flight_when_airborne_and_fast() {
        let mut e = sample_elems();
        e[ElemIdx::AirspeedIndicated as usize] = 150.0;
        e[ElemIdx::SimOnGround as usize] = 0.0;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(flight_status(&fv), SimStatus::InFlight);
    }

    #[test]
    fn flight_status_connected_on_ground() {
        let mut e = sample_elems();
        e[ElemIdx::SimOnGround as usize] = 1.0;
        e[ElemIdx::AirspeedIndicated as usize] = 150.0;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(flight_status(&fv), SimStatus::Connected);
    }

    #[test]
    fn flight_status_connected_when_slow_airborne() {
        let mut e = sample_elems();
        e[ElemIdx::AirspeedIndicated as usize] = 20.0;
        e[ElemIdx::SimOnGround as usize] = 0.0;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert_eq!(flight_status(&fv), SimStatus::Connected);
    }

    #[test]
    fn pmdg_diagnostic_lvars_default_to_false_on_other_aircraft() {
        let fv = parse_main_elems(&sample_elems(), false, 1.0, "");
        assert!(!fv.eng1_pmdg_starter_ext);
        assert!(!fv.eng2_pmdg_starter_ext);
        assert!(!fv.eng1_starter_active);
        assert!(!fv.eng2_starter_active);
    }

    #[test]
    fn pmdg_diagnostic_lvars_parsed_from_correct_indices() {
        let mut e = [0.0; ElemIdx::COUNT];
        e[ElemIdx::PmdgEngineStart1b as usize] = 1.0;
        e[ElemIdx::PmdgEngineStart2b as usize] = 1.0;
        e[ElemIdx::Eng1StarterActive as usize] = 1.0;
        e[ElemIdx::Eng2StarterActive as usize] = 1.0;
        let fv = parse_main_elems(&e, false, 1.0, "");
        assert!(fv.eng1_pmdg_starter_ext);
        assert!(fv.eng2_pmdg_starter_ext);
        assert!(fv.eng1_starter_active);
        assert!(fv.eng2_starter_active);
    }

    #[test]
    fn overspeed_threshold_uses_airspeed_barber_pole_by_default() {
        let mut e = [0.0; ElemIdx::COUNT];
        e[ElemIdx::AirspeedBarberPole as usize] = 340.0;
        // L:I_PFD_VMAX — must be ignored on non-Fenix aircraft.
        e[ElemIdx::FenixOverspeedVmax as usize] = 320.0;
        let fv = parse_main_elems(&e, false, 1.0, "Boeing 737-800");
        assert_eq!(fv.overspeed_barber_pole_kn, 340.0);
    }

    #[test]
    fn overspeed_threshold_uses_i_pfd_vmax_on_fenix() {
        let mut e = [0.0; ElemIdx::COUNT];
        // AIRSPEED BARBER POLE — must be ignored on Fenix.
        e[ElemIdx::AirspeedBarberPole as usize] = 340.0;
        e[ElemIdx::FenixOverspeedVmax as usize] = 320.0;
        let fv = parse_main_elems(&e, false, 1.0, "Fenix A320");
        assert_eq!(fv.overspeed_barber_pole_kn, 320.0);
    }

    #[test]
    fn overspeed_threshold_fenix_match_is_case_insensitive() {
        let mut e = [0.0; ElemIdx::COUNT];
        e[ElemIdx::FenixOverspeedVmax as usize] = 320.0;
        let fv = parse_main_elems(&e, false, 1.0, "fenix a320neo");
        assert_eq!(fv.overspeed_barber_pole_kn, 320.0);
    }

    #[test]
    fn overspeed_threshold_defaults_to_350_when_unavailable() {
        let fv = parse_main_elems(&sample_elems(), false, 1.0, "Boeing 737-800");
        assert_eq!(fv.overspeed_barber_pole_kn, 350.0);
    }

    // --- collect_lvar_defs / parse_lvar_values ---

    // collect_lvar_defs (и, следовательно, эти тесты) существует только под
    // фичой "app" — см. cfg на самой функции выше и комментарий у импорта
    // custom_fx в начале файла.
    #[cfg(feature = "app")]
    mod collect_lvar_defs_tests {
        use super::*;
        use crate::custom_fx::model::{LvarSpec, new_effect};

        fn lvar_effect(name: &str, unit: &str) -> CustomEffect {
            let mut e = new_effect(format!("fx-{name}"), SourceId::Lvar);
            e.lvar = Some(LvarSpec {
                name: name.to_string(),
                unit: unit.to_string(),
            });
            e
        }

        #[test]
        fn collect_lvar_defs_ignores_non_lvar_effects() {
            let effects = vec![new_effect("A".into(), SourceId::FlightAirspeedKn)];
            let (defs, warnings) = collect_lvar_defs(&effects);
            assert!(defs.is_empty());
            assert!(warnings.is_empty());
        }

        #[test]
        fn collect_lvar_defs_ignores_lvar_source_without_spec() {
            // source == Lvar, но lvar == None (UI ещё не заполнил спецификацию).
            let effects = vec![new_effect("A".into(), SourceId::Lvar)];
            let (defs, warnings) = collect_lvar_defs(&effects);
            assert!(defs.is_empty());
            assert!(warnings.is_empty());
        }

        #[test]
        fn collect_lvar_defs_skips_blank_names() {
            let effects = vec![lvar_effect("   ", "Number")];
            let (defs, warnings) = collect_lvar_defs(&effects);
            assert!(defs.is_empty());
            assert!(warnings.is_empty());
        }

        #[test]
        fn collect_lvar_defs_returns_trimmed_unique_names() {
            let effects = vec![lvar_effect("  L:Foo  ", "Number")];
            let (defs, _warnings) = collect_lvar_defs(&effects);
            assert_eq!(defs, vec![("L:Foo".to_string(), "Number".to_string())]);
        }

        #[test]
        fn collect_lvar_defs_dedupes_same_name_same_unit_silently() {
            let effects = vec![
                lvar_effect("L:Foo", "Number"),
                lvar_effect("L:Foo", "Number"),
            ];
            let (defs, warnings) = collect_lvar_defs(&effects);
            assert_eq!(defs.len(), 1);
            assert!(warnings.is_empty());
        }

        #[test]
        fn collect_lvar_defs_keeps_first_unit_and_warns_on_conflict() {
            let effects = vec![
                lvar_effect("L:Foo", "Number"),
                lvar_effect("L:Foo", "Percent"),
            ];
            let (defs, warnings) = collect_lvar_defs(&effects);
            assert_eq!(defs, vec![("L:Foo".to_string(), "Number".to_string())]);
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].contains("L:Foo"));
        }

        #[test]
        fn collect_lvar_defs_caps_at_max_and_warns_on_overflow() {
            let effects: Vec<CustomEffect> = (0..MAX_CUSTOM_LVARS + 3)
                .map(|i| lvar_effect(&format!("L:Var{i}"), "Number"))
                .collect();
            let (defs, warnings) = collect_lvar_defs(&effects);
            assert_eq!(defs.len(), MAX_CUSTOM_LVARS);
            assert_eq!(warnings.len(), 3);
        }
    }

    #[test]
    fn parse_lvar_values_zips_names_and_values_in_order() {
        let names = vec!["L:A".to_string(), "L:B".to_string(), "L:C".to_string()];
        let values = [1.0, 2.0, 3.0];
        let map = parse_lvar_values(&names, &values);
        assert_eq!(map.get("L:A"), Some(&1.0));
        assert_eq!(map.get("L:B"), Some(&2.0));
        assert_eq!(map.get("L:C"), Some(&3.0));
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn parse_lvar_values_handles_shorter_value_buffer() {
        let names = vec!["L:A".to_string(), "L:B".to_string()];
        let values = [1.0];
        let map = parse_lvar_values(&names, &values);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("L:A"), Some(&1.0));
        assert_eq!(map.get("L:B"), None);
    }

    #[test]
    fn parse_lvar_values_ignores_extra_values_beyond_names() {
        let names = vec!["L:A".to_string()];
        let values = [1.0, 99.0, 99.0];
        let map = parse_lvar_values(&names, &values);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("L:A"), Some(&1.0));
    }

    #[test]
    fn parse_lvar_values_empty_names_yields_empty_map() {
        let map = parse_lvar_values(&[], &[1.0, 2.0]);
        assert!(map.is_empty());
    }
}
