// Элементарная GUI-утилита для ручного теста вибромоторов на живом железе:
// выбор устройства (из реально подключённых Winwing HID-девайсов), выбор
// мотора 1/2 (только для РУД — у него два независимых мотора на одном
// устройстве, см. THROTTLE_MOTOR_LEFT/RIGHT в hid/protocol.rs), слайдер
// 0..255 — сырая сила эффекта ровно в тех единицах, в которых её понимает
// сам девайс, и режим ШИМ для получения ощущения слабее «порога рывка»
// мотора (см. комментарий у PWM-полей App ниже).
//
// cargo run --bin test_motor_gui
//
// ВАЖНО: закрой основное приложение и SimAppPro перед запуском — HID-
// устройство может быть открыто только одним процессом одновременно.

use aurora_vibra::hid::protocol::{
    THROTTLE_MOTOR_LEFT, THROTTLE_MOTOR_RIGHT, WW_VID, build_simapp_vibe_frame,
    build_throttle_vibe_frame, is_ursa_minor_throttle, ursa_model_name,
};
use eframe::egui;
use hidapi::{HidApi, HidDevice};
use std::ffi::CString;
use std::time::{Duration, Instant};

// Report ID и длина буфера одинаковы для всех Winwing Ursa Minor устройств
// (джойстики и РУД) — см. golden bytes в hid/protocol.rs тестах и все
// существующие test_*.rs утилиты в этой же папке.
const REPORT_ID: u8 = 0x02;
const OUT_LEN: u16 = 14;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Motor {
    One, // THROTTLE_MOTOR_LEFT
    Two, // THROTTLE_MOTOR_RIGHT
}

impl Motor {
    fn addr(self) -> u8 {
        match self {
            Motor::One => THROTTLE_MOTOR_LEFT,
            Motor::Two => THROTTLE_MOTOR_RIGHT,
        }
    }
}

struct FoundDevice {
    path: CString,
    pid: u16,
    ifnum: i32,
}

struct App {
    api: HidApi,
    devices: Vec<FoundDevice>,
    selected: Option<usize>,
    open_device: Option<HidDevice>,
    open_pid: u16,
    motor: Motor,
    intensity: u8,
    status: String,

    // ═══════════════════════════════════════════════════════════════════
    // Режим ШИМ (PWM). У ERM-вибромоторов есть механический порог
    // трогания: ниже него мотор молчит, а прямо на пороге сразу «дёргает»
    // на рабочую амплитуду — плавного перехода 0 → еле заметно нет, так
    // как для старта нужно преодолеть стартовый момент/трение, а держать
    // вращение можно уже меньшей силой. Поэтому единственный способ
    // получить ощущение СЛАБЕЕ этого порога — не снижать сырое значение
    // (там просто дно), а импульсно включать его на pwm_peak КОРОТКИМИ
    // включениями (pwm_duty_pct от периода 1/pwm_freq_hz) — мотор успевает
    // «дёрнуться» и погаснуть до следующего импульса, и чем реже импульсы
    // (низкая скважность/частота), тем слабее ощущается суммарная
    // вибрация. См. тот же приём (программный ШИМ) для эффекта закрылков
    // в rumble.rs — там частота 25 Гц подтверждена на живом железе.
    //
    // pwm_floor: между импульсами шлём не жёсткий 0, а этот «пол». Грузик
    // мотора РУД физически ЛЕГЧЕ грузика в моторе джойстика — меньше масса
    // = меньше момент инерции = ротор теряет вращение (тормозится трением)
    // гораздо быстрее за то же время паузы между импульсами. В итоге РУД
    // успевает полностью остановиться между импульсами, и КАЖДЫЙ следующий
    // импульс заново преодолевает трение покоя = рывок на каждом цикле
    // ШИМ, тогда как более тяжёлый (инерционный) ротор джойстика за ту же
    // паузу продолжает крутиться по инерции — отсюда и разница в
    // ощущениях при одинаковых duty/freq. pwm_floor держит ротор
    // проворачивающимся между импульсами (сам по себе может быть ниже
    // порога трогания и один, без импульсов, ничего не даст), так что
    // импульсу не нужно каждый раз стартовать с полной остановки.
    // ═══════════════════════════════════════════════════════════════════
    pwm_enabled: bool,
    pwm_peak: u8,
    pwm_floor: u8,
    pwm_duty_pct: u8,
    pwm_freq_hz: f32,
    pwm_phase_start: Instant,
    last_pwm_byte: Option<u8>,
}

impl App {
    fn new() -> Self {
        let api = HidApi::new().expect("Не удалось инициализировать HID API");
        let mut app = Self {
            api,
            devices: Vec::new(),
            selected: None,
            open_device: None,
            open_pid: 0,
            motor: Motor::One,
            intensity: 0,
            status: "Нажми «Обновить список», чтобы найти устройства".to_string(),

            pwm_enabled: false,
            pwm_peak: 80,
            pwm_floor: 0,
            pwm_duty_pct: 15,
            pwm_freq_hz: 25.0,
            pwm_phase_start: Instant::now(),
            last_pwm_byte: None,
        };
        app.rescan();
        app
    }

