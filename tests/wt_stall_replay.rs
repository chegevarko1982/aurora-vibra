//! Реплей реальной записанной сессии War Thunder через эффект срыва
//! потока/сваливания (см. план `cuddly-coalescing-llama.md`, §8.3, и
//! временный fallback в `wt_link::aero_profiles::match_profile`): чужой
//! самолёт (Bf 109 E-3, не F-4) теперь ОБЯЗАН получить профиль
//! `BF_109_F4` как универсальный fallback (временное решение — общий
//! профиль для всех бортов, пока нет отдельных профилей), и эффект может
//! сработать по его порогам. Проверяем, что реплей проходит без паники и
//! что fallback-профиль действительно применяется к чужому борту.

#![cfg(feature = "app")]

use std::fs;

use aurora_vibra::types::WtConfig;
use aurora_vibra::wt_link::aero_profiles;
use aurora_vibra::wt_link::rumble::WtRumbleState;
use aurora_vibra::wt_link::vars;
use serde_json::Value;

const SESSIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wt_probe_sessions");

/// Записанные сессии — локальные дев-захваты: они в .gitignore, весят десятки
/// мегабайт и в репозиторий не кладутся. На CI их нет, поэтому реплей-тесты не
/// падают, а громко пропускаются — у себя они по-прежнему гоняют полный корпус.
fn read_session(name: &str) -> Option<String> {
    let path = std::path::Path::new(SESSIONS_DIR).join(name);
    match fs::read_to_string(&path) {
        Ok(raw) => Some(raw),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIP: no recorded session at {}", path.display());
            None
        }
        Err(e) => panic!("read {}: {e}", path.display()),
    }
}

#[test]
fn bf109e3_session_replays_with_bf109f4_fallback_profile() {
    let Some(raw) = read_session("session_20260728_204134.jsonl") else {
        return;
    };

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
        // step() must not panic across the whole session with the fallback
        // profile applied — that's the extent of what this replay checks
        // (whether stall_active fires depends on the recorded AoA crossing
        // the F-4 thresholds, which is expected/desired now, not a bug).
        let _ = engine.step(&wt_vars, &cfg, false);
        ticks_replayed += 1;
    }

    assert!(
        ticks_replayed > 100,
        "expected a substantial recorded session, got {ticks_replayed} ticks"
    );
    assert_eq!(
        vehicle_type_seen, "bf-109e-3",
        "fixture is expected to be the Bf 109 E-3 session"
    );
    assert_eq!(
        aero_profiles::match_profile(&vehicle_type_seen),
        Some(&aero_profiles::BF_109_F4),
        "bf-109e-3 (not in the profile table) must fall back to BF_109_F4"
    );
}
