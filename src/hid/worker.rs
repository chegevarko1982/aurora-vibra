use std::collections::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use hidapi::{HidApi, HidDevice};

use crate::hid::protocol::{
    build_simapp_vibe_frame, build_throttle_vibe_frame, is_ursa_minor_throttle, ursa_model_name,
    THROTTLE_MOTOR_LEFT, THROTTLE_MOTOR_RIGHT, WW_VID,
};
use crate::hid::win32::hid_query_caps_from_path;
use crate::{HidCmd, LogBuffer};

struct HidEntry {
    dev: HidDevice,
    path: String,
    pid: u16,
    ifnum: i32,
    usage_page: u16,
    usage: u16,
    out_len: u16,
    report_id: u8,
}

/// Отправляет вибрацию на РУД (WINCTRL URSA MINOR Throttle). У Throttle два
/// физических вибромотора на одном HID-устройстве, адресуемых отдельными
/// байтами в отчёте, поэтому шлём два фрейма — по одному на каждый мотор.
fn send_throttle_rumble(
    device: &HidDevice,
    out_len: u16,
    report_id: u8,
    left_motor: u8,
    right_motor: u8,
) -> (usize, usize) {
    let mut ok = 0usize;
    let mut fail = 0usize;

    for (motor_addr, intensity) in [
        (THROTTLE_MOTOR_LEFT, left_motor),
        (THROTTLE_MOTOR_RIGHT, right_motor),
    ] {
        let frame = build_throttle_vibe_frame(report_id, out_len, motor_addr, intensity);
        match device.write(&frame) {
            Ok(n) if n == frame.len() => ok += 1,
            _ => fail += 1,
        }
    }

    (ok, fail)
}

fn hid_send_out(
    devs: &[HidEntry],
    joystick_intensity: u8,
    throttle_left_intensity: u8,
    throttle_right_intensity: u8,
    _logs: &LogBuffer,
) -> (usize, usize) {
    let mut ok = 0usize;
    let mut fail = 0usize;

    for d in devs {
        if !(d.usage_page == 0x0001 && d.usage == 0x0004) {
            continue;
        }

        if is_ursa_minor_throttle(d.pid) {
            // РУД: теперь левый и правый мотор получают независимую
            // интенсивность — RumbleEngine считает их раздельно.
            let (t_ok, t_fail) = send_throttle_rumble(
                &d.dev,
                d.out_len,
                d.report_id,
                throttle_left_intensity,
                throttle_right_intensity,
            );
            ok += t_ok;
            fail += t_fail;
            continue;
        }

        let frame = build_simapp_vibe_frame(d.pid, d.report_id, d.out_len, joystick_intensity);
        match d.dev.write(&frame) {
            Ok(n) => {
                if n == frame.len() {
                    ok += 1;
                } else {
                    fail += 1;
                }
            }
            Err(_) => {
                fail += 1;
            }
        }
    }

    (ok, fail)
}

