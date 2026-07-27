// Держит/рампует заданный уровень ШИМ (0..255) на Throttle (оба мотора) ИЛИ
// на джойстике (Fighter R) — для быстрой проверки "на ощупь" конкретного
// значения или диапазона (напр. ENGINE_SPOOL_MIN_AMPLITUDE), без прогона
// всего сима.
// cargo run --bin test_hold -- <start> [end] <duration_s> [throttle|joystick]
// Два числовых аргумента = держать start постоянно duration_s секунд.
// Три числовых аргумента = линейная рампа start..end за duration_s секунд.
// Последний нечисловой аргумент выбирает устройство (по умолчанию throttle).
//
// Спец-режим: cargo run --bin test_hold -- fadejerk [throttle|joystick]
// Демо "плавное окончание + пара микро-рывков": держим 10 1с, спад 10->0 за
// 4с, затем два коротких импульса до 3 в хвосте (см. эффект отмены запуска
// без воспламенения в rumble.rs).

use hidapi::HidApi;
use std::thread;
use std::time::Duration;

fn open_device(is_joystick: bool) -> (hidapi::HidDevice, [u8; 14]) {
    let api = HidApi::new().expect("Не удалось инициализировать HID API");
    let pid: u16 = if is_joystick { 0xBC2A } else { 0xB920 };
    let device = api.open(0x4098, pid).unwrap_or_else(|_| {
        panic!("Устройство (PID {pid:#06X}) не найдено! Закрой основное приложение и SimAppPro.")
    });

    let mut buf = [0u8; 14];
    buf[0] = 0x02;
    if is_joystick {
        buf[1] = 0x0A;
        buf[2] = 0xBF;
    } else {
        buf[1] = 0x10;
        buf[2] = 0xb9;
    }
    buf[3] = 0x00;
    buf[4] = 0x00;
    buf[5] = 0x03;
    buf[6] = 0x49;
    if is_joystick {
        buf[7] = 0x00;
    }
    (device, buf)
}

fn send(device: &hidapi::HidDevice, buf: &mut [u8; 14], is_joystick: bool, intensity: u8) {
    if is_joystick {
        buf[8] = intensity;
        device.write(buf).expect("write joystick");
    } else {
        buf[7] = 0x0e;
        buf[8] = intensity;
        device.write(buf).expect("write left");
        buf[7] = 0x10;
        buf[8] = intensity;
        device.write(buf).expect("write right");
    }
}

fn main() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();

    if raw_args.first().map(String::as_str) == Some("fadejerk") {
        let is_joystick = raw_args.get(1).map(String::as_str) == Some("joystick");
        let label = if is_joystick {
            "Joystick (Fighter R)"
        } else {
            "Throttle"
        };
        println!(
            "Демо fadejerk на {label}: держим 10/255 1с, спад 10->0 за 4с, затем 2 микро-рывка (<=3)..."
        );
        let (device, mut buf) = open_device(is_joystick);

        // Держим 10 1с
        for _ in 0..20 {
            send(&device, &mut buf, is_joystick, 10);
            thread::sleep(Duration::from_millis(50));
        }

        // Спад 10->0 за 4с
        let steps = 80u32;
        for step in 0..steps {
            let progress = step as f64 / (steps - 1) as f64;
            let intensity = (10.0 * (1.0 - progress)).round() as u8;
            send(&device, &mut buf, is_joystick, intensity);
            if step % 16 == 0 {
                println!("fade t={:.1}s intensity={}", step as f64 / 20.0, intensity);
            }
            thread::sleep(Duration::from_millis(50));
        }
        send(&device, &mut buf, is_joystick, 0);
        thread::sleep(Duration::from_millis(250));

        // Два микро-рывка (<=3), короткие импульсы с паузой между ними
        for i in 1..=2 {
            println!("jerk #{i}: intensity=3, 80ms");
            send(&device, &mut buf, is_joystick, 3);
            thread::sleep(Duration::from_millis(80));
            send(&device, &mut buf, is_joystick, 0);
            thread::sleep(Duration::from_millis(220));
        }

        println!("Готово, моторы выключены.");
        return;
    }

    let device_name = raw_args
        .last()
        .filter(|a| a.parse::<f64>().is_err())
        .cloned()
        .unwrap_or_else(|| "throttle".to_string());
    let is_joystick = device_name == "joystick";
    let nums: Vec<&String> = raw_args
        .iter()
        .filter(|a| a.parse::<f64>().is_ok())
        .collect();

    let (start, end, duration_s): (u8, u8, f64) = match nums.len() {
        3 => (
            nums[0].parse().unwrap_or(0),
            nums[1].parse().unwrap_or(20),
            nums[2].parse().unwrap_or(12.0),
        ),
        2 => {
            let v: u8 = nums[0].parse().unwrap_or(64);
            (v, v, nums[1].parse().unwrap_or(5.0))
        }
        _ => (64, 64, 5.0),
    };

    let label = if is_joystick {
        "Joystick (Fighter R)"
    } else {
        "Throttle"
    };
    if start == end {
        println!("Держим {start}/255 на {label} {duration_s:.1}с...");
    } else {
        println!("Рампа {start}->{end}/255 на {label} за {duration_s:.1}с...");
    }

    let (device, mut buf) = open_device(is_joystick);

    let steps = (duration_s * 20.0).max(1.0) as u32; // ~50мс тик
    for step in 0..steps {
        let progress = step as f64 / (steps - 1).max(1) as f64;
        let intensity = (start as f64 + (end as f64 - start as f64) * progress).round() as u8;
        send(&device, &mut buf, is_joystick, intensity);

        if step % 20 == 0 {
            println!("t={:.1}s intensity={}", step as f64 / 20.0, intensity);
        }

        thread::sleep(Duration::from_millis(50));
    }

    send(&device, &mut buf, is_joystick, 0);
    println!("Готово, моторы выключены.");
}
