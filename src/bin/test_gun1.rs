// GUI-утилита для прототипирования эффекта "стрельба из пулемёта M2 Browning"
// на живом железе (вибромоторы РУД/джойстика).
//
// ЗАЧЕМ ОСОБАЯ УТИЛИТА, А НЕ ПРОСТО test_motor_gui С ДРУГИМИ ЦИФРАМИ:
// реальный AN/M2 Browning (авиационный вариант) стреляет с темпом 750-850
// выстр/мин на ствол (~70-80мс между выстрелами) — это БЫСТРЕЕ, чем канал
// может надёжно передать. hid/worker.rs (продовый путь) шлёт интенсивность
// не чаще раза в SEND_INTERVAL=50мс (20Гц) — частота Найквиста ~10Гц, и
// rumble.rs (см. комментарий у thump_min_period_s) уже задокументировал,
// что безопасный минимум периода импульса ~0.15с (~6.7Гц), иначе часть
// импульсов "съедается" между отправками. test_throttle_stutter.rs НАПРЯМУЮ
// нашёл сбои прошивки при частых импульсах (10-50мс). Значит "выстрел на
// каждый патрон" на этом железе не воспроизвести честно — вместо этого
// лепим НЕПРЕРЫВНЫЙ ГУЛ С ТЕКСТУРОЙ: несущая частота в безопасной зоне +
// джиттер амплитуды на каждом цикле (имитирует батарею из нескольких
// стволов, бьющих не в идеальный унисон, а не один метрономный мотор).
//
// Этот файл НАПРЯМУЮ пишет в устройство (как test_motor_gui.rs), минуя
// hid_worker и его 20Гц-ограничение — специально, чтобы можно было
// прощупать РЕАЛЬНЫЙ потолок железа (в т.ч. зону сбоев из
// test_throttle_stutter.rs), а не только то, что разрешает прод.
//
// cargo run --features dev-tools --bin test_gun1
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

// Справочные ттх реального AN/M2 Browning (авиационный, лёгкий ствол) —
// только для читаемого сравнения в UI, на сам генератор не влияют.
const M2_RPM_MIN: f32 = 750.0;
const M2_RPM_MAX: f32 = 850.0;

struct FoundDevice {
    path: CString,
    pid: u16,
    ifnum: i32,
}

/// Минимальный xorshift32 PRNG — только для джиттера амплитуды текстуры,
/// криптостойкость не нужна, а тянуть крейт `rand` ради одного эффекта
/// избыточно. Состояние не может быть 0 (алгоритм вырождается), поэтому
/// сид всегда приводится к нечётному/ненулевому значению.
struct Xorshift32(u32);

impl Xorshift32 {
    fn seeded() -> Self {
        let nanos = Instant::now().elapsed().subsec_nanos();
        Self((nanos | 1).max(1))
    }

    /// Следующее случайное число в [0.0, 1.0).
    fn next_unit(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f64) / (u32::MAX as f64 + 1.0)
    }
}

struct App {
    api: HidApi,
    devices: Vec<FoundDevice>,
    selected: Option<usize>,
    open_device: Option<HidDevice>,
    open_pid: u16,
    status: String,

    // ═══════════════════════════════════════════════════════════════════
    // Генератор "гул с текстурой": несущая — программный ШИМ (та же идея,
    // что и в test_motor_gui.rs / эффекте закрылков в rumble.rs), плюс
    // джиттер пиковой амплитуды на КАЖДОМ цикле несущей — чтобы вместо
    // одного ровного мотора ощущалась батарея из нескольких стволов.
    // ═══════════════════════════════════════════════════════════════════
    carrier_freq_hz: f32,
    duty_pct: u8,
    jitter_pct: u8,
    peak: u8,
    floor: u8,
    attack_ms: f32,

    /// Чекбокс "непрерывный огонь" — держит эффект включённым постоянно,
    /// без необходимости жать/удерживать кнопку мышкой или пробел.
    continuous_fire: bool,
    firing: bool,
    fire_started_at: Instant,
    phase_origin: Instant,
    last_cycle_idx: i64,
    cycle_jitter_mul: f64,
    rng: Xorshift32,
    last_sent_byte: Option<u8>,
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
            status: "Нажми «Обновить список», чтобы найти устройства".to_string(),

            // Дефолт нарочно в задокументированной безопасной зоне канала
            // (rumble.rs: ~0.15с/6.7Гц) — не в честном темпе M2 (~13Гц),
            // с которого начинать бессмысленно, см. заголовок файла.
            carrier_freq_hz: 6.5,
            duty_pct: 35,
            jitter_pct: 25,
            peak: 200,
            floor: 40,
            attack_ms: 80.0,

