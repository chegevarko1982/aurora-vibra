use std::sync::{Arc, atomic::AtomicBool};

use crossbeam_channel::Receiver;

use crate::{HidCmd, LogBuffer};

pub fn hid_worker(
    _controller_connected: Arc<AtomicBool>,
    _throttle_connected: Arc<AtomicBool>,
    _rx: Receiver<HidCmd>,
    _logs: LogBuffer,
) {
    // Non-Windows stub: HID hardware is unavailable.
}
