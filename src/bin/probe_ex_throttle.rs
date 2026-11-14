// Probe for the WINCTRL Ursa Minor Combat Throttle EX (PID 0xB970).
// Its vibration frame is not documented, so this sweeps candidate frames
//   [report_id, ident, code, 0, 0, 3, 0x49, addr, intensity, 0 x5]
// one step at a time and prints each step BEFORE sending it. Note the step
// number printed when a grip buzzes.
//
// cargo run --features dev-tools --bin probe_ex_throttle
// cargo run --features dev-tools --bin probe_ex_throttle -- --from 40
//
// Close Aurora Vibra and SimAppPro first (one process per HID device).

use aurora_vibra::hid::protocol::WW_VID;
use hidapi::HidApi;
use std::thread::sleep;
use std::time::{Duration, Instant};

const REPORT_ID: u8 = 0x02;
const OUT_LEN: usize = 14;
const ON_MS: u64 = 1200;
const OFF_MS: u64 = 400;

fn frame(ident: u8, code: u8, addr: u8, intensity: u8) -> Vec<u8> {
    let mut buf = vec![0u8; OUT_LEN];
    buf[0] = REPORT_ID;
    buf[1] = ident;
    buf[2] = code;
    buf[5] = 0x03;
    buf[6] = 0x49;
    buf[7] = addr;
    buf[8] = intensity;
    buf
}

fn main() {
    let mut pid: u16 = 0xB970;
    let mut from: usize = 1;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--from" => {
                from = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(1);
                i += 2;
            }
            "--pid" => {
                pid = args
                    .get(i + 1)
                    .and_then(|v| {
                        u16::from_str_radix(v.trim_start_matches("0x").trim_start_matches("0X"), 16)
                            .ok()
                    })
                    .unwrap_or(pid);
                i += 2;
            }
            _ => i += 1,
        }
    }

    let api = HidApi::new().expect("Could not initialise HID API");
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == WW_VID
                && d.product_id() == pid
                && d.usage_page() == 0x0001
                && d.usage() == 0x0004
        })
        .unwrap_or_else(|| panic!("Device 0x{pid:04X} not found"));
    let dev = api.open_path(info.path()).expect("Could not open device");
    println!("Opened PID 0x{pid:04X}. Hold the throttle grips lightly and feel for buzzing.");
    println!("Each step buzzes for {ON_MS} ms. Note the STEP number when you feel it.\n");

    let mut steps = Vec::new();
    for ident in 0x00u8..=0x14 {
        for code in [0xBFu8, 0xCF, 0xB9] {
            for addr in [0x00u8, 0x0E, 0x10] {
                steps.push((ident, code, addr));
            }
        }
    }
    let total = steps.len();

    for (n, (ident, code, addr)) in steps.into_iter().enumerate() {
        let step = n + 1;
        if step < from {
            continue;
        }
        println!("STEP {step}/{total}: ident=0x{ident:02X} code=0x{code:02X} addr=0x{addr:02X}");
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(ON_MS) {
            if let Err(e) = dev.write(&frame(ident, code, addr, 255)) {
                eprintln!("  write failed: {e}");
                break;
            }
            sleep(Duration::from_millis(20));
        }
        let _ = dev.write(&frame(ident, code, addr, 0));
        sleep(Duration::from_millis(OFF_MS));
    }
    println!("\nDone.");
}
