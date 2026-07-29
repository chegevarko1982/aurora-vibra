//! Реплей реальной записанной сессии War Thunder через эффект пуска/останова
//! двигателя (RPM 1). Проверяет на живых данных то, что нельзя надёжно
//! проверить синтетическими юнит-тестами: что эффект молчит на устойчивом
//! холостом ходу, отрабатывает и на прокрутке/раскрутке, и на выбеге, и что
//! на бортах с уже раскрученным на споне двигателем (другая сессия, другой
//! борт) ложного пуска не возникает.

#![cfg(feature = "app")]

use std::fs;

use aurora_vibra::types::WtConfig;
use aurora_vibra::wt_link::rumble::WtRumbleState;
use aurora_vibra::wt_link::vars;
use serde_json::Value;

struct Tick {
    t: f64,
    rpm_1: f64,
    engine_active: bool,
}

fn replay(path: &str) -> Vec<Tick> {
    let raw = fs::read_to_string(path).expect("recorded session fixture must exist");

    let cfg = WtConfig::default();
    let mut engine = WtRumbleState::new();

    let mut state = Value::Null;
    let mut indicators = Value::Null;
    let mut ticks = Vec::new();

    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: Value = serde_json::from_str(line).expect("each line must be valid JSON");
        let Some(endpoint) = entry.get("endpoint").and_then(Value::as_str) else {
            continue;
        };
        let t = entry.get("t").and_then(Value::as_f64).unwrap_or(0.0);
        let body = entry.get("body").cloned().unwrap_or(Value::Null);

        match endpoint {
            "state" => state = body,
            "indicators" => indicators = body,
            _ => continue,
        }

        if state.is_null() || indicators.is_null() {
            continue;
        }

        let wt_vars = vars::parse(t, &state, &indicators);
        let out = engine.step(&wt_vars, &cfg, false);
        ticks.push(Tick {
            t,
            rpm_1: wt_vars.rpm_1,
            engine_active: out.effects.engine_start_active,
        });
    }

    ticks
}

/// Живой лог полного цикла пуск→останов на Bf 109 F-4 (см. память
/// `wt_engine_start_effect_implemented` и план реализации): двигатель
/// заглушен до t≈27.9с, схватывание ≈30.3с, устойчивые холостые
/// 33.25–39.55с, команда "стоп" на t≈39.6с, выбег до t≈57.8с.
#[test]
fn bf109f4_full_start_stop_cycle_matches_expected_windows() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/wt_probe_sessions/session_20260729_151535.jsonl"
    );
    let ticks = replay(path);
    assert!(
        ticks.len() > 1000,
        "expected a substantial recorded session, got {} ticks",
        ticks.len()
    );

    // Двигатель заглушен до старта — эффект молчит.
    let before_start_silent = ticks
        .iter()
        .filter(|tk| tk.t < 27.0)
        .all(|tk| !tk.engine_active);
    assert!(before_start_silent, "must stay silent before t=27.0 (engine off)");

    // Где-то в окне пуска (прокрутка+схватывание+раскрутка, 28-33с) эффект
    // обязан сработать хотя бы раз.
    let fires_during_start = ticks
        .iter()
        .any(|tk| (28.0..33.0).contains(&tk.t) && tk.engine_active);
    assert!(fires_during_start, "engine effect never fired during 28-33s start window");

    // На устойчивых холостых (глубоко внутри плато 33.25-39.55, с запасом на
    // фейд после выхода на холостые) — тишина.
    let silent_during_steady_idle = ticks
        .iter()
        .filter(|tk| (36.0..39.5).contains(&tk.t))
        .all(|tk| !tk.engine_active);
    assert!(
        silent_during_steady_idle,
        "must be silent on steady idle (36.0-39.5s) — transients-only per design"
    );

    // Где-то в окне выбега (40-57с) эффект обязан сработать хотя бы раз.
    let fires_during_coast = ticks
        .iter()
        .any(|tk| (40.0..57.0).contains(&tk.t) && tk.engine_active);
    assert!(fires_during_coast, "engine effect never fired during 40-57s coast window");

    // После полной остановки винта (RPM=0 с запасом по времени) — тишина.
    let silent_after_full_stop = ticks
        .iter()
        .filter(|tk| tk.t > 59.0)
        .all(|tk| !tk.engine_active);
    assert!(silent_after_full_stop, "must be silent well after full stop (t>59.0)");
}

/// Регрессия на ложный пуск: борт может заспавниться в бою с уже
/// раскрученным двигателем (RPM=2301 на самом первом тике). Наивный
/// детектор "RPM ушёл с нуля" не должен принять это за пуск.
///
/// Окно проверки — ДО t=100с, не вся сессия: у этой записи реально есть
/// момент (t≈106.89–112.25с), где пилот убирает газ в полёте (throttle 1,
/// %→0, power 1, hp→0) и RPM честно проседает 2166→545 под аэродинамическим
/// сопротивлением — то же самое физическое явление, что и выбег на земле
/// (см. решение "если разбили в бою — это тоже выбег, физически верно" в
/// плане реализации). Эффект там СРАБАТЫВАЕТ, и это не баг, а корректное
/// поведение, поэтому в тест не заявляется молчание всей сессии.
#[test]
fn bf109e3_combat_session_no_false_start_on_already_running_engine() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/wt_probe_sessions/session_20260728_204134.jsonl"
    );
    let ticks = replay(path);
    assert!(ticks.len() > 100, "expected a substantial recorded session");
    assert!(
        ticks[0].rpm_1 > 1000.0,
        "fixture is expected to start with the engine already spun up"
    );
    assert!(
        ticks.iter().filter(|tk| tk.t < 100.0).all(|tk| !tk.engine_active),
        "must not fire a false start at mission spawn (checked well before the real \
         in-flight throttle-cut at t≈106.9s later in this recording)"
    );
}

/// Тот же регрессионный сценарий на другом борту с другими холостыми (595
/// вместо 485/2301) — подтверждает, что защита от ложного пуска не завязана
/// на конкретное число оборотов. Эта запись тоже содержит реальный
/// throttle-cut в полёте (t≈20.70–22.15с) — окно проверки до него.
#[test]
fn other_session_no_false_start_regardless_of_idle_rpm() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/wt_probe_sessions/session_20260729_094638.jsonl"
    );
    let ticks = replay(path);
    assert!(ticks.len() > 100, "expected a substantial recorded session");
    assert!(
        ticks[0].rpm_1 > 0.0,
        "fixture is expected to start with the engine already running"
    );
    assert!(
        ticks.iter().filter(|tk| tk.t < 20.0).all(|tk| !tk.engine_active),
        "must not fire a false start at mission spawn (checked well before the real \
         in-flight throttle-cut at t≈20.7s later in this recording)"
    );
}
