// Симулятор посадки для A/B прослушивания эффекта Gear Strut Compression
// (Touchdown) на РЕАЛЬНОМ железе, ДО правок в rumble.rs. Обсуждение и план
// см. C:\Users\chege\.claude\plans\functional-napping-iverson.md
// ("Touchdown: раскладка по моторам + огибающая attack-hold-decay").
//
// Изолированные удары БЕЗ фонового шума (гул захода/пробега намеренно не
// проигрывается) — для чистоты сравнения слушаем только сам эффект:
//   left  — только левое колесо
//   right — только правое колесо
//   all   — левая основная касается первой, правая — с небольшим сдвигом
//           (0.15с, посадка не бывает строго симметричной), затем
//           ФИКСИРОВАННАЯ пауза 3с, затем носовое (среднее) колесо —
//           финальный удар, самый сильный (эффект "всех трёх стоек разом",
//           длительность равна основной, идёт на все три мотора)
// Каждая группа проигрывается ДВАЖДЫ подряд: сначала СЕЙЧАС (легаси), потом
// НОВОЕ, с паузой и меткой в консоли между ними.
//
// Проверяет на живом железе ДВЕ независимые правки разом:
//  1. Раскладка по моторам: сейчас (split_touchdown=true, с чем тестировал
//     пользователь) левая основная стойка идёт ТОЛЬКО на throttle_left
//     (один из двух моторов РУДа), правая — на джойстик, нос — на
//     throttle_right. Новая раскладка: левая основная — на ОБА мотора РУДа
//     разом, правая — на джойстик (без изменений), нос — на ВСЕ ТРИ мотора
//     одновременно.
//  2. Огибающая: сейчас peak*(1-p)^n, где peak = 200..255 по жёсткости
//     посадки (разница пика всего ~21%, ERM физически не показывает такую
//     грануляцию за десятки мс). Новая: attack-hold-decay, пик ВСЕГДА 255
//     (гарантирует раскрутку ротора), жёсткость двигает ДЛИТЕЛЬНОСТЬ
//     (230..550мс основные, ×2.4 — заметно лучше кодирует жёсткость).
//
// Framing и обнаружение устройств: byte-in-byte как в hid/worker.rs (см.
// send_throttle_rumble/hid_send_out) и test_throttle_stutter.rs. REPORT_ID/
// OUT_LEN захардкожены как в test_hold.rs/test_throttle_stutter.rs (golden
// bytes, подтверждено на живом железе) — hid_query_caps_from_path приватна
// для hid::win32 и недоступна из bin-крейтов.
//
// cargo run --bin test_touchdown -- [left|right|all] [--severity F] [--interval MS]
// Без аргумента группы — прогоняет left, right, all подряд.
//
// КРИТИЧНО: интервал отправки по умолчанию 50мс — ТА ЖЕ сетка, что
// SEND_INTERVAL в hid/worker.rs. Не меняй без причины: смысл теста —
// почувствовать именно то, что дойдёт до мотора в бою, а не оптимистичную
// версию на более частой сетке.

use aurora_vibra::hid::protocol::{
    THROTTLE_MOTOR_LEFT, THROTTLE_MOTOR_RIGHT, WW_VID, build_simapp_vibe_frame,
    build_throttle_vibe_frame, is_ursa_minor_joystick, is_ursa_minor_throttle,
};
use hidapi::{HidApi, HidDevice};
use std::thread;
use std::time::{Duration, Instant};

const REPORT_ID: u8 = 0x02;
const OUT_LEN: u16 = 14;

// ═══════════════════════════════════════════════════════════════════════
// Обнаружение устройств
// ═══════════════════════════════════════════════════════════════════════

struct Devices {
    joystick: Option<(HidDevice, u16)>, // (handle, pid — нужен для channel_byte_for_pid)
    throttle: Option<HidDevice>,
}

