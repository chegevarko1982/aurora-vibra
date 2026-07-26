use egui::{Color32, RichText, Vec2};
#[cfg(debug_assertions)]
use egui_extras::{Column, TableBuilder};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use parking_lot::Mutex;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::{
    aircraft_profiles::{self, AircraftProfile, AircraftProfiles},
    i18n::{self, Lang, Strings},
    profiles::ProfileState,
    tray, ConfigShared, EffectDeviceTarget, EffectsShared, FlightVars, HidCmd, LogBuffer,
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

fn status_badge(ui: &mut egui::Ui, status: &SimStatus, t: &Strings) {
    let (text, color, filled) = match status {
        SimStatus::Disconnected => (t.disconnected, Color32::from_rgb(200, 60, 60), false),
        SimStatus::Connected => (t.connected, Color32::from_rgb(220, 180, 40), false),
        SimStatus::InFlight => (t.in_flight, Color32::from_rgb(30, 180, 90), true),
    };
    ui.horizontal(|ui| {
        circle_indicator_colored(ui, color, filled);
        ui.colored_label(color, text);
    });
}

fn controller_badge_dot(ui: &mut egui::Ui, label: &str, connected: bool, t: &Strings) {
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
                format!("{label}: {}", t.connected)
            } else {
                format!("{label}: {}", t.disconnected)
            },
        );
    });
}

/// Formats the aircraft title for display, with a fallback while
/// SimConnect hasn't delivered TITLE yet (or delivered an empty string,
/// e.g. right after connecting or during a sim restart).
fn format_aircraft_label(title: &str, t: &Strings) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        t.unknown_aircraft.to_string()
    } else {
        trimmed.to_string()
    }
}

pub struct UiState {
    pub controller_connected: Arc<AtomicBool>,
    pub throttle_connected: Arc<AtomicBool>,

    pub status: Arc<Mutex<SimStatus>>,
    pub aircraft_title: Arc<Mutex<String>>,
    pub aircraft_profiles: Arc<Mutex<AircraftProfiles>>,
    pub profile_state: Arc<Mutex<ProfileState>>,
    // Чекбокс рядом с кнопкой Save: "также как Default" — при следующем Save
    // текущий конфиг дополнительно применится как default (для всех
    // самолётов без своего именного профиля), даже если сейчас активен
    // именной профиль текущего борта.
    pub save_as_default_too: bool,

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

    pub lang: Lang,
}

impl UiState {
    /// Строка эффекта, где сила/амплитуда вибрации отображается и настраивается
    /// пользователем всегда в диапазоне 0..100%, независимо от технического
    /// предела эффекта в движке (255, 50, 55, 200...).
    /// `native_max` — во что превращается 100% при передаче в RumbleConfig;
    /// хранимое значение (`val`) остаётся в исходных технических единицах —
    /// rumble.rs ничего не знает о процентах.