    fn rescan(&mut self) {
        let _ = self.api.refresh_devices();
        self.devices.clear();
        for d in self.api.device_list() {
            // usage_page/usage фильтруют интерфейс вибро-канала — тот же
            // критерий, что и в проде (см. hid/worker.rs hid_send_out).
            if d.vendor_id() != WW_VID || d.usage_page() != 0x0001 || d.usage() != 0x0004 {
                continue;
            }
            self.devices.push(FoundDevice {
                path: d.path().to_owned(),
                pid: d.product_id(),
                ifnum: d.interface_number(),
            });
        }

        self.status = if self.devices.is_empty() {
            "Устройства не найдены. Проверь, что закрыты основное приложение и SimAppPro."
                .to_string()
        } else {
            format!("Найдено устройств: {}", self.devices.len())
        };

        // Выбранное устройство могло отключиться между сканами.
        if let Some(sel) = self.selected
            && sel >= self.devices.len()
        {
            self.stop_and_close();
            self.selected = None;
        }
    }

    fn connect(&mut self, idx: usize) {
        self.stop_and_close();
        let d = &self.devices[idx];
        match self.api.open_path(&d.path) {
            Ok(dev) => {
                self.status = format!(
                    "Подключено: {} (PID 0x{:04X}, if#{})",
                    ursa_model_name(d.pid),
                    d.pid,
                    d.ifnum
                );
                self.open_pid = d.pid;
                self.open_device = Some(dev);
                self.selected = Some(idx);
                self.motor = Motor::One;
                self.intensity = 0;
                // Настройки ШИМ (pwm_enabled/peak/duty/freq) — намеренно НЕ
                // трогаем: переключение устройства не должно сбивать уже
                // подобранный режим, только перезапускаем фазу импульса.
                self.last_pwm_byte = None;
                self.pwm_phase_start = Instant::now();
            }
            Err(e) => {
                self.status = format!("Ошибка открытия устройства: {e}");
                self.open_device = None;
                self.selected = None;
            }
        }
    }

    fn is_throttle(&self) -> bool {
        self.open_device.is_some() && is_ursa_minor_throttle(self.open_pid)
    }

    /// Отправляет `byte` на текущее устройство/мотор. Не трогает
    /// `self.intensity` — вызывается и с сырым значением слайдера, и с
    /// вычисленным значением ШИМ-импульса.
    fn send_byte(&mut self, byte: u8) {
        let Some(dev) = &self.open_device else {
            return;
        };

        let frame = if is_ursa_minor_throttle(self.open_pid) {
            build_throttle_vibe_frame(REPORT_ID, OUT_LEN, self.motor.addr(), byte)
        } else {
            build_simapp_vibe_frame(self.open_pid, REPORT_ID, OUT_LEN, byte)
        };

        if let Err(e) = dev.write(&frame) {
            self.status = format!("Ошибка отправки: {e}");
        }
    }

    /// Глушит конкретный мотор РУД (используется при переключении 1↔2,
    /// чтобы «отпущенный» мотор не остался крутиться на старом значении).
    fn zero_motor(&self, motor_addr: u8) {
        if let Some(dev) = &self.open_device {
            let _ = dev.write(&build_throttle_vibe_frame(
                REPORT_ID, OUT_LEN, motor_addr, 0,
            ));
        }
    }

    /// Глушит оба мотора (если это РУД — у него их два) и закрывает устройство.
    fn stop_and_close(&mut self) {
        if let Some(dev) = &self.open_device {
            if is_ursa_minor_throttle(self.open_pid) {
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
            } else {
                let _ = dev.write(&build_simapp_vibe_frame(
                    self.open_pid,
                    REPORT_ID,
                    OUT_LEN,
                    0,
                ));
            }
        }
        self.open_device = None;
        // pwm_enabled намеренно не трогаем — см. connect(): переключение/
        // переподключение устройства не должно сбрасывать режим ШИМ. Тик
        // в ui() уже безопасно бездействует, пока open_device == None.
    }

