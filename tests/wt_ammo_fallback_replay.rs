//! Реплей уже записанных сессий War Thunder через новый фолбэк
//! `AmmoTracker::infer_firing_from_ammo_sum` (см. `wt_link::ammo`, план
//! `golden-squishing-lerdorf.md`): проверяет отсутствие регрессии на бортах
//! с обычными weapon1..weapon4 ключами и точность детекта на реальной
//! стрельбе, если эти ключи искусственно убрать (симуляция борта без них).

#![cfg(feature = "app")]

use std::fs;

use aurora_vibra::wt_link::ammo::AmmoTracker;
use aurora_vibra::wt_link::vars;
use serde_json::Value;

fn each_tick(raw: &str, mut f: impl FnMut(f64, &Value, &Value)) {
    let mut state = Value::Null;
    let mut indicators = Value::Null;
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
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
        f(t, &state, &indicators);
    }
}

#[test]
fn ammo_sum_fallback_never_misfires_on_recorded_sessions() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/wt_probe_sessions");
    let mut sessions_checked = 0usize;
    let mut total_ticks = 0usize;
    let mut fallback_active_ticks = 0usize;
    let mut fallback_inferred_firing_ticks = 0usize;

    for entry in fs::read_dir(dir).expect("wt_probe_sessions dir must exist") {
        let path = entry.expect("dir entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("session_") || !name.ends_with(".jsonl") {
            continue;
        }
        sessions_checked += 1;

        let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
        let mut ammo = AmmoTracker::new();

        each_tick(&raw, |t, state, indicators| {
            let wt_vars = vars::parse(t, state, indicators);
            if !wt_vars.in_mission {
                ammo.reset();
                return;
            }
            ammo.observe(indicators, wt_vars.weapon1_firing, wt_vars.weapon2_firing);

            let no_weapon_trigger_keys = ["weapon1", "weapon2", "weapon3", "weapon4"]
                .iter()
                .all(|k| indicators.get(*k).is_none());

            total_ticks += 1;
            if no_weapon_trigger_keys {
                fallback_active_ticks += 1;
                let inferred = ammo.infer_firing_from_ammo_sum(indicators);
                if inferred.weapon1 || inferred.weapon2 {
                    fallback_inferred_firing_ticks += 1;
                }
            }
        });
    }

    println!(
        "[ammo_sum_fallback regression] sessions_checked={sessions_checked} \
         total_ticks={total_ticks} fallback_active_ticks={fallback_active_ticks} \
         fallback_inferred_firing_ticks={fallback_inferred_firing_ticks}"
    );

    assert!(
        sessions_checked >= 5,
        "expected several recorded sessions to replay, found {sessions_checked}"
    );
    // Все реальные борта в этом корпусе отдают хотя бы один из
    // weapon1..weapon4 (см. vars::tests::falls_back_to_weapon3_4_...);
    // fallback_active_ticks > 0 бывает только на переходных тиках
    // рассинхрона /state и /indicators (респавн/загрузка — indicators
    // на миг возвращает форму {"valid": false} без единого поля), где
    // is_ammo_like_key ничего не находит — эффект не должен срабатывать
    // ни разу, даже если сам фолбэк формально активировался.
    assert_eq!(
        fallback_inferred_firing_ticks, 0,
        "fallback inferred firing during a transient no-weapon-key tick on a real session — investigate"
    );
}

