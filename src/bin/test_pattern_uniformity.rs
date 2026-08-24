// Диагностический стенд для жалобы "пульсирующий эффект ощущается
// неравномерно". Юнит-тест уже доказал, что ДАННЫЕ формы ровные (5 Гц / 50%
// даёт ровно 20 участков по 5 отсчётов) — остаётся два подозреваемых, и
// различить их можно только на живом железе:
//   1. Тракт: моменты отправки не лежат на ровной сетке (планировщик ОС,
//      кадры egui в предпросмотре, дрожание сетевого опроса).
//   2. Железо/моторы: инерция разгона/остановки вибромотора.
//
// Этот стенд снимает подозреваемого №1 полностью: паттерн считается ЦЕЛО-
// ЧИСЛЕННО (никакой арифметики с плавающей точкой в генерации — иначе стенд
// унаследует ровно тот дефект, который проверяет), а отправка идёт по
// АБСОЛЮТНЫМ дедлайнам (не sleep(tick) в цикле, чтобы ошибка не копилась).
// Никакого движка эффектов, egui или телеметрии — только hidapi напрямую,
// как в test_gun1.rs/test_vibro.rs, минуя hid_worker и его 20Гц-ограничение.
//
// В конце печатается отчёт по сетке (отклонение от идеального дедлайна,
// интервалы между отправками) и по времени device.write() — если сетка сама
// уехала или write() блокируется надолго, вывод про железо делать нельзя.
//
// cargo run --features dev-tools --bin test_pattern_uniformity -- \
//     --tick-ms 20 --period-ticks 10 --duty-ticks 5 --amp 255 --seconds 20 --device joystick
//
// ВАЖНО: закрой основное приложение и SimAppPro перед запуском — HID-
// устройство может быть открыто только одним процессом одновременно.

use aurora_vibra::hid::protocol::{
    THROTTLE_MOTOR_LEFT, THROTTLE_MOTOR_RIGHT, WW_VID, build_orion_joystick_vibe_frame,
    build_orion_throttle_vibe_frames, build_simapp_vibe_frame, build_throttle_vibe_frame,
    is_orion_joystick, is_orion_throttle, is_ursa_minor_throttle, ursa_model_name,
};
use hidapi::{HidApi, HidDevice};
use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

// Report ID и длина буфера одинаковы для всех Winwing Ursa Minor устройств —
// см. golden bytes в hid/protocol.rs и все test_*.rs стенды в этой папке.
const REPORT_ID: u8 = 0x02;
const OUT_LEN: u16 = 14;

// Штатная гранулярность сна на Windows — около 15.6мс. Спим до дедлайна
// минус этот запас, а последние миллисекунды докручиваем busy-wait'ом, иначе
// голый thread::sleep разрушит сетку 20мс.
const SPIN_MARGIN: Duration = Duration::from_millis(2);

/// Значение паттерна на такте `i`. ЦЕЛОЧИСЛЕННО, без float — это и есть тот
/// самый "ровный" генератор, который проверяет весь стенд.
fn pattern_value(i: u64, period_ticks: u64, duty_ticks: u64, amp: u8) -> u8 {
    if period_ticks == 0 {
        return 0;
    }
    let duty = duty_ticks.min(period_ticks);
    if i % period_ticks < duty { amp } else { 0 }
}

#[derive(Clone, Copy)]
enum DeviceKind {
    Joystick,
    Throttle,
}

struct Args {
    tick_ms: u64,
    period_ticks: u64,
    duty_ticks: u64,
    amp: u8,
    seconds: u64,
    device: DeviceKind,
    /// Прогнать ТУ ЖЕ сетку расписания, но без устройства и без вибрации —
    /// нужен, чтобы измерить точность самого планировщика на конкретной
    /// машине ДО того, как делать выводы про железо. Если сетка уехала уже
    /// здесь, разговор про инерцию моторов преждевременен.
    dry_run: bool,
    /// Только перечислить найденные вибро-интерфейсы Winwing и выйти —
    /// ничего не открывать и не включать. Первое, что стоит сделать, когда
    /// стенд «ничего не делает».
    list: bool,
    /// Ограничить работу одним конкретным PID (см. `--list`) — когда
    /// интерфейсов несколько и надо понять, за каким именно стоит мотор.
    pid: Option<u16>,
}