    /// Полная остановка: гасит текущий мотор и выключает ШИМ.
    fn stop(&mut self) {
        self.pwm_enabled = false;
        self.intensity = 0;
        self.last_pwm_byte = None;
        self.send_byte(0);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        egui::CentralPanel::default().show(ui, |ui| {
            // Растягиваем слайдеры почти на всю ширину окна — с дефолтной
            // egui-шириной (~120пт) ими неудобно точно управлять мышью.
            // Оставляем запас (56пт) под текстовое поле значения справа от
            // бегунка (egui рисует его отдельно, оно не входит в slider_width),
            // иначе оно вылезало бы за пределы панели.
            let slider_width = (ui.available_width() - 56.0).max(120.0);
            ui.spacing_mut().slider_width = slider_width;

            ui.heading("Aurora Vibra — тест мотора");
            ui.label("Слайдер отправляет силу эффекта 0..255 напрямую, как её понимает сам девайс.");
            ui.separator();

            if ui.button("Обновить список устройств").clicked() {
                self.rescan();
            }

            ui.label(&self.status);
            ui.separator();

            let mut connect_idx: Option<usize> = None;
            let current_label = self
                .selected
                .and_then(|i| self.devices.get(i))
                .map(|d| format!("{} (PID 0x{:04X}, if#{})", ursa_model_name(d.pid), d.pid, d.ifnum))
                .unwrap_or_else(|| "— не выбрано —".to_string());

            egui::ComboBox::from_label("Устройство")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for (i, d) in self.devices.iter().enumerate() {
                        let label =
                            format!("{} (PID 0x{:04X}, if#{})", ursa_model_name(d.pid), d.pid, d.ifnum);
                        if ui.selectable_label(self.selected == Some(i), label).clicked() {
                            connect_idx = Some(i);
                        }
                    }
                });
            if let Some(i) = connect_idx {
                self.connect(i);
            }

            ui.separator();

            if self.is_throttle() {
                ui.label("РУД: выбор мотора");
                ui.horizontal(|ui| {
                    let prev_motor = self.motor;
                    ui.radio_value(&mut self.motor, Motor::One, "Мотор 1 (левый)");
                    ui.radio_value(&mut self.motor, Motor::Two, "Мотор 2 (правый)");
                    if self.motor != prev_motor {
                        self.zero_motor(prev_motor.addr());
                        self.last_pwm_byte = None;
                        self.pwm_phase_start = Instant::now();
                        if !self.pwm_enabled {
                            let v = self.intensity;
                            self.send_byte(v);
                        }
                    }
                });
                ui.separator();
            }

            ui.add_enabled_ui(self.open_device.is_some(), |ui| {
                let pwm_toggled = ui.checkbox(&mut self.pwm_enabled, "Режим ШИМ (мягче порога рывка мотора)").changed();
                if pwm_toggled {
                    self.last_pwm_byte = None;
                    self.pwm_phase_start = Instant::now();
                    if !self.pwm_enabled {
                        let v = self.intensity;
                        self.send_byte(v);
                    }
                }

                // Подписи вынесены НАД слайдерами (а не через .text(), которое
                // рисует подпись справа от бегунка) — иначе при полной ширине
                // слайдера подписи с числом справа обрезались бы/переносились.
                if self.pwm_enabled {
                    ui.label(
                        "Короткие импульсы пиковой силы вместо постоянного сигнала — снижай \
                         скважность и/или частоту, пока вибрация не станет мягче того самого \
                         рывка на непрерывном сигнале.",
                    );

                    ui.label("Пиковая сила импульса:");
                    ui.add(egui::Slider::new(&mut self.pwm_peak, 0..=255));

                    ui.label("Нижний уровень между импульсами (не даёт мотору полностью остановиться — попробуй увеличить, если РУД дёргается на каждом импульсе):");
                    let floor_max = self.pwm_peak.saturating_sub(1);
                    self.pwm_floor = self.pwm_floor.min(floor_max);
                    ui.add(egui::Slider::new(&mut self.pwm_floor, 0..=floor_max));

                    ui.label("Скважность (доля времени на пике):");
                    ui.add(egui::Slider::new(&mut self.pwm_duty_pct, 1..=100).suffix("%"));

                    ui.label("Частота импульсов:");
                    ui.add(egui::Slider::new(&mut self.pwm_freq_hz, 1.0..=100.0).suffix(" Гц"));
                } else {
                    ui.label("Сила эффекта:");
                    let slider = ui.add(egui::Slider::new(&mut self.intensity, 0..=255));
                    if slider.changed() {
                        let v = self.intensity;
                        self.send_byte(v);
                    }
                }

                if ui.button("Стоп (0)").clicked() {
                    self.stop();
                }
            });

            if self.open_device.is_none() {
                ui.colored_label(egui::Color32::YELLOW, "Выбери устройство из списка выше, чтобы начать.");
            }
        });

        // Тик ШИМ: считаем текущее значение импульса по реальному времени
        // и шлём его на устройство только когда байт реально меняется —
        // при этом просим у egui частые перерисовки, иначе тик будет идти
        // только по взаимодействию с UI (drag/click), а не сам по себе.
        if self.pwm_enabled && self.open_device.is_some() {
            let period_s = 1.0 / self.pwm_freq_hz.max(0.1) as f64;
            let elapsed_s = self.pwm_phase_start.elapsed().as_secs_f64();
            let phase = (elapsed_s / period_s).fract();
            let duty = (self.pwm_duty_pct as f64 / 100.0).clamp(0.0, 1.0);
            let out = if phase < duty {
                self.pwm_peak
            } else {
                self.pwm_floor
            };

            if self.last_pwm_byte != Some(out) {
                self.last_pwm_byte = Some(out);
                self.send_byte(out);
            }

            ctx.request_repaint_after(Duration::from_millis(4));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_and_close();
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([620.0, 560.0])
            .with_min_inner_size([420.0, 420.0])
            .with_resizable(true),
        ..Default::default()
    };
    eframe::run_native(
        "Aurora Vibra — Motor Test",
        native_options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
