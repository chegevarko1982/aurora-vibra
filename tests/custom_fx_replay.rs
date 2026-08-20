//! Интеграционный тест на связку "рекордер -> загрузчик -> движок кастомных
//! эффектов" ЦЕЛИКОМ, а не по частям (в отличие от юнит-тестов в
//! `src/recorder.rs`, `src/custom_fx/player.rs`, `src/custom_fx/engine.rs`,
//! которые каждый бьют только свой модуль). Проверяем именно склейку: то, что
//! реально пишет `SessionRecorder::tick_flightvars`, реально читает
//! `custom_fx::player::load_session`, а результат реально прогоняется через
//! `CustomFxEngine::step` и даёт ожидаемую вибрацию в ожидаемые моменты.
//!
//! Запись синтезируется прямо в тесте (а не берётся из `wt_probe_sessions/`,
//! как `tests/wt_*_replay.rs`) намеренно: те тесты читают реальные захваты,
//! которых нет в репозитории (см. `.gitignore` — файлы весят десятки
//! мегабайт), и поэтому на CI молча пропускаются (`SKIP`, см. `read_session`
//! там же). Этот тест обязан выполняться ВСЕГДА и без внешних файлов, поэтому
//! он сам порождает синтетическую телеметрию и прогоняет её через настоящий
//! `SessionRecorder` (не пишет JSONL руками — иначе не проверялся бы реальный
//! формат записи).

#![cfg(feature = "app")]

use std::path::{Path, PathBuf};

use aurora_vibra::LogBuffer;
use aurora_vibra::custom_fx::engine::CustomFxEngine;
use aurora_vibra::custom_fx::model::{
    CustomEffect, OutputRouting, ResponseCurve, Shape, Trigger, new_effect,
};
use aurora_vibra::custom_fx::player::{self, Session, load_session};
use aurora_vibra::custom_fx::sources::{self, SourceId, TelemetryFrame};
use aurora_vibra::recorder::SessionRecorder;
use aurora_vibra::types::{ActiveGame, FlightVars};

/// `SessionRecorder` сам решает, куда писать файл (см. приватную
/// `sessions_dir()` в `src/recorder.rs`: рядом с exe / `%LOCALAPPDATA%` /
/// temp — тот же приоритет путей, что и у публичной
/// `custom_fx::player::sessions_dir()`, которой мы и пользуемся ниже, чтобы
/// найти результат). Тест не может передать рекордеру произвольный путь,
/// поэтому вместо этого делает записываемое имя файла уникальным: маркер игры
/// попадает в имя (`session_<marker>_<ts>.jsonl`, см. `SessionRecorder::start`)
/// — `process::id()` отличает параллельные прогоны `cargo test` друг от
/// друга, суффикс с именем теста отличает тесты этого файла друг от друга
/// внутри одного процесса.
fn unique_marker(test_name: &str) -> String {
    format!("customfxreplay{}_{test_name}", std::process::id())
}

fn find_marker_file(dir: &Path, marker: &str) -> Option<PathBuf> {
    let prefix = format!("session_{marker}_");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
        })
}

/// Пишет кадры через настоящий `SessionRecorder::tick_flightvars` (никакого
/// ручного JSONL — иначе тест проверял бы придуманный формат, а не тот, что
/// реально льётся на диск из воркеров), затем читает их обратно через
/// `custom_fx::player::load_session`. Файл удаляется СРАЗУ после чтения — до
/// всех проверок ниже (та же последовательность, что в
/// `src/custom_fx/player.rs`, `mod tests`), — поэтому он не остаётся на диске
/// даже при падении assert'а дальше по тесту.
fn record_and_load(marker: &str, frames: &[(f64, FlightVars)]) -> Session {
    assert!(!frames.is_empty(), "нужен хотя бы один кадр");
    let logs = LogBuffer::default();
    let mut recorder = SessionRecorder::new();
    let dir = player::sessions_dir();
    let _ = std::fs::create_dir_all(&dir);

    for (t, fv) in frames {
        recorder.tick_flightvars(true, *t, fv, marker, &logs);
    }
    // Закрывающий тик с enabled=false — фронт, на который `sync_enabled`
    // реально закрывает и флашит файл (см. `SessionRecorder::stop`); сам он
    // новую строку не пишет (writer уже `None` к моменту `write_line`).
    let (last_t, last_fv) = frames.last().unwrap();
    recorder.tick_flightvars(false, *last_t, last_fv, marker, &logs);

    let path = find_marker_file(&dir, marker).unwrap_or_else(|| {
        panic!("recorder should have written a session file for marker {marker}")
    });
    let session = load_session(&path).expect("load_session on freshly recorded file");
    let _ = std::fs::remove_file(&path);
    session
}

fn flight_airspeed(frame: &TelemetryFrame) -> f64 {
    match frame {
        TelemetryFrame::Flight(fv) => fv.airspeed_indicated,
        TelemetryFrame::Wt(_) => panic!("ожидался Flight-кадр (MSFS/X-Plane), получен WT"),
    }
}