    fn effect_row_percent_hinted(
        ui: &mut egui::Ui,
        name: &str,
        val: &mut f32,
        native_max: f32,
        enabled: &mut bool,
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
                // Процент всегда пересчитывается заново из исходного (технического)
                // значения — никакого накопления погрешности округления между кадрами.
                let mut pct = if native_max > 0.0 {
                    (*val / native_max * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };
                let slider = egui::Slider::new(&mut pct, 0.0..=100.0)
                    .trailing_fill(true)
                    .show_value(true)
                    .suffix("%")
                    .fixed_decimals(0);
                let resp = ui.add(slider);
                if let Some(h) = hint {
                    resp.clone().on_hover_text(h);
                }
                if resp.changed() {
                    *val = (pct / 100.0) * native_max;
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

    /// Строка эффекта Overspeed: порог больше не задаётся вручную ползунком,
    /// а приходит динамически из SimConnect (AIRSPEED BARBER POLE — красная
    /// черта Vmo/Mmo, сим сам двигает её вниз при наборе высоты) для текущего
    /// загруженного самолёта. `threshold_kn` — None, если SimConnect ещё не
    /// отдал значение (например, самолёт не загружен) — в этом случае
    /// показываем "Limit: N/A" и эффект не может сработать.
    fn overspeed_row(
        ui: &mut egui::Ui,
        enabled: &mut bool,
        threshold_kn: Option<f64>,
        active: bool,
        on_change: &mut bool,
        override_enabled: &mut bool,
        override_kn: &mut f64,
        t: &Strings,
    ) {
        ui.horizontal(|ui| {
            let cb = ui.checkbox(enabled, "");
            if cb.changed() {
                *on_change = true;
            }

            ui.label(RichText::new(t.overspeed_effect_name).strong());

            let override_hint = t.hover_override;
            let override_cb = ui.checkbox(override_enabled, t.chk_override).on_hover_text(override_hint);
            if override_cb.changed() {
                *on_change = true;
            }
            ui.add_enabled_ui(*override_enabled, |ui| {
                let mut manual_kn = *override_kn as f32;
                let resp = ui.add(
                    egui::DragValue::new(&mut manual_kn)
                        .speed(1.0)
                        .clamp_range(50.0..=700.0)
                        .suffix(" kt"),
                );
                resp.clone().on_hover_text(override_hint);
                if resp.changed() {
                    *override_kn = manual_kn as f64;
                    *on_change = true;
                }
            });

            let limit_text = match threshold_kn {
                Some(kn) => format!("{} {:.0} kts", t.lbl_limit, kn),
                None => t.limit_na.to_string(),
            };
            ui.add_enabled_ui(*enabled, |ui| {
                ui.label(RichText::new(limit_text).weak());
            });

            let is_triggering = active && *enabled && threshold_kn.is_some();
            let (color, filled) = if is_triggering {
                (Color32::from_rgb(220, 60, 60), true)
            } else if *enabled && threshold_kn.is_some() {
                (Color32::from_rgb(30, 180, 90), false)
            } else {
                (Color32::from_gray(90), false)
            };
            circle_indicator_colored(ui, color, filled);
            if is_triggering {
                ui.colored_label(color, t.status_active);
            }
        });
    }

    /// Компактная строка с двумя чекбоксами маршрутизации эффекта на
    /// устройства: "J" (Combat Joystick R) и "T" (URSA MINOR Throttle).
    /// Рисуется сразу под соответствующей строкой эффекта (effect_row_percent/_hinted).
    fn device_target_row(
        ui: &mut egui::Ui,
        target: &mut EffectDeviceTarget,
        on_change: &mut bool,
        t: &Strings,
    ) {
        ui.horizontal(|ui| {
            ui.add(egui::Label::new("    ").sense(egui::Sense::hover()));
            ui.label(RichText::new(t.device_label).weak().small());

            let cb_j = ui
                .checkbox(&mut target.enable_joystick, "J")
                .on_hover_text(t.hover_joystick_hw);
            if cb_j.changed() {
                *on_change = true;
            }

            let cb_t = ui
                .checkbox(&mut target.enable_throttle, "T")
                .on_hover_text(t.hover_throttle_hw);
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
                let mut tmp = *val as f32;
                let r = (*range.start() as f32)..=(*range.end() as f32);
                let resp = ui.add(egui::Slider::new(&mut tmp, r).trailing_fill(true).show_value(true));
                if let Some(h) = hint {
                    resp.clone().on_hover_text(h);
                }
                if resp.changed()
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

        let t = self.lang.strings();

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let st = *self.status.lock();
                status_badge(ui, &st, t);
                ui.separator();

                let controller_ok = self.controller_connected.load(Ordering::Relaxed);
                controller_badge_dot(ui, t.sidestick, controller_ok, t);

                let throttle_ok = self.throttle_connected.load(Ordering::Relaxed);
                controller_badge_dot(ui, t.throttle, throttle_ok, t);

                let ac = self.aircraft_title.lock().clone();
                ui.separator();
                // Кнопку "+ Save profile for this aircraft" убрали как лишний шаг —
                // Save (ниже) теперь сам создаёт именной профиль при первом
                // сохранении для нового борта. Вместо отдельного текста-статуса —
                // сама метка с названием борта просто меняет цвет: белый — для
                // этого борта ещё нет сохранённого профиля, #82f16a (ярче старого
                // зелёного, не сливается с индикатором "Connected" у джойстика/РУД
                // рядом) — профиль сохранён.
                let has_saved_profile = self.aircraft_profiles.lock().active_match.is_some();
                let ac_color = if has_saved_profile {
                    Color32::from_rgb(0x82, 0xf1, 0x6a) // #82f16a
                } else {
                    Color32::WHITE
                };
                ui.label(RichText::new(format_aircraft_label(&ac, t)).italics().color(ac_color));

                ui.separator();

                if ui.button(t.btn_load).clicked() {
                    match crate::settings::load() {
                        Some(sf) => {
                            let title = self.aircraft_title.lock().clone();
                            let mut ap = self.aircraft_profiles.lock();
                            ap.default = sf.default;
                            ap.profiles = sf.profiles;
                            aircraft_profiles::apply_for_aircraft(
                                &mut ap,
                                &self.config,
                                &mut self.profile_state.lock(),
                                &title,
                                &self.logs,
                            );
                            self.logs.push("Settings reloaded from disk".to_string());
                        }
                        None => self
                            .logs
                            .push("No settings file found on disk to reload".to_string()),
                    }
                }

                let dirty = self.config.current_rev() != self.aircraft_profiles.lock().loaded_rev;
                let save_label = if dirty {
                    RichText::new(t.btn_save).color(Color32::from_rgb(230, 170, 40))
                } else {
                    RichText::new(t.btn_save)
                };
                if ui.button(save_label).clicked() {
                    let live = self.config.get();
                    let sanitized = self.profile_state.lock().sanitize_for_save(&live);

                    // Первое сохранение для ещё не имеющего именного профиля борта:
                    // создаём его прямо здесь (раньше для этого была отдельная кнопка
                    // "+ Save profile for this aircraft" — убрана как лишний шаг).
                    {
                        let mut ap = self.aircraft_profiles.lock();
                        if ap.active_match.is_none() && !ac.trim().is_empty() {
                            ap.profiles.push(AircraftProfile {
                                match_substring: ac.clone(),
                                config: sanitized.clone(),
                            });
                            ap.active_match = Some(ac.clone());
                        }
                    }

                    match aircraft_profiles::save_active(
                        &self.aircraft_profiles,
                        sanitized,
                        self.save_as_default_too,
                    ) {
                        Ok(p) => {
                            self.aircraft_profiles.lock().loaded_rev = self.config.current_rev();
                            self.logs.push(format!("Settings saved → {}", p.display()));
                        }
                        Err(e) => self.logs.push(format!("Failed to save settings: {}", e)),
                    }
                }

                let has_named_profile_active =
                    self.aircraft_profiles.lock().active_match.is_some();
                ui.add_enabled(
                    has_named_profile_active,
                    egui::Checkbox::new(&mut self.save_as_default_too, t.chk_also_default),
                )
                .on_hover_text(t.hover_also_default);

                #[cfg(debug_assertions)]
                {
                    ui.separator();
                    ui.selectable_value(&mut self.active_tab, Tab::Main, t.tab_main);
                    ui.selectable_value(&mut self.active_tab, Tab::Debug, t.tab_debug);
                }

                // TODO: кнопка "Check for updates" временно скрыта из тулбара
                // (функциональность сохранена в updater::spawn_check и в
                // трей-меню — см. tray.rs), включим обратно позже.
                ui.separator();

                let holding = self.hold.load(Ordering::Relaxed);
                if !holding {
                    let stop_button = egui::Button::new(
                        RichText::new(t.btn_stop).color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(0x6d, 0x12, 0x1b)); // #6d121b
                    if ui.add(stop_button).clicked() {
                        self.hold.store(true, Ordering::Relaxed);
                        let _ = self.tx_hid.send(HidCmd::SetHold(true));
                        tray::notify_held(true);
                    }
                } else if ui.button(t.btn_resume).clicked() {
                    self.hold.store(false, Ordering::Relaxed);
                    let _ = self.tx_hid.send(HidCmd::SetHold(false));
                    tray::notify_held(false);
                }

                ui.separator();

                // Резерв на будущее — пока не заполнены.
                ui.menu_button(t.btn_options, |_ui| {});
                ui.button("?").on_hover_text(t.hover_help);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ru_btn = ui.add(egui::SelectableLabel::new(self.lang == Lang::Ru, "RU"));
                    let en_btn = ui.add(egui::SelectableLabel::new(self.lang == Lang::En, "EN"));
                    let new_lang = if en_btn.clicked() {
                        Some(Lang::En)
                    } else if ru_btn.clicked() {
                        Some(Lang::Ru)
                    } else {
                        None
                    };
                    if let Some(new_lang) = new_lang {
                        if new_lang != self.lang {
                            self.lang = new_lang;
                            i18n::set(new_lang);
                            tray::refresh_tooltip();
                            let ap = self.aircraft_profiles.lock();
                            let _ = crate::settings::save(&crate::settings::SettingsFile {
                                default: ap.default.clone(),
                                profiles: ap.profiles.clone(),
                                lang: new_lang,
                            });
                        }
                    }
                });
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
                        egui::CollapsingHeader::new(t.heading_aircraft_profiles)
                            .default_open(false)
                            .show(ui, |ui| {
                                let mut profiles_snapshot =
                                    self.aircraft_profiles.lock().profiles.clone();
                                if profiles_snapshot.is_empty() {
                                    ui.label(t.empty_profiles_hint);
                                }
                                let mut rename: Option<(usize, String, String)> = None;
                                let mut delete: Option<usize> = None;
                                for (i, p) in profiles_snapshot.iter_mut().enumerate() {
                                    ui.horizontal(|ui| {
                                        let before = p.match_substring.clone();
                                        let resp = ui.text_edit_singleline(&mut p.match_substring);
                                        if resp.changed() {
                                            rename = Some((i, before, p.match_substring.clone()));
                                        }
                                        if ui.button("🗑").on_hover_text(t.hover_delete_profile).clicked() {
                                            delete = Some(i);
                                        }
                                    });
                                }
                                if let Some((i, old_name, new_name)) = rename {
                                    let mut ap = self.aircraft_profiles.lock();
                                    if let Some(p) = ap.profiles.get_mut(i) {
                                        p.match_substring = new_name.clone();
                                    }
                                    if ap.active_match.as_deref() == Some(old_name.as_str()) {
                                        ap.active_match = Some(new_name);
                                    }
                                }
                                if let Some(i) = delete {
                                    let title = self.aircraft_title.lock().clone();
                                    let mut ap = self.aircraft_profiles.lock();
                                    if i < ap.profiles.len() {
                                        let removed_was_active = ap.active_match.as_deref()
                                            == Some(ap.profiles[i].match_substring.as_str());
                                        ap.profiles.remove(i);
                                        if removed_was_active {
                                            aircraft_profiles::apply_for_aircraft(
                                                &mut ap,
                                                &self.config,
                                                &mut self.profile_state.lock(),
                                                &title,
                                                &self.logs,
                                            );
                                        }
                                    }
                                }
                            });
                        ui.add_space(4.0);

                        ui.heading(t.heading_rumble_effects);
                        ui.add_space(4.0);

                        let mut _changed = false;

                        let ground_active = self.effects.ground_active.load(Ordering::Relaxed);
                        let ground_thump_active = self.effects.ground_thump_active.load(Ordering::Relaxed);
                        let taxi_start_crossed = self.effects.taxi_start_crossed.load(Ordering::Relaxed);
                        let taxi_end_crossed = self.effects.taxi_end_crossed.load(Ordering::Relaxed);

                        // Динамический порог Overspeed (AIRSPEED BARBER POLE),
                        // полученный от SimConnect для текущего самолёта — либо,
                        // если включён Override, значение, заданное вручную.
                        // None, если порог не определён (сим не подключён/выключен override без значения).
                        let overspeed_cfg_snapshot = self.config.get();
                        let overspeed_threshold_kn = if overspeed_cfg_snapshot.overspeed_override_enabled {
                            Some(overspeed_cfg_snapshot.overspeed_manual_kn).filter(|kn| *kn > 0.0)
                        } else {
                            self.last_vars
                                .lock()
                                .as_ref()
                                .map(|fv| fv.overspeed_barber_pole_kn)
                                .filter(|kn| *kn > 0.0)
                        };

                        self.config.with_mut(|cfg| {
                             // Overspeed
                             let mut overspeed_enabled = cfg.overspeed_enabled;
                             let mut overspeed_override = cfg.overspeed_override_enabled;
                             let mut overspeed_manual_kn = cfg.overspeed_manual_kn;

                             UiState::overspeed_row(
                                 ui,
                                 &mut overspeed_enabled,
                                 overspeed_threshold_kn,
                                 self.effects.overspeed_active.load(Ordering::Relaxed),
                                 &mut _changed,
                                 &mut overspeed_override,
                                 &mut overspeed_manual_kn,
                                 t,
                             );
                             cfg.overspeed_enabled = overspeed_enabled;
                             cfg.overspeed_override_enabled = overspeed_override;
                             cfg.overspeed_manual_kn = overspeed_manual_kn;
                             UiState::device_target_row(ui, &mut cfg.device_targets.overspeed, &mut _changed, t);

                             // Overspeed intensity slider - отображается как 0..100%, технический предел 255
                             ui.horizontal(|ui| {
                                 ui.add(egui::Label::new("    ").sense(egui::Sense::hover()));
                                 let intensity_hint = t.hover_overspeed_intensity;
                                 let lbl = ui.label(RichText::new(t.lbl_intensity).strong());
                                 lbl.on_hover_text(intensity_hint);
                                 let mut pct = (cfg.overspeed_intensity / 255.0 * 100.0).clamp(0.0, 100.0);
                                 let resp = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0)
                                     .trailing_fill(true)
                                     .show_value(true)
                                     .suffix("%")
                                     .fixed_decimals(0));
                                 resp.clone().on_hover_text(intensity_hint);
                                 if resp.changed() {
                                     cfg.overspeed_intensity = pct / 100.0 * 255.0;
                                     _changed = true;
                                 }

                                 let limit_text = match overspeed_threshold_kn {
                                     Some(kn) => format!("{} {:.0} kts", t.lbl_limit, kn),
                                     None => t.limit_na.to_string(),
                                 };
                                 ui.label(RichText::new(limit_text).weak())
                                     .on_hover_text(t.hover_overspeed_limit_barberpole);
                             });

                            ui.add_space(8.0);

                            // Ground Roll
                            let mut ground_enabled = cfg.ground_enabled;
                            UiState::effect_row_percent_hinted(
                                ui,
                                t.name_ground_roll,
                                &mut cfg.ground_roll,
                                50.0,
                                &mut ground_enabled,
                                ground_active || ground_thump_active,
                                &mut _changed,
                                Some(t.hover_ground_roll),
                            );
                            cfg.ground_enabled = ground_enabled;
                            UiState::device_target_row(ui, &mut cfg.device_targets.ground_roll, &mut _changed, t);

                            ui.add_space(8.0);

                            // Taxi thump bounds
                            ui.label(RichText::new(t.heading_taxi_thump).heading());
                            ui.add_space(4.0);

                            {
                                let mut start = cfg.taxi_start_kn;
                                let mut end = cfg.taxi_end_kn;
                                let mut start_enabled = cfg.taxi_start_enabled;
                                let mut end_enabled = cfg.taxi_end_enabled;

                                UiState::taxi_bound_row(
                                    ui,
                                    t.name_taxi_start,
                                    &mut start,
                                    &mut start_enabled,
                                    0.0..=20.0,
                                    taxi_start_crossed,
                                    &mut _changed,
                                    Some(t.hover_taxi_start),
                                );
                                cfg.taxi_start_enabled = start_enabled;

                                if start >= end - 0.5 {
                                    end = (start + 0.5).min(250.0);
                                }

                                UiState::taxi_bound_row(
                                    ui,
                                    t.name_taxi_end,
                                    &mut end,
                                    &mut end_enabled,
                                    1.0..=250.0, // Изменили верхний порог слайдера до 250
                                    taxi_end_crossed,
                                    &mut _changed,
                                    Some(t.hover_taxi_end),
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
                                let lbl = ui.label(RichText::new(t.lbl_period_curve).strong());
                                lbl.on_hover_text(t.hover_period_curve_full);
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
                            UiState::effect_row_percent_hinted(
                                ui,
                                t.name_flaps,
                                &mut cfg.flaps_peak,
                                255.0,
                                &mut flaps_enabled,
                                self.effects.flaps_bump_active.load(Ordering::Relaxed),
                                &mut _changed,
                                Some(t.hover_flaps),
                            );
                            cfg.flaps_enabled = flaps_enabled;
                            UiState::device_target_row(ui, &mut cfg.device_targets.flaps, &mut _changed, t);

                            // Gear effect — временно скрыт из UI по просьбе (пока не
                            // используется). cfg.gear_enabled=false по умолчанию
                            // (см. types.rs), сама логика в rumble.rs не тронута —
                            // раскомментировать этот блок, чтобы вернуть в UI.
                            // let mut gear_enabled = cfg.gear_enabled;
                            // UiState::effect_row_percent_hinted(
                            //     ui,
                            //     "Landing Gear (bump)",
                            //     &mut cfg.gear_peak,
                            //     255.0,
                            //     &mut gear_enabled,
                            //     self.effects.gear_bump_active.load(Ordering::Relaxed),
                            //     &mut _changed,
                            //     Some("Толчок при перемещении рычага шасси (вверх/вниз) — не путать с ударом при касании земли (Gear Strut Compression)"),
                            // );
                            // cfg.gear_enabled = gear_enabled;
                            // UiState::device_target_row(ui, &mut cfg.device_targets.gear_bump, &mut _changed);

                            // Stall effect
                            let mut stall_enabled = cfg.stall_enabled;
                            UiState::effect_row_percent_hinted(
                                ui,
                                t.name_stall,
                                &mut cfg.stall_ceiling,
                                255.0,
                                &mut stall_enabled,
                                self.effects.stall_active.load(Ordering::Relaxed),
                                &mut _changed,
                                Some(t.hover_stall),
                            );
                            cfg.stall_enabled = stall_enabled;
                            UiState::device_target_row(ui, &mut cfg.device_targets.stall, &mut _changed, t);

                             // Spoilers effect
                             let mut spoilers_enabled = cfg.spoilers_enabled;
                             UiState::effect_row_percent_hinted(
                                 ui,
                                 t.name_spoilers,
                                 &mut cfg.spoilers_intensity,
                                 250.0,
                                 &mut spoilers_enabled,
                                 self.effects.spoilers_active.load(Ordering::Relaxed),
                                 &mut _changed,
                                 Some(t.hover_spoilers),
                             );
                             cfg.spoilers_enabled = spoilers_enabled;
                             UiState::device_target_row(ui, &mut cfg.device_targets.spoilers, &mut _changed, t);

                             // Spoilers threshold slider - диапазон 0..100%
                             ui.horizontal(|ui| {
                                 ui.add(egui::Label::new("    ").sense(egui::Sense::hover()));
                                 let threshold_hint = t.hover_spoilers_threshold;
                                 let lbl = ui.label(RichText::new(t.lbl_threshold_pct).strong());
                                 lbl.on_hover_text(threshold_hint);
                                 let mut threshold = cfg.spoilers_threshold_pct as f32;
                                 let resp = ui.add(egui::Slider::new(&mut threshold, 0.0..=100.0)
                                     .trailing_fill(true)
                                     .show_value(true));
                                 resp.clone().on_hover_text(threshold_hint);
                                 if resp.changed() {
                                     cfg.spoilers_threshold_pct = threshold as f64;
                                     _changed = true;
                                 }
                             });

                             ui.add_space(8.0);

                             // Engine Start / Ignition effect
                             let mut engine_start_enabled = cfg.enable_engine_start;
                             let engine_start_hint = t.hover_engine_start;
                             ui.horizontal(|ui| {
                                 let cb = ui.checkbox(&mut engine_start_enabled, "");
                                 if cb.changed() {
                                     _changed = true;
                                 }

                                 // Настраиваемый порог N2 Idle — сразу после переключателя
                                 // включения эффекта, естественным потоком строки (без
                                 // выравнивания по правому краю).
                                 ui.add_space(8.0);
                                 let n2_hint = t.hover_n2_idle;
                                 let n2_label = ui.label(RichText::new(t.lbl_n2_idle).strong());
                                 n2_label.on_hover_text(n2_hint);
                                 let n2_resp = ui.add(
                                     egui::DragValue::new(&mut cfg.engine_idle_n2)
                                         .speed(1.0)
                                         .clamp_range(10.0..=100.0),
                                 );
                                 n2_resp.clone().on_hover_text(n2_hint);
                                 if n2_resp.changed() {
                                     _changed = true;
                                 }

                                 ui.add_space(8.0);

                                 let name_label = ui.label(RichText::new(t.name_engine_start).strong());
                                 name_label.on_hover_text(engine_start_hint);

                                 ui.add_enabled_ui(engine_start_enabled, |ui| {
                                     let mut pct = (cfg.engine_start_strength / 255.0 * 100.0).clamp(0.0, 100.0);
                                     let slider = egui::Slider::new(&mut pct, 0.0..=100.0)
                                         .trailing_fill(true)
                                         .show_value(true)
                                         .suffix("%")
                                         .fixed_decimals(0);
                                     let resp = ui.add(slider);
                                     resp.clone().on_hover_text(engine_start_hint);
                                     if resp.changed() {
                                         cfg.engine_start_strength = pct / 100.0 * 255.0;
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
                                     .checkbox(&mut cfg.four_engine_mode, t.chk_four_eng_mode)
                                     .on_hover_text(t.hover_four_eng_mode)
                                     .changed()
                                 {
                                     _changed = true;
                                 }
                             });

                             ui.horizontal(|ui| {
                                 ui.add(egui::Label::new("    ").sense(egui::Sense::hover()));
                                 if ui
                                     .checkbox(&mut cfg.swap_hand_layout, t.chk_swap_hands)
                                     .on_hover_text(t.hover_swap_hands)
                                     .changed()
                                 {
                                     _changed = true;
                                 }
                             });

                             ui.add_space(8.0);

                             // Base effect removed per user request
                             // ui.add_space(8.0);

                            // Gear Strut Compression (Touchdown) effect
                            ui.label(RichText::new(t.heading_gear_comp).heading());
                            ui.add_space(4.0);

                            let mut gear_comp_enabled = cfg.gear_comp_enabled;
                            ui.horizontal(|ui| {
                                if ui.checkbox(&mut gear_comp_enabled, t.chk_enabled).changed() {
                                    _changed = true;
                                }

                                ui.add_space(12.0);

                                if ui
                                    .checkbox(&mut cfg.split_touchdown, t.chk_split_touchdown)
                                    .on_hover_text(t.hover_split_touchdown)
                                    .changed()
                                {
                                    _changed = true;
                                }
                            });
                            cfg.gear_comp_enabled = gear_comp_enabled;

                            ui.add_enabled_ui(gear_comp_enabled, |ui| {
                                let headroom_hint = t.hover_headroom;
                                // Nose
                                let mut nose_enabled = cfg.gear_comp_nose_enabled;
                                UiState::effect_row_percent_hinted(
                                    ui,
                                    t.name_nose_peak,
                                    &mut cfg.gear_comp_nose_peak,
                                    55.0,
                                    &mut nose_enabled,
                                    self.effects.gear_comp_nose_active.load(Ordering::Relaxed),
                                    &mut _changed,
                                    Some(headroom_hint),
                                );
                                cfg.gear_comp_nose_enabled = nose_enabled;
                                UiState::device_target_row(ui, &mut cfg.device_targets.gear_comp_nose, &mut _changed, t);

                                // Left
                                let mut left_enabled = cfg.gear_comp_left_enabled;
                                UiState::effect_row_percent_hinted(
                                    ui,
                                    t.name_left_peak,
                                    &mut cfg.gear_comp_left_peak,
                                    55.0,
                                    &mut left_enabled,
                                    self.effects.gear_comp_left_active.load(Ordering::Relaxed),
                                    &mut _changed,
                                    Some(headroom_hint),
                                );
                                cfg.gear_comp_left_enabled = left_enabled;
                                UiState::device_target_row(ui, &mut cfg.device_targets.gear_comp_left, &mut _changed, t);

                                // Right
                                let mut right_enabled = cfg.gear_comp_right_enabled;
                                UiState::effect_row_percent_hinted(
                                    ui,
                                    t.name_right_peak,
                                    &mut cfg.gear_comp_right_peak,
                                    55.0,
                                    &mut right_enabled,
                                    self.effects.gear_comp_right_active.load(Ordering::Relaxed),
                                    &mut _changed,
                                    Some(headroom_hint),
                                );
                                cfg.gear_comp_right_enabled = right_enabled;
                                UiState::device_target_row(ui, &mut cfg.device_targets.gear_comp_right, &mut _changed, t);
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
                                ui.label(RichText::new(t.lbl_gear_transit).strong());

                                let active = self.effects.gear_transit_active.load(Ordering::Relaxed);
                                let (color, filled) = if active && gear_transit_enabled {
                                    (Color32::WHITE, true)
                                } else {
                                    (Color32::from_gray(90), false)
                                };
                                circle_indicator_colored(ui, color, filled);
                            });
                            UiState::device_target_row(ui, &mut cfg.device_targets.gear_transit, &mut _changed, t);

                            ui.add_space(8.0);

                            // Bank effect - только чекбокс и порог
                            let mut bank_enabled = cfg.bank_enabled;
                            ui.horizontal(|ui| {
                                let cb = ui.checkbox(&mut bank_enabled, "");
                                if cb.changed() {
                                    _changed = true;
                                }

                                ui.label(RichText::new(t.lbl_bank_turb).strong());

                                let active = self.effects.bank_active.load(Ordering::Relaxed);
                                let (color, filled) = if active && bank_enabled {
                                    (Color32::WHITE, true)
                                } else {
                                    (Color32::from_gray(90), false)
                                };
                                circle_indicator_colored(ui, color, filled);
                            });
                            cfg.bank_enabled = bank_enabled;
                            UiState::device_target_row(ui, &mut cfg.device_targets.bank, &mut _changed, t);

                            // Bank intensity slider - отображается как 0..100%, технический предел 200
                            ui.horizontal(|ui| {
                                ui.add(egui::Label::new("    ").sense(egui::Sense::hover()));
                                let intensity_hint = t.hover_bank_intensity;
                                let lbl = ui.label(RichText::new(t.lbl_intensity).strong());
                                lbl.on_hover_text(intensity_hint);
                                let mut pct = (cfg.bank_intensity / 200.0 * 100.0).clamp(0.0, 100.0);
                                let resp = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0)
                                    .trailing_fill(true)
                                    .show_value(true)
                                    .suffix("%")
                                    .fixed_decimals(0));
                                resp.clone().on_hover_text(intensity_hint);
                                if resp.changed()
                                {
                                    cfg.bank_intensity = pct / 100.0 * 200.0;
                                    _changed = true;
                                }
                            });

                            // Bank threshold slider - диапазон 0..90°
                            ui.horizontal(|ui| {
                                ui.add(egui::Label::new("    ").sense(egui::Sense::hover()));
                                let threshold_hint = t.hover_bank_threshold;
                                let lbl = ui.label(RichText::new(t.lbl_threshold_deg).strong());
                                lbl.on_hover_text(threshold_hint);
                                let mut threshold = cfg.bank_threshold_deg;
                                let resp = ui.add(egui::Slider::new(&mut threshold, 0.0..=90.0)
                                    .trailing_fill(true)
                                    .show_value(true));
                                resp.clone().on_hover_text(threshold_hint);
                                if resp.changed()
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
                        let mut telemetry_expanded = self.config.get().telemetry_expanded;
                        ui.horizontal(|ui| {
                            if ui.button(t.btn_reset_defaults).clicked() {
                                // Только сбрасывает ЖИВОЙ конфиг — на диск ничего не пишет,
                                // как и любое другое изменение. Нажмите Save (дискета в
                                // верхней панели), чтобы зафиксировать сброс.
                                self.config.set(RumbleConfig::default());
                                telemetry_expanded = true;
                            }

                            let toggle_label = if telemetry_expanded { t.btn_hide_telemetry } else { t.btn_show_telemetry };
                            if ui.button(toggle_label).clicked() {
                                telemetry_expanded = !telemetry_expanded;
                                self.config.with_mut(|cfg| cfg.telemetry_expanded = telemetry_expanded);
                            }
                        });

                        ui.separator();

if telemetry_expanded {
ui.columns(2, |columns| {
    // --- Левая колонка: общая телеметрия борта ---
    let ui = &mut columns[0];
    ui.heading(t.heading_live_aircraft_data);

    egui::Grid::new("aircraft_data")
        .num_columns(2)
        .spacing(Vec2::new(20.0, 4.0))
        .show(ui, |ui| {
            let v = *self.last_vars.lock();
            match v {
                Some(v) => {
                    ui.label(t.lbl_airspeed);
                    ui.label(format!("{:.1}", v.airspeed_indicated));
                    ui.end_row();

                    ui.label(t.lbl_barber_pole);
                    if v.overspeed_barber_pole_kn > 0.0 {
                        ui.label(format!("{:.1}", v.overspeed_barber_pole_kn));
                    } else {
                        ui.label(t.val_na);
                    }
                    ui.end_row();

                    ui.label(t.lbl_overspeed_warning);
                    ui.label(v.overspeed_warning.to_string());
                    ui.end_row();

                    ui.label(t.lbl_lear_horn);
                    ui.label(v.overspeed_lear_horn.to_string());
                    ui.end_row();

                    ui.label(t.lbl_gs);
                    ui.label(format!("{:.1}", v.ground_speed_kt));
                    ui.end_row();

                    ui.label(t.lbl_on_ground);
                    ui.label(v.on_ground.to_string());
                    ui.end_row();

                    ui.label(t.lbl_bank_deg);
                    ui.label(format!("{:.1}", v.bank_deg));
                    ui.end_row();

                    ui.label(t.lbl_flaps_pct);
                    ui.label(format!("{:.0}", v.flaps_pct));
                    ui.end_row();

                    ui.label(t.lbl_slats_pct);
                    ui.label(format!("{:.0}", v.slats_pct));
                    ui.end_row();

                    ui.label(t.lbl_gear);
                    ui.label(if v.gear_handle > 0.5 { t.val_down } else { t.val_up });
                    ui.end_row();

                    ui.label(t.lbl_spoilers_pct);
                    ui.label(format!("{:.0}", v.spoilers_pct));
                    ui.end_row();

                    ui.label(t.lbl_spoiler_l);
                    ui.label(format!("{:.0}", v.spoilers_left_pct));
                    ui.end_row();

                    ui.label(t.lbl_spoiler_r);
                    ui.label(format!("{:.0}", v.spoilers_right_pct));
                    ui.end_row();

                    // --- ДОБАВЛЕНА ТЕЛЕМЕТРИЯ ОБЖАТИЯ СТОЕК ШАССИ ---
                    ui.label(t.lbl_nose_gear);
                    ui.label(format!("{:.1}", v.gear_comp_nose));
                    ui.end_row();

                    ui.label(t.lbl_left_main);
                    ui.label(format!("{:.1}", v.gear_comp_left));
                    ui.end_row();

                    ui.label(t.lbl_right_main);
                    ui.label(format!("{:.1}", v.gear_comp_right));
                    ui.end_row();
                    // ------------------------------------------------

                    ui.label(t.lbl_stall);
                    ui.label(v.stalled.to_string());
                    ui.end_row();

                    ui.label(t.lbl_paused);
                    ui.label(v.paused.to_string());
                    ui.end_row();
                }
                None => {
                    ui.label(t.lbl_no_data);
                    ui.label("");
                    ui.end_row();
                }
            }
        });

    // --- Правая колонка: телеметрия двигателей ---
    let ui = &mut columns[1];
    ui.heading(t.heading_engine_telemetry);
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
                            ui.colored_label(Color32::from_rgb(30, 180, 90), t.val_on);
                        } else {
                            ui.colored_label(Color32::from_gray(140), t.val_off);
                        }
                    };

                    ui.label(RichText::new(t.eng1_header).strong());
                    ui.label("");
                    ui.end_row();

                    ui.label(t.lbl_n2);
                    ui.label(format!("{:.1}%", v.eng1_n2_percent));
                    ui.end_row();

                    ui.label(t.lbl_combustion);
                    combustion_label(ui, v.eng1_combustion > 0.5);
                    ui.end_row();

                    // PMDG 737 (NG3): L:EngineStart1b_Ext используется в
                    // rumble.rs для pre-spool разгона, здесь — для сверки.
                    ui.label(t.lbl_starter_active);
                    combustion_label(ui, v.eng1_starter_active);
                    ui.end_row();

                    ui.label(t.lbl_pmdg_starter_lvar);
                    combustion_label(ui, v.eng1_pmdg_starter_ext);
                    ui.end_row();

                    ui.label(t.lbl_pct_max_rpm);
                    ui.label(format!("{:.1}%", v.eng1_pct_max_rpm));
                    ui.end_row();

                    ui.label(t.lbl_engine_rpm);
                    ui.label(format!("{:.0}", v.eng1_rpm));
                    ui.end_row();

                    ui.label(t.lbl_prop_rpm);
                    ui.label(format!("{:.0}", v.prop1_rpm));
                    ui.end_row();

                    ui.label(RichText::new(t.eng2_header).strong());
                    ui.label("");
                    ui.end_row();

                    ui.label(t.lbl_n2);
                    ui.label(format!("{:.1}%", v.eng2_n2_percent));
                    ui.end_row();

                    ui.label(t.lbl_combustion);
                    combustion_label(ui, v.eng2_combustion > 0.5);
                    ui.end_row();

                    ui.label(t.lbl_starter_active);
                    combustion_label(ui, v.eng2_starter_active);
                    ui.end_row();

                    ui.label(t.lbl_pmdg_starter_lvar);
                    combustion_label(ui, v.eng2_pmdg_starter_ext);
                    ui.end_row();

                    ui.label(t.lbl_pct_max_rpm);
                    ui.label(format!("{:.1}%", v.eng2_pct_max_rpm));
                    ui.end_row();

                    ui.label(t.lbl_engine_rpm);
                    ui.label(format!("{:.0}", v.eng2_rpm));
                    ui.end_row();

                    ui.label(t.lbl_prop_rpm);
                    ui.label(format!("{:.0}", v.prop2_rpm));
                    ui.end_row();

                    ui.label(RichText::new(t.eng3_header).strong());
                    ui.label("");
                    ui.end_row();

                    ui.label(t.lbl_n2);
                    ui.label(format!("{:.1}%", v.eng3_n2_percent));
                    ui.end_row();

                    ui.label(t.lbl_combustion);
                    combustion_label(ui, v.eng3_combustion > 0.5);
                    ui.end_row();

                    ui.label(t.lbl_pct_max_rpm);
                    ui.label(format!("{:.1}%", v.eng3_pct_max_rpm));
                    ui.end_row();

                    ui.label(t.lbl_engine_rpm);
                    ui.label(format!("{:.0}", v.eng3_rpm));
                    ui.end_row();

                    ui.label(t.lbl_prop_rpm);
                    ui.label(format!("{:.0}", v.prop3_rpm));
                    ui.end_row();

                    ui.label(RichText::new(t.eng4_header).strong());
                    ui.label("");
                    ui.end_row();

                    ui.label(t.lbl_n2);
                    ui.label(format!("{:.1}%", v.eng4_n2_percent));
                    ui.end_row();

                    ui.label(t.lbl_combustion);
                    combustion_label(ui, v.eng4_combustion > 0.5);
                    ui.end_row();

                    ui.label(t.lbl_pct_max_rpm);
                    ui.label(format!("{:.1}%", v.eng4_pct_max_rpm));
                    ui.end_row();

                    ui.label(t.lbl_engine_rpm);
                    ui.label(format!("{:.0}", v.eng4_rpm));
                    ui.end_row();

                    ui.label(t.lbl_prop_rpm);
                    ui.label(format!("{:.0}", v.prop4_rpm));
                    ui.end_row();
                }
                None => {
                    ui.label(t.lbl_no_data);
                    ui.label("");
                    ui.end_row();
                }
            }
        });
});
}
                    });
            });
        }

        #[cfg(debug_assertions)]
        if show_debug {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(t.heading_logs);
                    ui.separator();
                    ui.checkbox(&mut self.autoscroll, t.chk_autoscroll);
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

        // Автосохранение убрано намеренно: запись на диск теперь происходит
        // ТОЛЬКО по явному нажатию кнопки Save (дискета) в верхней панели —
        // см. floppy_icon_button / aircraft_profiles::save_active. Ни смена
        // слайдера, ни смена самолёта, ни закрытие окна сами по себе больше
        // ничего не пишут на диск.

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