fn parse_args() -> Args {
    let mut args = Args {
        tick_ms: 20,
        period_ticks: 10,
        duty_ticks: 5,
        amp: 255,
        seconds: 20,
        device: DeviceKind::Joystick,
        dry_run: false,
        list: false,
        pid: None,
    };

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        let key = raw[i].as_str();
        let val = raw.get(i + 1).map(String::as_str);
        match key {
            "--tick-ms" => {
                args.tick_ms = val.and_then(|v| v.parse().ok()).unwrap_or(args.tick_ms);
                i += 2;
            }
            "--period-ticks" => {
                args.period_ticks = val
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(args.period_ticks);
                i += 2;
            }
            "--duty-ticks" => {
                args.duty_ticks = val.and_then(|v| v.parse().ok()).unwrap_or(args.duty_ticks);
                i += 2;
            }
            "--amp" => {
                args.amp = val.and_then(|v| v.parse().ok()).unwrap_or(args.amp);
                i += 2;
            }
            "--seconds" => {
                args.seconds = val.and_then(|v| v.parse().ok()).unwrap_or(args.seconds);
                i += 2;
            }
            "--device" => {
                args.device = match val {
                    Some("throttle") => DeviceKind::Throttle,
                    Some("joystick") => DeviceKind::Joystick,
                    other => {
                        eprintln!("Неизвестное значение --device: {other:?}, использую joystick");
                        DeviceKind::Joystick
                    }
                };
                i += 2;
            }
            "--dry-run" => {
                args.dry_run = true;
                i += 1;
            }
            "--list" => {
                args.list = true;
                i += 1;
            }
            "--pid" => {
                args.pid = val.and_then(|v| {
                    let v = v.trim_start_matches("0x").trim_start_matches("0X");
                    u16::from_str_radix(v, 16).ok()
                });
                if args.pid.is_none() {
                    eprintln!(
                        "Не разобрал --pid {val:?}, ожидается шестнадцатеричное, например 0xBC2A"
                    );
                }
                i += 2;
            }
            other => {
                eprintln!("Неизвестный аргумент: {other}, пропускаю");
                i += 1;
            }
        }
    }

    args
}

/// Семейство устройства — от него зависит ФОРМАТ кадра вибрации. Разделение
/// ровно то же, что в `hid::worker::hid_send_out`, и это принципиально:
/// раньше стенд знал только про Ursa Minor и слал её кадр всему, что не
/// является РУД Ursa Minor. Джойстик и РУД линейки Orion такой кадр не
/// понимают — устройство находилось, запись проходила без ошибки, а
/// вибрации не было.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DeviceKindFound {
    UrsaThrottle,
    OrionThrottle,
    OrionJoystick,
    UrsaJoystick,
}

impl DeviceKindFound {
    fn of(pid: u16) -> Self {
        if is_ursa_minor_throttle(pid) {
            Self::UrsaThrottle
        } else if is_orion_throttle(pid) {
            Self::OrionThrottle
        } else if is_orion_joystick(pid) {
            Self::OrionJoystick
        } else {
            Self::UrsaJoystick
        }
    }

    fn is_throttle(self) -> bool {
        matches!(self, Self::UrsaThrottle | Self::OrionThrottle)
    }

    fn label(self) -> &'static str {
        match self {
            Self::UrsaThrottle => "РУД Ursa Minor (моторы L/R раздельно)",
            Self::OrionThrottle => "РУД Orion (два кадра, одна интенсивность)",
            Self::OrionJoystick => "джойстик Orion",
            Self::UrsaJoystick => "джойстик Ursa Minor / SimApp",
        }
    }
}