/// Разгон 0 -> `v_end_kn` узлов за `duration_s` секунд с шагом опроса
/// `hz` Гц (20 Гц — тот же ритм, с которым реальные конвейеры MSFS/X-Plane
/// тикают телеметрию, см. `sim::worker`/`xp_link::worker`). `sim_time_s`
/// заполняется тем же значением, что и `t` записи — в реальном полёте это
/// одно и то же время симулятора.
fn synth_accel_frames(duration_s: f64, hz: f64, v_end_kn: f64) -> Vec<(f64, FlightVars)> {
    let dt = 1.0 / hz;
    let steps = (duration_s / dt).round() as usize;
    (0..=steps)
        .map(|i| {
            let t = i as f64 * dt;
            let fv = FlightVars {
                sim_time_s: t,
                airspeed_indicated: (t / duration_s) * v_end_kn,
                on_ground: false,
                ..Default::default()
            };
            (t, fv)
        })
        .collect()
}

fn overspeed_effect(curve: ResponseCurve) -> CustomEffect {
    let mut effect = new_effect("Overspeed".into(), SourceId::FlightAirspeedKn);
    effect.trigger = Trigger::Above {
        value: 250.0,
        hysteresis: 5.0,
    };
    effect.shape = Shape::Constant;
    effect.curve = curve;
    effect.out = OutputRouting {
        joystick: true,
        throttle_left: false,
        throttle_right: false,
    };
    effect.strength_pct = 100.0;
    effect
}

/// Сценарий "разгон и превышение скорости": разгон 0 -> 400 узлов за 60с при
/// 20Гц опроса, записанный на диск настоящим рекордером и прочитанный
/// обратно, затем прогнанный через движок с эффектом-порогом на 250 узлов.
#[test]
fn accel_runway_overspeed_replay_grows_after_threshold() {
    let marker = unique_marker("overspeed");
    let frames_in = synth_accel_frames(60.0, 20.0, 400.0);
    let session = record_and_load(&marker, &frames_in);

    // Шаг 3 задачи: число кадров и значения (t, airspeed, on_ground) должны
    // пережить запись+чтение без потерь.
    assert_eq!(session.frames.len(), frames_in.len());
    for (recorded, (t, fv)) in session.frames.iter().zip(frames_in.iter()) {
        assert!((recorded.t - t).abs() < 1e-9);
        match &recorded.frame {
            TelemetryFrame::Flight(rfv) => {
                assert!((rfv.airspeed_indicated - fv.airspeed_indicated).abs() < 1e-6);
                assert!((rfv.sim_time_s - fv.sim_time_s).abs() < 1e-9);
                assert_eq!(rfv.on_ground, fv.on_ground);
            }
            TelemetryFrame::Wt(_) => panic!("ожидался Flight-кадр"),
        }
    }

    // Кривая начинается ровно на пороге триггера (250) — ниже порога триггер
    // и так неактивен (Above строго ">"), поэтому раньше 250 эффект молчит
    // независимо от домена кривой; выше порога кривая честно растёт к 400.
    let effect = overspeed_effect(ResponseCurve::linear(250.0, 400.0));
    let effect_id = effect.id.clone();
    let effects = vec![effect];

    let mut engine = CustomFxEngine::new();
    let mut joystick_outputs = Vec::with_capacity(session.frames.len());
    let mut first_active_idx: Option<usize> = None;

    for (i, frame) in session.frames.iter().enumerate() {
        let out = engine.step(
            &frame.frame,
            frame.t,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );

        // Шаг 5, последний пункт: моторы вне OutputRouting молчат всегда.
        assert_eq!(out.throttle_left, 0, "throttle_left не в маршрутизации");
        assert_eq!(out.throttle_right, 0, "throttle_right не в маршрутизации");

        let speed = flight_airspeed(&frame.frame);
        if speed <= 250.0 {
            assert_eq!(
                out.joystick, 0,
                "t={} v={speed} — ниже/на пороге, должен молчать",
                frame.t
            );
            assert!(
                out.active_ids.is_empty(),
                "t={} v={speed} — active_ids должен быть пуст",
                frame.t
            );
        } else {
            // Выше порога active_ids обязан включать эффект — это внутренний
            // флаг "эффект даёт ненулевую (в f64, до округления в u8)
            // интенсивность", см. `CustomFxEngine::step`. У самой границы (в
            // пределах пары тиков после срабатывания) сырая интенсивность
            // настолько мала, что округляется в u8 до 0 — active_ids уже
            // взведён, а байтовый выход ещё 0. Это не баг теста: чтобы не
            // завязываться на этот эффект округления, "должна быть РЕАЛЬНАЯ
            // вибрация" проверяем только с запасом (>250 + 2 узла), а не в
            // каждом тике подряд.
            assert!(
                out.active_ids.contains(&effect_id),
                "t={} v={speed} — active_ids должен включать эффект выше порога",
                frame.t
            );
            first_active_idx.get_or_insert(i);
            if speed > 252.0 {
                assert!(
                    out.joystick > 0,
                    "t={} v={speed} — выше порога с запасом обязана быть реальная вибрация",
                    frame.t
                );
            }
        }
        joystick_outputs.push(out.joystick);
    }

    let first_idx =
        first_active_idx.expect("порог 250 узлов должен был быть пройден за 60с разгона");
    let last = *joystick_outputs.last().unwrap();
    assert!(
        last > joystick_outputs[first_idx],
        "интенсивность должна расти вместе со скоростью: сразу после срабатывания {} -> в конце записи {last}",
        joystick_outputs[first_idx]
    );
    assert_eq!(last, 255, "на v=400kn (верхняя точка кривой) — полная сила");
}

