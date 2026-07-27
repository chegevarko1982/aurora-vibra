// Диагностика прерывистой вибрации на РУД (WinWing URSA MINOR Throttle,
// VID 0x4098 PID 0xB920): по очереди прогоняет три гипотезы из обсуждения —
// таймаут прошивки, потеря второго write() в паре мотор-фреймов подряд,
// инерция мотора. Framing 14-байтового Output Report идентичен
// hid::protocol::build_throttle_vibe_frame / test_hold.rs — здесь
// переиспользуется сама функция, байты не пересобираются вручную.
//
// cargo run --bin test_throttle_stutter -- [1|2|3|all]
// Без аргумента = all (все три этапа подряд). Между вариантами внутри
// каждого этапа есть пауза и печать в консоль, чтобы можно было
// прочувствовать разницу на реальном железе и понять, где именно провал.

use aurora_vibra::hid::protocol::{THROTTLE_MOTOR_LEFT, THROTTLE_MOTOR_RIGHT, build_throttle_vibe_frame};
use hidapi::{HidApi, HidDevice};
use std::thread;
use std::time::{Duration, Instant};

const REPORT_ID: u8 = 0x02;
const OUT_LEN: u16 = 14;
const PID_THROTTLE: u16 = 0xB920;

// Общий интервал для Этапа 2 и Этапа 3 — быстрее текущего SEND_INTERVAL
// (50мс) в worker.rs, чтобы не путать эффект таймаута прошивки (Этап 1) с
// тем, что мы на самом деле хотим проверить в этих этапах.
const BASE_INTERVAL_MS: u64 = 20;

fn open_throttle() -> HidDevice {
    let api = HidApi::new().expect("Не удалось инициализировать HID API");
    api.open(0x4098, PID_THROTTLE).unwrap_or_else(|_| {
        panic!("Throttle (PID 0xB920) не найден! Закрой основное приложение и SimAppPro.")
    })
}

fn write_motor(device: &HidDevice, motor_addr: u8, intensity: u8) {
    let frame = build_throttle_vibe_frame(REPORT_ID, OUT_LEN, motor_addr, intensity);
    if let Err(e) = device.write(&frame) {
        println!(
            "  !! write() FAILED motor=0x{:02X} intensity={}: {}",
            motor_addr, intensity, e
        );
    }
}

fn stop_all(device: &HidDevice) {
    write_motor(device, THROTTLE_MOTOR_LEFT, 0);
    write_motor(device, THROTTLE_MOTOR_RIGHT, 0);
}

fn pause(label: &str, ms: u64) {
    println!("{label}");
    thread::sleep(Duration::from_millis(ms));
}

fn run_for<F: FnMut(&HidDevice)>(
    device: &HidDevice,
    duration: Duration,
    interval_ms: u64,
    mut send_fn: F,
) {
    let start = Instant::now();
    let mut frames = 0u32;
    while start.elapsed() < duration {
        send_fn(device);
        frames += 1;
        thread::sleep(Duration::from_millis(interval_ms));
    }
    println!("   отправлено {frames} кадров");
}

/// Гипотеза 1: firmware auto-timeout. Держим оба мотора на постоянных 255,
/// БЕЗ какого-либо ШИМ — просто ре-отправляем один и тот же кадр с разным
/// интервалом. Если при 50мс чувствуются провалы, а при 20мс уже ровно —
/// это таймаут прошивки, и SEND_INTERVAL в worker.rs просто слишком
/// медленный именно для этого устройства.
fn phase1_timeout_test(device: &HidDevice) {
    println!("\n=== ЭТАП 1: таймаут прошивки ===");
    println!("Держим ОБА мотора на постоянных 255 (без ШИМ), меняем только интервал повтора.");
    println!("Слушай: на каких интервалах чувствуются провалы/рывки, а на каких — ровно.\n");

    for interval_ms in [50u64, 30, 20, 10] {
        println!("-- интервал {interval_ms}мс, держим 4с --");
        run_for(device, Duration::from_secs(4), interval_ms, |d| {
            write_motor(d, THROTTLE_MOTOR_LEFT, 255);
            write_motor(d, THROTTLE_MOTOR_RIGHT, 255);
        });
        stop_all(device);
        pause("   [моторы выключены, пауза 1с перед следующим интервалом]", 1000);
    }
}

