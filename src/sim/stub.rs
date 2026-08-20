use std::sync::{Arc, atomic::AtomicBool};

use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::{
    ConfigShared, EffectsShared, FlightVars, HidCmd, LogBuffer, SimStatus,
    aircraft_profiles::AircraftProfiles,
    custom_fx::store::CustomFxShared,
    game_state::{GameSlot, PreviewLock},
    profiles::ProfileState,
};

// Сигнатура обязана совпадать с windows-версией в worker.rs (тот же
// #[allow] стоит и там): вызывающий код в main.rs один на обе платформы.
#[allow(clippy::too_many_arguments)]
pub fn sim_worker(
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
    _recording: Arc<AtomicBool>,
    _custom_fx: Arc<CustomFxShared>,
    _active_custom_ids: Arc<Mutex<Vec<String>>>,
    _preview: PreviewLock,
) {
    // Non-Windows stub: SimConnect is unavailable.
}
