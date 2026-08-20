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
use crate::{ActiveGame, ConfigShared, EffectsShared, HidCmd, LogBuffer, SimStatus};

/// Ритм ОПРОСА /state и /indicators (блокирующий HTTP-запрос) — держим
/// примерно на прежней частоте (20 Гц, как дефолт recon-инструмента,
/// wt_probe/cli.rs): HTTP медленный и блокирующий, гонять его на такте
/// движка/HID (см. TICK_INTERVAL ниже) нельзя. Тик движка развязан от этого
/// ритма — между двумя реальными опросами сети движок тикает на последнем
/// полученном кадре телеметрии чаще, с продвинутым временем (см.
/// `subtick_offsets` и подтиковый цикл в `wt_worker`).
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Такт тика движка/отправки на HID ВНУТРИ одного кадра сетевого опроса —
/// тот же, что у `hid::worker::SEND_INTERVAL` и у `xp_link::worker`. Форма
/// сигнала (ШИМ стрельбы, удары, пульсация) считается движком от времени
/// (`WtVars::t`), не от факта прихода нового кадра телеметрии, поэтому
/// учащение тика делает её гладкой даже на кадре сети, которому уже
/// несколько подтиков.
const TICK_INTERVAL: Duration = Duration::from_millis(20);
/// Минимальный шаг времени между двумя подряд идущими тиками движка —
/// страховка монотонности (см. `last_tick_t` в `wt_worker`). 1 мс: заметно
/// меньше `TICK_INTERVAL`, поэтому на нормальном ходу ни на что не влияет,
/// и при этом строго больше нуля.
const MIN_TICK_ADVANCE_S: f64 = 0.001;
/// Ритм опроса, пока WT ещё не отвечал (или перестал отвечать) — реже, чтобы
/// не долбить localhost:8111 20 раз в секунду, когда игра не запущена.
const PROBE_INTERVAL: Duration = Duration::from_millis(1000);
const HTTP_TIMEOUT: Duration = Duration::from_millis(200);
/// Тот же грейс-период, что у MSFS-вотчдога (sim/worker.rs) — для
/// консистентности дебаунса между двумя конвейерами.
const GRACE_PERIOD: Duration = Duration::from_millis(2500);

