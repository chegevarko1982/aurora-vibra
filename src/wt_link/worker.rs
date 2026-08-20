//! Фоновый поток War Thunder (этап 1): опрашивает `/state` + `/indicators`
//! (localhost:8111, тот же `WtClient`, что и recon-инструмент, см.
//! `super::http`), считает эффекты через `WtRumbleState::step` и шлёт
//! `HidCmd` в тот же канал, что и `sim::sim_worker`. Оба конвейера
//! всегда опрашивают свою игру (никакого ручного флага-гейта), а
//! взаимоисключающее владение HID-каналом/GUI арбитрируется через
//! `crate::game_state::GameSlot` — липкое владение, первый заявивший игру
//! владеет ей, пока сам её не отпустит (см. game_state.rs).
//!
//! Живость WT определяется через `Liveness` по результату опроса
//! `/state`+`/indicators` — грейс-период переживает кратковременные провалы
//! (например, долгий чёрный экран загрузки миссии), не отпуская слот сразу.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::aircraft_profiles::{self, AircraftProfiles};
use crate::custom_fx::engine::CustomFxEngine;
use crate::custom_fx::sources::TelemetryFrame;
use crate::custom_fx::store::CustomFxShared;
use crate::game_state::{GameSlot, Liveness, PreviewLock};
use crate::profiles::ProfileState;
use crate::recorder::SessionRecorder;
use crate::wt_link::ammo::AmmoTracker;
use crate::wt_link::http::WtClient;
use crate::wt_link::rumble::WtRumbleState;
use crate::wt_link::vars::{self, WtVars};
use crate::wt_link::weapon_profiles;
use crate::{ActiveGame, ConfigShared, EffectMode, EffectsShared, HidCmd, LogBuffer, SimStatus};

/// Ритм опроса /state и /indicators, пока WT жив — 20 Гц, как дефолт
/// recon-инструмента (wt_probe/cli.rs) и как частота отправки HID в
/// hid/worker.rs.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Ритм опроса, пока WT ещё не отвечал (или перестал отвечать) — реже, чтобы
/// не долбить localhost:8111 20 раз в секунду, когда игра не запущена.
const PROBE_INTERVAL: Duration = Duration::from_millis(1000);
const HTTP_TIMEOUT: Duration = Duration::from_millis(200);
/// Тот же грейс-период, что у MSFS-вотчдога (sim/worker.rs) — для
/// консистентности дебаунса между двумя конвейерами.
const GRACE_PERIOD: Duration = Duration::from_millis(2500);

