use std::sync::{Arc, atomic::AtomicBool};

use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::aircraft_profiles::AircraftProfiles;
use crate::game_state::GameSlot;
use crate::profiles::ProfileState;
use crate::wt_link::vars::WtVars;
use crate::{ConfigShared, EffectsShared, HidCmd, LogBuffer, SimStatus};

// Сигнатура обязана совпадать с windows-версией в worker.rs (тот же
// #[allow] стоит и там): вызывающий код в main.rs один на обе платформы.
#[allow(clippy::too_many_arguments)]
pub fn wt_worker(
    _last_wt_vars: Arc<Mutex<Option<WtVars>>>,
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
) {
    // Non-Windows stub: HID/War Thunder hardware pipeline is unavailable.
}