struct FoundDevice {
    path: CString,
    pid: u16,
    ifnum: i32,
    kind: DeviceKindFound,
}

/// Все вибро-интерфейсы Winwing (usage_page/usage фильтруют именно тот
/// интерфейс, в который пишет продовый `hid::worker`).
fn list_devices(api: &HidApi) -> Vec<FoundDevice> {
    api.device_list()
        .filter(|d| d.vendor_id() == WW_VID && d.usage_page() == 0x0001 && d.usage() == 0x0004)
        .map(|d| FoundDevice {
            path: d.path().to_owned(),
            pid: d.product_id(),
            ifnum: d.interface_number(),
            kind: DeviceKindFound::of(d.product_id()),
        })
        .collect()
}

/// ВСЕ подходящие интерфейсы, а не первый попавшийся. Так делает продовый
/// `hid::worker::hid_send_out`, и это не мелочь: у одного физического
/// джойстика может быть несколько вибро-интерфейсов с разными PID (на
/// машине пользователя — 0xB980 «UNKNOWN» и 0xBC2A «URSA MINOR FIGHTER R»),
/// и заранее неизвестно, за каким из них реально стоит мотор. Стенд,
/// выбиравший первый по списку, писал в 0xB980 — запись проходила без
/// ошибки, а вибрации не было, из-за чего казалось, что «джойстик не
/// работает вовсе».
fn find_devices(api: &HidApi, want_throttle: bool, pid_filter: Option<u16>) -> Vec<FoundDevice> {
    list_devices(api)
        .into_iter()
        .filter(|d| d.kind.is_throttle() == want_throttle)
        .filter(|d| pid_filter.is_none_or(|pid| d.pid == pid))
        .collect()
}

fn stop_motors(dev: &HidDevice, pid: u16, kind: DeviceKindFound) {
    if kind == DeviceKindFound::UrsaThrottle {
        let _ = dev.write(&build_throttle_vibe_frame(
            REPORT_ID,
            OUT_LEN,
            THROTTLE_MOTOR_LEFT,
            0,
        ));
        let _ = dev.write(&build_throttle_vibe_frame(
            REPORT_ID,
            OUT_LEN,
            THROTTLE_MOTOR_RIGHT,
            0,
        ));
    } else if kind == DeviceKindFound::OrionThrottle {
        for frame in build_orion_throttle_vibe_frames(REPORT_ID, OUT_LEN, 0) {
            let _ = dev.write(&frame);
        }
    } else if kind == DeviceKindFound::OrionJoystick {
        let _ = dev.write(&build_orion_joystick_vibe_frame(REPORT_ID, OUT_LEN, 0));
    } else {
        let _ = dev.write(&build_simapp_vibe_frame(pid, REPORT_ID, OUT_LEN, 0));
    }
}

/// Гарантия "по завершении, в т.ч. по ошибке — нулевая интенсивность".
/// Drop выполняется и при panic (unwind), поэтому достаточно держать одну
/// такую страховку на всё время открытого устройства.
struct StopGuard<'a> {
    opened: &'a [(FoundDevice, HidDevice)],
}

impl Drop for StopGuard<'_> {
    fn drop(&mut self) {
        for (f, dev) in self.opened {
            stop_motors(dev, f.pid, f.kind);
        }
    }
}

