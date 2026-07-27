use egui::{Color32, RichText, Vec2};
#[cfg(debug_assertions)]
use egui_extras::{Column, TableBuilder};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use parking_lot::Mutex;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{
    ConfigShared, EffectDeviceTarget, EffectsShared, FlightVars, HidCmd, LogBuffer, RumbleConfig,
    SimStatus, UiCmd,
    aircraft_profiles::{self, AircraftProfile, AircraftProfiles},
    i18n::{self, Lang, Strings},
    profiles::ProfileState,
    tray,
};

/// Цветовая палитра карточек эффектов и Live Monitor. Раньше цвета были
/// разбросаны литералами (`Color32::from_rgb(...)`) по десятку мест — свели
/// в одно место, чтобы контраст карточка/фон и роли акцентов (primary vs
/// live vs warning) были согласованы по всему приложению.
mod palette {
    use egui::Color32;

    pub const BG_APP: Color32 = Color32::from_rgb(0x0B, 0x0E, 0x14);
    pub const BG_SIDEBAR: Color32 = Color32::from_rgb(0x0F, 0x13, 0x1A);
    pub const BG_CARD: Color32 = Color32::from_rgb(0x16, 0x1B, 0x24);
    pub const BG_CARD_DISABLED: Color32 = Color32::from_rgb(0x12, 0x16, 0x1D);

    pub const BORDER_DEFAULT: Color32 = Color32::from_rgb(0x2A, 0x33, 0x44);
    pub const BORDER_ACTIVE: Color32 = Color32::from_rgb(0x3B, 0x4A, 0x61);

    pub const ACCENT_PRIMARY: Color32 = Color32::from_rgb(0x3B, 0x82, 0xF6);
    pub const ACCENT_LIVE: Color32 = Color32::from_rgb(0x22, 0xD3, 0xEE);

    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0x94, 0xA3, 0xB8);
    pub const TEXT_DISABLED: Color32 = Color32::from_rgb(0x64, 0x74, 0x8B);

    /// Единая тёмная тема приложения поверх egui::Visuals::dark() —
    /// применяется один раз при старте (см. UiState::apply_theme), а не
    /// каждый кадр, т.к. это глобальное состояние Context, а не что-то,
    /// что нужно пересчитывать в render-цикле.
    pub fn apply(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        // panel_fill — общий фон по умолчанию для side/top/central-панелей
        // (см. Frame::side_top_panel / Frame::central_panel в egui). Ставим
        // его в BG_APP, а sidebar и Live Monitor явно перекрывают своим
        // Frame::fill(BG_SIDEBAR) в местах создания панелей — иначе все
        // панели красились бы в один и тот же цвет и контраст пропадал бы.
        visuals.panel_fill = BG_APP;
        visuals.window_fill = BG_APP;
        visuals.extreme_bg_color = BG_APP;
        visuals.faint_bg_color = BG_SIDEBAR;
        visuals.selection.bg_fill = ACCENT_PRIMARY.gamma_multiply(0.35);
        visuals.selection.stroke.color = ACCENT_PRIMARY;
        visuals.hyperlink_color = ACCENT_PRIMARY;
        ctx.set_visuals(visuals);
    }
}

/// Точка входа для main.rs: применяет тёмную тему один раз при старте
/// приложения (см. `eframe::run_native`'s creation callback), до того как
/// нарисован первый кадр.
pub fn apply_theme(ctx: &egui::Context) {
    palette::apply(ctx);
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Tab {
    Main,
    #[cfg(debug_assertions)]
    Debug,
}

/// Раздел навигации внутри вкладки Main (левый SidePanel). Чисто UI-состояние
/// текущей сессии — не персистится вместе с RumbleConfig/настройками.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Section {
    Rumble,
    Taxi,
    Engines,
    Gear,
    Telemetry,
}

/// Placeholder device glyph kind — see `UiState::device_icon_button`.
#[derive(Clone, Copy)]
enum DeviceIcon {
    Joystick,
    Throttle,
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

/// Тот же кружок-индикатор, но с явно заданным радиусом — нужен для плотных
/// строк Live Monitor, где полноразмерная (interact_size) точка из
/// `circle_indicator_colored` съедала бы слишком много вертикального места.
fn dot_indicator(ui: &mut egui::Ui, color: Color32, filled: bool, diameter: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(diameter), egui::Sense::hover());
    let center = rect.center();
    let r = diameter * 0.36;
    let fill_color = if filled { color } else { Color32::TRANSPARENT };
    ui.painter().circle_filled(center, r, fill_color);
    ui.painter()
        .circle_stroke(center, r, egui::Stroke::new(1.2, color));
}

/// Статус-бейдж карточки эффекта: круглый маркер без текстовой подписи.
/// Когда эффект реально сработал (`active && enabled`), маркер закрашивается
/// белым — этого сигнала достаточно, отдельной текстовой подписи рядом с ним
/// нет (маркер уже показывает состояние).
fn effect_status_badge(ui: &mut egui::Ui, enabled: bool, active: bool, t: &Strings) {
    if !enabled {
        ui.label(RichText::new(t.status_off).small().color(palette::TEXT_DISABLED));
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if active {
            dot_indicator(ui, Color32::WHITE, true, 8.0);
        } else {
            dot_indicator(ui, palette::BORDER_ACTIVE, false, 8.0);
        }
    });
}