/// Гистерезис `Trigger::Above` на коротком разгоне-торможении, записанном и
/// прочитанном через ту же связку рекордер -> плеер, что и первый тест.
/// Кривая отклика намеренно НЕ совпадает с порогом триггера (0..400, а не
/// 250..400): иначе при откате скорости ниже 250 (но всё ещё внутри полосы
/// гистерезиса) выход обнулился бы просто из-за домена кривой, а не из-за
/// реального состояния триггера — тест проверял бы не то, что заявлено.
#[test]
fn hysteresis_survives_record_and_replay() {
    let marker = unique_marker("hysteresis");
    let mk = |t: f64, speed: f64| {
        (
            t,
            FlightVars {
                sim_time_s: t,
                airspeed_indicated: speed,
                on_ground: false,
                ..Default::default()
            },
        )
    };
    let frames_in = vec![
        mk(0.0, 200.0), // ниже порога — молчит
        mk(1.0, 260.0), // выше порога (250) — взводится
        mk(2.0, 248.0), // откат, но внутри полосы гистерезиса (245..250) — держится
        mk(3.0, 244.0), // ниже полосы (245) — гаснет
    ];
    let session = record_and_load(&marker, &frames_in);
    assert_eq!(session.frames.len(), frames_in.len());

    let effect = overspeed_effect(ResponseCurve::linear(0.0, 400.0));
    let effects = vec![effect];
    let mut engine = CustomFxEngine::new();

    let step_at = |engine: &mut CustomFxEngine, idx: usize| {
        engine.step(
            &session.frames[idx].frame,
            session.frames[idx].t,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        )
    };

    let out0 = step_at(&mut engine, 0);
    assert_eq!(out0.joystick, 0, "200kn — ниже порога, молчит");

    let out1 = step_at(&mut engine, 1);
    assert!(out1.joystick > 0, "260kn — выше порога, сработал");

    let out2 = step_at(&mut engine, 2);
    assert!(
        out2.joystick > 0,
        "248kn — внутри полосы гистерезиса (245..250), НЕ должен погаснуть"
    );

    let out3 = step_at(&mut engine, 3);
    assert_eq!(
        out3.joystick, 0,
        "244kn — ниже полосы гистерезиса (245), должен погаснуть"
    );
}

/// Эффект, настроенный на источник War Thunder (`WtAoaDeg`), не должен
/// подавать никаких признаков жизни на записи MSFS: `sources::read` обязан
/// вернуть `None` (кадра этой игры просто нет), а не молча подставить 0.0,
/// который выглядел бы как настоящее (и, например, ложно прошёл бы триггер
/// `Below`).
#[test]
fn wt_source_is_silent_on_msfs_recording() {
    let marker = unique_marker("wtsourceonmsfs");
    let frames_in = vec![
        (
            0.0,
            FlightVars {
                sim_time_s: 0.0,
                airspeed_indicated: 100.0,
                ..Default::default()
            },
        ),
        (
            1.0,
            FlightVars {
                sim_time_s: 1.0,
                airspeed_indicated: 300.0,
                ..Default::default()
            },
        ),
    ];
    let session = record_and_load(&marker, &frames_in);
    assert_eq!(session.frames.len(), 2);

    for f in &session.frames {
        assert_eq!(
            sources::read(SourceId::WtAoaDeg, &f.frame),
            None,
            "WT-источник на MSFS-кадре обязан быть None, а не Some(0.0)"
        );
    }

    let mut effect = new_effect("WT AoA on MSFS".into(), SourceId::WtAoaDeg);
    // Даже Trigger::Always не должен помочь — до trigger дело не доходит:
    // `sources::read` возвращает None раньше, и `CustomFxEngine::step`
    // пропускает эффект целиком (см. `let Some(raw) = sources::read(...) else
    // { continue; }`).
    effect.trigger = Trigger::Always;
    effect.shape = Shape::Constant;
    effect.curve = ResponseCurve::linear(0.0, 25.0);
    effect.out = OutputRouting {
        joystick: true,
        throttle_left: true,
        throttle_right: true,
    };
    effect.strength_pct = 100.0;
    let effects = vec![effect];

    let mut engine = CustomFxEngine::new();
    for f in &session.frames {
        let out = engine.step(&f.frame, f.t, &effects, 1, "", ActiveGame::Msfs, false, 255);
        assert_eq!(out.joystick, 0);
        assert_eq!(out.throttle_left, 0);
        assert_eq!(out.throttle_right, 0);
        assert!(
            out.active_ids.is_empty(),
            "WT-эффект не должен считаться активным на MSFS-записи"
        );
    }
}