/// Отправляет один такт паттерна на устройство и возвращает, сколько
/// заняла сама запись в HID (device.write()) — если она блокируется, это
/// источник неравномерности, и это должно быть видно в отчёте.
fn send_tick(dev: &HidDevice, pid: u16, kind: DeviceKindFound, byte: u8) -> Duration {
    let t0 = Instant::now();
    match kind {
        DeviceKindFound::UrsaThrottle => {
            for addr in [THROTTLE_MOTOR_LEFT, THROTTLE_MOTOR_RIGHT] {
                let frame = build_throttle_vibe_frame(REPORT_ID, OUT_LEN, addr, byte);
                if let Err(e) = dev.write(&frame) {
                    eprintln!("  !! write() FAILED (мотор 0x{addr:02X}): {e}");
                }
            }
        }
        DeviceKindFound::OrionThrottle => {
            for frame in build_orion_throttle_vibe_frames(REPORT_ID, OUT_LEN, byte) {
                if let Err(e) = dev.write(&frame) {
                    eprintln!("  !! write() FAILED (Orion throttle): {e}");
                }
            }
        }
        DeviceKindFound::OrionJoystick => {
            let frame = build_orion_joystick_vibe_frame(REPORT_ID, OUT_LEN, byte);
            if let Err(e) = dev.write(&frame) {
                eprintln!("  !! write() FAILED (Orion joystick): {e}");
            }
        }
        DeviceKindFound::UrsaJoystick => {
            let frame = build_simapp_vibe_frame(pid, REPORT_ID, OUT_LEN, byte);
            if let Err(e) = dev.write(&frame) {
                eprintln!("  !! write() FAILED: {e}");
            }
        }
    }
    t0.elapsed()
}

/// Спит до `deadline` по абсолютному времени: сон крупными шагами, пока до
/// дедлайна больше SPIN_MARGIN (штатная гранулярность сна на Windows ~15.6мс
/// не даёт спать точнее), затем busy-wait на spin_loop() для последних
/// миллисекунд.
fn sleep_until(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let remaining = deadline - now;
        if remaining > SPIN_MARGIN {
            thread::sleep(remaining - SPIN_MARGIN);
        } else {
            std::hint::spin_loop();
        }
    }
}

fn print_report(deviations_ms: &[f64], instants: &[Instant], write_ms: &[f64]) {
    println!();
    println!("=== Отчёт по сетке отправки ===");
    println!("Отправок: {}", deviations_ms.len());

    if !deviations_ms.is_empty() {
        let avg_dev = deviations_ms.iter().sum::<f64>() / deviations_ms.len() as f64;
        let max_dev = deviations_ms.iter().copied().fold(0.0_f64, f64::max);
        println!(
            "Отклонение от идеального дедлайна: среднее {avg_dev:.3} мс, максимум {max_dev:.3} мс"
        );
    }

    if instants.len() >= 2 {
        let intervals_ms: Vec<f64> = instants
            .windows(2)
            .map(|w| w[1].duration_since(w[0]).as_secs_f64() * 1000.0)
            .collect();
        let avg_iv = intervals_ms.iter().sum::<f64>() / intervals_ms.len() as f64;
        let min_iv = intervals_ms.iter().copied().fold(f64::INFINITY, f64::min);
        let max_iv = intervals_ms.iter().copied().fold(0.0_f64, f64::max);
        println!(
            "Интервал между соседними отправками: среднее {avg_iv:.3} мс, минимум {min_iv:.3} мс, максимум {max_iv:.3} мс"
        );
    }

    // Пустой `write_ms` бывает только в режиме --dry-run: устройство там не
    // открывается вовсе, поэтому и про моторы врать не надо.
    if write_ms.is_empty() {
        println!();
        println!("Готово. Устройство не открывалось, вибрации не было.");
    } else {
        let avg_w = write_ms.iter().sum::<f64>() / write_ms.len() as f64;
        let max_w = write_ms.iter().copied().fold(0.0_f64, f64::max);
        println!("Время device.write(): среднее {avg_w:.3} мс, максимум {max_w:.3} мс");
        println!();
        println!("Готово, моторы выключены.");
    }
}

