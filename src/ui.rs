use egui::{Color32, RichText, Vec2};
use egui_extras::{Column, TableBuilder};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use parking_lot::Mutex;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use windows::Win32::Foundation::HWND;

use crate::{
    tray, updater, ConfigShared, EffectDeviceTarget, EffectsShared, FlightVars, HidCmd, LogBuffer,
    RumbleConfig, SimStatus, UiCmd,
};

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Tab {
    Main,
    #[cfg(debug_assertions)]
    Debug,
}

fn circle_indicator_colored(ui: &mut egui::Ui, color: Color32, filled: bool) {
    let h = ui.style().spacing.interact_size.y.max(14.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(h, h), egui::Sense::hover());
    let center = rect.center();
    let r = (h * 0.36).max(5.0);
    let stroke_color = color;
    let fill_color = if filled { color } else { Color32::TRANSPARENT };
    ui.painter().circle_filled(center, r, fill_color);
    ui.painter()
        .circle_stroke(center, r, egui::Stroke::new(1.4, stroke_color));
}

fn status_badge(ui: &mut egui::Ui, status: &SimStatus) {
    let (text, color, filled) = match status {
        SimStatus::Disconnected => ("Disconnected", Color32::from_rgb(200, 60, 60), false),
        SimStatus::Connected => ("Connected", Color32::from_rgb(220, 180, 40), false),
        SimStatus::InFlight => ("In Flight", Color32::from_rgb(30, 180, 90), true),
    };
    ui.horizontal(|ui| {
        circle_indicator_colored(ui, color, filled);
        ui.colored_label(color, text);
    });
}

fn controller_badge_dot(ui: &mut egui::Ui, label: &str, connected: bool) {
    let (color, filled) = if connected {
        (Color32::from_rgb(30, 180, 90), true)
    } else {
        (Color32::from_rgb(200, 60, 60), false)
    };
    ui.horizontal(|ui| {
        circle_indicator_colored(ui, color, filled);
        ui.colored_label(
            color,
            if connected {
                format!("{label}: Connected")
            } else {
                format!("{label}: Disconnected")
            },
        );
    });
}

/// Formats the aircraft title for display, with a fallback while
/// SimConnect hasn't delivered TITLE yet (or delivered an empty string,
/// e.g. right after connecting or during a sim restart).
fn format_stop_label(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        "Stop Unknown Aircraft".to_string()
    } else {
        format!("Stop {trimmed}")
    }
}

pub struct UiState {
    pub controller_connected: Arc<AtomicBool>,
    pub throttle_connected: Arc<AtomicBool>,

    pub status: Arc<Mutex<SimStatus>>,
    pub aircraft_title: Arc<Mutex<String>>,

    pub config: Arc<ConfigShared>,
    pub effects: EffectsShared,

    #[cfg(debug_assertions)]
    pub test_level: u8,
    #[cfg(debug_assertions)]
    pub raw_hex: String,

    pub tx_hid: Sender<HidCmd>,
    pub logs: LogBuffer,
    pub last_vars: Arc<Mutex<Option<FlightVars>>>,

    pub autoscroll: bool,
    pub last_log_count: usize,

    #[cfg(debug_assertions)]
    pub show_hid_out: bool,
    #[cfg(debug_assertions)]
    pub show_hid_opened: bool,

    pub active_tab: Tab,
    pub hold: Arc<AtomicBool>,

    pub rx_ui: Receiver<UiCmd>,
    pub tx_ui: Sender<UiCmd>,

    // Сохранение настроек ползунков на диск
    pub saved_config_rev: u64,
    pub pending_save_at: Option<Instant>,
}

impl UiState {
    fn effect_row(
        ui: &mut egui::Ui,
        name: &str,
        val: &mut f32,
        enabled: &mut bool,
        range: std::ops::RangeInclusive<f32>,
        active: bool,
        on_change: &mut bool,
    ) {
        Self::effect_row_hinted(ui, name, val, enabled, range, active, on_change, None);
    }

