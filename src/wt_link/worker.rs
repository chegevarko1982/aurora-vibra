//! Фоновый поток War Thunder (этап 1): опрашивает `/state` + `/indicators`
//! (localhost:8111, тот же `WtClient`, что и recon-инструмент, см.
//! `super::http`), считает эффекты через `WtRumbleState::step` и шлёт
//! `HidCmd` в тот же канал, что и `sim::sim_worker` — оба конвейера
//! взаимоисключающие (см. `crate::settings::wt_enabled`, гейт в
//! `sim/worker.rs`), поэтому конкуренции за HID-канал нет.
//!
//! Пока режим не включён — поток спит (без HTTP-запросов) и ничего не шлёт,
//! точно так же, как MSFS-конвейер не шлёт HID, пока включён этот режим.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::wt_link::ammo::AmmoTracker;
use crate::wt_link::http::WtClient;
use crate::wt_link::rumble::WtRumbleState;
use crate::wt_link::vars::{self, WtVars};
use crate::{ConfigShared, EffectsShared, HidCmd, LogBuffer, SimStatus};

/// Ритм опроса /state и /indicators — 20 Гц, как дефолт recon-инструмента
/// (wt_probe/cli.rs) и как частота отправки HID в hid/worker.rs.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Тик "режим выключен" — редкий, никаких сетевых запросов не делает.
const IDLE_INTERVAL: Duration = Duration::from_millis(250);
const HTTP_TIMEOUT: Duration = Duration::from_millis(200);

#[allow(clippy::too_many_arguments)]
pub fn wt_worker(
    last_wt_vars: Arc<Mutex<Option<WtVars>>>,
    tx_hid: Sender<HidCmd>,
    logs: LogBuffer,
    config: Arc<ConfigShared>,
    effects: EffectsShared,
    hold: Arc<AtomicBool>,
    status: Arc<Mutex<SimStatus>>,
) {
    logs.push("WT: worker started");

    let session_start = Instant::now();
    let mut engine = WtRumbleState::new();
    let mut ammo = AmmoTracker::new();
    let mut client: Option<WtClient> = None;
    let mut was_enabled = false;

    loop {
        if !crate::settings::wt_enabled() {
            if was_enabled {
                logs.push("WT: mode disabled, clearing state");
                *last_wt_vars.lock() = None;
                *status.lock() = SimStatus::Disconnected;
                effects.clear_all();
                let _ = tx_hid.send(HidCmd::SendIntensity {
                    joystick: 0,
                    throttle_left: 0,
                    throttle_right: 0,
                });
                engine.reset();
                ammo.reset();
                client = None;
                was_enabled = false;
            }
            thread::sleep(IDLE_INTERVAL);
            continue;
        }

        if !was_enabled {
            logs.push("WT: mode enabled, starting to poll localhost:8111");
            was_enabled = true;
        }

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
                *status.lock() = if wt_vars.in_mission {
                    SimStatus::InFlight
                } else {
                    SimStatus::Connected
                };
                *last_wt_vars.lock() = Some(wt_vars);

                let cfg_now = config.get().wt;
                let out = engine.step(&wt_vars, &cfg_now, hold.load(Ordering::Relaxed));
                effects.apply_snapshot(&out.effects);
                let _ = tx_hid.send(HidCmd::SendIntensity {
                    joystick: out.joystick_intensity,
                    throttle_left: out.throttle_left_intensity,
                    throttle_right: out.throttle_right_intensity,
                });
            }
            _ => {
                // Игра не запущена / порт закрыт — то же самое молчание, что
                // и recon-инструмент показывает как ConnStatus::Disconnected.
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

        thread::sleep(POLL_INTERVAL);
    }
}