/// Прогон расписания БЕЗ устройства: та же арифметика дедлайнов и тот же
/// `sleep_until`, только вместо записи в HID — ничего. Отчёт печатается тот
/// же, поэтому по нему видно, на что способен планировщик этой машины сам по
/// себе, отдельно от вибромоторов и от времени `device.write()`.
fn run_dry(args: &Args, tick_ms: u64, duty_ticks: u64) {
    let total_ticks = (args.seconds * 1000) / tick_ms;
    println!("РЕЖИМ БЕЗ УСТРОЙСТВА: играю {total_ticks} тактов вхолостую, вибрации не будет.");
    println!();

    let mut deviations_ms: Vec<f64> = Vec::with_capacity(total_ticks as usize);
    let mut send_instants: Vec<Instant> = Vec::with_capacity(total_ticks as usize);

    let start = Instant::now();
    for i in 0..total_ticks {
        let deadline = start + Duration::from_millis(tick_ms * i);
        sleep_until(deadline);
        // Значение всё равно вычисляем — чтобы стоимость самой генерации
        // паттерна тоже попала в измеряемую сетку, а не выпала из неё.
        let byte = pattern_value(i, args.period_ticks, duty_ticks, args.amp);
        std::hint::black_box(byte);
        let actual = Instant::now();
        deviations_ms.push(actual.saturating_duration_since(deadline).as_secs_f64() * 1000.0);
        send_instants.push(actual);
    }

    print_report(&deviations_ms, &send_instants, &[]);
}