fn status_badge(ui: &mut egui::Ui, status: &SimStatus, t: &Strings) {
    let (text, color, filled) = match status {
        SimStatus::Disconnected => (t.disconnected, Color32::from_rgb(200, 60, 60), false),
        SimStatus::Connected => (t.connected, Color32::from_rgb(220, 180, 40), false),
        SimStatus::InFlight => (t.in_flight, Color32::from_rgb(30, 180, 90), true),
        SimStatus::SimConnectMissing => {
            (t.simconnect_missing, Color32::from_rgb(230, 130, 30), true)
        }
    };
    ui.horizontal(|ui| {
        circle_indicator_colored(ui, color, filled);
        let label = ui.colored_label(color, text);
        // Подсказка только там, где пользователю нужно что-то сделать руками.
        if matches!(status, SimStatus::SimConnectMissing) {
            label.on_hover_text(t.hover_simconnect_missing);
        }
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
    pub active_section: Section,
    // В развёрнутом виде монитор по умолчанию показывает только включённые
    // эффекты; выключенные скрыты за "+N disabled", пока не нажали явно.
    pub monitor_show_disabled: bool,
    pub hold: Arc<AtomicBool>,

    // "Close to tray" (Options menu): при close_requested окно прячется
    // (ViewportCommand::Visible(false)) вместо реального завершения процесса —
    // если только это не настоящий Exit из трея (force_quit), который должен
    // этот перехват обойти. См. UiState::ui() и tray.rs.
    pub close_to_tray: bool,
    pub force_quit: Arc<AtomicBool>,

    pub rx_ui: Receiver<UiCmd>,
    pub tx_ui: Sender<UiCmd>,

    pub lang: Lang,
}

impl UiState {
    /// Сохраняет глобальные (не привязанные к самолёту) настройки — язык и
    /// close-to-tray — вместе с текущим набором профилей. Вызывается при
    /// изменении любого из этих двух переключателей.
    fn save_global_settings(&self) {
        crate::settings::set_close_to_tray(self.close_to_tray);
        let ap = self.aircraft_profiles.lock();
        let _ = crate::settings::save(&crate::settings::SettingsFile {
            default: ap.default.clone(),
            profiles: ap.profiles.clone(),
            lang: self.lang,
            close_to_tray: self.close_to_tray,
            simconnect_dll_path: crate::settings::simconnect_dll_path(),
        });
    }

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
        device: &mut EffectDeviceTarget,
        t: &Strings,
    ) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let cb = ui.checkbox(enabled, "");
                if cb.changed() {
                    *on_change = true;
                }

                let name_label = ui.label(RichText::new(name).strong());
                if let Some(h) = hint {
                    name_label.on_hover_text(h);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    effect_status_badge(ui, *enabled, active, t);
                    ui.add(egui::Separator::default().vertical().spacing(10.0));
                    Self::device_target_toggle(ui, device, on_change, t);
                });
            });

            ui.add_enabled_ui(*enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().slider_width = (ui.available_width() - 65.0).max(60.0);
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
            });
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
        device: &mut EffectDeviceTarget,
        t: &Strings,
    ) {
        ui.vertical(|ui| {
            // Триггерится только когда порог реально пришёл от SimConnect (или
            // задан вручную через Override) — без threshold_kn эффект физически
            // не может сработать, даже если галочка включена.
            let is_triggering = active && *enabled && threshold_kn.is_some();

            ui.horizontal(|ui| {
                let cb = ui.checkbox(enabled, "");
                if cb.changed() {
                    *on_change = true;
                }

                ui.label(RichText::new(t.overspeed_effect_name).strong());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    effect_status_badge(ui, *enabled, is_triggering, t);
                    ui.add(egui::Separator::default().vertical().spacing(10.0));
                    Self::device_target_toggle(ui, device, on_change, t);
                });
            });

            ui.horizontal(|ui| {
                let limit_text = match threshold_kn {
                    Some(kn) => format!("{} {:.0} kts", t.lbl_limit, kn),
                    None => t.limit_na.to_string(),
                };
                ui.add_enabled_ui(*enabled, |ui| {
                    ui.label(RichText::new(limit_text).weak())
                        .on_hover_text(t.hover_overspeed_limit_barberpole);
                });
            });

            ui.horizontal(|ui| {
                let override_hint = t.hover_override;
                let override_cb = ui
                    .checkbox(override_enabled, t.chk_override)
                    .on_hover_text(override_hint);
                if override_cb.changed() {
                    *on_change = true;
                }
                ui.add_enabled_ui(*override_enabled, |ui| {
                    let mut manual_kn = *override_kn as f32;
                    let resp = ui.add(
                        egui::DragValue::new(&mut manual_kn)
                            .speed(1.0)
                            .range(50.0..=700.0)
                            .suffix(" kt"),
                    );
                    resp.clone().on_hover_text(override_hint);
                    if resp.changed() {
                        *override_kn = manual_kn as f64;
                        *on_change = true;
                    }
                });
            });
        });
    }

    /// Компактный сегментированный переключатель маршрутизации эффекта на
    /// устройства (Joystick / Throttle), заменивший пару подписанных
    /// чекбоксов "Device: J T" на отдельной строке — теперь встраивается в
    /// конец той же ui.horizontal, где рисуется сам слайдер эффекта.
    fn device_target_toggle(
        ui: &mut egui::Ui,
        target: &mut EffectDeviceTarget,
        on_change: &mut bool,
        t: &Strings,
    ) {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            let j = Self::device_icon_button(ui, target.enable_joystick, DeviceIcon::Joystick)
                .on_hover_text(t.hover_joystick_hw);
            if j.clicked() {
                target.enable_joystick = !target.enable_joystick;
                *on_change = true;
            }
            let th = Self::device_icon_button(ui, target.enable_throttle, DeviceIcon::Throttle)
                .on_hover_text(t.hover_throttle_hw);
            if th.clicked() {
                target.enable_throttle = !target.enable_throttle;
                *on_change = true;
            }
        });
    }

    /// Рисует иконку устройства (джойстик/РУД) как кнопку с картинкой
    /// (assets/icon_joystick.png, assets/icon_throttle.png), подсвеченную
    /// акцентным цветом в выбранном состоянии — та же selected-стилистика,
    /// что была у selectable_label, но с реальной иконкой вместо эмодзи.
    fn device_icon_button(ui: &mut egui::Ui, selected: bool, icon: DeviceIcon) -> egui::Response {
        let source = match icon {
            DeviceIcon::Joystick => egui::include_image!("../assets/icon_joystick.png"),
            DeviceIcon::Throttle => egui::include_image!("../assets/icon_throttle.png"),
        };
        let tint = if selected {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        let image = egui::Image::new(source)
            .tint(tint)
            .fit_to_exact_size(egui::vec2(19.2, 19.2));
        let mut button = egui::Button::new(image).selected(selected);
        if selected {
            // Явный fill вместо стандартной полупрозрачной selection.bg_fill —
            // просили конкретную подложку под включённой иконкой устройства,
            // не трогая глобальный акцент выбора (он используется и в других
            // местах — nav, чекбоксы).
            button = button.fill(Color32::from_rgb(0xCC, 0xCE, 0xFF));
        }
        ui.add(button)
    }

    /// Обёртка-карточка вокруг одного эффекта: рамка со скруглением,
    /// подсветка обводки акцентным цветом при active&&enabled, приглушение
    /// (opacity) содержимого, если эффект выключен.
    fn effect_card(
        ui: &mut egui::Ui,
        enabled: bool,
        _active: bool,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        // `active` сознательно не влияет на обводку карточки — раньше она
        // мигала cyan в такт реальным импульсам эффекта (active моргает на
        // каждый "тик" мотора), что было слишком навязчиво на весь блок.
        // Live-индикация оставлена только в Live Monitor справа.
        let (stroke, fill) = if enabled {
            (
                egui::Stroke::new(1.0, palette::BORDER_ACTIVE),
                palette::BG_CARD,
            )
        } else {
            (
                egui::Stroke::new(1.0, palette::BORDER_DEFAULT),
                palette::BG_CARD_DISABLED,
            )
        };
        egui::Frame::group(ui.style())
            .stroke(stroke)
            .fill(fill)
            .shadow(egui::Shadow {
                offset: [0, 2],
                blur: 4,
                spread: 0,
                color: Color32::from_black_alpha(60),
            })
            .corner_radius(8u8)
            .inner_margin(egui::Margin::same(12))
            .outer_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Выключенную карточку гасим через override текста, а не
                // set_opacity(): opacity глушит заодно slider handle и рамки
                // контролов, из-за чего карточка выглядит "сломанной", а не
                // просто неактивной.
                if !enabled {
                    ui.visuals_mut().override_text_color = Some(palette::TEXT_DISABLED);
                }
                add_contents(ui);
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
        t: &Strings,
    ) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let cb = ui.checkbox(enabled, "");
                if cb.changed() {
                    *on_change = true;
                }

                let name_label = ui.label(RichText::new(name).strong());
                if let Some(h) = hint {
                    name_label.on_hover_text(h);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    effect_status_badge(ui, *enabled, active, t);
                });
            });

            ui.add_enabled_ui(*enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().slider_width = (ui.available_width() - 65.0).max(60.0);
                    let mut tmp = *val as f32;
                    let r = (*range.start() as f32)..=(*range.end() as f32);
                    let resp = ui.add(
                        egui::Slider::new(&mut tmp, r)
                            .trailing_fill(true)
                            .show_value(true),
                    );
                    if let Some(h) = hint {
                        resp.clone().on_hover_text(h);
                    }
                    if resp.changed() {
                        *val = tmp as f64;
                        *on_change = true;
                    }
                });
            });
        });
    }
}

