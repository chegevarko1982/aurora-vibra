use std::sync::{Arc, atomic::AtomicBool};

use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::aircraft_profiles::AircraftProfiles;
use crate::game_state::GameSlot;
use crate::profiles::ProfileState;
use crate::{ConfigShared, EffectsShared, FlightVars, HidCmd, LogBuffer, SimStatus};

// Сигнатура обязана совпадать с windows-версией в worker.rs (тот же
// #[allow] стоит и там): вызывающий код в main.rs один на обе платформы.
#[allow(clippy::too_many_arguments)]
pub fn xp_worker(
    _last_vars: Arc<Mutex<Option<FlightVars>>>,
    _tx_hid: Sender<HidCmd>,
    _logs: LogBuffer,
    _config: Arc<ConfigShared>,
    _effects: EffectsShared,
    _hold: Arc<AtomicBool>,
    _status: Arc<Mutex<SimStatus>>,
    _aircraft_title: Arc<Mutex<String>>,
    _aircraft_profiles: Arc<Mutex<AircraftProfiles>>,
    _profile_state: Arc<Mutex<ProfileState>>,
    _game: GameSlot,
) {
    // Non-Windows stub: RREF/HID hardware pipeline is unavailable.
}