    fn effect_row_hinted(
        ui: &mut egui::Ui,
        name: &str,
        val: &mut f32,
        enabled: &mut bool,
        range: std::ops::RangeInclusive<f32>,
        active: bool,
        on_change: &mut bool,
        hint: Option<&str>,
    ) {
        ui.horizontal(|ui| {
            let cb = ui.checkbox(enabled, "");
            if cb.changed() {
                *on_change = true;
            }
            
            let name_label = ui.label(RichText::new(name).strong());
            if let Some(h) = hint {
                name_label.on_hover_text(h);
            }
            
            ui.add_enabled_ui(*enabled, |ui| {
                let slider = egui::Slider::new(val, range)
                    .trailing_fill(true)
                    .show_value(true);
                let resp = ui.add(slider);
                if let Some(h) = hint {
                    resp.clone().on_hover_text(h);
                }
                if resp.changed() {
                    *on_change = true;
                }
            });
            
            let (color, filled) = if active && *enabled {
                (Color32::WHITE, true)
            } else {
                (Color32::from_gray(90), false)
            };
            circle_indicator_colored(ui, color, filled);
        });
    }

    /// Компактная строка с двумя чекбоксами маршрутизации эффекта на
    /// устройства: "J" (Combat Joystick R) и "T" (URSA MINOR Throttle).
    /// Рисуется сразу под соответствующим effect_row/effect_row_hinted.
    fn device_target_row(ui: &mut egui::Ui, target: &mut EffectDeviceTarget, on_change: &mut bool) {
        ui.horizontal(|ui| {
            ui.add(egui::Label::new("    ").sense(egui::Sense::hover()));
            ui.label(RichText::new("Устройство:").weak().small());

            let cb_j = ui
                .checkbox(&mut target.enable_joystick, "J")
                .on_hover_text("Combat Joystick R");
            if cb_j.changed() {
                *on_change = true;
            }

            let cb_t = ui
                .checkbox(&mut target.enable_throttle, "T")
                .on_hover_text("WINCTRL URSA MINOR Throttle (РУД)");
            if cb_t.changed() {
                *on_change = true;
            }
        });
    }

    fn taxi_bound_row(
        ui: &mut egui::Ui,
        name: &str,
        val: &mut f64,
        enabled: &mut bool,
        range: std::ops::RangeInclusive<f64>,
        active: bool,
        on_change: &mut bool,
    ) {
        ui.horizontal(|ui| {
            let cb = ui.checkbox(enabled, "");
            if cb.changed() {
                *on_change = true;
            }
            
            ui.label(RichText::new(name).strong());
            
            ui.add_enabled_ui(*enabled, |ui| {
                let mut tmp = *val as f32;
                let r = (*range.start() as f32)..=(*range.end() as f32);
                if ui
                    .add(egui::Slider::new(&mut tmp, r).trailing_fill(true).show_value(true))
                    .changed()
                {
                    *val = tmp as f64;
                    *on_change = true;
                }
            });

            let (color, filled) = if active && *enabled {
                (Color32::WHITE, true)
            } else {
                (Color32::from_gray(90), false)
            };
            circle_indicator_colored(ui, color, filled);
        });
    }
}