#[test]
fn ammo_sum_fallback_detects_real_gunfire_when_weapon_keys_are_stripped() {
    // Симулируем борт без единого ключа weapon1..weapon4: берём реальную
    // сессию с настоящей стрельбой (Fw 190 A-4, см. docstring в ammo.rs про
    // ту же запись 2026-07-29 — 447 тиков с weapon1/2/3=1) и перед прогоном
    // через infer_firing_from_ammo_sum вырезаем из /indicators все
    // weapon-ключи — чтобы проверить, ловит ли суммовый фолбэк реальные
    // очереди по убыванию ammo_counterN без опоры на истинный флаг стрельбы.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/wt_probe_sessions/session_20260729_094638.jsonl"
    );
    let raw = fs::read_to_string(path).expect("fixture must exist");

    let mut ground_truth_ammo = AmmoTracker::new();
    let mut fallback_ammo = AmmoTracker::new();

    let mut ticks = 0usize;
    let mut ground_truth_firing_ticks = 0usize;
    let mut inferred_firing_ticks = 0usize;
    let mut true_positive_ticks = 0usize;
    let mut false_positive_ticks = 0usize;
    let mut weapon2_true_positive_ticks = 0usize;

    each_tick(&raw, |t, state, indicators| {
        let mut gt_vars = vars::parse(t, state, indicators);
        if !gt_vars.in_mission {
            ground_truth_ammo.reset();
            fallback_ammo.reset();
            return;
        }

        // "Земля правды": штатный путь worker.rs (реальные флаги + гейтинг
        // по нулю патронов) — то, что фолбэк должен приблизить, не видя
        // самих weapon-ключей.
        ground_truth_ammo.observe(indicators, gt_vars.weapon1_firing, gt_vars.weapon2_firing);
        if ground_truth_ammo.weapon1_empty(indicators) {
            gt_vars.weapon1_firing = false;
        }
        if ground_truth_ammo.weapon2_empty(indicators) {
            gt_vars.weapon2_firing = false;
        }
        let ground_truth_firing = gt_vars.weapon1_firing || gt_vars.weapon2_firing;

        let mut stripped = indicators.clone();
        if let Some(obj) = stripped.as_object_mut() {
            for k in ["weapon1", "weapon2", "weapon3", "weapon4"] {
                obj.remove(k);
            }
        }
        let inferred = fallback_ammo.infer_firing_from_ammo_sum(&stripped);
        let inferred_firing = inferred.weapon1 || inferred.weapon2;

        ticks += 1;
        if ground_truth_firing {
            ground_truth_firing_ticks += 1;
        }
        if inferred_firing {
            inferred_firing_ticks += 1;
            if ground_truth_firing {
                true_positive_ticks += 1;
            } else {
                false_positive_ticks += 1;
            }
        }
        if inferred.weapon2 && gt_vars.weapon2_firing {
            weapon2_true_positive_ticks += 1;
        }
    });

    println!(
        "[ammo_sum_fallback accuracy] ticks={ticks} ground_truth_firing_ticks={ground_truth_firing_ticks} \
         inferred_firing_ticks={inferred_firing_ticks} true_positive={true_positive_ticks} \
         false_positive={false_positive_ticks} weapon2_true_positive={weapon2_true_positive_ticks}"
    );

    assert!(
        ground_truth_firing_ticks > 0,
        "fixture must contain real gunfire to be a meaningful test"
    );
    assert!(
        inferred_firing_ticks > 0,
        "sum-based fallback failed to detect any firing at all on a session with real gunfire"
    );
    assert!(
        weapon2_true_positive_ticks > 0,
        "fallback never routed any tick to weapon2 on a session with real weapon2 (cannon) \
         gunfire — this is the exact bug being fixed (second weapon's ammo counter must be \
         able to reach weapon2_firing, not just weapon1)"
    );
    // "Земля правды" здесь — сам булев флаг, а он по документированному в
    // ammo.rs поведению API иногда отпускается на 1-2 тика РАНЬШЕ, чем
    // реально долетевший снаряд спишется со счётчика (проверено вручную на
    // этой самой записи: t=10.550731 weapon1=1.0, ammo_counter2=878;
    // t=10.600664 weapon1 уже 0.0, а ammo_counter2 только что стал 877).
    // На таких тиках фолбэк по сумме патронов оказывается ТОЧНЕЕ булева
    // флага, а не ошибается — поэтому здесь не жёсткий 0, а малый допуск,
    // соответствующий этому уже известному артефакту API, а не шуму от
    // посторонних полей.
    assert!(
        false_positive_ticks <= 5,
        "fallback inferred firing on {false_positive_ticks} ticks with no real gunfire — \
         more than the known flag/counter trailing-edge artifact accounts for, check keyword \
         matching for noise from unrelated fields"
    );
}
