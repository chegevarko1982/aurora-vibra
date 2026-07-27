// GUI-версия test_throttle_stutter.rs: та же последовательность из трёх
// гипотез (таймаут прошивки / двойной write() подряд / инерция мотора),
// но вместо чтения таймингов в консоли — большое окно с надписью, какой
// именно вариант сейчас проигрывается на устройстве. Так удобнее сверять
// "что чувствую" с "что сейчас шлётся", не бегая взглядом между железом и
// терминалом.
//
// cargo run --bin test_throttle_stutter_gui
//
// ВАЖНО: закрой основное приложение и SimAppPro перед запуском — HID-
// устройство может быть открыто только одним процессом одновременно.
//
// Framing идентичен hid::protocol::build_throttle_vibe_frame (см. golden
// bytes в protocol.rs тестах) — байты здесь не пересобираются вручную.

use aurora_vibra::hid::protocol::{THROTTLE_MOTOR_LEFT, THROTTLE_MOTOR_RIGHT, build_throttle_vibe_frame};
use eframe::egui;
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

const REPORT_ID: u8 = 0x02;
const OUT_LEN: u16 = 14;
const PID_THROTTLE: u16 = 0xB920;

#[derive(Clone, Copy)]
enum MotorSet {
    Left,
    Both,
}

#[derive(Clone, Copy)]
enum SendPlan {
    /// Шаг без отправки — моторы уже выключены (пауза между вариантами).
    None,
    /// Постоянная отправка с заданным интервалом (Этап 1, 2a, 2b).
    Const { motors: MotorSet, interval: Duration },
    /// Оба мотора, но с паузой между двумя write() внутри кадра (Этап 2c).
    ConstGap { gap: Duration, interval: Duration },
    /// Чередование левого/правого мотора по кадрам (Этап 2d).
    Alternate { interval: Duration },
    /// Всплеск: оба мотора на 255, ре-отправка каждые `interval` (Этап 3,
    /// "включено"-часть импульса).
    Pulse { interval: Duration },
}

struct Step {
    phase: &'static str,
    variant: String,
    duration: Duration,
    send: SendPlan,
}

fn build_steps() -> Vec<Step> {
    let mut steps = Vec::new();

    // ─── Этап 1: таймаут прошивки ────────────────────────────────────────
    for interval_ms in [50u64, 30, 20, 10] {
        steps.push(Step {
            phase: "ЭТАП 1 · таймаут прошивки",
            variant: format!("оба мотора на 255, интервал {interval_ms}мс"),
            duration: Duration::from_secs(4),
            send: SendPlan::Const {
                motors: MotorSet::Both,
                interval: Duration::from_millis(interval_ms),
            },
        });
        steps.push(pause(1000, "ЭТАП 1 · таймаут прошивки"));
    }

    // ─── Этап 2: двойной write() подряд (интервал фиксирован 20мс) ───────
    const BASE_INTERVAL_MS: u64 = 20;
    steps.push(Step {
        phase: "ЭТАП 2 · двойной write() подряд",
        variant: "2a: ТОЛЬКО левый мотор, один write() в кадр".to_string(),
        duration: Duration::from_secs(4),
        send: SendPlan::Const {
            motors: MotorSet::Left,
            interval: Duration::from_millis(BASE_INTERVAL_MS),
        },
    });
    steps.push(pause(1000, "ЭТАП 2 · двойной write() подряд"));

    steps.push(Step {
        phase: "ЭТАП 2 · двойной write() подряд",
        variant: "2b: ОБА мотора, два write() подряд без паузы (как сейчас в проде)".to_string(),
        duration: Duration::from_secs(4),
        send: SendPlan::Const {
            motors: MotorSet::Both,
            interval: Duration::from_millis(BASE_INTERVAL_MS),
        },
    });
    steps.push(pause(1000, "ЭТАП 2 · двойной write() подряд"));

    steps.push(Step {
        phase: "ЭТАП 2 · двойной write() подряд",
        variant: "2c: ОБА мотора, 2мс паузы между write()".to_string(),
        duration: Duration::from_secs(4),
        send: SendPlan::ConstGap {
            gap: Duration::from_millis(2),
            interval: Duration::from_millis(BASE_INTERVAL_MS),
        },
    });
    steps.push(pause(1000, "ЭТАП 2 · двойной write() подряд"));

    steps.push(Step {
        phase: "ЭТАП 2 · двойной write() подряд",
        variant: "2d: чередование моторов по кадрам (левый/правый через раз)".to_string(),
        duration: Duration::from_secs(4),
        send: SendPlan::Alternate {
            interval: Duration::from_millis(BASE_INTERVAL_MS),
        },
    });
    steps.push(pause(1000, "ЭТАП 2 · двойной write() подряд"));

    // ─── Этап 3: инерция мотора ───────────────────────────────────────────
    for pulse_ms in [20u64, 40, 80, 160, 320] {
        steps.push(Step {
            phase: "ЭТАП 3 · инерция мотора",
            variant: format!("импульс {pulse_ms}мс на 255 (оба мотора)"),
            duration: Duration::from_millis(pulse_ms),
            send: SendPlan::Pulse {
                interval: Duration::from_millis(10),
            },
        });
        steps.push(pause(400, "ЭТАП 3 · инерция мотора"));
    }

    steps
}