fn open_devices() -> Devices {
    let api = HidApi::new().expect("Не удалось инициализировать HID API");

    let mut joystick = None;
    let mut throttle = None;

    for devinfo in api.device_list() {
        if devinfo.vendor_id() != WW_VID {
            continue;
        }
        // Тот же фильтр usage_page/usage, что и в hid_send_out (hid/worker.rs) —
        // у устройств WinWing на VID 0x4098 бывают посторонние интерфейсы
        // (не вибро-канал), их трогать не нужно.
        if !(devinfo.usage_page() == 0x0001 && devinfo.usage() == 0x0004) {
            continue;
        }
        let pid = devinfo.product_id();

        if joystick.is_none() && is_ursa_minor_joystick(pid) {
            if let Ok(d) = devinfo.open_device(&api) {
                println!("Найден джойстик: PID=0x{pid:04X}");
                joystick = Some((d, pid));
            }
        } else if throttle.is_none() && is_ursa_minor_throttle(pid) {
            if let Ok(d) = devinfo.open_device(&api) {
                println!("Найден РУД: PID=0x{pid:04X}");
                throttle = Some(d);
            }
        }
    }

    if joystick.is_none() {
        println!("!! Джойстик НЕ найден — правое колесо не будет ощущаться.");
    }
    if throttle.is_none() {
        println!("!! РУД НЕ найден — левое колесо и нос не будут ощущаться.");
    }

    Devices { joystick, throttle }
}

fn send(devices: &Devices, joystick: u8, throttle_left: u8, throttle_right: u8) {
    if let Some((dev, pid)) = &devices.joystick {
        let frame = build_simapp_vibe_frame(*pid, REPORT_ID, OUT_LEN, joystick);
        let _ = dev.write(&frame);
    }
    if let Some(dev) = &devices.throttle {
        let fl = build_throttle_vibe_frame(REPORT_ID, OUT_LEN, THROTTLE_MOTOR_LEFT, throttle_left);
        let _ = dev.write(&fl);
        let fr =
            build_throttle_vibe_frame(REPORT_ID, OUT_LEN, THROTTLE_MOTOR_RIGHT, throttle_right);
        let _ = dev.write(&fr);
    }
}

fn stop_all(devices: &Devices) {
    send(devices, 0, 0, 0);
}

// ═══════════════════════════════════════════════════════════════════════
// Огибающие
// ═══════════════════════════════════════════════════════════════════════

/// СЕЙЧАС (то, с чем тестировал пользователь): затухание от peak, без
/// гарантированного плато. Пик зависит от жёсткости (200..255), но разница
/// слишком мала, чтобы её различить на ERM за 100-250мс.
fn bump_legacy(t_s: f64, severity_frac: f64, duration_s: f64, decay_exp: i32) -> u8 {
    if t_s < 0.0 || t_s > duration_s {
        return 0;
    }
    let peak = 200.0 + 55.0 * severity_frac.clamp(0.0, 1.0);
    let p = (t_s / duration_s).clamp(0.0, 1.0);
    (peak * (1.0 - p).powi(decay_exp)).round() as u8
}

struct BumpSpec {
    attack_s: f64,
    hold_base_s: f64,
    hold_extra_s: f64,
    decay_base_s: f64,
    decay_extra_s: f64,
    decay_exp: i32,
}

// Параметры см. Шаг 2 плана. Итого: основные 230..550мс, нос 170..250мс.
const BUMP_MAIN_NEW: BumpSpec = BumpSpec {
    attack_s: 0.10,
    hold_base_s: 0.0,
    hold_extra_s: 0.20,
    decay_base_s: 0.13,
    decay_extra_s: 0.12,
    decay_exp: 2,
};
// Поправка по хардтесту: нос — это момент, когда самолёт УЖЕ полностью
// на земле (обе основные отработали), физически это эффект "всех трёх
// стоек разом", а не отдельное более слабое касание. Длительность
// сравнена с основной (230..550мс) — раньше нос был короче (170..250мс),
// что ощущалось слабее, хотя должно быть самым сильным событием посадки.
const BUMP_NOSE_NEW: BumpSpec = BUMP_MAIN_NEW;

/// НОВОЕ: пик ВСЕГДА 255 (attack + hold), жёсткость двигает длительность
/// плато/спада, а не амплитуду.
fn bump_new(t_s: f64, severity_frac: f64, spec: &BumpSpec) -> u8 {
    if t_s < 0.0 {
        return 0;
    }
    let severity_frac = severity_frac.clamp(0.0, 1.0);
    let flat = spec.attack_s + spec.hold_base_s + spec.hold_extra_s * severity_frac;
    let decay = spec.decay_base_s + spec.decay_extra_s * severity_frac;
    if t_s < flat {
        255
    } else if t_s < flat + decay {
        let p = (t_s - flat) / decay;
        (255.0 * (1.0 - p).powi(spec.decay_exp)).round() as u8
    } else {
        0
    }
}