impl eframe::App for UiState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        {
            const TARGET_FPS: u64 = 30;
            ctx.request_repaint_after(Duration::from_millis(1000 / TARGET_FPS));
        }

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(6.0, 6.0);
        style.spacing.slider_width = 160.0;
        ctx.set_style(style);

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let st = *self.status.lock();
                status_badge(ui, &st);
                ui.separator();

                let controller_ok = self.controller_connected.load(Ordering::Relaxed);
                controller_badge_dot(ui, "Sidestick", controller_ok);

                let throttle_ok = self.throttle_connected.load(Ordering::Relaxed);
                controller_badge_dot(ui, "Throttle", throttle_ok);

                let ac = self.aircraft_title.lock().clone();
                ui.separator();
                ui.label(RichText::new(format_stop_label(&ac)).italics());

                #[cfg(debug_assertions)]
                {
                    ui.separator();
                    ui.selectable_value(&mut self.active_tab, Tab::Main, "Main");
                    ui.selectable_value(&mut self.active_tab, Tab::Debug, "Debug");
                }

                ui.separator();

                if ui.button("🔄 Check for updates").clicked() {
                    updater::spawn_check(HWND(0), env!("CARGO_PKG_VERSION"));
                }

                let holding = self.hold.load(Ordering::Relaxed);
                if !holding {
                    if ui.button("⛔ Stop").clicked() {
                        self.hold.store(true, Ordering::Relaxed);
                        let _ = self.tx_hid.send(HidCmd::SetHold(true));
                        tray::notify_held(true);
                    }
                } else if ui.button("▶ Resume").clicked() {
                    self.hold.store(false, Ordering::Relaxed);
                    let _ = self.tx_hid.send(HidCmd::SetHold(false));
                    tray::notify_held(false);
                }
            });
        });

       let show_main = self.active_tab == Tab::Main;
        #[cfg(debug_assertions)]
        let show_debug = self.active_tab == Tab::Debug;
        #[cfg(not(debug_assertions))]
        let show_debug = false;
        let _ = show_debug;

        if show_main {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.heading("Rumble Effects");
                        ui.add_space(4.0);

                        let mut _changed = false;

                        let ground_active = self.effects.ground_active.load(Ordering::Relaxed);
                        let ground_thump_active = self.effects.ground_thump_active.load(Ordering::Relaxed);
                        let taxi_start_crossed = self.effects.taxi_start_crossed.load(Ordering::Relaxed);
                        let taxi_end_crossed = self.effects.taxi_end_crossed.load(Ordering::Relaxed);

                        self.config.with_mut(|cfg| {
                             // Overspeed
                             let mut overspeed_enabled = cfg.overspeed_enabled;
                             
                             UiState::effect_row(
                                 ui,
                                 "Overspeed",
                                 &mut cfg.overspeed_threshold_kn,
                                 &mut overspeed_enabled,
                                 0.0..=2000.0,
                                 self.effects.overspeed_active.load(Ordering::Relaxed),
                                 &mut _changed,
                             );
                             cfg.overspeed_enabled = overspeed_enabled;
                             UiState::device_target_row(ui, &mut cfg.device_targets.overspeed, &mut _changed);

                            ui.add_space(8.0);

                            // Ground Roll
                            let mut ground_enabled = cfg.ground_enabled;
                            UiState::effect_row_hinted(
                                ui,
                                "Ground Roll",
                                &mut cfg.ground_roll,
                                &mut ground_enabled,
                                0.0..=50.0,
                                ground_active || ground_thump_active,
                                &mut _changed,
                                Some("Факт. сила на выходе: 0–50 (мягкий фон, ниже удара сжатия стоек)"),
                            );
                            cfg.ground_enabled = ground_enabled;
                            UiState::device_target_row(ui, &mut cfg.device_targets.ground_roll, &mut _changed);

                            ui.add_space(8.0);

                            // Taxi thump bounds
                            ui.label(RichText::new("Taxi Thump Settings").heading());
                            ui.add_space(4.0);
                            
                            {
                                let mut start = cfg.taxi_start_kn;
                                let mut end = cfg.taxi_end_kn;
                                let mut start_enabled = cfg.taxi_start_enabled;
                                let mut end_enabled = cfg.taxi_end_enabled;

                                UiState::taxi_bound_row(
                                    ui,
                                    "Start (kt)",
                                    &mut start,
                                    &mut start_enabled,
                                    0.0..=20.0,
                                    taxi_start_crossed,
                                    &mut _changed,
                                );
                                cfg.taxi_start_enabled = start_enabled;

                                if start >= end - 0.5 {
                                    end = (start + 0.5).min(250.0);
                                }

                                UiState::taxi_bound_row(
                                    ui,
                                    "End (kt)",
                                    &mut end,
                                    &mut end_enabled,
                                    1.0..=250.0, // Изменили верхний порог слайдера до 250
                                    taxi_end_crossed,
                                    &mut _changed,
                                );
                                cfg.taxi_end_enabled = end_enabled;

                                if end <= start + 0.5 {
                                    start = (end - 0.5).max(0.0);
                                }

                                cfg.taxi_start_kn = start.clamp(0.0, 249.5);
                                cfg.taxi_end_kn = end.clamp(cfg.taxi_start_kn + 0.5, 250.0); // Изменили clamp до 250
                            }

                            // Коэффициент кривизны нарастания частоты ударов.
                            // >1.0 = плавнее на старте (пауза между ударами сокращается медленнее),
                            // <1.0 = резче, чем чистая физика t=S/V; 1.0 = без коррекции.
                            ui.horizontal(|ui| {
                                ui.add(egui::Label::new("    ").sense(egui::Sense::hover()));
                                let lbl = ui.label(RichText::new("Period curve:").strong());
                                lbl.on_hover_text(
                                    "Как быстро сокращается пауза между ударами с ростом скорости.\n\
                                     1.0 = чистая физика (t = длина плиты / скорость).\n\
                                     Больше 1.0 = плавнее на старте рулёжки.\n\
                                     Меньше 1.0 = резче, чем чистая физика."
                                );
                                let mut curve = cfg.thump_period_curve;
                                let resp = ui.add(egui::Slider::new(&mut curve, 0.3..=5.0)
                                    .trailing_fill(true)
                                    .show_value(true));
                                resp.clone().on_hover_text(
                                    "1.0 = физика. Больше — медленнее ускоряется ритм ударов на старте."
                                );
                                if resp.changed() {
                                    cfg.thump_period_curve = curve;
                                    _changed = true;
                                }
                            });

                            ui.add_space(8.0);

                            // Flaps effect
                            let mut flaps_enabled = cfg.flaps_enabled;
                            UiState::effect_row(
                                ui,
                                "Flaps (bump)",
                                &mut cfg.flaps_peak,
                                &mut flaps_enabled,
                                0.0..=255.0,
                                self.effects.flaps_bump_active.load(Ordering::Relaxed),
                                &mut _changed,
                            );
                            cfg.flaps_enabled = flaps_enabled;
                            UiState::device_target_row(ui, &mut cfg.device_targets.flaps, &mut _changed);

                            // Gear effect
                            let mut gear_enabled = cfg.gear_enabled;
                            UiState::effect_row(
                                ui,
                                "Landing Gear (bump)",
                                &mut cfg.gear_peak,
                                &mut gear_enabled,
                                0.0..=255.0,
                                self.effects.gear_bump_active.load(Ordering::Relaxed),
                                &mut _changed,
                            );
                            cfg.gear_enabled = gear_enabled;
                            UiState::device_target_row(ui, &mut cfg.device_targets.gear_bump, &mut _changed);

                            // Stall effect
                            let mut stall_enabled = cfg.stall_enabled;
                            UiState::effect_row(
                                ui,
                                "Stall ceiling",
                                &mut cfg.stall_ceiling,
                                &mut stall_enabled,
                                0.0..=255.0,
                                self.effects.stall_active.load(Ordering::Relaxed),
                                &mut _changed,
                            );
                            cfg.stall_enabled = stall_enabled;
                            UiState::device_target_row(ui, &mut cfg.device_targets.stall, &mut _changed);

                             // Spoilers effect
                             let mut spoilers_enabled = cfg.spoilers_enabled;
                             UiState::effect_row(
                                 ui,
                                 "Spoilers Airflow",
                                 &mut cfg.spoilers_intensity,
                                 &mut spoilers_enabled,
                                 0.0..=250.0,
                                 self.effects.spoilers_active.load(Ordering::Relaxed),
                                 &mut _changed,
                             );
                             cfg.spoilers_enabled = spoilers_enabled;
                             UiState::device_target_row(ui, &mut cfg.device_targets.spoilers, &mut _changed);

                             ui.add_space(8.0);

                             // Engine Start / Ignition effect
                             let mut engine_start_enabled = cfg.enable_engine_start;
                             let engine_start_hint = "Раскрутка стартером (пульсация растущей частоты) + удар воспламенения при запуске двигателя";
                             ui.horizontal(|ui| {
                                 let cb = ui.checkbox(&mut engine_start_enabled, "");
                                 if cb.changed() {
                                     _changed = true;
                                 }

                                 // Настраиваемый порог N2 Idle — сразу после переключателя
                                 // включения эффекта, естественным потоком строки (без
                                 // выравнивания по правому краю).
                                 ui.add_space(8.0);
                                 ui.label(RichText::new("N2 Idle%:").strong());
                                 let n2_resp = ui.add(
                                     egui::DragValue::new(&mut cfg.engine_idle_n2)
                                         .speed(1.0)
                                         .clamp_range(10.0..=100.0),
                                 );
                                 if n2_resp.changed() {
                                     _changed = true;
                                 }

                                 ui.add_space(8.0);

                                 let name_label = ui.label(RichText::new("Engine Start / Ignition").strong());
                                 name_label.on_hover_text(engine_start_hint);

                                 ui.add_enabled_ui(engine_start_enabled, |ui| {
                                     let slider = egui::Slider::new(&mut cfg.engine_start_strength, 0.0..=255.0)
                                         .trailing_fill(true)
                                         .show_value(true);
                                     let resp = ui.add(slider);
                                     resp.clone().on_hover_text(engine_start_hint);
                                     if resp.changed() {
                                         _changed = true;
                                     }
                                 });

                                 let active = self.effects.engine_start_active.load(Ordering::Relaxed);
                                 let (color, filled) = if active && engine_start_enabled {
                                     (Color32::WHITE, true)
                                 } else {
                                     (Color32::from_gray(90), false)
                                 };
                                 circle_indicator_colored(ui, color, filled);
                             });
                             cfg.enable_engine_start = engine_start_enabled;

                             ui.horizontal(|ui| {
                                 ui.add(egui::Label::new("    ").sense(egui::Sense::hover()));
                                 if ui
                                     .checkbox(
                                         &mut cfg.four_engine_mode,
                                         "4-Eng Mode (1&2->Left, 3&4->Right)",
                                     )
                                     .on_hover_text(
                                         "4-моторные самолёты: Eng1/Eng2 (левое крыло) группируются \
                                          на РУД (левая рука), Eng3/Eng4 (правое крыло) — на джойстик \
                                          (правая рука). Используется максимум N2 в паре, удар \
                                          воспламенения срабатывает от любого двигателя своей группы.",
                                     )
                                     .changed()
                                 {
                                     _changed = true;
                                 }
                             });

                             ui.add_space(8.0);

                             // Base effect removed per user request
                             // ui.add_space(8.0);

                            // Gear Strut Compression (Touchdown) effect
                            ui.label(RichText::new("Gear Strut Compression (Touchdown)").heading());
                            ui.add_space(4.0);

                            let mut gear_comp_enabled = cfg.gear_comp_enabled;
                            ui.horizontal(|ui| {
                                if ui.checkbox(&mut gear_comp_enabled, "Enabled").changed() {
                                    _changed = true;
                                }

                                ui.add_space(12.0);

                                if ui
                                    .checkbox(&mut cfg.split_touchdown, "SPLIT (Left->Throttle, Right->Stick)")
                                    .on_hover_text(
                                        "Crosswind awareness: левая основная стойка идёт исключительно \
                                         на РУД, правая — исключительно на джойстик, независимо от \
                                         чекбоксов маршрутизации ниже. Носовая стойка не затрагивается.",
                                    )
                                    .changed()
                                {
                                    _changed = true;
                                }
                            });
                            cfg.gear_comp_enabled = gear_comp_enabled;

                            ui.add_enabled_ui(gear_comp_enabled, |ui| {
                                let headroom_hint = "Слайдер: запас 0–55 над полом 200. Факт. сила на выходе: 200–255";
                                // Nose
                                let mut nose_enabled = cfg.gear_comp_nose_enabled;
                                UiState::effect_row_hinted(
                                    ui,
                                    "Nose Peak",
                                    &mut cfg.gear_comp_nose_peak,
                                    &mut nose_enabled,
                                    0.0..=55.0,
                                    self.effects.gear_comp_nose_active.load(Ordering::Relaxed),
                                    &mut _changed,
                                    Some(headroom_hint),
                                );
                                cfg.gear_comp_nose_enabled = nose_enabled;
                                UiState::device_target_row(ui, &mut cfg.device_targets.gear_comp_nose, &mut _changed);

                                // Left
                                let mut left_enabled = cfg.gear_comp_left_enabled;
                                UiState::effect_row_hinted(
                                    ui,
                                    "Left Peak",
                                    &mut cfg.gear_comp_left_peak,
                                    &mut left_enabled,
                                    0.0..=55.0,
                                    self.effects.gear_comp_left_active.load(Ordering::Relaxed),
                                    &mut _changed,
                                    Some(headroom_hint),
                                );
                                cfg.gear_comp_left_enabled = left_enabled;
                                UiState::device_target_row(ui, &mut cfg.device_targets.gear_comp_left, &mut _changed);

                                // Right
                                let mut right_enabled = cfg.gear_comp_right_enabled;
                                UiState::effect_row_hinted(
                                    ui,
                                    "Right Peak",
                                    &mut cfg.gear_comp_right_peak,
                                    &mut right_enabled,
                                    0.0..=55.0,
                                    self.effects.gear_comp_right_active.load(Ordering::Relaxed),
                                    &mut _changed,
                                    Some(headroom_hint),
                                );
                                cfg.gear_comp_right_enabled = right_enabled;
                                UiState::device_target_row(ui, &mut cfg.device_targets.gear_comp_right, &mut _changed);
                            });

                            ui.add_space(8.0);

                            // Gear Transit & Doors Closed (вибрация во время движения стоек
                            // + удар при фиксации на замке). Раньше работали без чекбокса.
                            ui.horizontal(|ui| {
                                let mut gear_transit_enabled = cfg.gear_transit_enabled;
                                if ui.checkbox(&mut gear_transit_enabled, "").changed() {
                                    _changed = true;
                                }
                                cfg.gear_transit_enabled = gear_transit_enabled;
                                ui.label(RichText::new("Gear Transit & Doors").strong());

                                let active = self.effects.gear_transit_active.load(Ordering::Relaxed);
                                let (color, filled) = if active && gear_transit_enabled {
                                    (Color32::WHITE, true)
                                } else {
                                    (Color32::from_gray(90), false)
                                };
                                circle_indicator_colored(ui, color, filled);
                            });
                            UiState::device_target_row(ui, &mut cfg.device_targets.gear_transit, &mut _changed);

                            ui.add_space(8.0);

                            // Bank effect - только чекбокс и порог
                            let mut bank_enabled = cfg.bank_enabled;
                            ui.horizontal(|ui| {
                                let cb = ui.checkbox(&mut bank_enabled, "");
                                if cb.changed() {
                                    _changed = true;
                                }
                                
                                ui.label(RichText::new("Bank / Turb").strong());
                                
                                let active = self.effects.bank_active.load(Ordering::Relaxed);
                                let (color, filled) = if active && bank_enabled {
                                    (Color32::WHITE, true)
                                } else {
                                    (Color32::from_gray(90), false)
                                };
                                circle_indicator_colored(ui, color, filled);
                            });
                            cfg.bank_enabled = bank_enabled;
                            UiState::device_target_row(ui, &mut cfg.device_targets.bank, &mut _changed);

                            // Bank threshold slider - диапазон 0..90°
                            ui.horizontal(|ui| {
                                ui.add(egui::Label::new("    ").sense(egui::Sense::hover()));
                                ui.label(RichText::new("Threshold (°):").strong());
                                let mut threshold = cfg.bank_threshold_deg;
                                if ui.add(egui::Slider::new(&mut threshold, 0.0..=90.0)
                                    .trailing_fill(true)
                                    .show_value(true))
                                    .changed() 
                                {
                                    cfg.bank_threshold_deg = threshold;
                                    _changed = true;
                                }
                            });

                            if _changed {
                                // Конфиг уже обновлен через with_mut
                            }
                        });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Reset to defaults").clicked() {
                                self.config.set(RumbleConfig::default());
                                if let Ok(p) = crate::settings::save(&self.config.get()) {
                                    self.logs.push(format!("Settings reset and saved → {}", p.display()));
                                }
                                self.saved_config_rev = self.config.current_rev();
                                self.pending_save_at = None;
                            }
                        });

                        ui.separator();

