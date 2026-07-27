use aurora_vibra::hid::protocol::{WW_PID_URSA_MINOR_AIRBUS_L, build_simapp_vibe_frame};
use aurora_vibra::rumble::RumbleEngine;
use aurora_vibra::sim::parse::{flight_status, parse_main_elems};
use aurora_vibra::{FlightVars, RumbleConfig, SimStatus};

mod support;

fn elems_from_flight(
    ias: f64,
    on_ground: f64,
    bank: f64,
    flaps_l: f64,
    flaps_r: f64,
    flaps_idx: f64,
    gear: f64,
    stalled: f64,
    sim_time: f64,
    gs: f64,
    paused: f64,
) -> [f64; 11] {
    [
        ias, on_ground, bank, flaps_l, flaps_r, flaps_idx, gear, stalled, sim_time, gs, paused,
    ]
}

#[test]
fn pipeline_ground_taxi_to_takeoff() {
    let cfg = RumbleConfig::default();
    let mut engine = RumbleEngine::new();

    let taxi_elems = elems_from_flight(0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.05, 5.0, 0.0);
    let taxi_fv = parse_main_elems(&taxi_elems, false, cfg.ias_deadband_kn, "");
    assert_eq!(flight_status(&taxi_fv), SimStatus::Connected);

    let taxi_out = engine.step(&taxi_fv, &cfg, 1, false);
    // Тайминг самого удара (нарастание фазы + touchdown fade-in) — предмет
    // отдельного, более точечного теста; здесь проверяем только то, что
    // конвейер корректно ПОМЕТИЛ состояние как активный стук о плиты, и что
    // кодирование в HID-кадр остаётся согласованным с посчитанной силой.
    assert!(taxi_out.effects.ground_thump_active);

    let taxi_frame = build_simapp_vibe_frame(
        WW_PID_URSA_MINOR_AIRBUS_L,
        0x02,
        14,
        taxi_out.joystick_intensity,
    );
    assert_eq!(taxi_frame[0], 0x02);
    assert_eq!(taxi_frame[8], taxi_out.joystick_intensity);

    // Background "cruise" rumble was intentionally removed (see rumble.rs), so
    // a plain in-flight state with no other effect condition met (bank below
    // threshold, no spoilers/stall/overspeed) must be silent.
    let takeoff_elems = elems_from_flight(120.0, 0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    let takeoff_fv = parse_main_elems(&takeoff_elems, false, cfg.ias_deadband_kn, "");
    assert_eq!(flight_status(&takeoff_fv), SimStatus::InFlight);

    let takeoff_out = engine.step(&takeoff_fv, &cfg, 1, false);
    assert_eq!(takeoff_out.joystick_intensity, 0);
}

#[test]
fn pipeline_flap_change_during_flight() {
    let cfg = RumbleConfig::default();
    let mut engine = RumbleEngine::new();

    let cruise = elems_from_flight(150.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0);
    let cruise_fv = parse_main_elems(&cruise, false, cfg.ias_deadband_kn, "");
    let _ = engine.step(&cruise_fv, &cfg, 1, false);

    let flaps = elems_from_flight(150.0, 0.0, 0.0, 50.0, 50.0, 2.0, 0.0, 0.0, 10.1, 0.0, 0.0);
    let flaps_fv = parse_main_elems(&flaps, false, cfg.ias_deadband_kn, "");
    let out = engine.step(&flaps_fv, &cfg, 1, false);

    assert!(out.effects.flaps_bump_active);
    let frame =
        build_simapp_vibe_frame(WW_PID_URSA_MINOR_AIRBUS_L, 0x02, 14, out.joystick_intensity);
    assert_eq!(frame[2], 0xBF);
    assert_eq!(frame[8], out.joystick_intensity);
}

#[test]
fn pipeline_flaps_pct_subpercent_jitter_does_not_sustain_hum_forever() {
    // Регрессия: на бортах типа MADDOG FLAPS PERCENT прыгает целиком за один
    // тик (0 -> 27). FLAPS HANDLE INDEX оказался ненадёжным источником для
    // ловли этого скачка (дребезжит на некоторых бортах и без конца продлевал
    // эффект — см. историю правок этого файла), поэтому детекция движения
    // теперь целиком построена на flaps_pct, округлённом до целого процента:
    // округление само гасит суб-процентный шум телеметрии, а сравнение по
    // проценту (а не по индексу защёлки) одинаково ловит и плавную анимацию,
    // и мгновенный скачок.
    let cfg = RumbleConfig::default();
    let mut engine = RumbleEngine::new();

    let closed = elems_from_flight(150.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let closed_fv = parse_main_elems(&closed, false, cfg.ias_deadband_kn, "");
    let _ = engine.step(&closed_fv, &cfg, 1, false);

    let jump = elems_from_flight(150.0, 0.0, 0.0, 27.0, 27.0, 0.0, 0.0, 0.0, 0.1, 0.0, 0.0);
    let jump_fv = parse_main_elems(&jump, false, cfg.ias_deadband_kn, "");
    let out = engine.step(&jump_fv, &cfg, 1, false);
    assert!(out.effects.flaps_bump_active);

    // flaps_pct дребезжит суб-процентным шумом вокруг 27% (26.6 <-> 27.4,
    // оба округляются к 27) каждый кадр в течение ~10 секунд симулированного
    // времени — заведомо дольше flaps_bump_duration_s (по умолчанию 1с) +
    // время затухания (5с). Реального движения здесь нет.
    let mut sim_time = 0.15;
    let mut still_active = true;
    for i in 0..200 {
        let noisy_pct = if i % 2 == 0 { 26.6 } else { 27.4 };
        let elems = elems_from_flight(
            150.0, 0.0, 0.0, noisy_pct, noisy_pct, 0.0, 0.0, 0.0, sim_time, 0.0, 0.0,
        );
        let fv = parse_main_elems(&elems, false, cfg.ias_deadband_kn, "");
        let out = engine.step(&fv, &cfg, 1, false);
        still_active = out.effects.flaps_bump_active;
        sim_time += 0.05;
    }

    assert!(
        !still_active,
        "flaps motor hum must eventually stop even while flaps_pct keeps jittering sub-percent at rest"
    );
}

#[test]
fn pipeline_maddog_slats_retract_after_flaps_reach_zero() {
    // Особенность MADDOG: предкрылки (slats) убираются отдельным, ПОСЛЕДНИМ
    // движением ручки закрылков — flaps_pct к этому моменту уже 0, и меняется
    // только slats_pct. Это реальная работа мотора, и она должна давать
    // motor-hum, но только когда борт распознан как MADDOG и профиль включил
    // cfg.flaps_track_slats (см. src/profiles.rs).
    let mut cfg = RumbleConfig::default();
    cfg.flaps_track_slats = true;
    let mut engine = RumbleEngine::new();

    // Закрылки и предкрылки уже некоторое время стоят неподвижно: flaps=0,
    // slats=27 (предкрылки ещё выпущены) — эффекта быть не должно.
    let steady_fv = FlightVars {
        flaps_pct: 0.0,
        slats_pct: 27.0,
        sim_time_s: 0.0,
        ..Default::default()
    };
    let _ = engine.step(&steady_fv, &cfg, 1, false);
    let steady_out = engine.step(&steady_fv, &cfg, 1, false);
    assert!(!steady_out.effects.flaps_bump_active);

    // Последний щелчок ручки: flaps_pct не меняется (уже 0), а slats_pct
    // падает в 0 — именно это движение должно поймать эффект.
    let retract_fv = FlightVars {
        flaps_pct: 0.0,
        slats_pct: 0.0,
        sim_time_s: 0.15,
        ..Default::default()
    };
    let out = engine.step(&retract_fv, &cfg, 1, false);
    assert!(
        out.effects.flaps_bump_active,
        "slats reaching 0 after flaps already at 0 must still trigger the motor hum on MADDOG"
    );
}

#[test]
fn pipeline_slats_change_ignored_without_flaps_track_slats() {
    // Без включённого профилем флага (обычный борт) движение slats_pct НЕ
    // должно влиять на эффект закрылков — иначе он ложно сработает на любом
    // самолёте, где leading edge flaps телеметрия просто существует.
    let cfg = RumbleConfig::default();
    assert!(!cfg.flaps_track_slats);
    let mut engine = RumbleEngine::new();

    let steady_fv = FlightVars {
        flaps_pct: 0.0,
        slats_pct: 27.0,
        sim_time_s: 0.0,
        ..Default::default()
    };
    let _ = engine.step(&steady_fv, &cfg, 1, false);
    let _ = engine.step(&steady_fv, &cfg, 1, false);

    let retract_fv = FlightVars {
        flaps_pct: 0.0,
        slats_pct: 0.0,
        sim_time_s: 0.15,
        ..Default::default()
    };
    let out = engine.step(&retract_fv, &cfg, 1, false);
    assert!(!out.effects.flaps_bump_active);
}

#[test]
fn pipeline_stall_ceiling() {
    let cfg = RumbleConfig::default();
    let mut engine = RumbleEngine::new();

    let stall_elems = elems_from_flight(80.0, 0.0, 30.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 0.0, 0.0);
    let stall_fv = parse_main_elems(&stall_elems, false, cfg.ias_deadband_kn, "");
    let out = engine.step(&stall_fv, &cfg, 1, false);

    assert!(out.effects.stall_active);
    assert!(out.joystick_intensity >= cfg.stall_ceiling as u8);
}

#[test]
fn pipeline_spoilers_symmetric_deployment_activates_effect() {
    let cfg = RumbleConfig::default();
    let mut engine = RumbleEngine::new();

    let fv = FlightVars {
        airspeed_indicated: 250.0,
        spoilers_pct: 100.0,
        spoilers_left_pct: 100.0,
        spoilers_right_pct: 100.0,
        ..Default::default()
    };
    let out = engine.step(&fv, &cfg, 1, false);

    assert!(out.effects.spoilers_active);
    assert!(out.joystick_intensity > 0);
}

#[test]
fn pipeline_spoilers_asymmetric_deployment_does_not_activate_effect() {
    // Репорт из реального полёта на TFDI MD-11: L=100%, R=40% (roll spoilers
    // при крене, рычаг спидбрейков не выпущен). min(L, R) = 40% всё ещё выше
    // порога срабатывания, поэтому одного min() недостаточно — нужна проверка
    // симметрии панелей, см. rumble.rs (SPOILERS_SYMMETRY_TOLERANCE_PCT).
    let cfg = RumbleConfig::default();
    let mut engine = RumbleEngine::new();

    let fv = FlightVars {
        airspeed_indicated: 250.0,
        spoilers_pct: 40.0,
        spoilers_left_pct: 100.0,
        spoilers_right_pct: 40.0,
        ..Default::default()
    };
    let out = engine.step(&fv, &cfg, 1, false);

    assert!(!out.effects.spoilers_active);
    assert_eq!(out.joystick_intensity, 0);
}

#[test]
fn pipeline_md11_panel_asymmetry_blocks_effect_even_when_standard_lr_looks_symmetric() {
    // На TFDI MD-11 SPOILERS LEFT/RIGHT POSITION сам по себе может выглядеть
    // симметрично (или просто некорректно отражать реальное состояние), но
    // собственные L-vars борта по секциям (см. скриншот из дебага) показывают
    // явный крен: одно крыло 29.511, другое 0.0. Эффект не должен срабатывать.
    let cfg = RumbleConfig::default();
    let mut engine = RumbleEngine::new();

    let fv = FlightVars {
        airspeed_indicated: 250.0,
        spoilers_pct: 100.0,
        spoilers_left_pct: 100.0,
        spoilers_right_pct: 100.0,
        spoilers_md11_left_avg: 29.511,
        spoilers_md11_right_avg: 0.0,
        ..Default::default()
    };
    let out = engine.step(&fv, &cfg, 1, false);

    assert!(!out.effects.spoilers_active);
    assert_eq!(out.joystick_intensity, 0);
}

#[test]
fn pipeline_pause_zeros_output() {
    let cfg = RumbleConfig::default();
    let mut engine = RumbleEngine::new();

    // Overspeed gives a clean "always active while above threshold" effect
    // (its internal oscillation never dips to exactly zero) to stand in for
    // the old background cruise rumble as the "something is active"
    // condition — background rumble itself was removed from the engine, and
    // ground/bank effects here are duty-cycled/timing-gated and would make
    // this test flaky depending on the exact sim_time_s sampled.
    let fv = FlightVars {
        airspeed_indicated: 250.0,
        overspeed_barber_pole_kn: 100.0,
        on_ground: false,
        sim_time_s: 1.0,
        ..Default::default()
    };
    let active = engine.step(&fv, &cfg, 1, false);
    assert!(active.joystick_intensity > 0);

    let paused_fv = FlightVars { paused: true, ..fv };
    let paused = engine.step(&paused_fv, &cfg, 1, false);
    assert_eq!(paused.joystick_intensity, 0);

    let held = engine.step(&fv, &cfg, 1, true);
    assert_eq!(held.joystick_intensity, 0);
}

#[test]
fn pipeline_gear_retraction_bump() {
    // gear_enabled по умолчанию false (фича временно скрыта из UI, см.
    // types.rs) — включаем явно, тест проверяет саму логику эффекта,
    // а не значение переключателя по умолчанию.
    let cfg = RumbleConfig {
        gear_enabled: true,
        ..RumbleConfig::default()
    };
    let mut engine = RumbleEngine::new();

    let gear_down = elems_from_flight(150.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 0.0, 0.0);
    let down_fv = parse_main_elems(&gear_down, false, cfg.ias_deadband_kn, "");
    let _ = engine.step(&down_fv, &cfg, 1, false);

    let gear_up = elems_from_flight(150.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 5.05, 0.0, 0.0);
    let up_fv = parse_main_elems(&gear_up, false, cfg.ias_deadband_kn, "");
    let out = engine.step(&up_fv, &cfg, 1, false);

    assert!(out.effects.gear_bump_active);
}

#[test]
fn pipeline_scripted_timeline_via_support() {
    let timeline = support::scripted_flight_timeline();
    let cfg = RumbleConfig::default();
    let mut engine = RumbleEngine::new();
    let mut intensities = Vec::new();

    for (elems, paused_events) in timeline {
        let fv = parse_main_elems(&elems, paused_events, cfg.ias_deadband_kn, "");
        let out = engine.step(&fv, &cfg, 1, false);
        intensities.push(out.joystick_intensity);
    }

    assert!(intensities.iter().any(|&i| i > 0));
    assert_eq!(intensities.last(), Some(&0));
}

#[test]
fn pipeline_frame_encoding_matches_intensity_at_each_step() {
    let cfg = RumbleConfig::default();
    let mut engine = RumbleEngine::new();
    let steps = [
        elems_from_flight(0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.1, 6.0, 0.0),
        elems_from_flight(100.0, 0.0, 10.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0),
    ];

    for elems in steps {
        let fv = parse_main_elems(&elems, false, cfg.ias_deadband_kn, "");
        let out = engine.step(&fv, &cfg, 1, false);
        let frame =
            build_simapp_vibe_frame(WW_PID_URSA_MINOR_AIRBUS_L, 0x02, 14, out.joystick_intensity);
        assert_eq!(frame[8], out.joystick_intensity);
    }
}

#[test]
fn config_shared_rev_triggers_smoothing_reset() {
    use aurora_vibra::ConfigShared;
    use std::sync::Arc;

    let shared = Arc::new(ConfigShared::new());
    let cfg1 = shared.get();
    let rev1 = shared.current_rev();

    shared.with_mut(|c| c.overspeed_intensity = 150.0);
    let rev2 = shared.current_rev();
    assert!(rev2 > rev1);

    let mut engine = RumbleEngine::new();
    let fv = FlightVars {
        airspeed_indicated: 250.0,
        overspeed_barber_pole_kn: 100.0,
        on_ground: false,
        sim_time_s: 1.0,
        ..Default::default()
    };

    let _ = engine.step(&fv, &cfg1, rev1, false);
    let out = engine.step(&fv, &shared.get(), rev2, false);
    assert!(out.joystick_intensity > 0);
}