            continuous_fire: false,
            firing: false,
            fire_started_at: Instant::now(),
            phase_origin: Instant::now(),
            last_cycle_idx: -1,
            cycle_jitter_mul: 1.0,
            rng: Xorshift32::seeded(),
            last_sent_byte: None,
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
                // Параметры генератора (carrier/duty/jitter/peak/floor/attack)
                // намеренно НЕ трогаем: переключение устройства не должно
                // сбивать уже подобранные настройки.
                self.last_sent_byte = None;
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

    /// Отправляет `byte` на текущее устройство. На РУД стрельба уходит
    /// СРАЗУ на оба мотора одновременно — разносить очередь по одному
    /// мотору смысла нет (это не два разных эффекта, а один и тот же
    /// "гул стрельбы", который должен ощущаться в обеих руках/пальцах на
    /// РУД одинаково, а не только в одной половине квадранта).
    fn send_byte(&mut self, byte: u8) {
        let Some(dev) = &self.open_device else {
            return;
        };

        if is_ursa_minor_throttle(self.open_pid) {
            let frame_l = build_throttle_vibe_frame(REPORT_ID, OUT_LEN, THROTTLE_MOTOR_LEFT, byte);
            let frame_r = build_throttle_vibe_frame(REPORT_ID, OUT_LEN, THROTTLE_MOTOR_RIGHT, byte);
            if let Err(e) = dev.write(&frame_l) {
                self.status = format!("Ошибка отправки: {e}");
                return;
            }
            if let Err(e) = dev.write(&frame_r) {
                self.status = format!("Ошибка отправки: {e}");
            }
        } else {
            let frame = build_simapp_vibe_frame(self.open_pid, REPORT_ID, OUT_LEN, byte);
            if let Err(e) = dev.write(&frame) {
                self.status = format!("Ошибка отправки: {e}");
            }
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
        self.firing = false;
    }

    fn start_fire(&mut self) {
        self.firing = true;
        self.fire_started_at = Instant::now();
        self.phase_origin = Instant::now();
        self.last_cycle_idx = -1;
        self.last_sent_byte = None;
    }

    fn stop_fire(&mut self) {
        self.firing = false;
        self.last_sent_byte = None;
        self.send_byte(0);
    }

    /// Считает и (если байт реально изменился) отправляет текущий кадр
    /// генератора: несущая ШИМ + джиттер амплитуды на цикл + атака в начале
    /// очереди. Вызывается на каждом такте UI, пока `firing == true`.
    fn tick_fire(&mut self, ctx: &egui::Context) {
        if !self.firing || self.open_device.is_none() {
            return;
        }

        let period_s = 1.0 / self.carrier_freq_hz.max(0.1) as f64;
        let elapsed_s = self.phase_origin.elapsed().as_secs_f64();
        let cycle_idx = (elapsed_s / period_s).floor() as i64;

        // Новый цикл несущей — перебрасываем джиттер (одна и та же
        // случайная амплитуда держится весь цикл, а не дребезжит внутри
        // него).
        if cycle_idx != self.last_cycle_idx {
            self.last_cycle_idx = cycle_idx;
            let jitter = (self.jitter_pct as f64 / 100.0).clamp(0.0, 1.0);
            self.cycle_jitter_mul = 1.0 - jitter * self.rng.next_unit();
        }

        let phase = (elapsed_s / period_s).fract();
        let duty = (self.duty_pct as f64 / 100.0).clamp(0.0, 1.0);

        // Атака: линейная рампа 0..1 за attack_ms от начала удержания —
        // даёт очереди ощутимый "толчок" в момент нажатия вместо
        // мгновенного выхода на полную силу.
        let attack_s = (self.attack_ms.max(0.0) / 1000.0) as f64;
        let since_fire_s = self.fire_started_at.elapsed().as_secs_f64();
        let attack_ramp = if attack_s <= 0.0 {
            1.0
        } else {
            (since_fire_s / attack_s).clamp(0.0, 1.0)
        };

        let out = if phase < duty {
            let peak_f = self.peak as f64 * self.cycle_jitter_mul * attack_ramp;
            peak_f.round().clamp(0.0, 255.0) as u8
        } else {
            self.floor
        };

        if self.last_sent_byte != Some(out) {
            self.last_sent_byte = Some(out);
            self.send_byte(out);
        }

        ctx.request_repaint_after(Duration::from_millis(4));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        egui::CentralPanel::default().show(ui, |ui| {
            let slider_width = (ui.available_width() - 56.0).max(120.0);
            ui.spacing_mut().slider_width = slider_width;

            ui.heading("Aurora Vibra — тест эффекта M2 Browning (MG Fire)");
            ui.label(format!(
                "Реальный AN/M2 (авиационный): {:.0}-{:.0} выстр/мин на ствол \
                 (~{:.0}-{:.0}мс между выстрелами) — быстрее, чем канал может \
                 честно передать импульс за импульс, см. комментарий в начале файла.",
                M2_RPM_MIN,
                M2_RPM_MAX,
                60_000.0 / M2_RPM_MAX,
                60_000.0 / M2_RPM_MIN,
            ));
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
                .map(|d| {
                    format!(
                        "{} (PID 0x{:04X}, if#{})",
                        ursa_model_name(d.pid),
                        d.pid,
                        d.ifnum
                    )
                })
                .unwrap_or_else(|| "— не выбрано —".to_string());

            egui::ComboBox::from_label("Устройство")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for (i, d) in self.devices.iter().enumerate() {
                        let label = format!(
                            "{} (PID 0x{:04X}, if#{})",
                            ursa_model_name(d.pid),
                            d.pid,
                            d.ifnum
                        );
                        if ui
                            .selectable_label(self.selected == Some(i), label)
                            .clicked()
                        {
                            connect_idx = Some(i);
                        }
                    }
                });
            if let Some(i) = connect_idx {
                self.connect(i);
            }

            ui.separator();

            if self.is_throttle() {
                ui.label("РУД: очередь идёт сразу на оба мотора (левый + правый).");
                ui.separator();
            }

            ui.add_enabled_ui(self.open_device.is_some(), |ui| {
                ui.label(format!(
                    "Несущая частота (≈{:.0} выстр/мин видимой текстуры):",
                    self.carrier_freq_hz * 60.0
                ));
                ui.add(egui::Slider::new(&mut self.carrier_freq_hz, 1.0..=100.0).suffix(" Гц"))
                    .on_hover_text(
                        "6.7Гц и выше — задокументированная зона сбоев/съеденных импульсов \
                         (см. rumble.rs, test_throttle_stutter.rs). Двигай выше специально, \
                         чтобы прощупать, где начинается срыв на твоём железе.",
                    );

                ui.label("Скважность (доля цикла на пике):");
                ui.add(egui::Slider::new(&mut self.duty_pct, 1..=100).suffix("%"));

                ui.label(
                    "Джиттер амплитуды (текстура battery — несколько стволов не в унисон, \
                     а не один ровный мотор):",
                );
                ui.add(egui::Slider::new(&mut self.jitter_pct, 0..=100).suffix("%"));

                ui.label("Пиковая сила:");
                ui.add(egui::Slider::new(&mut self.peak, 0..=255));

                ui.label("Нижний уровень между импульсами:");
                let floor_max = self.peak.saturating_sub(1);
                self.floor = self.floor.min(floor_max);
                ui.add(egui::Slider::new(&mut self.floor, 0..=floor_max));

                ui.label("Атака (мс до полной силы очереди с момента нажатия):");
                ui.add(egui::Slider::new(&mut self.attack_ms, 0.0..=300.0).suffix(" мс"));

                ui.separator();

                let fire_btn = ui.add_sized(
                    [ui.available_width(), 64.0],
                    egui::Button::new(
                        egui::RichText::new("УДЕРЖИВАЙ, ЧТОБЫ СТРЕЛЯТЬ (или пробел)")
                            .size(18.0)
                            .strong(),
                    ),
                );
                ui.checkbox(
                    &mut self.continuous_fire,
                    "Непрерывный огонь (без удержания)",
                );

                let held = fire_btn.is_pointer_button_down_on()
                    || ui.input(|i| i.key_down(egui::Key::Space))
                    || self.continuous_fire;

                if held && !self.firing {
                    self.start_fire();
                } else if !held && self.firing {
                    self.stop_fire();
                }

                if self.firing {
                    ui.colored_label(egui::Color32::from_rgb(255, 120, 40), "ОГОНЬ");
                }
            });

            if self.open_device.is_none() {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Выбери устройство из списка выше, чтобы начать.",
                );
            }
        });

        self.tick_fire(&ctx);

        // Держим UI живым даже без движения мыши/клика — иначе удержание
        // кнопки/пробела мышью без движения не подхватится следующим
        // тактом (тот же приём, что и в test_motor_gui.rs для ШИМ-тика).
        if self.open_device.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_and_close();
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([640.0, 680.0])
            .with_min_inner_size([440.0, 480.0])
            .with_resizable(true),
        ..Default::default()
    };
    eframe::run_native(
        "Aurora Vibra — MG Fire Test (M2 Browning)",
        native_options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
