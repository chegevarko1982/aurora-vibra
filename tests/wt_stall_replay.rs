//! Реплей реальной записанной сессии War Thunder через новый эффект срыва
//! потока/сваливания (см. план `cuddly-coalescing-llama.md`, §8.3): проверяет,
//! что чужой самолёт (Bf 109 E-3, не F-4) НЕ триггерит захардкоженный профиль
//! `wt_link::aero_profiles::BF_109_F4`, независимо от того, какой AoA он
//! показывает в записи — эффект должен молчать всю сессию.

#![cfg(feature = "app")]

use std::fs;

use aurora_vibra::wt_link::rumble::WtRumbleState;
use aurora_vibra::wt_link::vars;
use aurora_vibra::types::WtConfig;
use serde_json::Value;

#[test]
fn bf109e3_session_never_triggers_bf109f4_stall_profile() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/wt_probe_sessions/session_20260728_204134.jsonl"
    );
    let raw = fs::read_to_string(path).expect("recorded session fixture must exist");

    let cfg = WtConfig::default();
    let mut engine = WtRumbleState::new();

    let mut state = Value::Null;
    let mut indicators = Value::Null;
    let mut ticks_replayed = 0usize;
    let mut vehicle_type_seen = String::new();

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
            _ => continue, // map_obj и т.п. — не относятся к вычислению эффектов
        }

        if state.is_null() || indicators.is_null() {
            continue; // ещё не встретили хотя бы по одному телу каждого вида
        }

        let wt_vars = vars::parse(t, &state, &indicators);
        if !wt_vars.vehicle_type.is_empty() {
            vehicle_type_seen = wt_vars.vehicle_type.clone();
        }
        let out = engine.step(&wt_vars, &cfg, false);
        ticks_replayed += 1;

        assert!(
            !out.effects.stall_active,
            "stall effect fired at t={t} on vehicle_type={:?} (aoa_deg={}, ias_kmh={}) — \
             BF_109_F4 profile must not match a different aircraft",
            wt_vars.vehicle_type, wt_vars.aoa_deg, wt_vars.ias_kmh
        );
    }

    assert!(ticks_replayed > 100, "expected a substantial recorded session, got {ticks_replayed} ticks");
    assert_eq!(vehicle_type_seen, "bf-109e-3", "fixture is expected to be the Bf 109 E-3 session");
}