fn main() {
    let args = parse_args();
    let tick_ms = args.tick_ms.max(1);
    let duty_ticks = args.duty_ticks.min(args.period_ticks);

    let freq_hz = if args.period_ticks > 0 {
        1000.0 / (tick_ms as f64 * args.period_ticks as f64)
    } else {
        0.0
    };
    let duty_pct = if args.period_ticks > 0 {
        (duty_ticks as f64 / args.period_ticks as f64) * 100.0
    } else {
        0.0
    };
    let on_ms = tick_ms * duty_ticks;
    let off_ms = tick_ms * args.period_ticks.saturating_sub(duty_ticks);
    let want_throttle = matches!(args.device, DeviceKind::Throttle);

    println!("=== Стенд равномерности паттерна (без движка эффектов, без egui) ===");
    println!("Такт отправки: {tick_ms} мс");
    println!("Тактов в периоде: {}", args.period_ticks);
    println!("Тактов «импульс»: {duty_ticks}");
    println!("Частота: {freq_hz:.3} Гц");
    println!("Скважность: {duty_pct:.1}%");
    println!("Импульс: {on_ms} мс, пауза: {off_ms} мс");
    println!("Амплитуда: {}/255", args.amp);
    println!("Длительность: {} с", args.seconds);
    println!(
        "Устройство: {}",
        if want_throttle {
            "throttle"
        } else {
            "joystick"
        }
    );
    println!();
    println!(
        "ВАЖНО: закрой основное приложение и SimAppPro — HID-устройство открывается только одним процессом."
    );
    println!();

    if args.dry_run {
        run_dry(&args, tick_ms, duty_ticks);
        return;
    }

    if args.list {
        let api = HidApi::new().expect("Не удалось инициализировать HID API");
        let found = list_devices(&api);
        if found.is_empty() {
            println!("Вибро-интерфейсов Winwing не найдено.");
            println!("Проверь подключение и что закрыты Aurora Vibra и SimAppPro.");
        } else {
            println!("Найдено вибро-интерфейсов: {}", found.len());
            for d in &found {
                println!(
                    "  PID 0x{:04X}  if#{:<2}  {:<22}  -> {}",
                    d.pid,
                    d.ifnum,
                    ursa_model_name(d.pid),
                    d.kind.label()
                );
            }
        }
        return;
    }

    let mut api = HidApi::new().expect("Не удалось инициализировать HID API");
    let _ = api.refresh_devices();
    let found = find_devices(&api, want_throttle, args.pid);
    if found.is_empty() {
        panic!(
            "Устройство не найдено (device={}{}). Запусти с --list, чтобы увидеть, что подключено.",
            if want_throttle {
                "throttle"
            } else {
                "joystick"
            },
            match args.pid {
                Some(p) => format!(", pid=0x{p:04X}"),
                None => String::new(),
            }
        );
    }

    let mut opened: Vec<(FoundDevice, HidDevice)> = Vec::new();
    for f in found {
        match api.open_path(&f.path) {
            Ok(dev) => {
                println!(
                    "Открыт: PID 0x{:04X} if#{} — {} ({})",
                    f.pid,
                    f.ifnum,
                    ursa_model_name(f.pid),
                    f.kind.label()
                );
                opened.push((f, dev));
            }
            Err(e) => {
                eprintln!("  !! не удалось открыть PID 0x{:04X}: {e}", f.pid);
            }
        }
    }
    if opened.is_empty() {
        panic!("Ни одно устройство не открылось. Закрыты ли Aurora Vibra и SimAppPro?");
    }
    println!(
        "Пишу во ВСЕ {} интерфейса(ов) сразу — как это делает само приложение.",
        opened.len()
    );

    // С этого момента и до конца main — страховка: любой выход (в т.ч.
    // panic) отправит нулевую интенсивность при Drop.
    let guard = StopGuard { opened: &opened };
    for (f, dev) in &opened {
        stop_motors(dev, f.pid, f.kind);
    }

    let total_ticks = (args.seconds * 1000) / tick_ms;
    let ticks_per_second = (1000 / tick_ms).max(1);

    let mut deviations_ms: Vec<f64> = Vec::with_capacity(total_ticks as usize);
    let mut send_instants: Vec<Instant> = Vec::with_capacity(total_ticks as usize);
    let mut write_ms: Vec<f64> = Vec::with_capacity(total_ticks as usize);

    println!("Играю {} тактов...", total_ticks);
    let start = Instant::now();
    for i in 0..total_ticks {
        let deadline = start + Duration::from_millis(tick_ms * i);
        sleep_until(deadline);

        let byte = pattern_value(i, args.period_ticks, duty_ticks, args.amp);
        let mut write_dur = Duration::ZERO;
        for (f, dev) in &opened {
            write_dur += send_tick(dev, f.pid, f.kind, byte);
        }
        let actual = Instant::now();

        let deviation_ms = actual.saturating_duration_since(deadline).as_secs_f64() * 1000.0;
        deviations_ms.push(deviation_ms);
        send_instants.push(actual);
        write_ms.push(write_dur.as_secs_f64() * 1000.0);

        if i % ticks_per_second == 0 {
            println!(
                "  t={:.1}с байт={}",
                i as f64 * tick_ms as f64 / 1000.0,
                byte
            );
        }
    }

    drop(guard);
    print_report(&deviations_ms, &send_instants, &write_ms);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_alternates_in_blocks_of_five() {
        let vals: Vec<u8> = (0..20).map(|i| pattern_value(i, 10, 5, 255)).collect();
        for (idx, chunk) in vals.chunks(5).enumerate() {
            let expect_on = idx % 2 == 0;
            for &v in chunk {
                assert_eq!(v, if expect_on { 255 } else { 0 });
            }
        }
    }

    #[test]
    fn pattern_zero_period_ticks_no_panic() {
        for i in 0..5u64 {
            assert_eq!(pattern_value(i, 0, 5, 255), 0);
        }
    }

    #[test]
    fn pattern_zero_duty_ticks_always_off() {
        for i in 0..20u64 {
            assert_eq!(pattern_value(i, 10, 0, 200), 0);
        }
    }

    #[test]
    fn pattern_duty_ge_period_always_on() {
        for i in 0..20u64 {
            assert_eq!(pattern_value(i, 10, 10, 200), 200);
            assert_eq!(pattern_value(i, 10, 15, 200), 200);
        }
    }
}