fn pause(ms: u64, phase: &'static str) -> Step {
    Step {
        phase,
        variant: "пауза — моторы выключены".to_string(),
        duration: Duration::from_millis(ms),
        send: SendPlan::None,
    }
}

fn write_motor(device: &HidDevice, motor_addr: u8, intensity: u8) -> Result<(), String> {
    let frame = build_throttle_vibe_frame(REPORT_ID, OUT_LEN, motor_addr, intensity);
    device.write(&frame).map(|_| ()).map_err(|e| e.to_string())
}

fn stop_all(device: &HidDevice) {
    let _ = write_motor(device, THROTTLE_MOTOR_LEFT, 0);
    let _ = write_motor(device, THROTTLE_MOTOR_RIGHT, 0);
}

struct App {
    device: HidDevice,
    steps: Vec<Step>,
    current: usize,
    step_start: Instant,
    last_tick: Instant,
    alt_toggle: bool,
    running: bool,
    status: String,
}

impl App {
    fn new() -> Self {
        let api = HidApi::new().expect("Не удалось инициализировать HID API");
        let device = api.open(0x4098, PID_THROTTLE).unwrap_or_else(|_| {
            panic!("Throttle (PID 0xB920) не найден! Закрой основное приложение и SimAppPro.")
        });
        stop_all(&device);
        let now = Instant::now();
        Self {
            device,
            steps: build_steps(),
            current: 0,
            step_start: now,
            last_tick: now - Duration::from_secs(1),
            alt_toggle: true,
            running: true,
            status: String::new(),
        }
    }

    fn restart(&mut self) {
        stop_all(&self.device);
        self.current = 0;
        self.step_start = Instant::now();
        self.last_tick = self.step_start - Duration::from_secs(1);
        self.alt_toggle = true;
        self.running = true;
        self.status.clear();
    }

    fn enter_step(&mut self, idx: usize) {
        stop_all(&self.device);
        self.current = idx;
        self.step_start = Instant::now();
        self.last_tick = self.step_start - Duration::from_secs(1);
        self.alt_toggle = true;
    }

