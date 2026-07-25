use crate::{FlightVars, SimStatus};

pub fn parse_main_elems(
    elem: &[f64],
    paused_from_events: bool,
    ias_deadband_kn: f64,
) -> FlightVars {
    // Спойлеры: берём МИНИМУМ левой/правой панели (индексы 41/42), а не
    // общий SPOILERS HANDLE POSITION (10) — на части самолётов (напр. TFDI
    // MD-11) часть спойлерных секций работает как roll spoilers и поднимается
    // асимметрично при одном крене, без выпуска рычага спидбрейков. min(L, R)
    // естественно гасит эффект в этом случае (одно крыло ~0%), но не мешает
    // честному симметричному выпуску, даже если поверх него добавляется
    // асимметрия от крена в развороте.
    let spoilers_left = elem.get(41).copied().unwrap_or(0.0).clamp(0.0, 100.0);
    let spoilers_right = elem.get(42).copied().unwrap_or(0.0).clamp(0.0, 100.0);

    // TFDI MD-11: среднее по 5 секциям на каждое крыло (индексы 43-47 / 48-52),
    // используется только как доп. проверка симметрии в rumble.rs, единицы
    // измерения этих L-vars не важны — сравнивается только L против R.
    let md11_panel = |i: usize| elem.get(i).copied().unwrap_or(0.0).max(0.0);
    let spoilers_md11_left_avg =
        (md11_panel(43) + md11_panel(44) + md11_panel(45) + md11_panel(46) + md11_panel(47)) / 5.0;
    let spoilers_md11_right_avg =
        (md11_panel(48) + md11_panel(49) + md11_panel(50) + md11_panel(51) + md11_panel(52)) / 5.0;

    let mut fv = FlightVars {
        airspeed_indicated: elem.get(0).copied().unwrap_or(0.0),
        on_ground: elem.get(1).copied().unwrap_or(0.0) != 0.0,
        bank_deg: elem.get(2).copied().unwrap_or(0.0),
        flaps_pct: ((elem.get(3).copied().unwrap_or(0.0) + elem.get(4).copied().unwrap_or(0.0))
            * 0.5)
            .clamp(0.0, 100.0),
        flaps_index: elem.get(5).copied().unwrap_or(0.0).round() as i32,
        gear_handle: elem.get(6).copied().unwrap_or(0.0),
        stalled: elem.get(7).copied().unwrap_or(0.0) != 0.0,
        sim_time_s: elem.get(8).copied().unwrap_or(0.0),
        ground_speed_kt: elem.get(9).copied().unwrap_or(0.0).max(0.0),
        paused: paused_from_events,
        spoilers_pct: spoilers_left.min(spoilers_right),
        spoilers_left_pct: spoilers_left,
        spoilers_right_pct: spoilers_right,
        spoilers_md11_left_avg,
        spoilers_md11_right_avg,
        gear_comp_nose: elem.get(11).copied().unwrap_or(0.0),
        gear_comp_left: elem.get(12).copied().unwrap_or(0.0),
        gear_comp_right: elem.get(13).copied().unwrap_or(0.0),
        trailing_edge_flaps_left_percent: elem.get(3).copied().unwrap_or(0.0).clamp(0.0, 100.0),
        // Телеметрия запуска двигателей (Engine Spool-up & Ignition)
        eng1_n2_percent: elem.get(14).copied().unwrap_or(0.0),
        eng1_combustion: elem.get(15).copied().unwrap_or(0.0),
        eng2_n2_percent: elem.get(16).copied().unwrap_or(0.0),
        eng2_combustion: elem.get(17).copied().unwrap_or(0.0),
        // Двигатели 3/4 (4-моторные самолёты, см. RumbleConfig::four_engine_mode)
        eng3_n2_percent: elem.get(18).copied().unwrap_or(0.0),
        eng3_combustion: elem.get(19).copied().unwrap_or(0.0),
        eng4_n2_percent: elem.get(20).copied().unwrap_or(0.0),
        eng4_combustion: elem.get(21).copied().unwrap_or(0.0),
        // GENERAL ENG PCT MAX RPM:1/2/3/4 — универсальная поддержка поршневых
        eng1_pct_max_rpm: elem.get(22).copied().unwrap_or(0.0),
        eng2_pct_max_rpm: elem.get(23).copied().unwrap_or(0.0),
        eng3_pct_max_rpm: elem.get(24).copied().unwrap_or(0.0),
        eng4_pct_max_rpm: elem.get(25).copied().unwrap_or(0.0),
        // GENERAL ENG STARTER:1/2/3/4 — универсальная модель запуска
        eng1_starter: elem.get(26).copied().unwrap_or(0.0) != 0.0,
        eng2_starter: elem.get(27).copied().unwrap_or(0.0) != 0.0,
        eng3_starter: elem.get(28).copied().unwrap_or(0.0) != 0.0,
        eng4_starter: elem.get(29).copied().unwrap_or(0.0) != 0.0,
        // Поршневые двигатели (Piston Engine Telemetry): GENERAL ENG RPM —
        // обороты коленвала, PROP RPM — обороты воздушного винта.
        eng1_rpm: elem.get(30).copied().unwrap_or(0.0),
        eng2_rpm: elem.get(31).copied().unwrap_or(0.0),
        eng3_rpm: elem.get(32).copied().unwrap_or(0.0),
        eng4_rpm: elem.get(33).copied().unwrap_or(0.0),
        prop1_rpm: elem.get(34).copied().unwrap_or(0.0),
        prop2_rpm: elem.get(35).copied().unwrap_or(0.0),
        prop3_rpm: elem.get(36).copied().unwrap_or(0.0),
        prop4_rpm: elem.get(37).copied().unwrap_or(0.0),
        // DESIGN SPEED VC — динамический порог Overspeed для текущего самолёта.
        design_speed_vc_kn: elem.get(38).copied().unwrap_or(0.0),
        // Предкрылки (Slats) — среднее LEADING EDGE FLAPS LEFT/RIGHT PERCENT.
        slats_pct: ((elem.get(39).copied().unwrap_or(0.0) + elem.get(40).copied().unwrap_or(0.0))
            * 0.5)
            .clamp(0.0, 100.0),
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
    // DESIGN SPEED VC приходит как 0.0 для самолётов/сценариев, где SimConnect
    // не отдаёт это значение (или ещё не подключились) — трактуем как "N/A".
    if !fv.design_speed_vc_kn.is_finite() || fv.design_speed_vc_kn < 0.0 {
        fv.design_speed_vc_kn = 0.0;
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

fn sample_elems() -> [f64; 53] {
        let mut e = [0.0; 53];
        e[0] = 120.0; // IAS
        e[1] = 0.0; // on ground
        e[2] = 15.0; // bank
        e[3] = 50.0; // flaps L
        e[4] = 70.0; // flaps R
        e[5] = 2.0; // flaps index
        e[6] = 1.0; // gear handle
        e[7] = 0.0; // stalled
        e[8] = 100.0; // sim time
        e[9] = 25.0; // ground speed
        e[10] = 999.0; // SPOILERS HANDLE POSITION — больше не читается, значение не должно влиять
        e[11] = 0.0; // gear_comp_nose (default)
        e[12] = 0.0; // gear_comp_left (default)
        e[13] = 0.0; // gear_comp_right (default)
        e[41] = 45.0; // spoilers left
        e[42] = 45.0; // spoilers right (симметрично с левой — честный выпуск спидбрейков)
        e
    }

    #[test]
    fn parses_all_fields_from_sample_elems() {
        let fv = parse_main_elems(&sample_elems(), false, 1.0);
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
        let fv = parse_main_elems(short_e, false, 1.0);
        assert_eq!(fv.spoilers_pct, 0.0);
    }

    #[test]
    fn spoilers_pct_ignores_legacy_handle_position_index() {
        // Индекс 10 (SPOILERS HANDLE POSITION) в sample_elems() выставлен в 999.0,
        // но spoilers_pct теперь считается из индексов 41/42 (L/R), а не из него.
        let fv = parse_main_elems(&sample_elems(), false, 1.0);
        assert_eq!(fv.spoilers_pct, 45.0);
    }

    #[test]
    fn spoilers_pct_uses_min_for_symmetric_extension() {
        let mut e = sample_elems();
        e[41] = 60.0;
        e[42] = 60.0;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(fv.spoilers_pct, 60.0);
    }

    #[test]
    fn spoilers_pct_zero_during_pure_roll_asymmetry() {
        // Крен без выпуска спидбрейков: одна плоскость поднята (roll spoiler),
        // другая на нуле — эффект не должен срабатывать.
        let mut e = sample_elems();
        e[41] = 15.0;
        e[42] = 0.0;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(fv.spoilers_pct, 0.0);
    }

    #[test]
    fn spoilers_pct_uses_min_when_roll_adds_on_top_of_real_deployment() {
        // Честный выпуск спидбрейков (симметричная база) + крен в развороте
        // добавляет спойлерон сверху на одном крыле. Эффект не должен резко
        // обнуляться — интенсивность просто следует за менее выпущенным крылом.
        let mut e = sample_elems();
        e[41] = 70.0;
        e[42] = 50.0;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(fv.spoilers_pct, 50.0);
    }

    #[test]
    fn spoilers_md11_panel_averages_default_to_zero_on_other_aircraft() {
        let fv = parse_main_elems(&sample_elems(), false, 1.0);
        assert_eq!(fv.spoilers_md11_left_avg, 0.0);
        assert_eq!(fv.spoilers_md11_right_avg, 0.0);
    }

    #[test]
    fn spoilers_md11_panel_averages_computed_from_five_sections_per_wing() {
        let mut e = sample_elems();
        for i in 43..48 {
            e[i] = 29.511; // L1..L5
        }
        for i in 48..53 {
            e[i] = 0.0; // R1..R5 (крен без выпуска рычага, как на скриншоте)
        }
        let fv = parse_main_elems(&e, false, 1.0);
        assert!((fv.spoilers_md11_left_avg - 29.511).abs() < 1e-9);
        assert_eq!(fv.spoilers_md11_right_avg, 0.0);
    }

    #[test]
    fn flaps_pct_is_average_of_left_and_right() {
        let mut e = sample_elems();
        e[3] = 0.0;
        e[4] = 100.0;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(fv.flaps_pct, 50.0);
    }

    #[test]
    fn non_finite_ias_becomes_zero() {
        let mut e = sample_elems();
        e[0] = f64::NAN;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(fv.airspeed_indicated, 0.0);
    }

    #[test]
    fn out_of_range_ias_becomes_zero() {
        let mut e = sample_elems();
        e[0] = 1500.0;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(fv.airspeed_indicated, 0.0);
    }

    #[test]
    fn ias_within_deadband_becomes_zero() {
        let mut e = sample_elems();
        e[0] = 0.5;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(fv.airspeed_indicated, 0.0);
    }

    #[test]
    fn non_finite_bank_becomes_zero() {
        let mut e = sample_elems();
        e[2] = f64::INFINITY;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(fv.bank_deg, 0.0);
    }

    #[test]
    fn ground_speed_is_clamped_to_non_negative() {
        let mut e = sample_elems();
        e[9] = -5.0;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(fv.ground_speed_kt, 0.0);
    }

    #[test]
    fn flight_status_in_flight_when_airborne_and_fast() {
        let mut e = sample_elems();
        e[0] = 150.0;
        e[1] = 0.0;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(flight_status(&fv), SimStatus::InFlight);
    }

    #[test]
    fn flight_status_connected_on_ground() {
        let mut e = sample_elems();
        e[1] = 1.0;
        e[0] = 150.0;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(flight_status(&fv), SimStatus::Connected);
    }

    #[test]
    fn flight_status_connected_when_slow_airborne() {
        let mut e = sample_elems();
        e[0] = 20.0;
        e[1] = 0.0;
        let fv = parse_main_elems(&e, false, 1.0);
        assert_eq!(flight_status(&fv), SimStatus::Connected);
    }
}