fn bump_new_duration(severity_frac: f64, spec: &BumpSpec) -> f64 {
    let severity_frac = severity_frac.clamp(0.0, 1.0);
    spec.attack_s
        + spec.hold_base_s
        + spec.hold_extra_s * severity_frac
        + spec.decay_base_s
        + spec.decay_extra_s * severity_frac
}

// ═══════════════════════════════════════════════════════════════════════
// Группы ударов (изолированно, без фона)
// ═══════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy)]
enum Wheel {
    Left,
    Right,
    All,
}

impl Wheel {
    fn label(&self) -> &'static str {
        match self {
            Wheel::Left => "left",
            Wheel::Right => "right",
            Wheel::All => "all",
        }
    }
}

// Фиксированная пауза между касанием основных колёс и носового (среднего) —
// не завязана на жёсткость посадки, чтобы удары никогда не наслаивались и
// между ними было достаточно тишины для восприятия каждого по отдельности.
const NOSE_DELAY_S: f64 = 3.0;
const TAIL_S: f64 = 1.0; // тишина после последнего удара перед завершением

// Поправка по хардтесту: на посадке основные стойки касаются земли НЕ
// одновременно — сперва одна, затем другая (крен/снос на выравнивании).
// В ПРОДАКШЕНЕ (rumble.rs) это ничего не требует править: каждая стойка
// детектится независимо строго по своей телеметрии (gear_comp_left/right
// из SimConnect), так что естественная асимметрия времени касания там уже
// есть сама по себе. Здесь же, в оффлайн-тесте БЕЗ живой телеметрии, нужен
// хоть какой-то сдвиг, иначе демо на железе проиграет обе стойки как один
// идеально синхронный удар — нереалистично и маскирует раздельность каналов.
// Значение условное, для железного A/B, не проектное решение.
// NOSE_DELAY_S по-прежнему отсчитывается от t=0 (касание первой стойки),
// т.к. 3с делают разницу в доли секунды между стойками несущественной.
const MAIN_GAP_S: f64 = 0.15;

/// t=0 — касание основных колёс. Возвращает (joystick, throttle_left, throttle_right).
fn frame(t_s: f64, wheel: Wheel, severity: f64, use_new: bool) -> (u8, u8, u8) {
    match wheel {
        Wheel::Left => {
            // Легаси: левая основная -> только throttle_left.
            // Новое: левая основная -> ОБА мотора РУДа.
            let term = if use_new {
                bump_new(t_s, severity, &BUMP_MAIN_NEW)
            } else {
                bump_legacy(t_s, severity, 0.22, 3)
            };
            if use_new {
                (0, term, term)
            } else {
                (0, term, 0)
            }
        }
        Wheel::Right => {
            // Правая основная всегда идёт на джойстик — в обоих вариантах,
            // меняется только форма огибающей.
            let term = if use_new {
                bump_new(t_s, severity, &BUMP_MAIN_NEW)
            } else {
                bump_legacy(t_s, severity, 0.22, 3)
            };
            (term, 0, 0)
        }
        Wheel::All => {
            // Левая (throttle) касается первой в t=0, правая (joystick) —
            // на MAIN_GAP_S позже. Не одновременный удар, а два отдельных.
            let left_t = t_s;
            let right_t = t_s - MAIN_GAP_S;
            let left_main = if use_new {
                bump_new(left_t, severity, &BUMP_MAIN_NEW)
            } else {
                bump_legacy(left_t, severity, 0.22, 3)
            };
            let right_main = if use_new {
                bump_new(right_t, severity, &BUMP_MAIN_NEW)
            } else {
                bump_legacy(right_t, severity, 0.22, 3)
            };
            let nose_t = t_s - NOSE_DELAY_S;
            let nose = if use_new {
                bump_new(nose_t, severity, &BUMP_NOSE_NEW)
            } else {
                bump_legacy(nose_t, severity, 0.12, 5)
            };
            if use_new {
                // Нос -> ВСЕ ТРИ мотора одновременно (самолёт уже полностью
                // на земле — эффект "всех трёх стоек разом"), поэтому max
                // с уже идущим сигналом на каждом канале, а не отдельная фаза.
                let j = right_main.max(nose);
                let tl = left_main.max(nose);
                let tr = left_main.max(nose);
                (j, tl, tr)
            } else {
                // Легаси: левая -> throttle_left, правая -> joystick,
                // нос -> throttle_right (свободный мотор).
                (right_main, left_main, nose)
            }
        }
    }
}