impl eframe::App for UiState {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // egui 0.35: the App trait hands us the root Ui directly (no more
        // `&Context` parameter) — recover the Context from it for the calls
        // below that still need it (repaint scheduling, viewport commands).
        let ctx = ui.ctx().clone();
        {
            const TARGET_FPS: u64 = 30;
            ctx.request_repaint_after(Duration::from_millis(1000 / TARGET_FPS));
        }

        // Close to tray: перехватываем закрытие окна крестиком, если включено
        // в Options — вместо реального выхода просто прячем окно, процесс и
        // фоновые потоки (HID/SimConnect/трей) продолжают работать. Exit из
        // трея выставляет force_quit=true заранее, так что этот перехват его
        // не трогает и закрытие проходит по-настоящему.
        if ctx.input(|i| i.viewport().close_requested())
            && self.close_to_tray
            && !self.force_quit.load(Ordering::Relaxed)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        {
            let style = ui.style_mut();
            style.spacing.item_spacing = Vec2::new(6.0, 6.0);
            style.spacing.slider_width = 160.0;
        }

        let t = self.lang.strings();

        egui::Panel::top("top").show(ui, |ui| {
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
                //
                // Голубой цвет приоритетнее обоих: означает, что у борта есть
                // кастомная логика чтения SimConnect/эффектов — встроенный
                // профиль (MADDOG/LEARJET, см. src/profiles.rs), PMDG
                // (pre-spool разгон по L:EngineStart1b/2b_Ext) или Fenix A320
                // (порог Overspeed читается из L:I_PFD_VMAX вместо AIRSPEED
                // BARBER POLE, см. sim/parse.rs) — вне зависимости от того,
                // сохранён ли для него профиль настроек.
                //
                // КОНВЕНЦИЯ (сохранять при любой переработке этого UI/label):
                // любой борт с особой логикой чтения SimConnect-переменных
                // (свой L-var/simvar вместо стандартного, самонейтрализующийся
                // на прочих бортах — по образцу is_pmdg_aircraft/
                // is_fenix_aircraft в src/profiles.rs) обязан подсвечиваться
                // ИМЕННО этим голубым (70, 160, 255) в метке названия борта,
                // приоритетно над индикатором "профиль сохранён". Добавляя
                // новый такой борт — заводить детектор рядом с
                // is_pmdg_aircraft/is_fenix_aircraft в src/profiles.rs и
                // добавлять его сюда через || в has_custom_telemetry.
                let has_custom_telemetry = crate::profiles::has_built_in_profile(&ac)
                    || crate::profiles::is_pmdg_aircraft(&ac)
                    || crate::profiles::is_fenix_aircraft(&ac);
                let has_saved_profile = self.aircraft_profiles.lock().active_match.is_some();
                let ac_color = if has_custom_telemetry {
                    Color32::from_rgb(70, 160, 255)
                } else if has_saved_profile {
                    Color32::from_rgb(0x82, 0xf1, 0x6a) // #82f16a
                } else {
                    Color32::WHITE
                };
                ui.label(
                    RichText::new(format_aircraft_label(&ac, t))
                        .italics()
                        .color(ac_color),
                );

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(4.0);

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
                    // named_cfg — как есть, БЕЗ sanitize_for_save: именной
                    // профиль привязан к конкретному борту, поэтому и значения
                    // встроенного оверлея (MADDOG/LEARJET/Fenix CFM/IAE), и
                    // ручные правки пользователя поверх них (например Engine
                    // Idle N2% для конкретной ливреи Fenix) законно становятся
                    // частью ЭТОГО профиля и переживут следующую загрузку
                    // борта (см. aircraft_profiles::apply_for_aircraft —
                    // встроенный оверлей больше не накатывается поверх
                    // именного профиля).
                    let named_cfg = live.clone();
                    // default_cfg — очищенный снимок: default общий для ВСЕХ
                    // самолётов без своего именного профиля, значения,
                    // зашитые под конкретный борт, здесь по-прежнему обязаны
                    // откатываться к базовым (см. sanitize_for_save).
                    let default_cfg = self.profile_state.lock().sanitize_for_save(&live);

                    // Первое сохранение для ещё не имеющего именного профиля борта:
                    // создаём его прямо здесь (раньше для этого была отдельная кнопка
                    // "+ Save profile for this aircraft" — убрана как лишний шаг).
                    {
                        let mut ap = self.aircraft_profiles.lock();
                        if ap.active_match.is_none() && !ac.trim().is_empty() {
                            ap.profiles.push(AircraftProfile {
                                match_substring: ac.clone(),
                                config: named_cfg.clone(),
                            });
                            ap.active_match = Some(ac.clone());
                        }
                    }

                    match aircraft_profiles::save_active(
                        &self.aircraft_profiles,
                        named_cfg,
                        default_cfg,
                        self.save_as_default_too,
                    ) {
                        Ok(p) => {
                            self.aircraft_profiles.lock().loaded_rev = self.config.current_rev();
                            self.logs.push(format!("Settings saved → {}", p.display()));
                        }
                        Err(e) => self.logs.push(format!("Failed to save settings: {}", e)),
                    }
                }

                let has_named_profile_active = self.aircraft_profiles.lock().active_match.is_some();
                ui.add_enabled(
                    has_named_profile_active,
                    egui::Checkbox::new(&mut self.save_as_default_too, t.chk_also_default),
                )
                .on_hover_text(t.hover_also_default);

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(4.0);

                // TODO: кнопка "Check for updates" временно скрыта из тулбара
                // (функциональность сохранена в updater::spawn_check и в
                // трей-меню — см. tray.rs), включим обратно позже.

                // Stop/Resume перенесены в левую колонку навигации, под пункт
                // Telemetry — см. nav_panel ниже.

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ru_btn = ui.selectable_label(self.lang == Lang::Ru, "RU");
                    let en_btn = ui.selectable_label(self.lang == Lang::En, "EN");

                    ui.add_space(8.0);
                    // Overflow-меню: Options (резерв на будущее), Help и, в debug-сборке,
                    // переключатель Main/Debug — раньше это были три-четыре отдельные
                    // кнопки в тулбаре, теперь редко используемые действия собраны в одном месте.
                    ui.menu_button("...", |ui| {
                        #[cfg(debug_assertions)]
                        {
                            if ui
                                .selectable_label(self.active_tab == Tab::Main, t.tab_main)
                                .clicked()
                            {
                                self.active_tab = Tab::Main;
                            }
                            if ui
                                .selectable_label(self.active_tab == Tab::Debug, t.tab_debug)
                                .clicked()
                            {
                                self.active_tab = Tab::Debug;
                            }
                            ui.separator();
                        }
                        ui.menu_button(t.btn_options, |ui| {
                            if ui
                                .checkbox(&mut self.close_to_tray, t.chk_close_to_tray)
                                .on_hover_text(t.hover_close_to_tray)
                                .changed()
                            {
                                self.save_global_settings();
                            }
                        });
                        ui.label(t.hover_help);
                    });

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
                            self.save_global_settings();
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
            // Общий снимок "включён/активен" для каждого эффекта — используется и
            // бейджами счётчика в навигации слева, и списком Live Monitor справа,
            // чтобы не считать дважды.
            let mon = self.config.get();
            // Порядок элементов соответствует новой группировке по разделам
            // (Aerodynamics / Taxi / Engines / Gears) — см. диапазоны ниже.
            // Четвёртое поле — текущая интенсивность эффекта в процентах (та же
            // формула val/native_max*100, что использует слайдер самой карточки),
            // None — для triggered-по-порогу эффектов без единого "уровня"
            // (Gear Transit). Используется компактным Live Monitor, чтобы
            // показывать не только "включён/активен", но и реальное число.
            let rows: [(&str, bool, bool, Option<f32>); 11] = [
                (
                    t.overspeed_effect_name,
                    mon.overspeed_enabled,
                    self.effects.overspeed_active.load(Ordering::Relaxed),
                    Some((mon.overspeed_intensity / 255.0 * 100.0).clamp(0.0, 100.0)),
                ),
                (
                    t.name_stall,
                    mon.stall_enabled,
                    self.effects.stall_active.load(Ordering::Relaxed),
                    Some((mon.stall_ceiling / 255.0 * 100.0).clamp(0.0, 100.0)),
                ),
                (
                    t.name_spoilers,
                    mon.spoilers_enabled,
                    self.effects.spoilers_active.load(Ordering::Relaxed),
                    Some((mon.spoilers_intensity / 250.0 * 100.0).clamp(0.0, 100.0)),
                ),
                (
                    t.name_flaps,
                    mon.flaps_enabled,
                    self.effects.flaps_bump_active.load(Ordering::Relaxed),
                    Some((mon.flaps_peak / 255.0 * 100.0).clamp(0.0, 100.0)),
                ),
                (
                    t.lbl_bank_turb,
                    mon.bank_enabled,
                    self.effects.bank_active.load(Ordering::Relaxed),
                    Some((mon.bank_intensity / 200.0 * 100.0).clamp(0.0, 100.0)),
                ),
                (
                    t.name_engine_start,
                    mon.enable_engine_start,
                    self.effects.engine_start_active.load(Ordering::Relaxed),
                    Some((mon.engine_start_strength / 255.0 * 100.0).clamp(0.0, 100.0)),
                ),
                (
                    t.name_ground_roll,
                    mon.ground_enabled,
                    self.effects.ground_active.load(Ordering::Relaxed)
                        || self.effects.ground_thump_active.load(Ordering::Relaxed),
                    Some((mon.ground_roll / 50.0 * 100.0).clamp(0.0, 100.0)),
                ),
                (
                    t.name_left_peak,
                    mon.gear_comp_left_enabled,
                    self.effects.gear_comp_left_active.load(Ordering::Relaxed),
                    Some((mon.gear_comp_left_peak / 55.0 * 100.0).clamp(0.0, 100.0)),
                ),
                (
                    t.name_nose_peak,
                    mon.gear_comp_nose_enabled,
                    self.effects.gear_comp_nose_active.load(Ordering::Relaxed),
                    Some((mon.gear_comp_nose_peak / 55.0 * 100.0).clamp(0.0, 100.0)),
                ),
                (
                    t.name_right_peak,
                    mon.gear_comp_right_enabled,
                    self.effects.gear_comp_right_active.load(Ordering::Relaxed),
                    Some((mon.gear_comp_right_peak / 55.0 * 100.0).clamp(0.0, 100.0)),
                ),
                (
                    t.lbl_gear_transit,
                    mon.gear_transit_enabled,
                    self.effects.gear_transit_active.load(Ordering::Relaxed),
                    None,
                ),
            ];
            let section_active = |range: std::ops::Range<usize>| -> bool {
                rows[range]
                    .iter()
                    .any(|(_, enabled, active, _)| *enabled && *active)
            };
            let rumble_active = section_active(0..5);
            let taxi_active = false; // в Taxi Thump больше нет эффектов из Live Monitor (Flaps переехал в Aerodynamics)
            let engines_active = section_active(5..6);
            let gear_active = section_active(6..11);

            let nav_panel_width = if self.lang == Lang::Ru { 190.0 } else { 150.0 };
            egui::Panel::left("nav_panel")
                .resizable(false)
                .exact_size(nav_panel_width)
                .frame(egui::Frame::side_top_panel(ui.style()).fill(palette::BG_SIDEBAR))
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    // Кириллические подписи заметно длиннее английских, поэтому кнопка
                    // должна переноситься на 2 строки, а не выходить за границы панели.
                    // Индикатор активности резервирует своё место первым (через
                    // right_to_left), чтобы кнопка получила корректную оставшуюся ширину.
                    let nav_item =
                        |ui: &mut egui::Ui, selected: bool, label: &str, active: bool| -> bool {
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if active {
                                                circle_indicator_colored(
                                                    ui,
                                                    Color32::WHITE,
                                                    true,
                                                );
                                            }
                                            ui.with_layout(
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.add(
                                                        egui::Button::selectable(selected, label)
                                                            .wrap(),
                                                    )
                                                },
                                            )
                                            .inner
                                        },
                                    )
                                    .inner
                                })
                                .inner;
                            resp.clicked()
                        };
                    if nav_item(
                        ui,
                        self.active_section == Section::Rumble,
                        t.nav_rumble,
                        rumble_active,
                    ) {
                        self.active_section = Section::Rumble;
                    }
                    if nav_item(
                        ui,
                        self.active_section == Section::Taxi,
                        t.nav_taxi,
                        taxi_active,
                    ) {
                        self.active_section = Section::Taxi;
                    }
                    if nav_item(
                        ui,
                        self.active_section == Section::Engines,
                        t.nav_engines,
                        engines_active,
                    ) {
                        self.active_section = Section::Engines;
                    }
                    if nav_item(
                        ui,
                        self.active_section == Section::Gear,
                        t.nav_gear,
                        gear_active,
                    ) {
                        self.active_section = Section::Gear;
                    }
                    ui.separator();
                    if ui
                        .selectable_label(
                            self.active_section == Section::Telemetry,
                            t.nav_telemetry,
                        )
                        .clicked()
                    {
                        self.active_section = Section::Telemetry;
                    }

                    ui.separator();
                    let holding = self.hold.load(Ordering::Relaxed);
                    if !holding {
                        let stop_button =
                            egui::Button::new(RichText::new(t.btn_stop).color(Color32::WHITE))
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
                });

            // Компактный, всегда развёрнутый Live Monitor: раньше сворачивался в
            // узкую полоску безымянных точек по умолчанию — теперь фиксированные
            // 160/220px (EN/RU) с именем + реальным %, никакого режима "просто точки".
            let live_monitor_width = if self.lang == Lang::Ru { 220.0 } else { 160.0 };
            egui::Panel::right("live_monitor_panel")
                .resizable(false)
                .exact_size(live_monitor_width)
                .frame(egui::Frame::side_top_panel(ui.style()).fill(palette::BG_SIDEBAR))
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.label(RichText::new(t.heading_live_monitor).strong());
                    ui.separator();

                    let enabled_rows: Vec<_> =
                        rows.iter().filter(|(_, enabled, ..)| *enabled).collect();
                    let disabled_count = rows.len() - enabled_rows.len();

                    if enabled_rows.is_empty() {
                        ui.weak(t.lbl_no_active_effects);
                    }
                    for (name, enabled, active, pct) in enabled_rows {
                        ui.horizontal(|ui| {
                            let (dot_color, filled) = if *enabled && *active {
                                (palette::ACCENT_LIVE, true)
                            } else {
                                (palette::BORDER_ACTIVE, false)
                            };
                            // Значение резервирует место первым (right_to_left), затем
                            // точка + имя получают оставшуюся ширину и переносятся, если
                            // русская подпись не помещается в одну строку.
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let value_text = if *active {
                                    match pct {
                                        Some(p) => format!("{p:.0}%"),
                                        None => t.status_active.to_string(),
                                    }
                                } else {
                                    "—".to_string()
                                };
                                let color = if *active {
                                    palette::ACCENT_LIVE
                                } else {
                                    palette::TEXT_SECONDARY
                                };
                                ui.colored_label(color, RichText::new(value_text).small());
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        dot_indicator(ui, dot_color, filled, 8.0);
                                        ui.add(
                                            egui::Label::new(RichText::new(*name).small()).wrap(),
                                        );
                                    },
                                );
                            });
                        });
                    }

                    if disabled_count > 0 {
                        ui.add_space(4.0);
                        let label = i18n::lbl_disabled_count(self.lang, disabled_count);
                        if ui
                            .selectable_label(
                                self.monitor_show_disabled,
                                RichText::new(label).small().weak(),
                            )
                            .clicked()
                        {
                            self.monitor_show_disabled = !self.monitor_show_disabled;
                        }
                        if self.monitor_show_disabled {
                            for (name, _enabled, _active, _pct) in
                                rows.iter().filter(|(_, enabled, ..)| !*enabled)
                            {
                                ui.horizontal(|ui| {
                                    dot_indicator(ui, palette::TEXT_DISABLED, false, 8.0);
                                    ui.add_enabled(
                                        false,
                                        egui::Label::new(RichText::new(*name).small()).wrap(),
                                    );
                                });
                            }
                        }
                    }
                });

            egui::CentralPanel::default().show(ui, |ui| {
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
                                        if ui
                                            .button("🗑")
                                            .on_hover_text(t.hover_delete_profile)
                                            .clicked()
                                        {
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

                        if ui.button(t.btn_reset_defaults).clicked() {
                            // Только сбрасывает ЖИВОЙ конфиг — на диск ничего не пишет,
                            // как и любое другое изменение. Нажмите Save (дискета в
                            // верхней панели), чтобы зафиксировать сброс.
                            self.config.set(RumbleConfig::default());
                        }
                        ui.add_space(4.0);

                        let mut _changed = false;

                        let ground_active = self.effects.ground_active.load(Ordering::Relaxed);
                        let ground_thump_active =
                            self.effects.ground_thump_active.load(Ordering::Relaxed);
                        let taxi_start_crossed =
                            self.effects.taxi_start_crossed.load(Ordering::Relaxed);
                        let taxi_end_crossed =
                            self.effects.taxi_end_crossed.load(Ordering::Relaxed);

                        // Динамический порог Overspeed (AIRSPEED BARBER POLE),
                        // полученный от SimConnect для текущего самолёта — либо,
                        // если включён Override, значение, заданное вручную.
                        // None, если порог не определён (сим не подключён/выключен override без значения).
                        let overspeed_cfg_snapshot = self.config.get();
                        let overspeed_threshold_kn = if overspeed_cfg_snapshot
                            .overspeed_override_enabled
                        {
                            Some(overspeed_cfg_snapshot.overspeed_manual_kn).filter(|kn| *kn > 0.0)
                        } else {
                            self.last_vars
                                .lock()
                                .as_ref()
                                .map(|fv| fv.overspeed_barber_pole_kn)
                                .filter(|kn| *kn > 0.0)
                        };

                        // Живой снимок "физически подключено" — джойстик и РУД по
                        // отдельности. Используется ниже сразу для двух вещей:
                        //   • SPLIT (3 стойки → 3 мотора) требует ОБА устройства
                        //     (иначе некуда разводить стойки по разным рукам);
                        //   • Engine Start: если подключено только ОДНО устройство,
                        //     rumble-движок сливает в него весь эффект целиком (см.
                        //     cfg.joystick_hw_connected/throttle_hw_connected в
                        //     rumble.rs), а не теряет "чужую" половину.
                        // Пишем каждый кадр, независимо от того, какой раздел
                        // настроек сейчас открыт.
                        let joystick_hw_connected =
                            self.controller_connected.load(Ordering::Relaxed);
                        let throttle_hw_connected = self.throttle_connected.load(Ordering::Relaxed);
                        let split_touchdown_auto = joystick_hw_connected && throttle_hw_connected;

                        self.config.with_mut(|cfg| {
                            cfg.split_touchdown = split_touchdown_auto;
                            cfg.joystick_hw_connected = joystick_hw_connected;
                            cfg.throttle_hw_connected = throttle_hw_connected;
                            match self.active_section {
                                Section::Rumble => {
                                    ui.heading(t.nav_rumble);
                                    ui.add_space(4.0);
                                    ui.vertical(|ui| {
                                        // Overspeed
                                        {
                                            let mut overspeed_enabled = cfg.overspeed_enabled;
                                            let mut overspeed_override =
                                                cfg.overspeed_override_enabled;
                                            let mut overspeed_manual_kn = cfg.overspeed_manual_kn;
                                            let active = self
                                                .effects
                                                .overspeed_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                overspeed_enabled,
                                                active,
                                                |ui| {
                                                    UiState::overspeed_row(
                                                        ui,
                                                        &mut overspeed_enabled,
                                                        overspeed_threshold_kn,
                                                        active,
                                                        &mut _changed,
                                                        &mut overspeed_override,
                                                        &mut overspeed_manual_kn,
                                                        &mut cfg.device_targets.overspeed,
                                                        t,
                                                    );

                                                    ui.horizontal(|ui| {
                                                        let intensity_hint =
                                                            t.hover_overspeed_intensity;
                                                        let lbl = ui.label(
                                                            RichText::new(t.lbl_intensity).strong(),
                                                        );
                                                        lbl.on_hover_text(intensity_hint);
                                                        ui.spacing_mut().slider_width =
                                                            (ui.available_width() - 60.0).max(60.0);
                                                        let mut pct = (cfg.overspeed_intensity
                                                            / 255.0
                                                            * 100.0)
                                                            .clamp(0.0, 100.0);
                                                        let resp = ui.add(
                                                            egui::Slider::new(
                                                                &mut pct,
                                                                0.0..=100.0,
                                                            )
                                                            .trailing_fill(true)
                                                            .show_value(true)
                                                            .suffix("%")
                                                            .fixed_decimals(0),
                                                        );
                                                        resp.clone().on_hover_text(intensity_hint);
                                                        if resp.changed() {
                                                            cfg.overspeed_intensity =
                                                                pct / 100.0 * 255.0;
                                                            _changed = true;
                                                        }
                                                    });
                                                },
                                            );
                                            cfg.overspeed_enabled = overspeed_enabled;
                                            cfg.overspeed_override_enabled = overspeed_override;
                                            cfg.overspeed_manual_kn = overspeed_manual_kn;
                                        }

                                        // Stall
                                        {
                                            let mut stall_enabled = cfg.stall_enabled;
                                            let active =
                                                self.effects.stall_active.load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                stall_enabled,
                                                active,
                                                |ui| {
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.name_stall,
                                                        &mut cfg.stall_ceiling,
                                                        255.0,
                                                        &mut stall_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(t.hover_stall),
                                                        &mut cfg.device_targets.stall,
                                                        t,
                                                    );
                                                },
                                            );
                                            cfg.stall_enabled = stall_enabled;
                                        }

                                        // Spoilers (+ advanced threshold)
                                        {
                                            let mut spoilers_enabled = cfg.spoilers_enabled;
                                            let active = self
                                                .effects
                                                .spoilers_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                spoilers_enabled,
                                                active,
                                                |ui| {
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.name_spoilers,
                                                        &mut cfg.spoilers_intensity,
                                                        250.0,
                                                        &mut spoilers_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(t.hover_spoilers),
                                                        &mut cfg.device_targets.spoilers,
                                                        t,
                                                    );

                                                    ui.horizontal(|ui| {
                                                        let threshold_hint =
                                                            t.hover_spoilers_threshold;
                                                        let lbl = ui.label(
                                                            RichText::new(t.lbl_threshold_pct)
                                                                .strong(),
                                                        );
                                                        lbl.on_hover_text(threshold_hint);
                                                        ui.spacing_mut().slider_width =
                                                            (ui.available_width() - 60.0).max(60.0);
                                                        let mut threshold =
                                                            cfg.spoilers_threshold_pct as f32;
                                                        let resp = ui.add(
                                                            egui::Slider::new(
                                                                &mut threshold,
                                                                0.0..=100.0,
                                                            )
                                                            .trailing_fill(true)
                                                            .show_value(true),
                                                        );
                                                        resp.clone().on_hover_text(threshold_hint);
                                                        if resp.changed() {
                                                            cfg.spoilers_threshold_pct =
                                                                threshold as f64;
                                                            _changed = true;
                                                        }
                                                    });
                                                },
                                            );
                                            cfg.spoilers_enabled = spoilers_enabled;
                                        }

                                        // Flaps
                                        {
                                            let mut flaps_enabled = cfg.flaps_enabled;
                                            let active = self
                                                .effects
                                                .flaps_bump_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                flaps_enabled,
                                                active,
                                                |ui| {
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.name_flaps,
                                                        &mut cfg.flaps_peak,
                                                        255.0,
                                                        &mut flaps_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(t.hover_flaps),
                                                        &mut cfg.device_targets.flaps,
                                                        t,
                                                    );
                                                },
                                            );
                                            cfg.flaps_enabled = flaps_enabled;
                                        }

                                        // Bank / Turb (+ threshold)
                                        {
                                            let mut bank_enabled = cfg.bank_enabled;
                                            let active =
                                                self.effects.bank_active.load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(col, bank_enabled, active, |ui| {
                                                ui.horizontal(|ui| {
                                                    if ui.checkbox(&mut bank_enabled, "").changed()
                                                    {
                                                        _changed = true;
                                                    }
                                                    ui.label(
                                                        RichText::new(t.lbl_bank_turb).strong(),
                                                    );
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            effect_status_badge(
                                                                ui,
                                                                bank_enabled,
                                                                active,
                                                                t,
                                                            );
                                                            ui.add(
                                                                egui::Separator::default()
                                                                    .vertical()
                                                                    .spacing(10.0),
                                                            );
                                                            Self::device_target_toggle(
                                                                ui,
                                                                &mut cfg.device_targets.bank,
                                                                &mut _changed,
                                                                t,
                                                            );
                                                        },
                                                    );
                                                });

                                                ui.horizontal(|ui| {
                                                    let intensity_hint = t.hover_bank_intensity;
                                                    let lbl = ui.label(
                                                        RichText::new(t.lbl_intensity).strong(),
                                                    );
                                                    lbl.on_hover_text(intensity_hint);
                                                    ui.spacing_mut().slider_width =
                                                        (ui.available_width() - 60.0).max(60.0);
                                                    let mut pct = (cfg.bank_intensity / 200.0
                                                        * 100.0)
                                                        .clamp(0.0, 100.0);
                                                    let resp = ui.add(
                                                        egui::Slider::new(&mut pct, 0.0..=100.0)
                                                            .trailing_fill(true)
                                                            .show_value(true)
                                                            .suffix("%")
                                                            .fixed_decimals(0),
                                                    );
                                                    resp.clone().on_hover_text(intensity_hint);
                                                    if resp.changed() {
                                                        cfg.bank_intensity = pct / 100.0 * 200.0;
                                                        _changed = true;
                                                    }
                                                });

                                                ui.horizontal(|ui| {
                                                    let threshold_hint = t.hover_bank_threshold;
                                                    let lbl = ui.label(
                                                        RichText::new(t.lbl_threshold_deg).strong(),
                                                    );
                                                    lbl.on_hover_text(threshold_hint);
                                                    ui.spacing_mut().slider_width =
                                                        (ui.available_width() - 60.0).max(60.0);
                                                    let mut threshold = cfg.bank_threshold_deg;
                                                    let resp = ui.add(
                                                        egui::Slider::new(
                                                            &mut threshold,
                                                            0.0..=90.0,
                                                        )
                                                        .trailing_fill(true)
                                                        .show_value(true),
                                                    );
                                                    resp.clone().on_hover_text(threshold_hint);
                                                    if resp.changed() {
                                                        cfg.bank_threshold_deg = threshold;
                                                        _changed = true;
                                                    }
                                                });
                                            });
                                            cfg.bank_enabled = bank_enabled;
                                        }
                                    });
                                }

                                Section::Taxi => {
                                    ui.heading(t.nav_taxi);
                                    ui.add_space(4.0);
                                    ui.vertical(|ui| {
                                        // Taxi Thump Bounds (Start + End + advanced Period curve)
                                        {
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                true,
                                                taxi_start_crossed || taxi_end_crossed,
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(t.heading_taxi_thump)
                                                            .strong(),
                                                    );

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
                                                        t,
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
                                                        t,
                                                    );
                                                    cfg.taxi_end_enabled = end_enabled;

                                                    if end <= start + 0.5 {
                                                        start = (end - 0.5).max(0.0);
                                                    }

                                                    cfg.taxi_start_kn = start.clamp(0.0, 249.5);
                                                    cfg.taxi_end_kn =
                                                        end.clamp(cfg.taxi_start_kn + 0.5, 250.0); // Изменили clamp до 250

                                                    // Коэффициент кривизны нарастания частоты ударов.
                                                    // >1.0 = плавнее на старте (пауза между ударами сокращается медленнее),
                                                    // <1.0 = резче, чем чистая физика t=S/V; 1.0 = без коррекции.
                                                    ui.horizontal(|ui| {
                                                        let lbl = ui.label(
                                                            RichText::new(t.lbl_period_curve)
                                                                .strong(),
                                                        );
                                                        lbl.on_hover_text(
                                                            t.hover_period_curve_full,
                                                        );
                                                        ui.spacing_mut().slider_width =
                                                            (ui.available_width() - 60.0).max(60.0);
                                                        let mut curve = cfg.thump_period_curve;
                                                        let resp = ui.add(
                                                            egui::Slider::new(
                                                                &mut curve,
                                                                0.3..=5.0,
                                                            )
                                                            .trailing_fill(true)
                                                            .show_value(true),
                                                        );
                                                        resp.clone().on_hover_text(
                                                            t.hover_period_curve_short,
                                                        );
                                                        if resp.changed() {
                                                            cfg.thump_period_curve = curve;
                                                            _changed = true;
                                                        }
                                                    });
                                                },
                                            );
                                        }

                                        // Ground Roll
                                        {
                                            let mut ground_enabled = cfg.ground_enabled;
                                            let active = ground_active || ground_thump_active;
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                ground_enabled,
                                                active,
                                                |ui| {
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.name_ground_roll,
                                                        &mut cfg.ground_roll,
                                                        50.0,
                                                        &mut ground_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(t.hover_ground_roll),
                                                        &mut cfg.device_targets.ground_roll,
                                                        t,
                                                    );
                                                },
                                            );
                                            cfg.ground_enabled = ground_enabled;
                                        }
                                    });
                                }

                                Section::Engines => {
                                    ui.heading(t.nav_engines);
                                    ui.add_space(4.0);
                                    ui.vertical(|ui| {
                                        // Engine Start / Ignition (+ advanced N2 idle, 4-Eng mode, swap hands)
                                        {
                                            let mut engine_start_enabled = cfg.enable_engine_start;
                                            let active = self
                                                .effects
                                                .engine_start_active
                                                .load(Ordering::Relaxed);
                                            let engine_start_hint = t.hover_engine_start;
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                engine_start_enabled,
                                                active,
                                                |ui| {
                                                    ui.horizontal(|ui| {
                                                        let cb = ui.checkbox(
                                                            &mut engine_start_enabled,
                                                            "",
                                                        );
                                                        if cb.changed() {
                                                            _changed = true;
                                                        }

                                                        let name_label = ui.label(
                                                            RichText::new(t.name_engine_start)
                                                                .strong(),
                                                        );
                                                        name_label.on_hover_text(engine_start_hint);

                                                        ui.with_layout(
                                                            egui::Layout::right_to_left(
                                                                egui::Align::Center,
                                                            ),
                                                            |ui| {
                                                                effect_status_badge(
                                                                    ui,
                                                                    engine_start_enabled,
                                                                    active,
                                                                    t,
                                                                );
                                                            },
                                                        );
                                                    });

                                                    ui.add_enabled_ui(engine_start_enabled, |ui| {
                                                        ui.horizontal(|ui| {
                                                            ui.spacing_mut().slider_width =
                                                                (ui.available_width() - 65.0)
                                                                    .max(60.0);
                                                            let mut pct =
                                                                (cfg.engine_start_strength / 255.0
                                                                    * 100.0)
                                                                    .clamp(0.0, 100.0);
                                                            let slider = egui::Slider::new(
                                                                &mut pct,
                                                                0.0..=100.0,
                                                            )
                                                            .trailing_fill(true)
                                                            .show_value(true)
                                                            .suffix("%")
                                                            .fixed_decimals(0);
                                                            let resp = ui.add(slider);
                                                            resp.clone()
                                                                .on_hover_text(engine_start_hint);
                                                            if resp.changed() {
                                                                cfg.engine_start_strength =
                                                                    pct / 100.0 * 255.0;
                                                                _changed = true;
                                                            }
                                                        });
                                                    });
                                                    cfg.enable_engine_start = engine_start_enabled;

                                                    ui.horizontal(|ui| {
                                                        let n2_hint = t.hover_n2_idle;
                                                        let n2_label = ui.label(
                                                            RichText::new(t.lbl_n2_idle).strong(),
                                                        );
                                                        n2_label.on_hover_text(n2_hint);
                                                        let n2_resp = ui.add(
                                                            egui::DragValue::new(
                                                                &mut cfg.engine_idle_n2,
                                                            )
                                                            .speed(1.0)
                                                            .range(10.0..=100.0),
                                                        );
                                                        n2_resp.clone().on_hover_text(n2_hint);
                                                        if n2_resp.changed() {
                                                            _changed = true;
                                                        }
                                                    });
                                                    if ui
                                                        .checkbox(
                                                            &mut cfg.four_engine_mode,
                                                            t.chk_four_eng_mode,
                                                        )
                                                        .on_hover_text(t.hover_four_eng_mode)
                                                        .changed()
                                                    {
                                                        _changed = true;
                                                    }
                                                    if ui
                                                        .checkbox(
                                                            &mut cfg.swap_hand_layout,
                                                            t.chk_swap_hands,
                                                        )
                                                        .on_hover_text(t.hover_swap_hands)
                                                        .changed()
                                                    {
                                                        _changed = true;
                                                    }
                                                },
                                            );
                                        }
                                    });
                                }

                                Section::Gear => {
                                    ui.heading(t.nav_gear);
                                    ui.add_space(4.0);
                                    ui.label(RichText::new(t.heading_gear_comp).weak());
                                    ui.add_space(4.0);

                                    let mut gear_comp_enabled = cfg.gear_comp_enabled;
                                    ui.horizontal(|ui| {
                                        if ui
                                            .checkbox(&mut gear_comp_enabled, t.chk_enabled)
                                            .changed()
                                        {
                                            _changed = true;
                                        }
                                    });
                                    cfg.gear_comp_enabled = gear_comp_enabled;
                                    ui.add_space(4.0);

                                    let headroom_hint = t.hover_headroom;

                                    ui.vertical(|ui| {
                                        // Left / Nose / Right Peak — активны только пока Gear Strut Compression включён.
                                        {
                                            let mut left_enabled = cfg.gear_comp_left_enabled;
                                            let active = self
                                                .effects
                                                .gear_comp_left_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            col.add_enabled_ui(gear_comp_enabled, |ui| {
                                                UiState::effect_card(
                                                    ui,
                                                    left_enabled,
                                                    active,
                                                    |ui| {
                                                        UiState::effect_row_percent_hinted(
                                                            ui,
                                                            t.name_left_peak,
                                                            &mut cfg.gear_comp_left_peak,
                                                            55.0,
                                                            &mut left_enabled,
                                                            active,
                                                            &mut _changed,
                                                            Some(headroom_hint),
                                                            &mut cfg.device_targets.gear_comp_left,
                                                            t,
                                                        );
                                                    },
                                                );
                                            });
                                            cfg.gear_comp_left_enabled = left_enabled;
                                        }

                                        {
                                            let mut nose_enabled = cfg.gear_comp_nose_enabled;
                                            let active = self
                                                .effects
                                                .gear_comp_nose_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            col.add_enabled_ui(gear_comp_enabled, |ui| {
                                                UiState::effect_card(
                                                    ui,
                                                    nose_enabled,
                                                    active,
                                                    |ui| {
                                                        UiState::effect_row_percent_hinted(
                                                            ui,
                                                            t.name_nose_peak,
                                                            &mut cfg.gear_comp_nose_peak,
                                                            55.0,
                                                            &mut nose_enabled,
                                                            active,
                                                            &mut _changed,
                                                            Some(headroom_hint),
                                                            &mut cfg.device_targets.gear_comp_nose,
                                                            t,
                                                        );
                                                    },
                                                );
                                            });
                                            cfg.gear_comp_nose_enabled = nose_enabled;
                                        }

                                        {
                                            let mut right_enabled = cfg.gear_comp_right_enabled;
                                            let active = self
                                                .effects
                                                .gear_comp_right_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            col.add_enabled_ui(gear_comp_enabled, |ui| {
                                                UiState::effect_card(
                                                    ui,
                                                    right_enabled,
                                                    active,
                                                    |ui| {
                                                        UiState::effect_row_percent_hinted(
                                                            ui,
                                                            t.name_right_peak,
                                                            &mut cfg.gear_comp_right_peak,
                                                            55.0,
                                                            &mut right_enabled,
                                                            active,
                                                            &mut _changed,
                                                            Some(headroom_hint),
                                                            &mut cfg.device_targets.gear_comp_right,
                                                            t,
                                                        );
                                                    },
                                                );
                                            });
                                            cfg.gear_comp_right_enabled = right_enabled;
                                        }

                                        // Gear Transit & Doors
                                        {
                                            let mut gear_transit_enabled = cfg.gear_transit_enabled;
                                            let active = self
                                                .effects
                                                .gear_transit_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                gear_transit_enabled,
                                                active,
                                                |ui| {
                                                    ui.horizontal(|ui| {
                                                        if ui
                                                            .checkbox(&mut gear_transit_enabled, "")
                                                            .changed()
                                                        {
                                                            _changed = true;
                                                        }
                                                        ui.label(
                                                            RichText::new(t.lbl_gear_transit)
                                                                .strong(),
                                                        );
                                                        ui.with_layout(
                                                            egui::Layout::right_to_left(
                                                                egui::Align::Center,
                                                            ),
                                                            |ui| {
                                                                effect_status_badge(
                                                                    ui,
                                                                    gear_transit_enabled,
                                                                    active,
                                                                    t,
                                                                );
                                                                ui.add(
                                                                    egui::Separator::default()
                                                                        .vertical()
                                                                        .spacing(10.0),
                                                                );
                                                                Self::device_target_toggle(
                                                                    ui,
                                                                    &mut cfg
                                                                        .device_targets
                                                                        .gear_transit,
                                                                    &mut _changed,
                                                                    t,
                                                                );
                                                            },
                                                        );
                                                    });
                                                },
                                            );
                                            cfg.gear_transit_enabled = gear_transit_enabled;
                                        }
                                    });
                                }

                                Section::Telemetry => {}
                            }

                            if _changed {
                                // Конфиг уже обновлен через with_mut
                            }
                        });

                        ui.add_space(8.0);
                        ui.separator();

                        if self.active_section == Section::Telemetry {
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
                                                    ui.label(format!(
                                                        "{:.1}",
                                                        v.overspeed_barber_pole_kn
                                                    ));
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
                                                ui.label(if v.gear_handle > 0.5 {
                                                    t.val_down
                                                } else {
                                                    t.val_up
                                                });
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

                                                // Временная отладочная строка: сырой Fenix
                                                // L:A320_Gear_Nose (0..1000), пока не подключим
                                                // эффект уборки/выпуска шасси к этому борту.
                                                ui.label("F_Gear");
                                                ui.label(format!("{:.0}", v.fenix_gear_nose_raw));
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
                                                let combustion_label =
                                                    |ui: &mut egui::Ui, active: bool| {
                                                        if active {
                                                            ui.colored_label(
                                                                Color32::from_rgb(30, 180, 90),
                                                                t.val_on,
                                                            );
                                                        } else {
                                                            ui.colored_label(
                                                                Color32::from_gray(140),
                                                                t.val_off,
                                                            );
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
            egui::CentralPanel::default().show(ui, |ui| {
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
