use std::sync::{Arc, atomic::AtomicBool};

use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::{
    ConfigShared, EffectsShared, FlightVars, HidCmd, LogBuffer, SimStatus,
    aircraft_profiles::AircraftProfiles, profiles::ProfileState,
};

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
) {
    // Non-Windows stub: SimConnect is unavailable.
}