/// Смещения от начала кадра сетевого опроса, на которых должен сработать тик
/// движка — k-е смещение равно `k * tick_interval`. Смещение, которое ушло бы
/// НА или ЗА `poll_interval` (`k > 0 && offset >= poll_interval`), не
/// включается — вместо очередного подтика начинается новый кадр реальным
/// HTTP-опросом. Всегда возвращает хотя бы одно смещение (`Duration::ZERO`
/// на k=0) — движок обязан тикнуть хотя бы раз за кадр, даже если
/// `tick_interval >= poll_interval`. `tick_interval == Duration::ZERO`
/// вернул бы бесконечную последовательность нулевых смещений — специальный
/// случай на входе не даёт зациклиться.
fn subtick_offsets(poll_interval: Duration, tick_interval: Duration) -> Vec<Duration> {
    if tick_interval.is_zero() {
        return vec![Duration::ZERO];
    }
    let mut offsets = Vec::new();
    let mut k: u32 = 0;
    loop {
        let offset = tick_interval * k;
        if k > 0 && offset >= poll_interval {
            break;
        }
        offsets.push(offset);
        k += 1;
    }
    offsets
}

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
    // Пользовательский движок эффектов (см. custom_fx::engine) — своё
    // состояние независимое от `engine` выше, сбрасывается отдельно (потеря
    // владения слотом). Оба движка теперь считаются КАЖДЫЙ тик одновременно
    // (не режимы) — см. custom_fx::overrides для того, как их выходы
    // сводятся.
    let mut custom_engine = CustomFxEngine::new();
    let mut ammo = AmmoTracker::new();
    let mut recorder = SessionRecorder::new();
    let mut client: Option<WtClient> = None;
    let mut liveness = Liveness::new(GRACE_PERIOD);
    // Фронт захвата PreviewLock — один раз нули на моторы в момент захвата,
    // дальше молчим, пока не отпустят (см. game_state::PreviewLock).
    let mut preview_was_held = false;
    // Зануляем HID/эффекты ровно один раз на переходе владения true→false —
    // не на каждом тике `!owns` (см. game_state.rs docstring): проигравший
    // try_claim не шлёт нули каждый тик, чтобы не гоняться наперегонки с
    // текущим владельцем за один и тот же tx_hid.
    let mut was_owner = false;
    // Время последнего ТИКА, отданного движкам — монотонный сторож (см.
    // его использование в подтиковом цикле). `WtVars::t` снимается ПОСЛЕ
    // HTTP-запроса, поэтому при скачке задержки сети время следующего
    // кадра может оказаться РАНЬШЕ последнего подтика предыдущего, а оба
    // движка считают из него `dt` и фазу ШИМ от абсолютного времени —
    // шаг назад дал бы отрицательный `dt` и рывок фазы.
    let mut last_tick_t = f64::NEG_INFINITY;

    loop {
        // Якорь подтикового цикла (см. subtick_offsets/TICK_INTERVAL ниже) —
        // захватываем ДО самого HTTP-запроса, чтобы весь кадр (сеть + все
        // подтики) укладывался в POLL_INTERVAL, а не добавлял его поверх
        // времени, которое ушло на сам запрос.
        let frame_start = Instant::now();
        // true, если этот кадр дошёл до подтикового цикла ниже — тогда он
        // сам уже выждал время до следующего кадра своими подтиковыми
        // sleep(), и хвостовой sleep(POLL_INTERVAL) внизу цикла делать не
        // нужно (см. его использование в конце итерации).
        let mut ran_subticks = false;

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

                    // Подтиковый цикл: сеть опрошена один раз в этом кадре
                    // (state_v/indicators_v/wt_vars выше — РОВНО один раз, не
                    // на каждый подтик: разбор/парсинг, обучение
                    // ammo-трекера и запись сессии уже сделаны выше и здесь
                    // не повторяются), а движок и отправка на HID тикают
                    // чаще — каждые TICK_INTERVAL, на этом же кадре
                    // телеметрии, но с ПРОДВИНУТЫМ временем (t_кадра +
                    // offset), пока не подойдёт срок следующего реального
                    // HTTP-опроса (см. subtick_offsets/POLL_INTERVAL/
                    // TICK_INTERVAL выше). Без продвижения времени dt внутри
                    // engine.step схлопнулся бы в защитный минимум и
                    // сглаживание (см. crate::timing::ema_retain_for_dt)
                    // поехало бы. hold/preview.is_held() перечитываются НА
                    // КАЖДОМ подтике, а не один раз на весь кадр — иначе
                    // реакция на паузу/предпросмотр деградировала бы до
                    // целого кадра сетевого опроса вместо одного подтика.
                    ran_subticks = true;
                    for offset in subtick_offsets(POLL_INTERVAL, TICK_INTERVAL) {
                        let target = frame_start + offset;
                        let now = Instant::now();
                        if target > now {
                            thread::sleep(target - now);
                        }

                        if preview.is_held() {
                            // Редактор эффектов держит HID-канал под предпросмотр
                            // (см. game_state::PreviewLock) — воркер молчит, иначе
                            // оба источника переписывали бы друг друга на моторе.
                            // На ФРОНТЕ захвата шлём нули один раз, чтобы не
                            // застыло последнее значение движка.
                            if !preview_was_held {
                                let _ = tx_hid.send(HidCmd::SendIntensity {
                                    joystick: 0,
                                    throttle_left: 0,
                                    throttle_right: 0,
                                });
                                preview_was_held = true;
                            }
                            continue;
                        }
                        preview_was_held = false;

                        // Встроенный и пользовательский движки эффектов больше НЕ
                        // взаимоисключающие режимы (см. custom_fx::overrides) —
                        // оба считаются каждый подтик. Вытеснение точечное:
                        // пользовательский эффект гасит ТОЛЬКО тот встроенный, для
                        // которого его источник телеметрии основной (см.
                        // doc-комментарий overrides.rs). Подавление применяется к
                        // КОПИИ full_cfg.wt — WtRumbleState::step не в курсе, что
                        // что-то подавлено, движок вибрации не тронут.
                        let mut wt_vars_tick = wt_vars.clone();
                        // Монотонность (см. last_tick_t выше): время тика не
                        // может не только пойти назад, но и остановиться —
                        // `dt == 0` схлопнул бы нормировку сглаживания в
                        // "заморозку" (crate::timing::ema_blend_for_dt).
                        let tick_t = (wt_vars.t + offset.as_secs_f64())
                            .max(last_tick_t + MIN_TICK_ADVANCE_S);
                        last_tick_t = tick_t;
                        wt_vars_tick.t = tick_t;

                        let full_cfg = config.get();
                        let aircraft = wt_vars_tick.vehicle_type.clone();
                        let custom_effects = custom_fx.get();
                        let overridden = crate::custom_fx::overrides::overridden_builtins(
                            &custom_effects,
                            ActiveGame::Wt,
                            &aircraft,
                        );
                        let mut suppressed_wt_cfg = full_cfg.wt;
                        overridden.apply_to_wt_config(&mut suppressed_wt_cfg);

                        let held_now = hold.load(Ordering::Relaxed);
                        let builtin_out = engine.step(&wt_vars_tick, &suppressed_wt_cfg, held_now);

                        let frame = TelemetryFrame::Wt(wt_vars_tick.clone());
                        let custom_out = custom_engine.step(
                            &frame,
                            wt_vars_tick.t,
                            &custom_effects,
                            custom_fx.current_rev(),
                            &aircraft,
                            ActiveGame::Wt,
                            held_now,
                            full_cfg.max_output,
                        );

                        *last_wt_vars.lock() = Some(wt_vars_tick);
                        effects.apply_snapshot(&builtin_out.effects);
                        *active_custom_ids.lock() = custom_out.active_ids;

                        let _ = tx_hid.send(HidCmd::SendIntensity {
                            joystick: crate::custom_fx::overrides::combine_channel(
                                builtin_out.joystick_intensity,
                                custom_out.joystick,
                                full_cfg.max_output,
                            ),
                            throttle_left: crate::custom_fx::overrides::combine_channel(
                                builtin_out.throttle_left_intensity,
                                custom_out.throttle_left,
                                full_cfg.max_output,
                            ),
                            throttle_right: crate::custom_fx::overrides::combine_channel(
                                builtin_out.throttle_right_intensity,
                                custom_out.throttle_right,
                                full_cfg.max_output,
                            ),
                        });
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

        // Кадр, дошедший до подтикового цикла выше, уже сам выждал время до
        // следующего опроса своими подтиковыми sleep() (см. ran_subticks) —
        // здесь спим, только если тот цикл не запускался (не жив/не наш слот/
        // сеть ничего не ответила).
        if !ran_subticks {
            thread::sleep(if alive { POLL_INTERVAL } else { PROBE_INTERVAL });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtick_offsets_covers_frame_without_gaps_or_duplicates() {
        let offsets = subtick_offsets(POLL_INTERVAL, TICK_INTERVAL);
        // 50 мс / 20 мс -> подтики на 0, 20, 40 мс (60 мс уже вышло бы за
        // рамки кадра, следующий кадр начнётся реальным HTTP-опросом).
        assert_eq!(
            offsets,
            vec![
                Duration::from_millis(0),
                Duration::from_millis(20),
                Duration::from_millis(40),
            ]
        );
        // Строго возрастает, без дублей и пропусков — каждый следующий
        // элемент ровно на TICK_INTERVAL дальше предыдущего.
        for w in offsets.windows(2) {
            assert_eq!(w[1] - w[0], TICK_INTERVAL);
        }
        // Все смещения строго меньше кадра.
        assert!(offsets.iter().all(|&o| o < POLL_INTERVAL));
    }

    #[test]
    fn subtick_offsets_always_has_at_least_one_element() {
        // Даже вырожденный случай "тик крупнее кадра" обязан тикнуть хотя бы
        // раз — движок не должен молчать целый кадр.
        let offsets = subtick_offsets(Duration::from_millis(20), Duration::from_millis(50));
        assert_eq!(offsets, vec![Duration::ZERO]);
    }

    #[test]
    fn subtick_offsets_handles_exact_multiple() {
        // Кадр — ровно кратное число подтиков (50/25 = 2): 0 и 25 мс входят,
        // 50 мс уже равно границе кадра и не входит.
        let offsets = subtick_offsets(Duration::from_millis(50), Duration::from_millis(25));
        assert_eq!(
            offsets,
            vec![Duration::from_millis(0), Duration::from_millis(25)]
        );
    }

    #[test]
    fn subtick_offsets_zero_tick_interval_does_not_loop_forever() {
        let offsets = subtick_offsets(POLL_INTERVAL, Duration::ZERO);
        assert_eq!(offsets, vec![Duration::ZERO]);
    }

    #[test]
    fn subtick_offsets_tick_equal_to_poll_gives_single_tick() {
        // Патологический, но легальный ввод: TICK_INTERVAL == POLL_INTERVAL
        // — ровно один подтик на кадр (то же поведение, что было ДО этой
        // задачи, когда движок тикал раз в кадр сетевого опроса).
        let offsets = subtick_offsets(POLL_INTERVAL, POLL_INTERVAL);
        assert_eq!(offsets, vec![Duration::ZERO]);
    }
}