ui.heading("Live Aircraft Data");

egui::Grid::new("aircraft_data")
    .num_columns(2)
    .spacing(Vec2::new(20.0, 4.0))
    .show(ui, |ui| {
        let v = *self.last_vars.lock();
        match v {
            Some(v) => {
                ui.label("Airspeed (kt):");
                ui.label(format!("{:.1}", v.airspeed_indicated));
                ui.end_row();

                ui.label("GS (kt):");
                ui.label(format!("{:.1}", v.ground_speed_kt));
                ui.end_row();

                ui.label("On Ground:");
                ui.label(v.on_ground.to_string());
                ui.end_row();

                ui.label("Bank (°):");
                ui.label(format!("{:.1}", v.bank_deg));
                ui.end_row();

                ui.label("Flaps (%):");
                ui.label(format!("{:.0}", v.flaps_pct));
                ui.end_row();

                ui.label("Gear:");
                ui.label(if v.gear_handle > 0.5 { "Down" } else { "Up" });
                ui.end_row();

                ui.label("Spoilers (%):");
                ui.label(format!("{:.0}", v.spoilers_pct));
                ui.end_row();

                // --- ДОБАВЛЕНА ТЕЛЕМЕТРИЯ ОБЖАТИЯ СТОЕК ШАССИ ---
                ui.label("Nose Gear (%):");
                ui.label(format!("{:.1}", v.gear_comp_nose));
                ui.end_row();

                ui.label("Left Main (%):");
                ui.label(format!("{:.1}", v.gear_comp_left));
                ui.end_row();

                ui.label("Right Main (%):");
                ui.label(format!("{:.1}", v.gear_comp_right));
                ui.end_row();
                // ------------------------------------------------

                ui.label("Stall:");
                ui.label(v.stalled.to_string());
                ui.end_row();

                ui.label("Paused:");
                ui.label(v.paused.to_string());
                ui.end_row();
            }
            None => {
                ui.label("No data");
                ui.label("");
                ui.end_row();
            }
        }
    });