fn group_end_s(wheel: Wheel, severity: f64, use_new: bool) -> f64 {
    let main_dur = if use_new {
        bump_new_duration(severity, &BUMP_MAIN_NEW)
    } else {
        0.22
    };
    let nose_dur = if use_new {
        bump_new_duration(severity, &BUMP_NOSE_NEW)
    } else {
        0.12
    };
    match wheel {
        Wheel::Left | Wheel::Right => main_dur + TAIL_S,
        Wheel::All => NOSE_DELAY_S + nose_dur + TAIL_S,
    }
}

fn play(devices: &Devices, wheel: Wheel, severity: f64, use_new: bool, interval_ms: u64) {
    let label = if use_new { "НОВОЕ" } else { "СЕЙЧАС (легаси)" };
    println!(
        "\n=== {label}: \"{}\" (severity={:.2}) ===",
        wheel.label(),
        severity
    );

    let end_s = group_end_s(wheel, severity, use_new);
    let start = Instant::now();
    let mut printed_left = false;
    let mut printed_right = false;
    let mut printed_nose = false;
    loop {
        let t_s = start.elapsed().as_secs_f64();
        if t_s > end_s {
            break;
        }
        if !printed_left {
            if matches!(wheel, Wheel::All) {
                println!("  -- КАСАНИЕ (левая) --");
            } else {
                println!("  -- КАСАНИЕ --");
            }
            printed_left = true;
        }
        if matches!(wheel, Wheel::All) && !printed_right && t_s >= MAIN_GAP_S {
            println!("  -- КАСАНИЕ (правая) --");
            printed_right = true;
        }
        if matches!(wheel, Wheel::All) && !printed_nose && t_s >= NOSE_DELAY_S {
            println!("  -- КАСАНИЕ (нос, финальный удар — все три стойки) --");
            printed_nose = true;
        }

        let (j, tl, tr) = frame(t_s, wheel, severity, use_new);
        send(devices, j, tl, tr);

        thread::sleep(Duration::from_millis(interval_ms));
    }
    stop_all(devices);
}

fn pause(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}

/// Легаси, потом новое, с паузой и меткой между ними — прямое A/B на одном
/// и том же событии.
fn play_compare(devices: &Devices, wheel: Wheel, severity: f64, interval_ms: u64) {
    println!("\n### Группа: {} ###", wheel.label());
    play(devices, wheel, severity, false, interval_ms);
    println!("\n[пауза 2с]");
    pause(2000);
    play(devices, wheel, severity, true, interval_ms);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut group_name: Option<String> = None;
    let mut interval_ms: u64 = 50; // ТА ЖЕ сетка, что SEND_INTERVAL в hid/worker.rs
    let mut severity: f64 = 1.0; // "финальный удар" — по умолчанию полная жёсткость

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--interval" => {
                if let Some(v) = args.get(i + 1) {
                    interval_ms = v.parse().unwrap_or(50);
                    i += 1;
                }
            }
            "--severity" => {
                if let Some(v) = args.get(i + 1) {
                    severity = v.parse().unwrap_or(1.0);
                    i += 1;
                }
            }
            other => group_name = Some(other.to_string()),
        }
        i += 1;
    }

    println!("Интервал отправки: {interval_ms}мс (боевой SEND_INTERVAL = 50мс)");
    println!("Жёсткость (severity): {severity:.2}");
    let devices = open_devices();

    let groups: Vec<Wheel> = match group_name.as_deref() {
        Some("left") => vec![Wheel::Left],
        Some("right") => vec![Wheel::Right],
        Some("all") => vec![Wheel::All],
        None => vec![Wheel::Left, Wheel::Right, Wheel::All],
        Some(other) => {
            println!("Неизвестная группа '{other}'. Используй: left | right | all");
            return;
        }
    };

    for wheel in groups {
        play_compare(&devices, wheel, severity, interval_ms);
    }

    stop_all(&devices);
    println!("\nГотово.");
}
