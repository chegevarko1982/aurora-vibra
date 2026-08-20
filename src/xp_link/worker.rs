//! Фоновый поток X-Plane 12: опрашивает UDP-протокол RREF на localhost:49000
//! (см. `super::rref::RrefClient`), собирает `crate::FlightVars` (см.
//! `super::vars`) и шлёт `HidCmd` в тот же канал, что и `sim::sim_worker` и
//! `wt_link::wt_worker`. Все три конвейера всегда опрашивают свою игру
//! (никакого ручного флага-гейта), а взаимоисключающее владение
//! HID-каналом/GUI арбитрируется через `crate::game_state::GameSlot` —
//! липкое владение, первый заявивший игру владеет ей, пока сам её не
//! отпустит (см. game_state.rs). Живость определяется через `Liveness` по
//! результату `drain()` — грейс-период переживает кратковременные провалы
//! (например, паузу передачи данных при смене вида в кабине).
//!
//! В отличие от WT (HTTP: сервер сам отдаёт актуальное состояние по каждому
//! запросу) у RREF нет heartbeat, а X-Plane ЗАБЫВАЕТ подписки, когда сам
//! перезапускается — поэтому переподписка тут не разовая операция при
//! старте потока, а часть логики отслеживания живости (см. п.6 в плане и
//! комментарии по ходу цикла ниже).

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
use crate::sim::parse::flight_status;
use crate::xp_link::datarefs::DrIdx;
use crate::xp_link::rref::RrefClient;
use crate::xp_link::vars;
use crate::{
    ActiveGame, ConfigShared, EffectMode, EffectsShared, FlightVars, HidCmd, LogBuffer,
    RumbleEngine, SimStatus,
};

/// Ритм опроса, пока X-Plane жив — 20 Гц, тот же такт, что у WT-конвейера и
/// у отправки HID (см. `hid/worker.rs`).
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Ритм опроса/переподписки, пока X-Plane не отвечает — реже, чтобы не
/// долбить localhost:49000 20 раз в секунду впустую, пока игра не запущена.
const PROBE_INTERVAL: Duration = Duration::from_millis(1000);
/// Тот же грейс-период, что у MSFS/WT вотчдогов — для консистентности
/// дебаунса между тремя конвейерами.
const GRACE_PERIOD: Duration = Duration::from_millis(2500);
/// Пауза перед self-check опечаток в именах datarefs (см. конец цикла),
/// отсчитываемая от первых полученных данных — с запасом хватает, чтобы даже
/// SLOW-datarefs (1 Гц) успели прийти хотя бы раз, если их имена верны.
const SELF_CHECK_DELAY: Duration = Duration::from_secs(3);