#[allow(clippy::too_many_arguments)]
pub fn wt_worker(
    last_wt_vars: Arc<Mutex<Option<WtVars>>>,
    tx_hid: Sender<HidCmd>,
    logs: LogBuffer,
    config: Arc<ConfigShared>,
    effects: EffectsShared,
    hold: Arc<AtomicBool>,
    status: Arc<Mutex<SimStatus>>,
    aircraft_title: Arc<Mutex<String>>,
    aircraft_profiles: Arc<Mutex<AircraftProfiles>>,
    profile_state: Arc<Mutex<ProfileState>>,
    game: GameSlot,
    recording: Arc<AtomicBool>,
    custom_fx: Arc<CustomFxShared>,
    active_custom_ids: Arc<Mutex<Vec<String>>>,
    preview: PreviewLock,
) {
    logs.push("WT: worker started, polling localhost:8111");

    let session_start = Instant::now();
    let mut engine = WtRumbleState::new();
    // Взаимоисключающий движок пользовательских эффектов (см.
    // custom_fx::engine) — своё состояние независимое от `engine` выше,
    // сбрасывается отдельно (потеря владения слотом / смена режима).
    let mut custom_engine = CustomFxEngine::new();
    let mut ammo = AmmoTracker::new();
    let mut recorder = SessionRecorder::new();
    let mut client: Option<WtClient> = None;
    let mut liveness = Liveness::new(GRACE_PERIOD);
    // Режим предыдущего тика — ловим МОМЕНТ переключения BuiltIn<->Custom для
    // разового сброса (см. точку отправки HID ниже), не сравниваем впустую.
    let mut last_effect_mode = crate::settings::effect_mode();
    // Фронт захвата PreviewLock — один раз нули на моторы в момент захвата,
    // дальше молчим, пока не отпустят (см. game_state::PreviewLock).
    let mut preview_was_held = false;
    // Зануляем HID/эффекты ровно один раз на переходе владения true→false —
    // не на каждом тике `!owns` (см. game_state.rs docstring): проигравший
    // try_claim не шлёт нули каждый тик, чтобы не гоняться наперегонки с
    // текущим владельцем за один и тот же tx_hid.
    let mut was_owner = false;

    loop {
        let c = match &client {
            Some(c) => c.clone(),
            None => match WtClient::new("127.0.0.1", 8111, HTTP_TIMEOUT) {
                Ok(c) => {
                    client = Some(c.clone());
                    c
                }
                Err(e) => {
                    logs.push(format!("WT: failed to build HTTP client: {e}"));
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            },
        };

        let t = session_start.elapsed().as_secs_f64();
        let state = c.state();
        let indicators = c.indicators();
        let ok = state.is_ok() && indicators.is_ok();

        // Форс-оверрайд встроен в тот же арбитраж, не отдельная ветка: если
        // пользователь прижал другую игру (MSFS или X-Plane), WT ведёт себя
        // как "не жив" независимо от реальной живости — release, не
        // try_claim. См. GameOverride::vetoes в types.rs.
        let vetoed = crate::settings::game_override().vetoes(ActiveGame::Wt);
        let alive = liveness.observe(ok, Instant::now());
        let owns = if alive && !vetoed {
            game.try_claim(ActiveGame::Wt)
        } else {
            game.release_if_owned(ActiveGame::Wt);
            false
        };

        if was_owner && !owns {
            logs.push("WT: lost connection, releasing slot");
            *last_wt_vars.lock() = None;
            effects.clear_all();
            let _ = tx_hid.send(HidCmd::SendIntensity {
                joystick: 0,
                throttle_left: 0,
                throttle_right: 0,
            });
            engine.reset();
            custom_engine.reset();
            active_custom_ids.lock().clear();
            ammo.reset();
            // Отдаём табличку с названием борта обратно MSFS-конвейеру — тот
            // сам выставит её при следующем подключении SimConnect (см.
            // sim/worker.rs), а до тех пор пусть будет пустой, а не залипший
            // борт War Thunder.
            *aircraft_title.lock() = String::new();
        } else if owns && !was_owner {
            logs.push("WT: connection alive, claimed slot");
        }
        was_owner = owns;

        match (state, indicators) {
            (Ok(state_v), Ok(indicators_v)) => {
                recorder.tick_wt(
                    recording.load(Ordering::Relaxed),
                    t,
                    &state_v,
                    &indicators_v,
                    &logs,
                );

                let mut wt_vars = vars::parse(t, &state_v, &indicators_v);
                if !wt_vars.in_mission {
                    // В ангаре/меню может смениться борт — забываем, какие
                    // ammo_counterN относились к weapon1/weapon2 прошлого
                    // самолёта, чтобы не гейтить по чужой раскладке стволов.
                    ammo.reset();
                } else {
                    ammo.observe(
                        &indicators_v,
                        wt_vars.weapon1_firing,
                        wt_vars.weapon2_firing,
                    );
                    if let Some(profile) =
                        weapon_profiles::match_weapon_profile(&wt_vars.vehicle_type)
                    {
                        ammo.set_weapon_capacity_hint(
                            profile.weapon1_ammo_capacity,
                            profile.weapon2_ammo_capacity,
                        );
                    }
                    if ammo.weapon1_empty(&indicators_v) {
                        wt_vars.weapon1_firing = false;
                    }
                    if ammo.weapon2_empty(&indicators_v) {
                        wt_vars.weapon2_firing = false;
                    }
                    wt_vars.weapon1_ammo = ammo.weapon1_ammo(&indicators_v);
                    wt_vars.weapon2_ammo = ammo.weapon2_ammo(&indicators_v);

                    // Борта без единого ключа weapon1..weapon4 в /indicators
                    // (не только weapon1/2 отсутствуют, но и weapon3/4 — иначе
                    // сработал бы OR-фолбэк в vars::parse): тогда стрельбу не
                    // определить по флагам вообще, и AmmoTracker::observe не
                    // может ничему обучиться (ему самому нужен истинный
                    // weapon1_firing/weapon2_firing хотя бы раз). Единственный
                    // сигнал — убывание похожих на боеприпасы полей;
                    // infer_firing_from_ammo_sum сам разводит их по weapon1/2
                    // (см. doc-комментарий ammo.rs) — раньше здесь всегда
                    // писалось в weapon1, из-за чего второе оружие на бортах
                    // с двумя типами боеприпасов молчало.
                    let no_weapon_trigger_keys = ["weapon1", "weapon2", "weapon3", "weapon4"]
                        .iter()
                        .all(|k| indicators_v.get(*k).is_none());
                    if no_weapon_trigger_keys {
                        let inferred = ammo.infer_firing_from_ammo_sum(&indicators_v);
                        wt_vars.weapon1_firing = inferred.weapon1;
                        wt_vars.weapon2_firing = inferred.weapon2;
                    }
                }

                if owns {
                    *status.lock() = if wt_vars.in_mission {
                        SimStatus::InFlight
                    } else {
                        SimStatus::Connected
                    };

                    // Показываем технику War Thunder в том же месте верхней
                    // панели, где MSFS-конвейер показывает aircraft_title (см.
                    // ui.rs) — переиспользуем то же поле и ту же систему
                    // именных профилей (aircraft_profiles::apply_for_aircraft),
                    // а не заводим отдельный WT-only виджет.
                    if wt_vars.in_mission && !wt_vars.vehicle_type.is_empty() {
                        let changed = *aircraft_title.lock() != wt_vars.vehicle_type;
                        if changed {
                            *aircraft_title.lock() = wt_vars.vehicle_type.clone();
                            aircraft_profiles::apply_for_aircraft(
                                &mut aircraft_profiles.lock(),
                                &config,
                                &mut profile_state.lock(),
                                &wt_vars.vehicle_type,
                                &logs,
                            );
                        }
                    }

                    if preview.is_held() {
                        // Редактор эффектов держит HID-канал под предпросмотр
                        // (см. game_state::PreviewLock) — воркер молчит, иначе
                        // оба источника 20 раз в секунду переписывали бы друг
                        // друга на моторе. На ФРОНТЕ захвата шлём нули один
                        // раз, чтобы не застыло последнее значение движка.
                        if !preview_was_held {
                            let _ = tx_hid.send(HidCmd::SendIntensity {
                                joystick: 0,
                                throttle_left: 0,
                                throttle_right: 0,
                            });
                            preview_was_held = true;
                        }
                    } else {
                        preview_was_held = false;

                        // Встроенный и пользовательский движки эффектов —
                        // ВЗАИМОИСКЛЮЧАЮЩИЕ (см. doc-комментарий types::EffectMode):
                        // два независимых движка на одних и тех же трёх моторах
                        // давали бы непредсказуемое наложение, поэтому здесь именно
                        // ВЫБОР считающего движка, а не смешивание их выходов. При
                        // смене режима на лету — разовый сброс состояния ОБОИХ
                        // движков и кадр нулей: без него эффект, активный в момент
                        // переключения, застыл бы на моторе последним значением (тот
                        // же приём, что уже используется выше при потере владения
                        // слотом).
                        let full_cfg = config.get();
                        let mode_now = crate::settings::effect_mode();
                        if mode_now != last_effect_mode {
                            engine.reset();
                            custom_engine.reset();
                            effects.clear_all();
                            active_custom_ids.lock().clear();
                            let _ = tx_hid.send(HidCmd::SendIntensity {
                                joystick: 0,
                                throttle_left: 0,
                                throttle_right: 0,
                            });
                            last_effect_mode = mode_now;
                        }

                        match mode_now {
                            EffectMode::BuiltIn => {
                                let out = engine.step(
                                    &wt_vars,
                                    &full_cfg.wt,
                                    hold.load(Ordering::Relaxed),
                                );
                                *last_wt_vars.lock() = Some(wt_vars);
                                effects.apply_snapshot(&out.effects);
                                active_custom_ids.lock().clear();
                                let _ = tx_hid.send(HidCmd::SendIntensity {
                                    joystick: out.joystick_intensity,
                                    throttle_left: out.throttle_left_intensity,
                                    throttle_right: out.throttle_right_intensity,
                                });
                            }
                            EffectMode::Custom => {
                                let t_now = wt_vars.t;
                                let aircraft = wt_vars.vehicle_type.clone();
                                *last_wt_vars.lock() = Some(wt_vars.clone());
                                let frame = TelemetryFrame::Wt(wt_vars);
                                let custom_effects = custom_fx.get();
                                let out = custom_engine.step(
                                    &frame,
                                    t_now,
                                    &custom_effects,
                                    custom_fx.current_rev(),
                                    &aircraft,
                                    ActiveGame::Wt,
                                    hold.load(Ordering::Relaxed),
                                    full_cfg.max_output,
                                );
                                // Встроенный EffectsSnapshot в режиме Custom
                                // заведомо пуст — Live Monitor подсвечивает
                                // активность через active_custom_ids, а не
                                // через effects.
                                effects.clear_all();
                                *active_custom_ids.lock() = out.active_ids;
                                let _ = tx_hid.send(HidCmd::SendIntensity {
                                    joystick: out.joystick,
                                    throttle_left: out.throttle_left,
                                    throttle_right: out.throttle_right,
                                });
                            }
                        }
                    }
                }
            }
            _ => {
                // Игра не запущена / порт закрыт — то же самое молчание, что
                // и recon-инструмент показывает как ConnStatus::Disconnected.
                if owns {
                    *status.lock() = SimStatus::Disconnected;
                    *last_wt_vars.lock() = None;
                    effects.clear_all();
                    active_custom_ids.lock().clear();
                    let _ = tx_hid.send(HidCmd::SendIntensity {
                        joystick: 0,
                        throttle_left: 0,
                        throttle_right: 0,
                    });
                }
            }
        }

        thread::sleep(if alive { POLL_INTERVAL } else { PROBE_INTERVAL });
    }
}