    fn tick(&mut self) {
        if !self.running || self.current >= self.steps.len() {
            return;
        }

        let elapsed = self.step_start.elapsed();
        let step_duration = self.steps[self.current].duration;
        if elapsed >= step_duration {
            let next = self.current + 1;
            if next >= self.steps.len() {
                stop_all(&self.device);
                self.running = false;
                self.status = "Готово! Моторы выключены.".to_string();
                return;
            }
            self.enter_step(next);
            return;
        }

        let interval = match self.steps[self.current].send {
            SendPlan::None => return,
            SendPlan::Const { interval, .. } => interval,
            SendPlan::ConstGap { interval, .. } => interval,
            SendPlan::Alternate { interval } => interval,
            SendPlan::Pulse { interval } => interval,
        };
        if self.last_tick.elapsed() < interval {
            return;
        }
        self.last_tick = Instant::now();

        let result = match self.steps[self.current].send {
            SendPlan::None => Ok(()),
            SendPlan::Const { motors, .. } => match motors {
                MotorSet::Left => write_motor(&self.device, THROTTLE_MOTOR_LEFT, 255),
                MotorSet::Both => write_motor(&self.device, THROTTLE_MOTOR_LEFT, 255)
                    .and_then(|_| write_motor(&self.device, THROTTLE_MOTOR_RIGHT, 255)),
            },
            SendPlan::ConstGap { gap, .. } => {
                let r = write_motor(&self.device, THROTTLE_MOTOR_LEFT, 255);
                std::thread::sleep(gap);
                r.and_then(|_| write_motor(&self.device, THROTTLE_MOTOR_RIGHT, 255))
            }
            SendPlan::Alternate { .. } => {
                let addr = if self.alt_toggle { THROTTLE_MOTOR_LEFT } else { THROTTLE_MOTOR_RIGHT };
                self.alt_toggle = !self.alt_toggle;
                write_motor(&self.device, addr, 255)
            }
            SendPlan::Pulse { .. } => write_motor(&self.device, THROTTLE_MOTOR_LEFT, 255)
                .and_then(|_| write_motor(&self.device, THROTTLE_MOTOR_RIGHT, 255)),
        };

        if let Err(e) = result {
            self.status = format!("Ошибка write(): {e}");
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.tick();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(16.0);

            if self.current < self.steps.len() {
                let step = &self.steps[self.current];
                let is_sending = !matches!(step.send, SendPlan::None);

                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Шаг {} из {}", self.current + 1, self.steps.len()))
                            .size(16.0)
                            .color(egui::Color32::GRAY),
                    );
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(step.phase).size(26.0).strong());
                    ui.add_space(20.0);

                    let color = if is_sending {
                        egui::Color32::from_rgb(255, 120, 40)
                    } else {
                        egui::Color32::from_rgb(120, 120, 120)
                    };
                    ui.label(egui::RichText::new(&step.variant).size(34.0).strong().color(color));
                    ui.add_space(20.0);

                    let badge = if is_sending { "● ВИБРАЦИЯ" } else { "○ ТИШИНА" };
                    ui.label(egui::RichText::new(badge).size(22.0).color(color));

                    ui.add_space(24.0);
                    let elapsed = self.step_start.elapsed().as_secs_f32();
                    let total = step.duration.as_secs_f32().max(0.001);
                    ui.add(
                        egui::ProgressBar::new((elapsed / total).clamp(0.0, 1.0))
                            .desired_width(ui.available_width() * 0.7)
                            .text(format!("{:.1}с / {:.1}с", elapsed.min(total), total)),
                    );
                });
            } else {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(
                        egui::RichText::new("Готово")
                            .size(34.0)
                            .strong()
                            .color(egui::Color32::from_rgb(120, 200, 120)),
                    );
                });
            }

            ui.add_space(24.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("⟲ Заново").clicked() {
                    self.restart();
                }
                let stop_label = if self.running { "⏹ Стоп" } else { "остановлено" };
                if ui.add_enabled(self.running, egui::Button::new(stop_label)).clicked() {
                    stop_all(&self.device);
                    self.running = false;
                    self.status = "Остановлено вручную. Моторы выключены.".to_string();
                }
            });

            if !self.status.is_empty() {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::YELLOW, &self.status);
            }
        });

        ctx.request_repaint_after(Duration::from_millis(16));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        stop_all(&self.device);
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 420.0])
            .with_min_inner_size([420.0, 360.0])
            .with_always_on_top(),
        ..Default::default()
    };
    eframe::run_native(
        "Aurora Vibra — Throttle Stutter Diagnostic",
        native_options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