#[allow(clippy::too_many_arguments)]
pub fn xp_worker(
    last_vars: Arc<Mutex<Option<FlightVars>>>,
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
    logs.push("XP: worker started, polling RREF on localhost:49000");

    let mut engine = RumbleEngine::new();
    let mut recorder = SessionRecorder::new();
    // Взаимоисключающий движок пользовательских эффектов (см.
    // custom_fx::engine) — своё состояние независимое от `engine` выше,
    // сбрасывается отдельно (потеря владения слотом / смена режима).
    let mut custom_engine = CustomFxEngine::new();
    let mut client: Option<RrefClient> = None;
    let mut liveness = Liveness::new(GRACE_PERIOD);
    // Режим предыдущего тика — ловим МОМЕНТ переключения BuiltIn<->Custom для
    // разового сброса (см. точку отправки HID ниже), не сравниваем впустую.
    let mut last_effect_mode = crate::settings::effect_mode();
    // Фронт захвата PreviewLock — один раз нули на моторы в момент захвата,
    // дальше молчим, пока не отпустят (см. game_state::PreviewLock).
    let mut preview_was_held = false;
    // Зануляем HID/эффекты ровно один раз на переходе владения true→false —
    // не на каждом тике `!owns` (см. подробный комментарий в
    // wt_link/worker.rs): проигравший try_claim не шлёт нули каждый тик,
    // чтобы не гоняться наперегонки с текущим владельцем за один и тот же
    // tx_hid.
    let mut was_owner = false;

    let mut values = vec![0f32; DrIdx::COUNT];
    let mut seen = vec![false; DrIdx::COUNT];

    // Момент, когда от сима пришли ПЕРВЫЕ данные — точка отсчёта для
    // self-check опечаток (см. конец цикла). Именно первые данные, а не
    // первая отправленная подписка: подписка уходит сразу при старте
    // приложения, и если X-Plane в этот момент не запущен, отсчёт от неё
    // выдал бы в лог "не пришло данных по всем 99 datarefs" — список,
    // неотличимый по виду от списка опечаток, хотя сим просто выключен.
    let mut self_check_anchor: Option<Instant> = None;
    let mut self_check_done = false;

    loop {
        // 1. Убеждаемся, что UDP-сокет открыт; если ещё нет — открываем и
        // сразу подписываемся. Без этой первой подписки drain() никогда не
        // вернёт данные (сим просто не знает, что нас нужно слушать), а
        // значит liveness никогда не станет true — курица и яйцо.
        if client.is_none() {
            match RrefClient::connect() {
                Ok(c) => {
                    logs.push("XP: UDP socket opened, subscribing to datarefs");
                    if let Err(e) = c.subscribe_all() {
                        logs.push(format!("XP: initial subscribe_all failed: {e}"));
                    }
                    client = Some(c);
                }
                Err(e) => {
                    logs.push(format!("XP: failed to open UDP socket: {e}"));
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            }
        }
        let c = client.as_ref().expect("client just ensured above");

        let drained = c.drain(&mut values, &mut seen);
        let ok = drained > 0;
        if ok && self_check_anchor.is_none() {
            self_check_anchor = Some(Instant::now());
        }

        // Форс-оверрайд встроен в тот же арбитраж, не отдельная ветка: если
        // пользователь прижал другую игру (MSFS или WT), X-Plane ведёт себя
        // как "не жив" независимо от реальной живости — release, не
        // try_claim. См. GameOverride::vetoes в types.rs.
        let vetoed = crate::settings::game_override().vetoes(ActiveGame::Xplane);
        let alive = liveness.observe(ok, Instant::now());
        let owns = if alive && !vetoed {
            game.try_claim(ActiveGame::Xplane)
        } else {
            game.release_if_owned(ActiveGame::Xplane);
            false
        };

        // 6. Переподписка по ЖИВОСТИ (не по владению) — X-Plane забывает
        // подписки при собственном перезапуске, а у RREF нет heartbeat,
        // чтобы это обнаружить иначе, чем "данные перестали идти". Пока мы
        // не живы, досылаем подписку заново на каждой итерации — благодаря
        // sleep(PROBE_INTERVAL) внизу цикла это и есть "не чаще раза в
        // секунду", как просит план. Если сим за это время запустился (или
        // перезапустился и забыл нас), он получит подписку и начнёт слать
        // данные уже со следующего тика. Подписки идемпотентны (freq у уже
        // подписанного dataref просто перезаписывается тем же значением),
        // так что повторный вызов безопасен и не требует доп. состояния.
        if !alive && let Err(e) = c.subscribe_all() {
            logs.push(format!("XP: re-subscribe (probing) failed: {e}"));
        }

        // 7. Переподписка/отписка по ВЛАДЕНИЮ: теряя слот, отписываемся,
        // чтобы сим не тратил эфир на нас, пока мы не владеем каналом
        // (например MSFS/WT забрали его форс-оверрайдом, а не потому что XP
        // сам умер). Получая слот — подписываемся заново на всякий случай:
        // это может произойти позже момента, когда liveness уже стала true
        // (например слот был занят другой игрой), так что подписка из п.6
        // выше могла уже не сработать к этому моменту.
        if was_owner && !owns {
            logs.push("XP: lost connection or ownership, releasing slot");
            *last_vars.lock() = None;
            effects.clear_all();
            let _ = tx_hid.send(HidCmd::SendIntensity {
                joystick: 0,
                throttle_left: 0,
                throttle_right: 0,
            });
            engine.reset();
            custom_engine.reset();
            active_custom_ids.lock().clear();
            // Отдаём табличку с названием борта обратно MSFS-конвейеру —
            // тот сам выставит её при следующем подключении SimConnect (см.
            // sim/worker.rs), а до тех пор пусть будет пустой, а не
            // залипший борт X-Plane.
            *aircraft_title.lock() = String::new();
            if let Err(e) = c.unsubscribe_all() {
                logs.push(format!("XP: unsubscribe_all failed: {e}"));
            }
        } else if owns && !was_owner {
            logs.push("XP: connection alive, claimed slot");
            if let Err(e) = c.subscribe_all() {
                logs.push(format!("XP: re-subscribe (on claim) failed: {e}"));
            }
        }
        was_owner = owns;

        if owns {
            let fv = vars::to_flight_vars(&values);

            // Тот же критерий "в полёте", что у MSFS-конвейера (см.
            // sim/parse.rs::flight_status) — переиспользуем функцию
            // напрямую вместо повторного изобретения порога, чтобы бейдж
            // статуса вёл себя одинаково у обоих авиасимов.
            *status.lock() = flight_status(&fv);

            // Запись сессии (тот же тумблер, что у WT/MSFS, см.
            // wt_link::worker/sim::worker) — только пока слот реально наш.
            recorder.tick_flightvars(
                recording.load(Ordering::Relaxed),
                fv.sim_time_s,
                &fv,
                "xplane",
                &logs,
            );

            // Смена борта — тем же путём, каким это делает wt_worker для
            // техники War Thunder: то же поле aircraft_title, та же система
            // именных профилей (не заводим отдельный XP-only виджет).
            let title = vars::acf_descrip(&values);
            if !title.is_empty() {
                let changed = *aircraft_title.lock() != title;
                if changed {
                    *aircraft_title.lock() = title.clone();
                    aircraft_profiles::apply_for_aircraft(
                        &mut aircraft_profiles.lock(),
                        &config,
                        &mut profile_state.lock(),
                        &title,
                        &logs,
                    );
                }
            }

            if preview.is_held() {
                // Редактор эффектов держит HID-канал под предпросмотр (см.
                // game_state::PreviewLock) — воркер молчит, иначе оба
                // источника 20 раз в секунду переписывали бы друг друга на
                // моторе. На ФРОНТЕ захвата шлём нули один раз, чтобы не
                // застыло последнее значение движка.
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
                // ВЗАИМОИСКЛЮЧАЮЩИЕ (см. doc-комментарий types::EffectMode): два
                // независимых движка на одних и тех же трёх моторах давали бы
                // непредсказуемое наложение, поэтому здесь именно ВЫБОР считающего
                // движка, а не смешивание их выходов. При смене режима на лету —
                // разовый сброс состояния ОБОИХ движков и кадр нулей: без него
                // эффект, активный в момент переключения, застыл бы на моторе
                // последним значением (тот же приём, что уже используется выше при
                // потере владения слотом).
                let cfg_now = config.get();
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
                            &fv,
                            &cfg_now,
                            config.current_rev(),
                            hold.load(Ordering::Relaxed),
                        );
                        *last_vars.lock() = Some(fv);
                        effects.apply_snapshot(&out.effects);
                        active_custom_ids.lock().clear();
                        let _ = tx_hid.send(HidCmd::SendIntensity {
                            joystick: out.joystick_intensity,
                            throttle_left: out.throttle_left_intensity,
                            throttle_right: out.throttle_right_intensity,
                        });
                    }
                    EffectMode::Custom => {
                        // FlightVars больше не Copy (добавлен словарь lvars) —
                        // .clone() вместо неявного копирования, fv ниже ещё
                        // нужен и для fv.sim_time_s, и для last_vars.
                        let frame = TelemetryFrame::Flight(fv.clone());
                        let custom_effects = custom_fx.get();
                        let out = custom_engine.step(
                            &frame,
                            fv.sim_time_s,
                            &custom_effects,
                            custom_fx.current_rev(),
                            &title,
                            ActiveGame::Xplane,
                            hold.load(Ordering::Relaxed),
                            cfg_now.max_output,
                        );
                        *last_vars.lock() = Some(fv);
                        // Встроенный EffectsSnapshot в режиме Custom заведомо пуст —
                        // Live Monitor подсвечивает активность через
                        // active_custom_ids, а не через effects.
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

        // Self-check опечаток в именах datarefs — ровно один раз, спустя
        // SELF_CHECK_DELAY после ПЕРВЫХ полученных данных (не после первой
        // подписки — см. комментарий у self_check_anchor выше). К этому
        // моменту сим заведомо на связи, поэтому пустой слот `seen` означает
        // именно проблему с конкретным именем, а не «игра не запущена». Не
        // привязано к `owns`: даже если слот сейчас занят другой игрой,
        // факт наличия данных от X-Plane стоит знать по логам. X-Plane молча
        // игнорирует подписку на несуществующий (например, из-за опечатки)
        // dataref — это не ошибка транспорта, слот просто никогда не станет
        // `seen`, и без этой проверки опечатка проявлялась бы только как
        // «эффект молча не работает», без единого следа в логах.
        if !self_check_done
            && let Some(t0) = self_check_anchor
            && t0.elapsed() >= SELF_CHECK_DELAY
        {
            self_check_done = true;
            // DEFS[i].0 — это ровно то же имя, что вернул бы
            // DrIdx::name() для индекса i (инвариант макроса
            // dataref_idx!, см. datarefs.rs), поэтому отдельно
            // реконструировать DrIdx из усечённого индекса не нужно.
            let missing: Vec<&str> = DrIdx::DEFS
                .iter()
                .enumerate()
                .filter(|(i, _)| !seen[*i])
                .map(|(_, (name, _))| *name)
                .collect();
            if missing.is_empty() {
                logs.push("XP: self-check ok, every subscribed dataref reported at least once");
            } else {
                logs.push(format!(
                    "XP: self-check found {} dataref(s) with no data ever received \
                     (typo in name, or not supported by this aircraft?): {}",
                    missing.len(),
                    missing.join(", ")
                ));
            }
        }

        thread::sleep(if alive { POLL_INTERVAL } else { PROBE_INTERVAL });
    }
}
