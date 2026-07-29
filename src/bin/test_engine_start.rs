// CLI-стенд для прощупывания эффекта пуска/останова двигателя на живом
// железе БЕЗ запуска игры и основного приложения.
//
// Гоняет РЕАЛЬНЫЙ движок — тот же `WtRumbleState`/`EngineState`, что и
// продовый WT-конвейер (src/wt_link/rumble.rs) — по скриптованному профилю
// RPM 1, воспроизводящему живой лог полного цикла пуск→останов
// (wt_probe_sessions/session_20260729_151535.jsonl, Bf 109 F-4):
//   1) ПРОКРУТКА (0-2.35с)     — стартер, RPM 0->35
//   2) СХВАТЫВАНИЕ (2.35-2.5с) — резкий скачок RPM 35->120
//   3) РАСКРУТКА (2.5-5.4с)    — RPM 120->485, флейр мощности
//   4) ХОЛОСТЫЕ (5.4-11.7с)    — устойчиво, эффект должен молчать
//   5) КОМАНДА "СТОП"          — мощность обнуляется, RPM ещё 485
//   6) ВЫБЕГ (11.7-29.9с)      — почти линейный спад RPM 485->0
// Один прогон, потом моторы глушатся и процесс завершается.
//
// cargo run --features "dev-tools app" --bin test_engine_start
//
// ВАЖНО: закрой основное приложение и SimAppPro перед запуском — HID-
// устройство может быть открыто только одним процессом одновременно.

use aurora_vibra::hid::protocol::{
    THROTTLE_MOTOR_LEFT, THROTTLE_MOTOR_RIGHT, WW_VID, build_simapp_vibe_frame,
    build_throttle_vibe_frame, is_ursa_minor_throttle, ursa_model_name,
};
use aurora_vibra::wt_link::rumble::WtRumbleState;
use aurora_vibra::wt_link::vars::WtVars;
use aurora_vibra::WtConfig;
use hidapi::{HidApi, HidDevice};
use std::time::Duration;

const REPORT_ID: u8 = 0x02;
const OUT_LEN: u16 = 14;
const TICK_MS: u64 = 20; // 50 Гц — с запасом на потолок 6.5 Гц пульсации эффекта

struct OpenDevice {
    dev: HidDevice,
    pid: u16,
}

impl OpenDevice {
    fn send(&self, joystick: u8, throttle: u8) {
        if is_ursa_minor_throttle(self.pid) {
            let l = build_throttle_vibe_frame(REPORT_ID, OUT_LEN, THROTTLE_MOTOR_LEFT, throttle);
            let r = build_throttle_vibe_frame(REPORT_ID, OUT_LEN, THROTTLE_MOTOR_RIGHT, throttle);
            let _ = self.dev.write(&l);
            let _ = self.dev.write(&r);
        } else {
            let f = build_simapp_vibe_frame(self.pid, REPORT_ID, OUT_LEN, joystick);
            let _ = self.dev.write(&f);
        }
    }
}

/// Обороты в момент времени `t` — воспроизводит форму живого лога кусочно
/// линейными участками (см. фазы в шапке файла). Возвращает (rpm, power).
fn scripted_rpm_power(t: f64) -> (f64, f64) {
    const CRANK_END: f64 = 2.35;
    const CATCH_END: f64 = 2.50;
    const SPOOL_END: f64 = 5.40;
    const IDLE_END: f64 = 11.70;
    const COAST_END: f64 = 29.90;

    if t < CRANK_END {
        // Прокрутка стартером: 0 -> 35 об/мин, ~15 об/мин/с.
        let p = t / CRANK_END;
        (35.0 * p, 0.0)
    } else if t < CATCH_END {
        // Схватывание: резкий скачок 35 -> 120.
        let p = (t - CRANK_END) / (CATCH_END - CRANK_END);
        (35.0 + 85.0 * p, 0.0)
    } else if t < SPOOL_END {
        // Раскрутка/флейр: 120 -> 485, мощность появляется во второй половине.
        let p = (t - CATCH_END) / (SPOOL_END - CATCH_END);
        let power = if p > 0.4 { 60.0 * (1.0 - (p - 0.4) / 0.6) } else { 0.0 };
        (120.0 + 365.0 * p, power.max(0.0))
    } else if t < IDLE_END {
        // Устойчивые холостые — эффект должен молчать.
        (485.0, 1.3)
    } else if t < COAST_END {
        // Выбег после команды "стоп": мощность в ноль сразу, RPM падает
        // почти линейно 485 -> 0.
        let p = (t - IDLE_END) / (COAST_END - IDLE_END);
        (485.0 * (1.0 - p).max(0.0), 0.0)
    } else {
        (0.0, 0.0)
    }
}

fn main() {
    println!("=== Aurora Vibra — стенд эффекта пуска/останова двигателя ===");
    println!("Прокрутка -> схватывание -> раскрутка -> холостые (тишина) -> стоп -> выбег.");
    println!("Закрой основное приложение и SimAppPro перед запуском!\n");

    let api = HidApi::new().expect("не удалось инициализировать HID API");
    let mut devices = Vec::new();
    for d in api.device_list() {
        if d.vendor_id() != WW_VID || d.usage_page() != 0x0001 || d.usage() != 0x0004 {
            continue;
        }
        let pid = d.product_id();
        match api.open_path(d.path()) {
            Ok(dev) => {
                println!("Подключено: {} (PID 0x{pid:04X})", ursa_model_name(pid));
                devices.push(OpenDevice { dev, pid });
            }
            Err(e) => println!("Не удалось открыть {:?}: {e}", d.path()),
        }
    }
    if devices.is_empty() {
        eprintln!("Устройства Winwing не найдены. Проверь подключение и что не занято другим процессом.");
        return;
    }
    println!();

    let mut engine = WtRumbleState::new();
    let cfg = WtConfig::default(); // engine_start_enabled=true, engine_start_peak=200

    let mut vars = WtVars {
        in_mission: true,
        vehicle_type: "bf-109f-4".to_string(),
        ..WtVars::default()
    };

    const RUN_END_S: f64 = 32.0; // с запасом после конца выбега (29.9с)

    let dt = TICK_MS as f64 / 1000.0;
    let mut t = 0.0f64;
    let mut last_phase = "";

    while t < RUN_END_S {
        let (rpm, power) = scripted_rpm_power(t);
        vars.rpm_1 = rpm;
        vars.power_1_hp = power;
        vars.t = t;

        let out = engine.step(&vars, &cfg, false);
        for d in &devices {
            d.send(out.joystick_intensity, out.throttle_left_intensity);
        }

        let phase_name = if t < 2.35 {
            "ПРОКРУТКА"
        } else if t < 2.50 {
            "СХВАТЫВАНИЕ"
        } else if t < 5.40 {
            "РАСКРУТКА/ФЛЕЙР"
        } else if t < 11.70 {
            "ХОЛОСТЫЕ (тишина)"
        } else if t < 29.90 {
            "ВЫБЕГ"
        } else {
            "ОСТАНОВЛЕН"
        };
        if phase_name != last_phase {
            println!("[{t:5.1}s] {phase_name}");
            last_phase = phase_name;
        }

        std::thread::sleep(Duration::from_millis(TICK_MS));
        t += dt;
    }

    for d in &devices {
        d.send(0, 0);
    }
    println!("\nГотово, моторы выключены.");
}