/// Гипотеза 2: второй write() в кадре может теряться, если прошивка
/// обрабатывает только один output report за USB-интервал. Сравниваем:
/// один мотор / один write() в кадр -> оба мотора, два write() подряд (как
/// сейчас в send_throttle_rumble) -> оба мотора с паузой между write() ->
/// чередование моторов по кадрам.
fn phase2_double_write_test(device: &HidDevice) {
    println!("\n=== ЭТАП 2: двойной write() подряд ===");
    println!(
        "Общий интервал {}мс для всех вариантов ниже — сравниваем не таймаут, а именно паттерн записи.\n",
        BASE_INTERVAL_MS
    );

    println!("-- 2a: ТОЛЬКО левый мотор, один write() в кадр, 4с --");
    run_for(device, Duration::from_secs(4), BASE_INTERVAL_MS, |d| {
        write_motor(d, THROTTLE_MOTOR_LEFT, 255);
    });
    stop_all(device);
    pause("   [пауза 1с]", 1000);

    println!("-- 2b: ОБА мотора, два write() подряд без паузы (как сейчас в проде), 4с --");
    run_for(device, Duration::from_secs(4), BASE_INTERVAL_MS, |d| {
        write_motor(d, THROTTLE_MOTOR_LEFT, 255);
        write_motor(d, THROTTLE_MOTOR_RIGHT, 255);
    });
    stop_all(device);
    pause("   [пауза 1с]", 1000);

    println!("-- 2c: ОБА мотора, 2мс паузы между write(), 4с --");
    run_for(device, Duration::from_secs(4), BASE_INTERVAL_MS, |d| {
        write_motor(d, THROTTLE_MOTOR_LEFT, 255);
        thread::sleep(Duration::from_millis(2));
        write_motor(d, THROTTLE_MOTOR_RIGHT, 255);
    });
    stop_all(device);
    pause("   [пауза 1с]", 1000);

    println!("-- 2d: чередование моторов по кадрам (левый/правый через раз), 4с --");
    let mut left_turn = true;
    run_for(device, Duration::from_secs(4), BASE_INTERVAL_MS, |d| {
        if left_turn {
            write_motor(d, THROTTLE_MOTOR_LEFT, 255);
        } else {
            write_motor(d, THROTTLE_MOTOR_RIGHT, 255);
        }
        left_turn = !left_turn;
    });
    stop_all(device);
    pause("   [пауза 1с]", 1000);
}

/// Гипотеза 3: инерция мотора — тяжёлый эксцентрик может не успевать
/// раскрутиться за короткое окно ШИМ. Даём короткие "всплески" разной
/// длины на полной интенсивности (ре-отправляя кадр каждые 10мс внутри
/// всплеска, чтобы не упереться в таймаут прошивки из Этапа 1) и слушаем,
/// с какой длины появляется полноценный гул, а не просто щелчок/толчок.
fn phase3_inertia_test(device: &HidDevice) {
    println!("\n=== ЭТАП 3: инерция мотора ===");
    println!(
        "Импульсы 255 разной длины на ОБОИХ моторах, между ними пауза. Слушай, с какой длины\nпоявляется полноценный гул, а не просто щелчок.\n"
    );

    for pulse_ms in [20u64, 40, 80, 160, 320] {
        println!("-- импульс {pulse_ms}мс --");
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(pulse_ms) {
            write_motor(device, THROTTLE_MOTOR_LEFT, 255);
            write_motor(device, THROTTLE_MOTOR_RIGHT, 255);
            thread::sleep(Duration::from_millis(10));
        }
        stop_all(device);
        thread::sleep(Duration::from_millis(400));
    }
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "all".to_string());
    println!("Диагностика прерывистой вибрации РУД (VID 0x4098 PID 0xB920)");
    println!("Закрой основное приложение и SimAppPro перед запуском!\n");

    let device = open_throttle();
    stop_all(&device);

    match arg.as_str() {
        "1" => phase1_timeout_test(&device),
        "2" => phase2_double_write_test(&device),
        "3" => phase3_inertia_test(&device),
        _ => {
            phase1_timeout_test(&device);
            phase2_double_write_test(&device);
            phase3_inertia_test(&device);
        }
    }

    stop_all(&device);
    println!("\nГотово. Моторы выключены.");
}