ui.add_space(8.0);
ui.separator();
ui.heading("Engine Telemetry");
ui.add_space(4.0);

egui::Grid::new("engine_telemetry")
    .num_columns(2)
    .spacing(Vec2::new(20.0, 4.0))
    .show(ui, |ui| {
        let v = *self.last_vars.lock();
        match v {
            Some(v) => {
                let combustion_label = |ui: &mut egui::Ui, active: bool| {
                    if active {
                        ui.colored_label(Color32::from_rgb(30, 180, 90), "ON");
                    } else {
                        ui.colored_label(Color32::from_gray(140), "OFF");
                    }
                };

                ui.label(RichText::new("Engine 1 (Left / Throttle)").strong());
                ui.label("");
                ui.end_row();

                ui.label("N2:");
                ui.label(format!("{:.1}%", v.eng1_n2_percent));
                ui.end_row();

                ui.label("Combustion:");
                combustion_label(ui, v.eng1_combustion > 0.5);
                ui.end_row();

                ui.label("% Max RPM:");
                ui.label(format!("{:.1}%", v.eng1_pct_max_rpm));
                ui.end_row();

                ui.label("Engine RPM:");
                ui.label(format!("{:.0}", v.eng1_rpm));
                ui.end_row();

                ui.label("Prop RPM:");
                ui.label(format!("{:.0}", v.prop1_rpm));
                ui.end_row();

                ui.label(RichText::new("Engine 2 (Right / Joystick)").strong());
                ui.label("");
                ui.end_row();

                ui.label("N2:");
                ui.label(format!("{:.1}%", v.eng2_n2_percent));
                ui.end_row();

                ui.label("Combustion:");
                combustion_label(ui, v.eng2_combustion > 0.5);
                ui.end_row();

                ui.label("% Max RPM:");
                ui.label(format!("{:.1}%", v.eng2_pct_max_rpm));
                ui.end_row();

                ui.label("Engine RPM:");
                ui.label(format!("{:.0}", v.eng2_rpm));
                ui.end_row();

                ui.label("Prop RPM:");
                ui.label(format!("{:.0}", v.prop2_rpm));
                ui.end_row();

                ui.label(RichText::new("Engine 3 (4-Eng: contributes to Left)").strong());
                ui.label("");
                ui.end_row();

                ui.label("N2:");
                ui.label(format!("{:.1}%", v.eng3_n2_percent));
                ui.end_row();

                ui.label("Combustion:");
                combustion_label(ui, v.eng3_combustion > 0.5);
                ui.end_row();

                ui.label("% Max RPM:");
                ui.label(format!("{:.1}%", v.eng3_pct_max_rpm));
                ui.end_row();

                ui.label("Engine RPM:");
                ui.label(format!("{:.0}", v.eng3_rpm));
                ui.end_row();

                ui.label("Prop RPM:");
                ui.label(format!("{:.0}", v.prop3_rpm));
                ui.end_row();

                ui.label(RichText::new("Engine 4 (4-Eng: contributes to Right)").strong());
                ui.label("");
                ui.end_row();

                ui.label("N2:");
                ui.label(format!("{:.1}%", v.eng4_n2_percent));
                ui.end_row();

                ui.label("Combustion:");
                combustion_label(ui, v.eng4_combustion > 0.5);
                ui.end_row();

                ui.label("% Max RPM:");
                ui.label(format!("{:.1}%", v.eng4_pct_max_rpm));
                ui.end_row();

                ui.label("Engine RPM:");
                ui.label(format!("{:.0}", v.eng4_rpm));
                ui.end_row();

                ui.label("Prop RPM:");
                ui.label(format!("{:.0}", v.prop4_rpm));
                ui.end_row();
            }
            None => {
                ui.label("No data");
                ui.label("");
                ui.end_row();
            }
        }
    });
                    });
            });
        }

        #[cfg(debug_assertions)]
        if show_debug {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Logs");
                    ui.separator();
                    ui.checkbox(&mut self.autoscroll, "Auto-scroll");
                });
                ui.separator();

                let logs_all = self.logs.snapshot();
                let logs: Vec<&str> = logs_all.iter().map(|s| s.as_str()).collect();

                let row_height = 16.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(false)
                    .show(ui, |ui| {
                        TableBuilder::new(ui)
                            .striped(true)
                            .cell_layout(egui::Layout::left_to_right(egui::Align::Min))
                            .column(Column::remainder())
                            .body(|body| {
                                body.rows(row_height, logs.len(), |mut row| {
                                    let i = row.index();
                                    row.col(|ui| {
                                        ui.label(RichText::new(logs[i]).color(Color32::LIGHT_GRAY));
                                    });
                                });
                            });

                        if self.autoscroll && logs.len() > self.last_log_count {
                            let _ = ui.label("");
                            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                        }
                        self.last_log_count = logs.len();
                    });

                ctx.request_repaint_after(Duration::from_millis(60));
            });
        }

        // Откладываем сохранение настроек на диск, чтобы не писать файл
        // на каждый кадр во время перетаскивания слайдера.
        {
            let current_rev = self.config.current_rev();
            if current_rev != self.saved_config_rev {
                // Конфиг изменился — (пере)запускаем таймер ожидания.
                self.pending_save_at = Some(Instant::now() + Duration::from_millis(500));
            }
            if let Some(at) = self.pending_save_at {
                if Instant::now() >= at {
                    let cfg = self.config.get();
                    match crate::settings::save(&cfg) {
                        Ok(p) => self.logs.push(format!("Settings saved → {}", p.display())),
                        Err(e) => self.logs.push(format!("Failed to save settings: {}", e)),
                    }
                    self.saved_config_rev = current_rev;
                    self.pending_save_at = None;
                } else {
                    // Пока ждём, но не реагировали на смену rev в этом кадре — перерисуем позже.
                    ctx.request_repaint_after(Duration::from_millis(120));
                }
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            // Сохраняем синхронно перед закрытием, чтобы не потерять последние изменения.
            let cfg = self.config.get();
            if self.config.current_rev() != self.saved_config_rev {
                match crate::settings::save(&cfg) {
                    Ok(p) => self.logs.push(format!("Settings saved on exit → {}", p.display())),
                    Err(e) => self.logs.push(format!("Failed to save settings on exit: {}", e)),
                }
            }
        }

        loop {
            match self.rx_ui.try_recv() {
                Ok(cmd) => match cmd {
                    UiCmd::Show => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        ctx.request_repaint();
                    }
                    UiCmd::Hide => {}
                    UiCmd::Toggle => {}
                    UiCmd::Stop => {
                        self.hold.store(true, Ordering::Relaxed);
                        let _ = self.tx_hid.send(HidCmd::SetHold(true));
                        tray::notify_held(true);
                    }
                    UiCmd::Resume => {
                        self.hold.store(false, Ordering::Relaxed);
                        let _ = self.tx_hid.send(HidCmd::SetHold(false));
                        tray::notify_held(false);
                    }
                    UiCmd::Quit => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }
}