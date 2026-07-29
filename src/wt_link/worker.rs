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
use crate::game_state::{GameSlot, Liveness};
use crate::profiles::ProfileState;
use crate::wt_link::ammo::AmmoTracker;
use crate::wt_link::http::WtClient;
use crate::wt_link::rumble::WtRumbleState;
use crate::wt_link::vars::{self, WtVars};
use crate::{ActiveGame, ConfigShared, EffectsShared, GameOverride, HidCmd, LogBuffer, SimStatus};

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
) {
    logs.push("WT: worker started, polling localhost:8111");

    let session_start = Instant::now();
    let mut engine = WtRumbleState::new();
    let mut ammo = AmmoTracker::new();
    let mut client: Option<WtClient> = None;
    let mut liveness = Liveness::new(GRACE_PERIOD);
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
        // пользователь прижал MSFS, WT ведёт себя как "не жив" независимо от
        // реальной живости — release, не try_claim.
        let vetoed = crate::settings::game_override() == GameOverride::ForceMsfs;
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
                let mut wt_vars = vars::parse(t, &state_v, &indicators_v);
                if !wt_vars.in_mission {
                    // В ангаре/меню может смениться борт — забываем, какие
                    // ammo_counterN относились к weapon1/weapon2 прошлого
                    // самолёта, чтобы не гейтить по чужой раскладке стволов.
                    ammo.reset();
                } else {
                    ammo.observe(&indicators_v, wt_vars.weapon1_firing, wt_vars.weapon2_firing);
                    if ammo.weapon1_empty(&indicators_v) {
                        wt_vars.weapon1_firing = false;
                    }
                    if ammo.weapon2_empty(&indicators_v) {
                        wt_vars.weapon2_firing = false;
                    }
                    wt_vars.weapon1_ammo = ammo.weapon1_ammo(&indicators_v);
                    wt_vars.weapon2_ammo = ammo.weapon2_ammo(&indicators_v);
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

                    let cfg_now = config.get().wt;
                    let out = engine.step(&wt_vars, &cfg_now, hold.load(Ordering::Relaxed));
                    *last_wt_vars.lock() = Some(wt_vars);
                    effects.apply_snapshot(&out.effects);
                    let _ = tx_hid.send(HidCmd::SendIntensity {
                        joystick: out.joystick_intensity,
                        throttle_left: out.throttle_left_intensity,
                        throttle_right: out.throttle_right_intensity,
                    });
                }
            }
            _ => {
                // Игра не запущена / порт закрыт — то же самое молчание, что
                // и recon-инструмент показывает как ConnStatus::Disconnected.
                if owns {
                    *status.lock() = SimStatus::Disconnected;
                    *last_wt_vars.lock() = None;
                    effects.clear_all();
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
