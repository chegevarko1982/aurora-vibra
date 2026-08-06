// CLI-стенд для прощупывания эффекта срыва потока (Bf 109 F-4) на живом
// железе БЕЗ запуска игры и основного приложения.
//
// Гоняет РЕАЛЬНЫЙ движок — тот же `WtRumbleState`/`StallState`, что и
// продовый WT-конвейер (src/wt_link/rumble.rs) — по скриптованному профилю
// угла атаки (AoA) и шлёт результат напрямую на подключённые Winwing-
// устройства. Три фазы одним прогоном (см. живой тест/правки в rumble.rs):
//   1) ПОДХОД — плавный ease-in рост при приближении к порогу сваливания;
//   2) СРЫВ/ШТОПОР — break-импульс на самом пороге, затем прямоугольная
//      пульсация пауза/полная мощность (0/255), держится всю фазу;
//   3) ВОССТАНОВЛЕНИЕ — AoA падает ниже порога с запасом, тишина, взвод
//      break-импульса на следующий круг.
// Три круга подряд, потом моторы глушатся и процесс завершается.
//
// cargo run --features "dev-tools app" --bin test_stall
//
// ВАЖНО: закрой основное приложение и SimAppPro перед запуском — HID-
// устройство может быть открыто только одним процессом одновременно.

use aurora_vibra::WtConfig;
use aurora_vibra::hid::protocol::{
    THROTTLE_MOTOR_LEFT, THROTTLE_MOTOR_RIGHT, WW_VID, build_simapp_vibe_frame,
    build_throttle_vibe_frame, is_ursa_minor_throttle, ursa_model_name,
};
use aurora_vibra::wt_link::aero_profiles::BF_109_F4;
use aurora_vibra::wt_link::rumble::WtRumbleState;
use aurora_vibra::wt_link::vars::WtVars;
use hidapi::{HidApi, HidDevice};
use std::time::Duration;

const REPORT_ID: u8 = 0x02;
const OUT_LEN: u16 = 14;
const TICK_MS: u64 = 20; // 50 Гц — с запасом на 5 Гц пилообразной пульсации

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

fn main() {
    println!("=== Aurora Vibra — стенд эффекта срыва потока (Bf 109 F-4) ===");
    println!(
        "Подход (линейный рост) -> break-импульс -> пауза/255 в срыве -> восстановление. 3 круга."
    );
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
        eprintln!(
            "Устройства Winwing не найдены. Проверь подключение и что не занято другим процессом."
        );
        return;
    }
    println!();

    let mut engine = WtRumbleState::new();
    let cfg = WtConfig::default(); // stall_enabled=true, stall_ceiling=80 (см. WT_STALL_CEILING_HARD_CAP)
    let stall_aoa = BF_109_F4.clean_stall_aoa_deg as f64; // 19.9° на чистом крыле (flaps_pct=0)

    let mut vars = WtVars {
        in_mission: true,
        vehicle_type: "bf-109f-4".to_string(),
        ias_kmh: 300.0, // выше safety cutoff, чтобы не глушило эффект
        flaps_pct: 0.0,
        ..WtVars::default()
    };

    let approach_s = 2.0;
    let stall_hold_s = 6.0;
    let recover_s = 2.0;
    let cycle_s = approach_s + stall_hold_s + recover_s;
    let cycles = 3;

    let dt = TICK_MS as f64 / 1000.0;
    let mut t = 0.0f64;
    let mut last_phase = "";

    while t < cycle_s * cycles as f64 {
        let phase_t = t % cycle_s;
        let (aoa, phase_name) = if phase_t < approach_s {
            let p = phase_t / approach_s;
            (stall_aoa - 6.0 + 5.95 * p, "ПОДХОД (плавный рост)")
        } else if phase_t < approach_s + stall_hold_s {
            (stall_aoa + 5.0, "СРЫВ/ШТОПОР (пауза/255)")
        } else {
            (stall_aoa - 5.0, "ВОССТАНОВЛЕНИЕ (тишина)")
        };
        vars.aoa_deg = aoa;
        vars.t = t;

        let out = engine.step(&vars, &cfg, false);
        for d in &devices {
            d.send(out.joystick_intensity, out.throttle_left_intensity);
        }

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