pub fn hid_worker(
    controller_connected: Arc<AtomicBool>,
    throttle_connected: Arc<AtomicBool>,
    rx: Receiver<HidCmd>,
    logs: LogBuffer,
) {
    logs.push("HID: worker starting…");

    let verbose_hid = std::env::var_os("URSA_VERBOSE_HID").is_some();
    let mut api = match HidApi::new() {
        Ok(a) => {
            logs.push("HID: HidApi initialized");
            a
        }
        Err(e) => {
            logs.push(format!("HID: HidApi::new FAILED: {}", e));
            return;
        }
    };

    let mut devices: Vec<HidEntry> = vec![];
    let mut last_scan = Instant::now() - Duration::from_secs(10);
    let mut last_status_log = Instant::now() - Duration::from_secs(10);
    let mut last_missing_log = Instant::now() - Duration::from_secs(10);

    const SEND_INTERVAL: Duration = Duration::from_millis(50);

    let mut desired_joystick: u8 = 0;
    let mut desired_throttle_left: u8 = 0;
    let mut desired_throttle_right: u8 = 0;
    let mut last_sent_joystick: u8 = 255;
    let mut last_sent_throttle_left: u8 = 255;
    let mut last_sent_throttle_right: u8 = 255;
    let mut last_send: Instant = Instant::now() - SEND_INTERVAL;
    let mut hold: bool = false;
    let mut prev_scan_sig = String::new();

    let mut ensure_open = |api: &mut HidApi, devices: &mut Vec<HidEntry>| {
        if last_scan.elapsed() < Duration::from_secs(2) && !devices.is_empty() {
            return;
        }

        let mut idx_by_path: HashMap<String, usize> = HashMap::new();
        for (i, d) in devices.iter().enumerate() {
            idx_by_path.insert(d.path.clone(), i);
        }

        if let Err(e) = api.refresh_devices() {
            logs.push(format!("HID: refresh_devices FAILED: {}", e));
        }

        let mut seen_paths: HashSet<String> = HashSet::new();
        let mut found_summary: Vec<String> = Vec::new();

        for devinfo in api.device_list() {
            if devinfo.vendor_id() != WW_VID {
                continue;
            }

            let path = devinfo.path().to_string_lossy().to_string();
            let pid = devinfo.product_id();
            let ifnum = devinfo.interface_number();
            let up = devinfo.usage_page();
            let u = devinfo.usage();

            seen_paths.insert(path.clone());
            found_summary.push(format!(
                "pid=0x{:04X} ({}) if#{} up=0x{:04X} u=0x{:04X} path='{}'",
                pid,
                ursa_model_name(pid),
                ifnum,
                up,
                u,
                path
            ));
        }

        found_summary.sort();
        let scan_sig = found_summary.join(" | ");
        if scan_sig != prev_scan_sig {
            logs.push(if found_summary.is_empty() {
                "HID: scan found 0 Winwing devices".to_string()
            } else {
                format!(
                    "HID: scan found {} Winwing devices: {}",
                    found_summary.len(),
                    scan_sig
                )
            });
            prev_scan_sig = scan_sig;
        }

        for devinfo in api.device_list() {
            if devinfo.vendor_id() != WW_VID {
                continue;
            }

            let path = devinfo.path().to_string_lossy().to_string();
            if idx_by_path.contains_key(&path) {
                continue;
            }

            let pid = devinfo.product_id();
            let (out_len, report_id) =
                hid_query_caps_from_path(&path, &logs).unwrap_or((14u16, 0x02u8));

            let d = match devinfo.open_device(api) {
                Ok(d) => d,
                Err(e) => {
                    logs.push(format!("HID: open failed on '{}' : {}", path, e));
                    continue;
                }
            };

            devices.push(HidEntry {
                dev: d,
                path: path.clone(),
                pid,
                ifnum: devinfo.interface_number(),
                usage_page: devinfo.usage_page(),
                usage: devinfo.usage(),
                out_len,
                report_id,
            });

            logs.push(format!(
                "HID: device OPENED (VID=0x{:04X}, PID=0x{:04X} {}, if#{}, up=0x{:04X}, u=0x{:04X}, out_len={}, report_id=0x{:02X}) path='{}'",
                devinfo.vendor_id(),
                pid,
                format!("({})", ursa_model_name(pid)),
                devinfo.interface_number(),
                devinfo.usage_page(),
                devinfo.usage(),
                out_len,
                report_id,
                devinfo.path().to_string_lossy()
            ));
        }

        if !devices.is_empty() {
            devices.retain(|d| {
                if seen_paths.contains(&d.path) {
                    true
                } else {
                    logs.push(format!("HID: device REMOVED path='{}'", d.path));
                    false
                }
            });
        }

        if seen_paths.is_empty() && last_missing_log.elapsed() > Duration::from_secs(5) {
            logs.push("HID: no Winwing devices found (VID=0x4098)".to_string());
            last_missing_log = Instant::now();
        }

        let joystick_present = devices.iter().any(|d| !is_ursa_minor_throttle(d.pid));
        let throttle_present = devices.iter().any(|d| is_ursa_minor_throttle(d.pid));
        controller_connected.store(joystick_present, Ordering::Relaxed);
        throttle_connected.store(throttle_present, Ordering::Relaxed);
        last_scan = Instant::now();
    };

    ensure_open(&mut api, &mut devices);

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(cmd) => match cmd {
                HidCmd::SendIntensity { joystick, throttle_left, throttle_right } => {
                    desired_joystick = joystick;
                    desired_throttle_left = throttle_left;
                    desired_throttle_right = throttle_right;
                    if verbose_hid
                        && ((i16::from(desired_joystick) - i16::from(last_sent_joystick)).abs()
                            >= 15
                            || (i16::from(desired_throttle_left) - i16::from(last_sent_throttle_left)).abs()
                                >= 15
                            || (i16::from(desired_throttle_right) - i16::from(last_sent_throttle_right)).abs()
                                >= 15)
                    {
                        logs.push(format!(
                            "HID: cmd SendIntensity(joystick={}, throttle_left={}, throttle_right={})",
                            desired_joystick, desired_throttle_left, desired_throttle_right
                        ));
                    }
                }
                HidCmd::SendRaw(bytes) => {
                    logs.push(format!("HID: cmd SendRaw(len={})", bytes.len()));
                    for d in &devices {
                        if let Err(e) = d.dev.write(&bytes) {
                            logs.push(format!(
                                "HID: raw write FAILED (PID=0x{:04X} {}, if#{}, usage_page=0x{:04X}, usage=0x{:04X}) path='{}': {}",
                                d.pid,
                                ursa_model_name(d.pid),
                                d.ifnum,
                                d.usage_page,
                                d.usage,
                                d.path,
                                e
                            ));
                        }
                    }
                }
                HidCmd::StopAll => {
                    logs.push("HID: cmd StopAll");
                    desired_joystick = 0;
                    desired_throttle_left = 0;
                    desired_throttle_right = 0;
                    last_send = Instant::now() - SEND_INTERVAL;
                }
                HidCmd::SetHold(x) => {
                    hold = x;
                    logs.push(format!("HID: cmd SetHold({})", hold));
                    if hold {
                        let (_ok, _fail) = hid_send_out(&devices, 0, 0, 0, &logs);
                        last_sent_joystick = 0;
                        last_sent_throttle_left = 0;
                        last_sent_throttle_right = 0;
                    }
                }
                HidCmd::ReopenDevices => {
                    logs.push("HID: cmd ReopenDevices");
                    ensure_open(&mut api, &mut devices);
                }
            },
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                logs.push("HID: channel disconnected → worker exit");
                break;
            }
        }

        ensure_open(&mut api, &mut devices);

        if last_send.elapsed() >= SEND_INTERVAL {
            let out_j = if hold { 0 } else { desired_joystick };
            let out_t_left = if hold { 0 } else { desired_throttle_left };
            let out_t_right = if hold { 0 } else { desired_throttle_right };
            if out_j != last_sent_joystick
                || out_t_left != last_sent_throttle_left
                || out_t_right != last_sent_throttle_right
            {
                let (ok, fail) = hid_send_out(&devices, out_j, out_t_left, out_t_right, &logs);

                let now = Instant::now();
                if fail > 0
                    || ok == 0
                    || now.duration_since(last_status_log) > Duration::from_millis(900)
                {
                    logs.push(format!(
                        "HID: send intensity joystick={} throttle_left={} throttle_right={} → ok={} fail={} (devs={}, hold={})",
                        out_j,
                        out_t_left,
                        out_t_right,
                        ok,
                        fail,
                        devices.len(),
                        hold
                    ));
                    last_status_log = now;
                }

                last_sent_joystick = out_j;
                last_sent_throttle_left = out_t_left;
                last_sent_throttle_right = out_t_right;
            }
            last_send = Instant::now();
        }
    }
}
