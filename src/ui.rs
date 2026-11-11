use egui::{Color32, RichText, Vec2};
#[cfg(debug_assertions)]
use egui_extras::{Column, TableBuilder};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use windows::Win32::Foundation::HWND;

use crate::{
    ActiveGame, ConfigShared, EffectDeviceTarget, EffectsShared, FlightVars, GameOverride, HidCmd,
    LogBuffer, RumbleConfig, SimStatus, UiCmd,
    aircraft_profiles::{self, AircraftProfile, AircraftProfiles},
    custom_fx::{
        model::{CustomEffect, new_effect},
        overrides::{self, BuiltinEffect, BuiltinMask},
        preview_player::PreviewPlayer,
        sources::{SourceId, TelemetryFrame},
        store::CustomFxShared,
    },
    game_state::PreviewLock,
    i18n::{self, Lang, Strings},
    profiles::ProfileState,
    tray, updater,
    wt_link::vars::WtVars,
};

// Редактор пользовательских эффектов ("Редактор эффектов") — отдельный подмодуль,
// а не ещё несколько сотен строк в этом и без того большом файле (см.
// doc-комментарий effects_editor.rs).
mod effects_editor;

/// Цветовая палитра карточек эффектов и Live Monitor. Раньше цвета были
/// разбросаны литералами (`Color32::from_rgb(...)`) по десятку мест — свели
/// в одно место, чтобы контраст карточка/фон и роли акцентов (primary vs
/// live vs warning) были согласованы по всему приложению.
///
/// Тона взяты из логотипа: тёмно-синий фон и мятный цвет полярного сияния.
/// Шаги лестницы поверхностей по контрасту оставлены такими же, как были
/// (см. комментарий к фонам ниже), сменился только оттенок.
pub(crate) mod palette {
    use egui::Color32;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Цвета и размеры, которые зависят от режима интерфейса.
    pub struct Palette {
        pub high_contrast: bool,
        /// Масштаб всего интерфейса.
        pub zoom: f32,
        /// Толщина рамок панелей и контролов.
        pub stroke_width: f32,
        /// Ширина ручки слайдера к её высоте, см. `apply` и `live_slider`.
        pub handle_aspect: f32,
        /// Рельс слайдера.
        pub rail: Color32,
        /// Рамка фокуса с клавиатуры.
        pub focus: Color32,
        /// Текст на кнопке Stop.
        pub on_danger: Color32,
        pub bg_app: Color32,
        pub bg_sidebar: Color32,
        pub bg_card: Color32,
        pub bg_card_disabled: Color32,
        pub surface_control: Color32,
        pub surface_control_hover: Color32,
        pub surface_control_active: Color32,
        pub border_default: Color32,
        pub border_active: Color32,
        pub accent_primary: Color32,
        pub accent_live: Color32,
        pub status_bad: Color32,
        pub status_warn: Color32,
        pub status_ok: Color32,
        pub status_attention: Color32,
        pub danger: Color32,
        pub selected_bg: Color32,
        pub selected_fg: Color32,
        pub device_on_fg: Color32,
        pub text_primary: Color32,
        pub text_secondary: Color32,
        pub text_disabled: Color32,
    }

    static HIGH_CONTRAST: AtomicBool = AtomicBool::new(false);

    /// Палитра текущего режима. Режим хранится глобально, как и язык в `i18n`:
    /// цвета нужны в десятках мест, и тащить их параметром незачем.
    pub fn get() -> &'static Palette {
        if HIGH_CONTRAST.load(Ordering::Relaxed) {
            &MONO
        } else {
            &DARK
        }
    }

    /// Чёрно-белый режим для слабовидящих: только чёрный, белый и серый, поэтому
    /// состояния в нём нельзя кодировать цветом. Везде, где обычная тема говорит
    /// цветом, здесь работают форма, толщина рамки, значок или слово.
    pub static MONO: Palette = Palette {
        high_contrast: true,
        zoom: 1.25,
        stroke_width: 2.0,
        handle_aspect: 0.55,
        rail: Color32::from_rgb(0x70, 0x70, 0x70),
        focus: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        on_danger: Color32::from_rgb(0x00, 0x00, 0x00),
        bg_app: Color32::from_rgb(0x00, 0x00, 0x00),
        bg_sidebar: Color32::from_rgb(0x00, 0x00, 0x00),
        bg_card: Color32::from_rgb(0x00, 0x00, 0x00),
        bg_card_disabled: Color32::from_rgb(0x00, 0x00, 0x00),
        surface_control: Color32::from_rgb(0x00, 0x00, 0x00),
        surface_control_hover: Color32::from_rgb(0x2B, 0x2B, 0x2B),
        surface_control_active: Color32::from_rgb(0x4A, 0x4A, 0x4A),
        border_default: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        border_active: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        accent_primary: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        accent_live: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        status_bad: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        status_warn: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        status_ok: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        status_attention: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        danger: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        selected_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        selected_fg: Color32::from_rgb(0x00, 0x00, 0x00),
        device_on_fg: Color32::from_rgb(0x00, 0x00, 0x00),
        text_primary: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        text_secondary: Color32::from_rgb(0xE6, 0xE6, 0xE6),
        text_disabled: Color32::from_rgb(0xBF, 0xBF, 0xBF),
    };

    pub static DARK: Palette = Palette {
        high_contrast: false,
        zoom: 1.0,
        stroke_width: 1.0,
        handle_aspect: 0.32,
        rail: Color32::from_rgb(0x26, 0x37, 0x53),
        focus: Color32::from_rgb(0xB6, 0xF7, 0xDC),
        on_danger: Color32::WHITE,

        // Фоны. Раньше BG_APP был почти чистый чёрный (#0B0E14) — на неоткалиброванных
        // мониторах он сливался с текстом egui по умолчанию (gray(140)). Подняли всю
        // лестницу поверхностей, чтобы карточка/панель/фон различались и на плохом
        // экране, и чтобы у текста была нормальная подложка.
        bg_app: Color32::from_rgb(0x0E, 0x17, 0x26),
        bg_sidebar: Color32::from_rgb(0x14, 0x1F, 0x32),
        bg_card: Color32::from_rgb(0x1B, 0x29, 0x40),
        bg_card_disabled: Color32::from_rgb(0x15, 0x20, 0x32),

        // Подложка кнопок/полей в покое. Ключевой момент доступности: без неё
        // невыбранные пункты навигации не имели вообще никакого фона и «появлялись»
        // только под курсором (жалоба тестировщика).
        surface_control: Color32::from_rgb(0x26, 0x37, 0x53),
        surface_control_hover: Color32::from_rgb(0x32, 0x46, 0x6A),
        surface_control_active: Color32::from_rgb(0x41, 0x58, 0x80),

        border_default: Color32::from_rgb(0x33, 0x47, 0x6A),
        border_active: Color32::from_rgb(0x5B, 0x73, 0x96),

        accent_primary: Color32::from_rgb(0x8F, 0xB4, 0xFF),
        accent_live: Color32::from_rgb(0x5E, 0xEB, 0xB0),

        // Статусы в верхней панели. Красный был #C83C3C — всего ~3.6:1 на BG_APP,
        // то есть самая важная строка («не подключено») читалась хуже остальных.
        status_bad: Color32::from_rgb(0xFF, 0x7A, 0x6E),
        status_warn: Color32::from_rgb(0xF5, 0xB8, 0x4A),
        status_ok: Color32::from_rgb(0x3E, 0xC9, 0x74),
        status_attention: Color32::from_rgb(0xF0, 0x91, 0x3A),
        // Заливка кнопки Stop. Белый текст на ней даёт 6.5:1.
        danger: Color32::from_rgb(0xB3, 0x26, 0x1E),

        // Подложка выбранного пункта — непрозрачная тёмно-бирюзовая, а не
        // полупрозрачный акцент поверх фона: раньше выбранный пункт был синим
        // текстом на синей заливке (~3:1) и читался хуже невыбранного.
        selected_bg: Color32::from_rgb(0x1C, 0x55, 0x52),
        selected_fg: Color32::from_rgb(0xB6, 0xF7, 0xDC),
        // Тинт иконки устройства во включённом состоянии: там под ней светлая
        // заливка (см. `device_on_bg`), поэтому сама иконка должна быть тёмной.
        device_on_fg: Color32::from_rgb(0x0E, 0x17, 0x26),

        // Базовый цвет текста. egui::Visuals::dark() ставит gray(140) — формально
        // это ~5.7:1 на нашем фоне, но на практике тонкий шрифт такого тона
        // читается как «серо-чёрный по чёрному». Держим ~15:1.
        text_primary: Color32::from_rgb(0xE9, 0xF1, 0xF8),
        text_secondary: Color32::from_rgb(0xB4, 0xC3, 0xD4),
        text_disabled: Color32::from_rgb(0x93, 0xA5, 0xBB),
    };

    /// Единая тема приложения поверх egui::Visuals::dark() — применяется при
    /// старте и при переключении режима, а не каждый кадр, т.к. это глобальное
    /// состояние Context, а не что-то, что нужно пересчитывать в render-цикле.
    pub fn apply(ctx: &egui::Context, high_contrast: bool) {
        HIGH_CONTRAST.store(high_contrast, Ordering::Relaxed);
        let p = get();
        let mut visuals = egui::Visuals::dark();
        // panel_fill — общий фон по умолчанию для side/top/central-панелей
        // (см. Frame::side_top_panel / Frame::central_panel в egui). Ставим
        // его в p.bg_app, а sidebar и Live Monitor явно перекрывают своим
        // Frame::fill(p.bg_sidebar) в местах создания панелей — иначе все
        // панели красились бы в один и тот же цвет и контраст пропадал бы.
        visuals.panel_fill = p.bg_app;
        visuals.window_fill = p.bg_app;
        visuals.extreme_bg_color = p.bg_app;
        visuals.faint_bg_color = p.bg_sidebar;
        visuals.selection.bg_fill = p.selected_bg;
        visuals.selection.stroke.color = p.selected_fg;
        visuals.hyperlink_color = p.accent_primary;

        // Текст. Все три «тона» задаём явно, иначе egui считает weak-текст как
        // base * 0.6 alpha (≈2.4:1 на тёмном фоне — нечитаемо).
        visuals.weak_text_color = Some(p.text_secondary);
        // disabled_alpha перемножается с override_text_color карточки и с
        // add_enabled_ui(false) у слайдеров: 0.5 давало ~1.5:1 у выключенного
        // эффекта. 0.8 оставляет выключенное заметно тусклее включённого, но
        // всё ещё читаемым.
        visuals.disabled_alpha = 0.8;

        let w = &mut visuals.widgets;
        // noninteractive — обычные ui.label(); bg_stroke здесь же красит
        // ui.separator() и линии отступов.
        w.noninteractive.fg_stroke = egui::Stroke::new(p.stroke_width, p.text_primary);
        w.noninteractive.bg_stroke = egui::Stroke::new(p.stroke_width, p.border_default);
        w.noninteractive.weak_bg_fill = p.bg_app;
        w.noninteractive.bg_fill = p.bg_app;

        w.inactive.weak_bg_fill = p.surface_control;
        // bg_fill у inactive красит рельс слайдера и его ручку, а кнопки берут
        // weak_bg_fill. В чёрно-белом режиме кнопки чёрные, а рельс обязан быть виден.
        w.inactive.bg_fill = p.rail;
        w.inactive.bg_stroke = egui::Stroke::new(p.stroke_width, p.border_default);
        w.inactive.fg_stroke = egui::Stroke::new(p.stroke_width, p.text_primary);

        w.hovered.weak_bg_fill = p.surface_control_hover;
        w.hovered.bg_fill = p.surface_control_hover;
        w.hovered.bg_stroke = egui::Stroke::new(p.stroke_width, p.border_active);
        w.hovered.fg_stroke = egui::Stroke::new(1.5, Color32::WHITE);

        w.active.weak_bg_fill = p.surface_control_active;
        w.active.bg_fill = p.surface_control_active;
        w.active.bg_stroke = egui::Stroke::new(p.stroke_width, p.accent_primary);
        w.active.fg_stroke = egui::Stroke::new(2.0, Color32::WHITE);

        w.open.weak_bg_fill = p.surface_control;
        w.open.bg_fill = p.surface_control;
        w.open.bg_stroke = egui::Stroke::new(p.stroke_width, p.border_default);
        w.open.fg_stroke = egui::Stroke::new(p.stroke_width, p.text_primary);

        for state in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            state.corner_radius = egui::CornerRadius::same(6);
        }
        visuals.window_corner_radius = egui::CornerRadius::same(10);
        visuals.menu_corner_radius = egui::CornerRadius::same(8);
        // Узкая ручка, как колпачок фейдера. Форма важна не только для вида:
        // live_slider считает по ней, где на рельсе находится значение.
        visuals.handle_shape = egui::style::HandleShape::Rect {
            aspect_ratio: p.handle_aspect,
        };

        if high_contrast {
            visuals.window_stroke = egui::Stroke::new(p.stroke_width, p.border_default);
            visuals.window_shadow = egui::Shadow::NONE;
            visuals.popup_shadow = egui::Shadow::NONE;
            visuals.disabled_alpha = 1.0;
        }

        // Тема всегда тёмная, независимо от Windows. set_visuals пишет только в
        // стиль текущей темы, а при светлой теме Windows egui переключался на
        // нетронутый светлый стиль: тёмный текст на тёмных карточках.
        ctx.set_theme(egui::Theme::Dark);
        ctx.set_visuals_of(egui::Theme::Dark, visuals);
        install_fonts(ctx, high_contrast);
        ctx.set_zoom_factor(p.zoom);
        ctx.all_styles_mut(|style| {
            use egui::{FontId, TextStyle};
            style.spacing.button_padding = egui::vec2(10.0, 5.0);
            style.spacing.interact_size.y = 24.0;
            style.visuals.slider_trailing_fill = true;
            style.spacing.slider_rail_height = 6.0;
            style.text_styles = [
                (TextStyle::Small, FontId::proportional(11.0)),
                (TextStyle::Body, FontId::proportional(13.5)),
                (TextStyle::Button, FontId::proportional(13.5)),
                (TextStyle::Monospace, FontId::monospace(12.5)),
                (TextStyle::Heading, FontId::new(20.0, semibold())),
            ]
            .into();
        });
    }

    pub fn medium() -> egui::FontFamily {
        egui::FontFamily::Name("plex-medium".into())
    }

    pub fn semibold() -> egui::FontFamily {
        egui::FontFamily::Name("plex-semibold".into())
    }

    /// IBM Plex выбран из-за кириллицы: интерфейс двуязычный, а гарнитуры
    /// вроде Space Grotesk русских букв не содержат. Шрифты egui остаются в
    /// конце списка как запасные: в них живут значки вроде стрелок на Load/Save
    /// и знака на Stop, которых в Plex нет.
    fn install_fonts(ctx: &egui::Context, high_contrast: bool) {
        use egui::{FontData, FontDefinitions, FontFamily};
        use std::sync::Arc;

        let mut fonts = FontDefinitions::default();
        let files: [(&str, &'static [u8]); 4] = [
            (
                "plex-regular",
                include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
            ),
            (
                "plex-medium",
                include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"),
            ),
            (
                "plex-semibold",
                include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
            ),
            (
                "plex-mono",
                include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
            ),
        ];
        for (name, bytes) in files {
            fonts
                .font_data
                .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
        }

        let fallback = fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default();
        for name in ["plex-medium", "plex-semibold"] {
            let mut list = vec![name.to_owned()];
            list.extend(fallback.iter().cloned());
            fonts.families.insert(FontFamily::Name(name.into()), list);
        }
        // В чёрно-белом режиме основной текст плотнее: тонкие штрихи при слабом
        // зрении расплываются первыми.
        let body = if high_contrast {
            "plex-medium"
        } else {
            "plex-regular"
        };
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, body.to_owned());
        }
        if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
            list.insert(0, "plex-mono".to_owned());
        }
        ctx.set_fonts(fonts);
    }
}

/// Точка входа для main.rs: применяет тему один раз при старте
/// приложения (см. `eframe::run_native`'s creation callback), до того как
/// нарисован первый кадр.
pub fn apply_theme(ctx: &egui::Context, high_contrast: bool) {
    palette::apply(ctx, high_contrast);
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
    // War Thunder (этап 1) — единственная секция эффектов, показывается вместо
    // Rumble/Taxi/Engines/Gear, пока active_game == ActiveGame::Wt (см.
    // nav_panel и dispatch ниже). Telemetry остаётся общей секцией для обоих
    // режимов, но её содержимое переключается на WT-поля.
    Wt,
    // Конструктор пользовательских эффектов ("Редактор эффектов") — как и
    // Telemetry, общая секция для всех игр (в отличие от Rumble/Taxi/
    // Engines/Gear/Wt, которые переключаются по active_game), поэтому пункт
    // навигации виден всегда, см. nav_panel ниже.
    Effects,
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

/// Переключатель вкл/выкл для строки эффекта.
fn toggle_switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(36.0, 20.0), egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    let state = *on;
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, state, ""));
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let track = palette::get()
            .surface_control_active
            .lerp_to_gamma(palette::get().accent_live, how_on);
        let knob = palette::get()
            .text_primary
            .lerp_to_gamma(palette::get().bg_app, how_on);
        let radius = rect.height() / 2.0;
        let painter = ui.painter();
        painter.rect_filled(rect, radius, track);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        painter.circle_filled(egui::pos2(x, rect.center().y), radius - 3.0, knob);
        if palette::get().high_contrast {
            painter.rect_stroke(
                rect,
                radius,
                egui::Stroke::new(1.5, palette::get().text_primary),
                egui::StrokeKind::Inside,
            );
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect.expand(2.0),
                radius + 2.0,
                egui::Stroke::new(2.0, palette::get().focus),
                egui::StrokeKind::Outside,
            );
        }
    }
    response
}

/// Уровень 0..1 для индикации срабатывания. Флаг `active` моргает на каждый тик
/// мотора, и прямая подсветка по нему уже отвергалась как навязчивая (см.
/// `effect_card`). Поэтому после последнего срабатывания уровень держится
/// `HOLD` секунд и только потом плавно гаснет: серия импульсов выглядит как
/// одно ровное «работает».
fn live_level(ui: &egui::Ui, key: egui::Id, active: bool) -> f32 {
    const HOLD: f64 = 0.25;
    const FADE: f64 = 0.45;
    let now = ui.input(|i| i.time);
    let last = ui.data_mut(|d| {
        let slot = d.get_temp_mut_or(key, f64::NEG_INFINITY);
        if active {
            *slot = now;
        }
        *slot
    });
    (1.0 - ((now - last - HOLD) / FADE).clamp(0.0, 1.0)) as f32
}

/// Слайдер силы эффекта с индикацией срабатывания прямо на рельсе. Яркая часть
/// показывает факт срабатывания, а не точную амплитуду: амплитуда есть только
/// у моторов целиком, её рисуют осциллограммы в Live Monitor.
fn live_slider(ui: &mut egui::Ui, pct: &mut f32, level: f32) -> egui::Response {
    let response = ui.add(
        egui::Slider::new(pct, 0.0..=100.0)
            .trailing_fill(true)
            .show_value(false),
    );
    if level > 0.0 && ui.is_enabled() {
        let rect = response.rect;
        let inset = rect.height() / 2.5 * palette::get().handle_aspect;
        let x = egui::lerp((rect.left() + inset)..=(rect.right() - inset), *pct / 100.0);
        let right = egui::lerp(rect.left()..=x, level) - inset;
        if right > rect.left() {
            let half = ui.spacing().slider_rail_height / 2.0;
            let bar = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.center().y - half),
                egui::pos2(right, rect.center().y + half),
            );
            ui.painter()
                .rect_filled(bar, half, palette::get().accent_live);
        }
    }
    response
}

/// Отступ строк параметров под названием эффекта: переключатель плюс зазор.
const PARAM_INDENT: f32 = 44.0;

/// Осциллограмма одного мотора по реальным значениям с `hid_worker`. Заливка
/// ступенчатая, потому что мотор и правда получает ступени, а не гладкую кривую.
fn motor_trace(ui: &mut egui::Ui, label: &str, color: Color32, samples: &VecDeque<u8>) {
    let last = samples.back().copied().unwrap_or(0);
    let pct = f32::from(last) / 255.0 * 100.0;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label)
                .size(12.0)
                .color(palette::get().text_secondary),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{pct:.0}%")).monospace());
        });
    });
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 4u8, palette::get().bg_app);
    if palette::get().high_contrast {
        painter.rect_stroke(
            rect,
            4u8,
            egui::Stroke::new(1.0, palette::get().border_default),
            egui::StrokeKind::Inside,
        );
    }

    let slots = crate::types::MotorHistory::CAPACITY.max(2);
    let dx = rect.width() / (slots - 1) as f32;
    let start = slots - samples.len().min(slots);
    let y_of = |v: u8| rect.bottom() - 1.0 - f32::from(v) / 255.0 * (rect.height() - 6.0);
    let top = color.gamma_multiply(0.7);
    let bottom = color.gamma_multiply(0.05);

    let mut mesh = egui::Mesh::default();
    let mut line = Vec::with_capacity(samples.len() * 2);
    for (i, v) in samples.iter().enumerate() {
        let x0 = rect.left() + (start + i) as f32 * dx;
        let x1 = (x0 + dx).min(rect.right());
        let y = y_of(*v);
        line.push(egui::pos2(x0, y));
        line.push(egui::pos2(x1, y));
        if *v == 0 {
            continue;
        }
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(x0, y), top);
        mesh.colored_vertex(egui::pos2(x1, y), top);
        mesh.colored_vertex(egui::pos2(x1, rect.bottom()), bottom);
        mesh.colored_vertex(egui::pos2(x0, rect.bottom()), bottom);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(egui::Shape::mesh(mesh));
    if line.len() >= 2 {
        painter.add(egui::Shape::line(line, egui::Stroke::new(1.4, color)));
    }
    ui.add_space(4.0);
}

/// Значок пункта навигации. Рисуется линиями, а не картинкой: цвет следует за
/// состоянием пункта, а чёткость не зависит от масштаба экрана.
#[derive(Clone, Copy)]
enum NavIcon {
    Aero,
    Taxi,
    Engine,
    Gear,
    Weapon,
    Telemetry,
    Editor,
}

fn paint_nav_icon(painter: &egui::Painter, c: egui::Pos2, icon: NavIcon, color: Color32) {
    let stroke = egui::Stroke::new(1.5, color);
    let p = |x: f32, y: f32| c + Vec2::new(x, y);
    let seg = |a: egui::Pos2, b: egui::Pos2| painter.line_segment([a, b], stroke);
    match icon {
        NavIcon::Aero => {
            for (y, len) in [(-5.0, 7.0), (0.0, 10.0), (5.0, 4.0)] {
                seg(p(-8.0, y), p(-8.0 + len, y));
                painter.circle_stroke(p(-6.0 + len, y - 2.0), 2.0, stroke);
            }
        }
        NavIcon::Taxi => {
            seg(p(-3.0, -8.0), p(-7.0, 8.0));
            seg(p(3.0, -8.0), p(7.0, 8.0));
            for (a, b) in [(-7.0, -4.0), (-1.5, 1.5), (4.0, 7.0)] {
                seg(p(0.0, a), p(0.0, b));
            }
        }
        NavIcon::Engine => {
            painter.circle_stroke(c, 8.0, stroke);
            seg(c, p(0.0, -6.0));
            seg(c, p(5.2, 3.0));
            seg(c, p(-5.2, 3.0));
        }
        NavIcon::Gear => {
            seg(p(0.0, -8.0), p(0.0, -1.0));
            painter.circle_stroke(p(0.0, 3.5), 4.5, stroke);
            painter.circle_filled(p(0.0, 3.5), 1.2, color);
        }
        NavIcon::Weapon => {
            painter.circle_stroke(c, 5.5, stroke);
            seg(p(0.0, -8.0), p(0.0, -3.0));
            seg(p(0.0, 3.0), p(0.0, 8.0));
            seg(p(-8.0, 0.0), p(-3.0, 0.0));
            seg(p(3.0, 0.0), p(8.0, 0.0));
        }
        NavIcon::Telemetry => {
            painter.add(egui::Shape::line(
                vec![
                    p(-8.0, 0.0),
                    p(-4.0, 0.0),
                    p(-1.5, -6.0),
                    p(1.5, 6.0),
                    p(4.0, 0.0),
                    p(8.0, 0.0),
                ],
                stroke,
            ));
        }
        NavIcon::Editor => {
            seg(p(-8.0, -4.0), p(8.0, -4.0));
            seg(p(-8.0, 4.0), p(8.0, 4.0));
            painter.circle_filled(p(3.0, -4.0), 2.4, color);
            painter.circle_filled(p(-3.0, 4.0), 2.4, color);
        }
    }
}

/// Пункт навигации на всю ширину панели. Подложка есть и у невыбранного пункта:
/// без неё пункты проявлялись только под курсором (жалоба тестировщика).
/// Длинные русские подписи переносятся, высота пункта растёт вместе с ними.
fn nav_item(ui: &mut egui::Ui, selected: bool, icon: NavIcon, label: &str) -> bool {
    let width = ui.available_width();
    let text_left = 34.0;
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Wrap),
        width - text_left - 8.0,
        egui::TextStyle::Button,
    );
    let height = (galley.size().y + 16.0).max(36.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
    });
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            palette::get().selected_bg
        } else if response.hovered() && enabled {
            palette::get().surface_control_hover
        } else {
            palette::get().bg_card
        };
        let color = if selected {
            palette::get().selected_fg
        } else if enabled {
            palette::get().text_primary
        } else {
            palette::get().text_disabled
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 6u8, fill);
        if palette::get().high_contrast {
            painter.rect_stroke(
                rect,
                6u8,
                egui::Stroke::new(1.5, palette::get().border_default),
                egui::StrokeKind::Inside,
            );
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                6u8,
                egui::Stroke::new(2.0, palette::get().focus),
                egui::StrokeKind::Outside,
            );
        }
        paint_nav_icon(
            painter,
            egui::pos2(rect.left() + 18.0, rect.center().y),
            icon,
            color,
        );
        let pos = egui::pos2(
            rect.left() + text_left,
            rect.center().y - galley.size().y / 2.0,
        );
        painter.galley(pos, galley, color);
    }
    response.clicked()
}

/// Картинка устройства (джойстик/РУД) заданного тона и размера. Вынесена из
/// `UiState::device_icon_button`, потому что ровно те же глифы нужны легенде
/// (`effects_legend`) — иначе легенда объясняла бы не те значки, что в карточках.
fn device_icon_image(icon: DeviceIcon, tint: Color32, size: f32) -> egui::Image<'static> {
    let source = match icon {
        DeviceIcon::Joystick => egui::include_image!("../assets/icon_joystick.png"),
        DeviceIcon::Throttle => egui::include_image!("../assets/icon_throttle.png"),
    };
    egui::Image::new(source)
        .tint(tint)
        .fit_to_exact_size(egui::vec2(size, size))
}

/// Заливка под «включённой» иконкой устройства — одна на кнопку в карточке и
/// на образец в легенде, чтобы они выглядели одинаково. Джойстик и РУД разного
/// цвета: маршрут эффекта читается с одного взгляда, даже без формы значка.
fn device_on_bg(icon: DeviceIcon) -> Color32 {
    match icon {
        DeviceIcon::Joystick => palette::get().accent_live,
        DeviceIcon::Throttle => palette::get().accent_primary,
    }
}

/// Легенда над списком эффектов: расшифровывает иконки устройств и состояния
/// эффекта постоянным видимым текстом, а не только тултипом. `show_devices`
/// выключается для секций, где маршрутизация зафиксирована и переключателей
/// устройства в карточках нет (Engines) — иначе легенда объясняла бы
/// отсутствующий элемент.
fn effects_legend(ui: &mut egui::Ui, t: &Strings, show_devices: bool) {
    // Образец иконки — ровно в том виде, в каком она выглядит включённой в
    // карточке (тёмный глиф на светлой подложке), чтобы её можно было узнать.
    fn device_sample(ui: &mut egui::Ui, icon: DeviceIcon, label: &str) {
        egui::Frame::new()
            .fill(device_on_bg(icon))
            .corner_radius(3u8)
            .inner_margin(egui::Margin::same(2))
            .show(ui, |ui| {
                ui.add(device_icon_image(icon, palette::get().device_on_fg, 14.0));
            });
        ui.label(
            RichText::new(label)
                .small()
                .strong()
                .color(palette::get().text_secondary),
        );
    }

    fn note(ui: &mut egui::Ui, text: &str) {
        ui.label(
            RichText::new(text)
                .small()
                .color(palette::get().text_disabled),
        );
    }

    egui::Frame::new()
        .fill(palette::get().bg_card_disabled)
        .stroke(egui::Stroke::new(1.0, palette::get().border_default))
        .corner_radius(6u8)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .outer_margin(egui::Margin::symmetric(6, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);

            if show_devices {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(t.legend_devices)
                            .small()
                            .color(palette::get().text_secondary),
                    );
                    device_sample(ui, DeviceIcon::Joystick, t.legend_stick);
                    ui.add_space(6.0);
                    device_sample(ui, DeviceIcon::Throttle, t.legend_throttle);
                    ui.add_space(6.0);
                    note(ui, t.legend_hint);
                });
            }

            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(t.legend_states)
                        .small()
                        .color(palette::get().text_secondary),
                );

                ui.label(
                    RichText::new(t.status_off)
                        .small()
                        .color(palette::get().text_disabled),
                );
                note(ui, t.legend_state_off);
                ui.add_space(6.0);

                dot_indicator(ui, palette::get().border_active, false, 8.0);
                ui.label(
                    RichText::new(t.status_idle)
                        .small()
                        .color(palette::get().text_secondary),
                );
                note(ui, t.legend_state_idle);
                ui.add_space(6.0);

                dot_indicator(ui, palette::get().accent_live, true, 8.0);
                ui.label(
                    RichText::new(t.status_active)
                        .small()
                        .color(palette::get().accent_live),
                );
                note(ui, t.legend_state_active);
            });
        });
    ui.add_space(6.0);
}

/// Пометка внутри карточки ВСТРОЕННОГО эффекта, вытесненного пользовательским
/// на том же источнике телеметрии (см. `custom_fx::overrides`) — задача 1б:
/// молча погасший встроенный эффект был тем самым "почему у меня ничего не
/// вибрирует", от которого в этом проекте избавлялись весь день. Рисуется
/// ПЕРВОЙ строкой внутри `add_contents` карточки, ДО остального содержимого;
/// сама карточка при этом передаётся в `UiState::effect_card` с приглушённым
/// видом (см. вызовы ниже — `enabled && overridden.is_none()`), а не через
/// отдельный параметр эффект_card — сигнатура у неё и так используется
/// редактором эффектов (`ui/effects_editor.rs`) для шагов мастера, не
/// имеющих отношения к вытеснению.
fn overridden_by_note(ui: &mut egui::Ui, t: &Strings, custom_effect_name: &str) {
    ui.label(
        RichText::new(format!(
            "{} {custom_effect_name}",
            t.lbl_builtin_overridden_by
        ))
        .color(palette::get().status_attention)
        .italics(),
    );
    ui.add_space(4.0);
}

/// Отображаемое имя встроенного эффекта для UI-пометок задачи 1 — единая
/// точка соответствия `BuiltinEffect -> имя из Strings`, используется и
/// карточками встроенных эффектов (задача 1б, этот файл), и шагом 1
/// редактора пользовательских эффектов (задача 1а, `ui/effects_editor.rs`,
/// вызывается оттуда как `super::builtin_effect_display_name`). Исчерпывающий
/// матч без `_ =>` НАМЕРЕННО — тот же приём, что `primary_builtin_for` в
/// `custom_fx::overrides`: новый вариант `BuiltinEffect` обязан ломать
/// сборку здесь, а не тихо остаться безымянным.
///
/// Имена переиспользуются из УЖЕ существующих строк `Strings` везде, где они
/// были — второго "Flaps"/"Gear Transit & Doors" для WT-варианта того же
/// видимого эффекта не заводим (тот же текст, что у MSFS-версии, это и в
/// остальном UI так). Единственное собственное имя — `name_gear_bump`, у
/// скрытого (временно отключённого, см. `RumbleConfig::gear_enabled`)
/// эффекта "касание шасси", которому раньше вообще не было под каким
/// показаться пользователю.
fn builtin_effect_display_name(effect: BuiltinEffect, t: &Strings) -> &'static str {
    match effect {
        BuiltinEffect::Overspeed => t.overspeed_effect_name,
        BuiltinEffect::GearComp => t.heading_gear_comp,
        BuiltinEffect::GearTransit => t.lbl_gear_transit,
        BuiltinEffect::Bank => t.lbl_bank_turb,
        BuiltinEffect::Taxi => t.heading_taxi_thump,
        BuiltinEffect::Ground => t.name_ground_roll,
        BuiltinEffect::Flaps => t.name_flaps,
        BuiltinEffect::Gear => t.name_gear_bump,
        BuiltinEffect::Stall => t.name_stall,
        BuiltinEffect::Spoilers => t.name_spoilers,
        BuiltinEffect::EngineStart => t.name_engine_start,
        BuiltinEffect::WtWeapon1 => t.name_wt_weapon1,
        BuiltinEffect::WtWeapon2 => t.name_wt_weapon2,
        BuiltinEffect::WtFlaps => t.name_flaps,
        BuiltinEffect::WtGearTransit => t.lbl_gear_transit,
        BuiltinEffect::WtStall => t.name_wt_stall,
        BuiltinEffect::WtEngineStart => t.name_wt_engine_start,
        BuiltinEffect::WtOverspeed => t.name_wt_overspeed,
        BuiltinEffect::WtGearOverspeed => t.name_wt_gear_overspeed,
    }
}

/// Реверс `BuiltinMask -> BuiltinEffect` для одиночной пометки на карточке —
/// маска из `overrides::overridden_builtins` устроена как плоский набор
/// bool-полей (см. её doc-комментарий: так `apply_to_*_config` остаются
/// прямыми присваиваниями без матчинга по enum), а карточке нужен только тот
/// один вариант, который относится именно к ней. Вызывается ТОЛЬКО когда уже
/// известно, что маска для одиночного эффекта (см. `overriding_effect_name`
/// ниже) — тогда истинно ровно одно поле, и порядок веток не важен.
fn builtin_effect_from_mask(mask: &BuiltinMask) -> Option<BuiltinEffect> {
    if mask.overspeed {
        Some(BuiltinEffect::Overspeed)
    } else if mask.gear_comp {
        Some(BuiltinEffect::GearComp)
    } else if mask.gear_transit {
        Some(BuiltinEffect::GearTransit)
    } else if mask.bank {
        Some(BuiltinEffect::Bank)
    } else if mask.taxi {
        Some(BuiltinEffect::Taxi)
    } else if mask.ground {
        Some(BuiltinEffect::Ground)
    } else if mask.flaps {
        Some(BuiltinEffect::Flaps)
    } else if mask.gear {
        Some(BuiltinEffect::Gear)
    } else if mask.stall {
        Some(BuiltinEffect::Stall)
    } else if mask.spoilers {
        Some(BuiltinEffect::Spoilers)
    } else if mask.engine_start {
        Some(BuiltinEffect::EngineStart)
    } else if mask.wt_weapon1 {
        Some(BuiltinEffect::WtWeapon1)
    } else if mask.wt_weapon2 {
        Some(BuiltinEffect::WtWeapon2)
    } else if mask.wt_flaps {
        Some(BuiltinEffect::WtFlaps)
    } else if mask.wt_gear_transit {
        Some(BuiltinEffect::WtGearTransit)
    } else if mask.wt_stall {
        Some(BuiltinEffect::WtStall)
    } else if mask.wt_engine_start {
        Some(BuiltinEffect::WtEngineStart)
    } else if mask.wt_overspeed {
        Some(BuiltinEffect::WtOverspeed)
    } else if mask.wt_gear_overspeed {
        Some(BuiltinEffect::WtGearOverspeed)
    } else {
        None
    }
}

/// Какой встроенный эффект вытесняет источник `source` В ПРИНЦИПЕ — не
/// зависит от того, включён ли КОНКРЕТНЫЙ эффект пользователя сейчас, для
/// какой игры/борта он настроен и т.п. (задача 1а, шаг 1 редактора: связь
/// нужно показать ДО того, как пользователь вообще включит и сохранит
/// эффект). Реализовано через синтетический одиночный ВКЛЮЧЁННЫЙ эффект,
/// прогнанный через настоящий `overrides::overridden_builtins` для каждой
/// игры, которой физически принадлежит источник — единственный источник
/// истины остаётся в `custom_fx::overrides::primary_builtin_for` (приватна,
/// сюда не импортируется), а не второй скопированной таблицей "источник ->
/// эффект", которая могла бы разойтись с оригиналом.
fn static_primary_builtin_for(source: SourceId) -> Option<BuiltinEffect> {
    let mut probe = new_effect(String::new(), source);
    probe.enabled = true;
    for game in [ActiveGame::Msfs, ActiveGame::Wt, ActiveGame::Xplane] {
        let mask = overrides::overridden_builtins(std::slice::from_ref(&probe), game, "");
        if let Some(effect) = builtin_effect_from_mask(&mask) {
            return Some(effect);
        }
    }
    None
}

/// Имя ПЕРВОГО включённого пользовательского эффекта из `effects`,
/// вытесняющего встроенный эффект, для которого `field` возвращает `true` в
/// его `BuiltinMask` (задача 1б/1в) — используется, только когда уже
/// известно (`hit`), что этот встроенный эффект вообще вытеснен на текущем
/// кадре (см. `builtin_override_mask`, посчитанный один раз на весь кадр):
/// без этой проверки пришлось бы прогонять `overridden_builtins` по всему
/// списку эффектов на КАЖДУЮ из ~20 карточек каждый кадр, а не только когда
/// там реально что-то вытеснено.
fn overriding_effect_name<'a>(
    effects: &'a [CustomEffect],
    game: ActiveGame,
    aircraft: &str,
    field: impl Fn(&BuiltinMask) -> bool,
    hit: bool,
) -> Option<&'a str> {
    if !hit {
        return None;
    }
    effects.iter().find_map(|e| {
        let mask = overrides::overridden_builtins(std::slice::from_ref(e), game, aircraft);
        field(&mask).then_some(e.name.as_str())
    })
}

/// Статус-бейдж карточки эффекта: круглый маркер И слово рядом с ним.
/// Раньше во включённом состоянии здесь был только кружок, а что он значит,
/// знала лишь всплывающая подсказка — тестировщик не смог разобраться в
/// интерфейсе именно из-за таких немых значков. Теперь состояние всегда
/// подписано словом, кружок остался как быстрый визуальный якорь.
fn effect_status_badge(ui: &mut egui::Ui, enabled: bool, active: bool, t: &Strings) {
    if !enabled {
        ui.label(
            RichText::new(t.status_off)
                .small()
                .color(palette::get().text_disabled),
        );
        return;
    }
    let (dot, text, color) = if active {
        (
            palette::get().accent_live,
            t.status_active,
            palette::get().accent_live,
        )
    } else {
        (
            palette::get().border_active,
            t.status_idle,
            palette::get().text_secondary,
        )
    };
    // Бейдж живёт внутри Layout::right_to_left (см. вызовы в карточках), а
    // ui.horizontal наследует направление родителя — значит первый добавленный
    // элемент оказывается САМЫМ ПРАВЫМ. Чтобы читалось «кружок, потом слово» и
    // там, и в легенде (где направление обычное), порядок выбираем по
    // фактическому направлению раскладки.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let word = RichText::new(text).small().color(color);
        if ui.layout().prefer_right_to_left() {
            ui.label(word);
            dot_indicator(ui, dot, active, 8.0);
        } else {
            dot_indicator(ui, dot, active, 8.0);
            ui.label(word);
        }
    });
}

fn status_badge(ui: &mut egui::Ui, status: &SimStatus, t: &Strings) {
    let (text, color, filled) = match status {
        SimStatus::Disconnected => (t.disconnected, palette::get().status_bad, false),
        SimStatus::Connected => (t.connected, palette::get().status_warn, false),
        SimStatus::InFlight => (t.in_flight, palette::get().status_ok, true),
        SimStatus::SimConnectMissing => {
            (t.simconnect_missing, palette::get().status_attention, true)
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

/// Текстовая метка активной игры в верхней панели — рядом со status_badge.
/// Раньше рисовала иконку (assets/MSFS.png, assets/WT.png), заменено на
/// простой текст по требованию пользователя. Молчит (ничего не рисует),
/// пока ActiveGame::None, поэтому вызывающий код не оборачивает вызов
/// условием отдельно. Названия брендов не переводятся (см. hover_game_msfs/
/// hover_game_wt — уже одинаковы в обеих локалях), поэтому текст меток тоже
/// захардкожен, а не заведён в i18n.
fn game_badge(ui: &mut egui::Ui, game: ActiveGame, t: &Strings) {
    let (text, hover): (&str, &str) = match game {
        ActiveGame::Msfs => ("MSFS", t.hover_game_msfs),
        ActiveGame::Wt => ("WarThunder", t.hover_game_wt),
        ActiveGame::Xplane => ("X-Plane", t.hover_game_xp),
        ActiveGame::None => return,
    };
    ui.label(RichText::new(text).strong()).on_hover_text(hover);
}

/// Устройство в верхней панели: значок, название и точка состояния. Подключённое
/// устройство тихое, а отключённое подсвечено красным и подписано словом в
/// подсказке, чтобы состояние не держалось на одном цвете. В `compact` подпись
/// скрыта: когда в панели тесно, название и состояние остаются в подсказке.
fn controller_badge_dot(
    ui: &mut egui::Ui,
    icon: DeviceIcon,
    label: &str,
    connected: bool,
    compact: bool,
    t: &Strings,
) {
    let (color, text_color) = if connected {
        (palette::get().status_ok, palette::get().text_secondary)
    } else {
        (palette::get().status_bad, palette::get().status_bad)
    };
    let state = if connected {
        t.connected
    } else {
        t.disconnected
    };
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let image = device_icon_image(icon, text_color, 18.0);
            let word = RichText::new(label).color(text_color);
            // Панель рисует этот блок внутри right_to_left, см. effect_status_badge.
            if ui.layout().prefer_right_to_left() {
                dot_indicator(ui, color, connected, 9.0);
                if !compact {
                    ui.label(word);
                }
                ui.add(image);
            } else {
                ui.add(image);
                if !compact {
                    ui.label(word);
                }
                dot_indicator(ui, color, connected, 9.0);
            }
        })
        .response;
    response.on_hover_text(format!("{label}: {state}"));
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

    // Конструктор пользовательских эффектов ("Редактор эффектов", см.
    // ui/effects_editor.rs): список эффектов — общий с воркерами через
    // CustomFxShared (тот же rev-приём, что у ConfigShared), живой список id
    // эффектов, реально дающих отдачу сейчас (для точки-индикатора в списке
    // слева). Глобального режима больше нет (см. custom_fx::overrides) —
    // оба движка эффектов, встроенный и пользовательский, считаются каждый
    // тик одновременно.
    pub custom_fx: Arc<CustomFxShared>,
    pub active_custom_ids: Arc<Mutex<Vec<String>>>,
    pub fx_editor: effects_editor::EditorState,
    // Перехват HID-канала на время предпросмотра эффекта (кнопка "Играть" в
    // редакторе) — тот же клон, что получили все три воркера в main.rs.
    // Владение самим предпросмотром (что играет, когда шлём кадр) живёт в
    // fx_editor::EditorState, здесь только сам замок.
    pub preview_lock: PreviewLock,
    // Поток ручного предпросмотра на точной сетке 20мс (см.
    // custom_fx::preview_player) — создаётся один раз в main.rs рядом с
    // preview_lock и живёт всё время работы приложения. EditorCtx получает
    // только ссылку на него.
    pub preview_player: PreviewPlayer,

    #[cfg(debug_assertions)]
    pub test_level: u8,
    #[cfg(debug_assertions)]
    pub raw_hex: String,

    pub tx_hid: Sender<HidCmd>,
    pub logs: LogBuffer,
    pub last_vars: Arc<Mutex<Option<FlightVars>>>,
    pub last_wt_vars: Arc<Mutex<Option<WtVars>>>,

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
    // Свёрнут ли столбец Live Monitor целиком (не путать с monitor_show_disabled,
    // который скрывает только неактивные эффекты внутри развёрнутой панели).
    // Персистится в SettingsFile, как lang/close_to_tray — см. save_global_settings.
    pub monitor_collapsed: bool,
    pub hold: Arc<AtomicBool>,
    // Реальный вывод на моторы от hid_worker и его история для осциллограмм
    // Live Monitor. История живёт здесь, потому что нужна только GUI-потоку.
    pub motor_monitor: Arc<crate::types::MotorMonitor>,
    pub motor_history: crate::types::MotorHistory,
    // Чёрно-белый режим для слабовидящих. Масштаб в нём крупнее, поэтому окну
    // надо подобрать новый размер: см. window_fit_pending в UiState::ui.
    pub high_contrast: bool,
    pub window_fit_pending: bool,
    // Тумблер "Записывать сессию WT" (Options menu) — читается wt_worker'ом
    // каждый тик (см. wt_link::recorder::SessionRecorder), сам факт
    // записи/пути к файлу не хранится тут, только желаемое состояние.
    pub recording: Arc<AtomicBool>,

    // "Close to tray" (Options menu): при close_requested окно прячется
    // (ViewportCommand::Visible(false)) вместо реального завершения процесса —
    // если только это не настоящий Exit из трея (force_quit), который должен
    // этот перехват обойти. См. UiState::ui() и tray.rs.
    pub close_to_tray: bool,
    // Автоопределение активной игры (MSFS/WT) — единый слот владения,
    // заполняется воркерами (sim::sim_worker/wt_link::wt_worker) через
    // game_state::GameSlot, читается GUI-потоком каждый кадр. `game_override`
    // — ручной оверрайд (меню Опции), персистится в SettingsFile.
    // `last_seen_game` — предыдущее значение active_game, чтобы обнаружить
    // переход и один раз переключить активную секцию (см. show_main).
    pub active_game: Arc<Mutex<ActiveGame>>,
    pub game_override: GameOverride,
    pub last_seen_game: ActiveGame,
    pub force_quit: Arc<AtomicBool>,
    pub show_help: bool,
    pub show_help_us: bool,

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
        crate::settings::set_monitor_collapsed(self.monitor_collapsed);
        crate::settings::set_high_contrast(self.high_contrast);
        crate::settings::set_game_override(self.game_override);
        let ap = self.aircraft_profiles.lock();
        let _ = crate::settings::save(&crate::settings::SettingsFile {
            default: ap.default.clone(),
            profiles: ap.profiles.clone(),
            lang: self.lang,
            close_to_tray: self.close_to_tray,
            simconnect_dll_path: crate::settings::simconnect_dll_path(),
            monitor_collapsed: self.monitor_collapsed,
            high_contrast: self.high_contrast,
            // wt_enabled — legacy-поле, производное от game_override (см.
            // комментарий у поля в SettingsFile): единственный источник
            // истины теперь game_override, wt_enabled существует только для
            // миграции файлов, сохранённых сборкой до этой фичи.
            wt_enabled: self.game_override == GameOverride::ForceWt,
            game_override: self.game_override,
        });
    }

    fn set_high_contrast(&mut self, ctx: &egui::Context, on: bool) {
        self.high_contrast = on;
        apply_theme(ctx, on);
        self.window_fit_pending = true;
        self.save_global_settings();
    }

    /// Строка эффекта, где сила/амплитуда вибрации отображается и настраивается
    /// пользователем всегда в диапазоне 0..100%, независимо от технического
    /// предела эффекта в движке (255, 50, 55, 200...).
    /// `native_max` — во что превращается 100% при передаче в RumbleConfig;
    /// хранимое значение (`val`) остаётся в исходных технических единицах —
    /// rumble.rs ничего не знает о процентах.
    ///
    /// Аргументов много намеренно: это чистая функция отрисовки одной строки,
    /// всё её состояние приходит снаружи по &mut. Обёртка-структура «параметры
    /// строки» была бы тем же списком полей плюс лишний слой на каждом вызове.
    #[allow(clippy::too_many_arguments)]
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
        Self::effect_row_percent(
            ui,
            name,
            val,
            native_max,
            enabled,
            active,
            on_change,
            hint,
            Some(device),
            t,
        );
    }

    /// То же, что `effect_row_percent_hinted`, но маршрутизация необязательна:
    /// у части эффектов она зафиксирована и переключателя в строке нет.
    #[allow(clippy::too_many_arguments)]
    fn effect_row_percent(
        ui: &mut egui::Ui,
        name: &str,
        val: &mut f32,
        native_max: f32,
        enabled: &mut bool,
        active: bool,
        on_change: &mut bool,
        hint: Option<&str>,
        device: Option<&mut EffectDeviceTarget>,
        t: &Strings,
    ) {
        let level = live_level(ui, ui.id().with(("fader", name)), active && *enabled);
        let toggled = Self::effect_line(ui, name, hint, enabled, active, t, |ui, enabled_now| {
            if let Some(device) = device {
                Self::device_target_toggle(ui, device, on_change, t);
                ui.add_space(6.0);
            }
            ui.add_enabled_ui(enabled_now, |ui| {
                // Процент всегда пересчитывается заново из исходного (технического)
                // значения — никакого накопления погрешности округления между кадрами.
                let mut pct = if native_max > 0.0 {
                    (*val / native_max * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };
                let value = ui.add(
                    egui::DragValue::new(&mut pct)
                        .range(0.0..=100.0)
                        .suffix("%")
                        .fixed_decimals(0),
                );
                ui.spacing_mut().slider_width = ui.available_width().max(60.0);
                let slider = live_slider(ui, &mut pct, level);
                if let Some(h) = hint {
                    slider.clone().on_hover_text(h);
                }
                if value.changed() || slider.changed() {
                    *val = (pct / 100.0) * native_max;
                    *on_change = true;
                }
            });
        });
        if toggled {
            *on_change = true;
        }
    }

    /// Каркас строки эффекта: переключатель, название с состоянием под ним, а
    /// всё остальное `right` кладёт справа налево в оставшееся место. Благодаря
    /// этому значение и маршрутизация всегда на краю и видны даже в узком окне,
    /// а сжимается только слайдер.
    #[allow(clippy::too_many_arguments)]
    fn effect_line(
        ui: &mut egui::Ui,
        name: &str,
        hint: Option<&str>,
        enabled: &mut bool,
        active: bool,
        t: &Strings,
        right: impl FnOnce(&mut egui::Ui, bool),
    ) -> bool {
        let mut toggled = false;
        let name_width = if i18n::get() == Lang::Ru {
            190.0
        } else {
            150.0
        };
        let size = Vec2::new(ui.available_width(), 38.0);
        ui.allocate_ui_with_layout(
            size,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                toggled = toggle_switch(ui, enabled).changed();
                ui.add_space(2.0);
                ui.vertical(|ui| {
                    ui.set_width(name_width);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let label = ui.add(
                        egui::Label::new(RichText::new(name).family(palette::medium())).truncate(),
                    );
                    if let Some(h) = hint {
                        label.on_hover_text(h);
                    }
                    // Слово и точка моргали бы на каждый тик мотора, см. live_level.
                    let held = live_level(ui, ui.id().with(("badge", name)), active && *enabled);
                    effect_status_badge(ui, *enabled, held > 0.0, t);
                });
                let enabled_now = *enabled;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    right(ui, enabled_now);
                });
            },
        );
        toggled
    }

    /// Строка эффекта Overspeed: порог больше не задаётся вручную ползунком,
    /// а приходит динамически из SimConnect (AIRSPEED BARBER POLE — красная
    /// черта Vmo/Mmo, сим сам двигает её вниз при наборе высоты) для текущего
    /// загруженного самолёта. `threshold_kn` — None, если SimConnect ещё не
    /// отдал значение (например, самолёт не загружен) — в этом случае
    /// показываем "Limit: N/A" и эффект не может сработать.
    ///
    /// Про число аргументов — см. `effect_row_percent_hinted` выше.
    #[allow(clippy::too_many_arguments)]
    fn overspeed_row(
        ui: &mut egui::Ui,
        enabled: &mut bool,
        threshold_kn: Option<f64>,
        active: bool,
        on_change: &mut bool,
        override_enabled: &mut bool,
        override_kn: &mut f64,
        intensity: &mut f32,
        device: &mut EffectDeviceTarget,
        t: &Strings,
    ) {
        // Триггерится только когда порог реально пришёл от SimConnect (или
        // задан вручную через Override) — без threshold_kn эффект физически
        // не может сработать, даже если галочка включена.
        let is_triggering = active && *enabled && threshold_kn.is_some();
        Self::effect_row_percent_hinted(
            ui,
            t.overspeed_effect_name,
            intensity,
            255.0,
            enabled,
            is_triggering,
            on_change,
            Some(t.hover_overspeed_intensity),
            device,
            t,
        );

        ui.horizontal(|ui| {
            ui.add_space(PARAM_INDENT);
            let limit_text = match threshold_kn {
                Some(kn) => format!("{} {:.0} kts", t.lbl_limit, kn),
                None => t.limit_na.to_string(),
            };
            ui.add_enabled_ui(*enabled, |ui| {
                ui.label(RichText::new(limit_text).weak())
                    .on_hover_text(t.hover_overspeed_limit_barberpole);
            });
            ui.add_space(12.0);

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
            // Как и у effect_status_badge: вызывающий код оборачивает нас в
            // Layout::right_to_left, а ui.scope сохраняет направление, поэтому
            // добавленная первой иконка уезжает вправо. До появления легенды
            // это было незаметно, но легенда называет первую иконку
            // «джойстик» — значит в карточке она обязана быть первой слева,
            // иначе легенда учит неверному соответствию.
            let order = if ui.layout().prefer_right_to_left() {
                [DeviceIcon::Throttle, DeviceIcon::Joystick]
            } else {
                [DeviceIcon::Joystick, DeviceIcon::Throttle]
            };
            for icon in order {
                let (selected, hover) = match icon {
                    DeviceIcon::Joystick => (target.enable_joystick, t.hover_joystick_hw),
                    DeviceIcon::Throttle => (target.enable_throttle, t.hover_throttle_hw),
                };
                if Self::device_icon_button(ui, selected, icon)
                    .on_hover_text(hover)
                    .clicked()
                {
                    match icon {
                        DeviceIcon::Joystick => target.enable_joystick = !target.enable_joystick,
                        DeviceIcon::Throttle => target.enable_throttle = !target.enable_throttle,
                    }
                    *on_change = true;
                }
            }
        });
    }

    /// Рисует иконку устройства (джойстик/РУД) как кнопку с картинкой
    /// (assets/icon_joystick.png, assets/icon_throttle.png), подсвеченную
    /// акцентным цветом в выбранном состоянии — та же selected-стилистика,
    /// что была у selectable_label, но с реальной иконкой вместо эмодзи.
    fn device_icon_button(ui: &mut egui::Ui, selected: bool, icon: DeviceIcon) -> egui::Response {
        // Во включённом состоянии под иконкой светлая заливка (device_on_bg),
        // поэтому тинт там должен быть ТЁМНЫМ — раньше он брался из
        // selection.stroke.color, и после подъёма акцента до светло-голубого
        // иконка стала бы светлой по светлому.
        let tint = if selected {
            palette::get().device_on_fg
        } else {
            ui.visuals().text_color()
        };
        let image = device_icon_image(icon, tint, 20.0);
        let mut button = egui::Button::new(image)
            .selected(selected)
            .min_size(Vec2::new(36.0, 28.0));
        if selected {
            // Явный fill вместо стандартной полупрозрачной selection.bg_fill —
            // просили конкретную подложку под включённой иконкой устройства,
            // не трогая глобальный акцент выбора (он используется и в других
            // местах — nav, чекбоксы).
            button = button.fill(device_on_bg(icon));
        }
        ui.add(button)
    }

    /// Обёртка вокруг одного эффекта: строка в общей стопке, приглушённая, если
    /// эффект выключен. Зазор между строками задаётся здесь же, чтобы стопка
    /// читалась как единая панель.
    ///
    /// Подсветка при срабатывании идёт только через `live_level`: прямая
    /// привязка к `active` мигала на каждый тик мотора и была навязчивой на весь
    /// блок, поэтому от неё уже отказывались.
    fn effect_card(
        ui: &mut egui::Ui,
        enabled: bool,
        active: bool,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        let level = live_level(ui, ui.next_auto_id(), active && enabled);
        let base = if enabled {
            palette::get().bg_card
        } else {
            palette::get().bg_card_disabled
        };
        ui.spacing_mut().item_spacing.y = 2.0;
        // Без цвета срабатывание передаёт толщина рамки, а выключенное состояние
        // тонкая серая рамка. Слово под названием дублирует и то и другое.
        let stroke = if !palette::get().high_contrast {
            egui::Stroke::NONE
        } else if !enabled {
            egui::Stroke::new(1.0, palette::get().text_disabled)
        } else if level > 0.0 {
            egui::Stroke::new(3.5, palette::get().border_default)
        } else {
            egui::Stroke::new(1.0, palette::get().border_default)
        };
        let frame = egui::Frame::new()
            .fill(base.lerp_to_gamma(palette::get().accent_live, 0.08 * level))
            .corner_radius(6u8)
            .inner_margin(egui::Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Выключенную строку гасим через override текста, а не
                // set_opacity(): opacity глушит заодно slider handle и рамки
                // контролов, из-за чего строка выглядит "сломанной", а не
                // просто неактивной.
                if !enabled {
                    ui.visuals_mut().override_text_color = Some(palette::get().text_disabled);
                }
                add_contents(ui);
            });
        // Рамку рисуем поверх готовой строки, а не через Frame::stroke: толщина
        // рамки во Frame входит в размер, и строка дёргалась бы при срабатывании.
        if stroke.width > 0.0 {
            ui.painter()
                .rect_stroke(frame.response.rect, 6u8, stroke, egui::StrokeKind::Inside);
        }
    }

    /// Панель шага в редакторе эффектов: там содержимое крупное и разнородное,
    /// поэтому нужна настоящая рамка, а не строка из стопки.
    fn panel_card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
        egui::Frame::new()
            .fill(palette::get().bg_card)
            .stroke(egui::Stroke::new(
                palette::get().stroke_width,
                palette::get().border_default,
            ))
            .corner_radius(10u8)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Поля ввода по умолчанию шире колонки сетки шагов и раздвигали бы её.
                ui.spacing_mut().text_edit_width = (ui.available_width() - 110.0).max(80.0);
                add_contents(ui);
            });
    }

    /// Про число аргументов — см. `effect_row_percent_hinted` выше.
    #[allow(clippy::too_many_arguments)]
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
        let toggled = Self::effect_line(ui, name, hint, enabled, active, t, |ui, enabled_now| {
            ui.add_enabled_ui(enabled_now, |ui| {
                let mut tmp = *val as f32;
                let r = (*range.start() as f32)..=(*range.end() as f32);
                let value = ui.add(egui::DragValue::new(&mut tmp).range(r.clone()).speed(0.5));
                ui.spacing_mut().slider_width = ui.available_width().max(60.0);
                let slider = ui.add(
                    egui::Slider::new(&mut tmp, r)
                        .trailing_fill(true)
                        .show_value(false),
                );
                if let Some(h) = hint {
                    slider.clone().on_hover_text(h);
                }
                if value.changed() || slider.changed() {
                    *val = tmp as f64;
                    *on_change = true;
                }
            });
        });
        if toggled {
            *on_change = true;
        }
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

        self.motor_history.tick(&self.motor_monitor);

        // Горячая клавиша нужна именно тем, кому меню в обычном режиме не разглядеть.
        let contrast_shortcut = egui::Modifiers::CTRL | egui::Modifiers::SHIFT;
        if ctx.input_mut(|i| i.consume_key(contrast_shortcut, egui::Key::H)) {
            self.set_high_contrast(&ctx, !self.high_contrast);
        }
        // Размер окна задаётся в точках интерфейса, а масштаб применяется кадром
        // позже. Ждём его, иначе окно получит размер по старому масштабу.
        if self.window_fit_pending && (ctx.zoom_factor() - palette::get().zoom).abs() < 0.01 {
            self.window_fit_pending = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(Vec2::new(
                1000.0, 600.0,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(1120.0, 720.0)));
        }

        // Страховка для предпросмотра эффектов (см. effects_editor.rs): это
        // ЕДИНСТВЕННОЕ место, которое гарантированно выполняется каждый кадр
        // независимо от того, что сейчас отрисовано — сам редактор рисуется
        // (и потому мог бы сам себя остановить) только когда одновременно
        // активны вкладка Main И секция Effects. Если предпросмотр всё ещё
        // считается включённым, а хотя бы одно из двух условий уже не
        // выполняется (ушли на другую секцию/вкладку любым путём — клик по
        // навигации, смена активной игры, переключение на Debug), глушим
        // здесь. stop_preview идемпотентен, так что при выключенном
        // предпросмотре это просто no-op каждый кадр.
        if !(self.active_tab == Tab::Main && self.active_section == Section::Effects) {
            self.fx_editor
                .stop_preview(&self.tx_hid, &self.preview_lock, &self.preview_player);
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

        if self.show_help {
            let mut open = true;
            egui::Window::new(t.hover_help)
                .open(&mut open)
                .default_size(egui::vec2(560.0, 640.0))
                .collapsible(false)
                .show(&ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        // Лёгкая markdown-подобная разметка: "## " -> heading,
                        // "### " -> подзаголовок, пустая строка -> отступ, всё
                        // остальное -> абзац/пункт с переносом по ширине окна.
                        // Цвет подзаголовка задан явно: полагаться на дефолт
                        // Visuals тут нельзя, см. историю с читаемостью тёмной
                        // темы.
                        for line in t.help_text.lines() {
                            if let Some(h) = line.strip_prefix("### ") {
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(h)
                                        .strong()
                                        .size(15.0)
                                        .color(palette::get().text_primary),
                                );
                                ui.add_space(1.0);
                            } else if let Some(h) = line.strip_prefix("## ") {
                                ui.add_space(8.0);
                                ui.heading(h);
                                ui.add_space(2.0);
                            } else if line.is_empty() {
                                ui.add_space(6.0);
                            } else {
                                ui.add(egui::Label::new(line).wrap());
                            }
                        }
                    });
                });
            self.show_help = open;
        }

        if self.show_help_us {
            let mut open = true;
            egui::Window::new(t.hover_help_us)
                .open(&mut open)
                .default_size(egui::vec2(480.0, 260.0))
                .collapsible(false)
                .show(&ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        // Тот же текст, но каждая строка selectable(true) — адреса
                        // кошельков и ссылку на YooMoney нужно выделять и копировать.
                        for line in t.help_us_text.lines() {
                            if line.is_empty() {
                                ui.add_space(6.0);
                            } else {
                                ui.add(egui::Label::new(line).wrap().selectable(true));
                            }
                        }
                    });
                });
            self.show_help_us = open;
        }

        let top_frame = egui::Frame::side_top_panel(ui.style())
            .fill(palette::get().bg_sidebar)
            .inner_margin(egui::Margin::symmetric(12, 8));
        egui::Panel::top("top").frame(top_frame).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                // Логотип цветной, в чёрно-белом режиме его заменяет название рядом.
                if !palette::get().high_contrast {
                    ui.add(
                        egui::Image::new(egui::include_image!("../assets/icon.png"))
                            .fit_to_exact_size(Vec2::splat(28.0)),
                    );
                }
                ui.label(
                    RichText::new("Aurora Vibra")
                        .family(palette::semibold())
                        .size(14.5),
                );
                ui.add_space(6.0);

                // Верхняя панель рисуется раньше блока show_main (где для
                // шага секций уже читается active_game в отдельную `ag`) —
                // читаем Mutex здесь же отдельно, а не полагаемся на порядок
                // между двумя блоками кода (дешёвая lock, не проблема).
                let st = *self.status.lock();
                let top_ag = *self.active_game.lock();
                egui::Frame::new()
                    .fill(palette::get().bg_card)
                    .corner_radius(6u8)
                    .inner_margin(egui::Margin::symmetric(8, 2))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            game_badge(ui, top_ag, t);
                            status_badge(ui, &st, t);
                        });
                    });
                ui.add_space(6.0);

                let ac = self.aircraft_title.lock().clone();
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
                let ac_color = if palette::get().high_contrast {
                    palette::get().text_primary
                } else if has_custom_telemetry {
                    Color32::from_rgb(70, 160, 255)
                } else if has_saved_profile {
                    Color32::from_rgb(0x82, 0xf1, 0x6a) // #82f16a
                } else {
                    Color32::WHITE
                };
                // В чёрно-белом режиме цвет названия ничего не скажет, поэтому то
                // же самое говорит значок перед ним и подсказка.
                let (ac_mark, ac_hover) = if !palette::get().high_contrast {
                    ("", None)
                } else if has_custom_telemetry {
                    ("★ ", Some(t.hover_aircraft_custom_telemetry))
                } else if has_saved_profile {
                    ("✔ ", Some(t.hover_aircraft_saved_profile))
                } else {
                    ("", None)
                };
                // Название сужается первым: справа от него кнопки и Stop, которым
                // нельзя наезжать друг на друга, а полное имя есть в подсказке.
                let reserved = if self.lang == Lang::Ru { 590.0 } else { 520.0 };
                let aircraft_width = (ui.available_width() - reserved).clamp(90.0, 170.0);
                ui.allocate_ui(
                    Vec2::new(aircraft_width, ui.spacing().interact_size.y),
                    |ui| {
                        let label = ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "{ac_mark}{}",
                                    format_aircraft_label(&ac, t)
                                ))
                                .monospace()
                                .color(ac_color),
                            )
                            .truncate(),
                        );
                        if let Some(hover) = ac_hover {
                            label.on_hover_text(hover);
                        }
                    },
                );
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
                let save_label = if dirty && palette::get().high_contrast {
                    // Жёлтого здесь нет: несохранённое показывает звёздочка.
                    RichText::new(format!("{} *", t.btn_save))
                } else if dirty {
                    RichText::new(t.btn_save).color(palette::get().status_warn)
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
                ui.add_space(6.0);
                let default_toggle = ui
                    .add_enabled(
                        has_named_profile_active,
                        egui::Button::selectable(self.save_as_default_too, "📌"),
                    )
                    .on_hover_text(t.hover_also_default);
                if default_toggle.clicked() {
                    self.save_as_default_too = !self.save_as_default_too;
                }

                ui.add_space(8.0);

                // "Check for updates" перенесена в оверфлоу-меню "..." → Options
                // (см. ниже) — вместо отдельной кнопки в тулбаре.

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Stop заглушает всё сразу, поэтому он на постоянном месте в
                    // верхнем правом углу, в какой бы секции ни был пользователь.
                    let holding = self.hold.load(Ordering::Relaxed);
                    let hold_button = if holding {
                        egui::Button::new(
                            RichText::new(t.btn_resume)
                                .family(palette::semibold())
                                .color(palette::get().bg_app),
                        )
                        .fill(palette::get().accent_live)
                    } else {
                        egui::Button::new(
                            RichText::new(t.btn_stop)
                                .family(palette::semibold())
                                .color(palette::get().on_danger),
                        )
                        .fill(palette::get().danger)
                    };
                    if ui.add(hold_button.min_size(Vec2::new(84.0, 0.0))).clicked() {
                        self.hold.store(!holding, Ordering::Relaxed);
                        let _ = self.tx_hid.send(HidCmd::SetHold(!holding));
                        tray::notify_held(!holding);
                    }
                    ui.add_space(8.0);

                    // Overflow-меню: Options (резерв на будущее), Help и, в debug-сборке,
                    // переключатель Main/Debug — раньше это были три-четыре отдельные
                    // кнопки в тулбаре, теперь редко используемые действия собраны в одном месте.
                    ui.menu_button("...", |ui| {
                        #[cfg(debug_assertions)]
                        {
                            if ui
                                .add(
                                    egui::Button::new(t.tab_main)
                                        .selected(self.active_tab == Tab::Main),
                                )
                                .clicked()
                            {
                                self.active_tab = Tab::Main;
                            }
                            if ui
                                .add(
                                    egui::Button::new(t.tab_debug)
                                        .selected(self.active_tab == Tab::Debug),
                                )
                                .clicked()
                            {
                                self.active_tab = Tab::Debug;
                            }
                            ui.separator();
                        }
                        let mut high_contrast = self.high_contrast;
                        if ui
                            .checkbox(&mut high_contrast, t.chk_high_contrast)
                            .on_hover_text(t.hover_high_contrast)
                            .changed()
                        {
                            self.set_high_contrast(&ctx, high_contrast);
                        }
                        ui.menu_button(t.btn_options, |ui| {
                            if ui
                                .checkbox(&mut self.close_to_tray, t.chk_close_to_tray)
                                .on_hover_text(t.hover_close_to_tray)
                                .changed()
                            {
                                self.save_global_settings();
                            }
                            ui.separator();
                            // Не персистится (см. UiState::recording) — читаем
                            // текущее состояние атомика напрямую, а не из
                            // отдельного поля UiState, чтобы не было двух
                            // источников истины между этим чекбоксом и тем,
                            // что реально видит wt_worker.
                            let mut recording_now = self.recording.load(Ordering::Relaxed);
                            if ui
                                .checkbox(&mut recording_now, t.chk_record_wt_session)
                                .on_hover_text(t.hover_record_wt_session)
                                .changed()
                            {
                                self.recording.store(recording_now, Ordering::Relaxed);
                            }
                            ui.separator();
                            // Ручной оверрайд автоопределения активной игры.
                            // Обычный режим — Auto: какая игра живая, ту и
                            // показываем (см. переход по ActiveGame в начале
                            // show_main). Force* прижимает конкретную игру
                            // независимо от реальной живости другой — сама
                            // veto-логика живёт в воркерах (sim/worker.rs,
                            // wt_link/worker.rs), здесь только контрол.
                            ui.label(t.lbl_game_override);
                            let mut changed = false;
                            changed |= ui
                                .radio_value(
                                    &mut self.game_override,
                                    GameOverride::Auto,
                                    t.opt_game_auto,
                                )
                                .changed();
                            changed |= ui
                                .radio_value(
                                    &mut self.game_override,
                                    GameOverride::ForceMsfs,
                                    t.opt_game_force_msfs,
                                )
                                .changed();
                            changed |= ui
                                .radio_value(
                                    &mut self.game_override,
                                    GameOverride::ForceWt,
                                    t.opt_game_force_wt,
                                )
                                .changed();
                            changed |= ui
                                .radio_value(
                                    &mut self.game_override,
                                    GameOverride::ForceXplane,
                                    t.opt_game_force_xp,
                                )
                                .changed();
                            if changed {
                                self.save_global_settings();
                            }
                            ui.separator();
                            if ui.button(t.tray_check_updates).clicked() {
                                updater::spawn_check(HWND(0), env!("CARGO_PKG_VERSION"));
                                ui.close();
                            }
                        });
                        if ui.button(t.hover_help).clicked() {
                            self.show_help = true;
                            ui.close();
                        }
                        if ui.button(t.hover_help_us).clicked() {
                            self.show_help_us = true;
                            ui.close();
                        }
                    });

                    let ru_btn = ui.add(egui::Button::new("RU").selected(self.lang == Lang::Ru));
                    let en_btn = ui.add(egui::Button::new("EN").selected(self.lang == Lang::En));

                    let new_lang = if en_btn.clicked() {
                        Some(Lang::En)
                    } else if ru_btn.clicked() {
                        Some(Lang::Ru)
                    } else {
                        None
                    };
                    if let Some(new_lang) = new_lang
                        && new_lang != self.lang
                    {
                        self.lang = new_lang;
                        i18n::set(new_lang);
                        tray::refresh_tooltip();
                        self.save_global_settings();
                    }

                    ui.add_space(10.0);
                    let compact = ui.available_width() < 210.0;
                    let throttle_ok = self.throttle_connected.load(Ordering::Relaxed);
                    controller_badge_dot(
                        ui,
                        DeviceIcon::Throttle,
                        t.throttle,
                        throttle_ok,
                        compact,
                        t,
                    );
                    let controller_ok = self.controller_connected.load(Ordering::Relaxed);
                    controller_badge_dot(
                        ui,
                        DeviceIcon::Joystick,
                        t.sidestick,
                        controller_ok,
                        compact,
                        t,
                    );
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
            // Автоопределение активной игры: как только active_game меняется
            // (воркеры заявили/освободили GameSlot), один раз переключаем
            // активную секцию — раньше это делал .changed() у чекбокса WT,
            // теперь источник истины не пользовательский клик, а сам факт
            // смены владельца слота.
            let ag = *self.active_game.lock();
            if ag != self.last_seen_game {
                // Редактор эффектов — единственная секция, не привязанная к
                // конкретной игре: его открывают, чтобы СОБРАТЬ эффект, и
                // запуск симулятора посреди работы не должен выкидывать из
                // него (наблюдалось вживую: MSFS подключился, и открытая
                // страница сменилась на Аэродинамику вместе с потерей места в
                // прокрутке). Остальные секции показывают телеметрию активной
                // игры, поэтому для них переключение по-прежнему верное.
                if self.active_section != Section::Effects {
                    self.active_section = match ag {
                        ActiveGame::Wt => Section::Wt,
                        _ => Section::Rumble,
                    };
                }
                self.last_seen_game = ag;
            }

            if ag == ActiveGame::None {
                // Ни одна игра не обнаружена — все секции, завязанные на
                // телеметрию конкретной игры (Rumble/Taxi/Engines/Gear/Wt/
                // Telemetry), по-прежнему недоступны: показывать им нечего.
                // НО конструктор пользовательских эффектов ("Редактор эффектов")
                // от игры не зависит — его собирают и отлаживают заранее,
                // поэтому навигация всё же рисуется (не прячется целиком, как
                // было раньше), просто с единственным включённым пунктом.
                // Live Monitor по-прежнему скрыт: показывать в нём нечего же.
                let nav_panel_width = if self.lang == Lang::Ru { 206.0 } else { 188.0 };
                egui::Panel::left("nav_panel")
                    .resizable(false)
                    .exact_size(nav_panel_width)
                    .frame(
                        egui::Frame::side_top_panel(ui.style())
                            .fill(palette::get().bg_sidebar)
                            .inner_margin(egui::Margin::same(8)),
                    )
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.add_enabled_ui(false, |ui| {
                            nav_item(ui, false, NavIcon::Aero, t.nav_rumble);
                            nav_item(ui, false, NavIcon::Taxi, t.nav_taxi);
                            nav_item(ui, false, NavIcon::Engine, t.nav_engines);
                            nav_item(ui, false, NavIcon::Gear, t.nav_gear);
                            ui.separator();
                            nav_item(ui, false, NavIcon::Telemetry, t.nav_telemetry);
                        });
                        if nav_item(
                            ui,
                            self.active_section == Section::Effects,
                            NavIcon::Editor,
                            t.nav_effects,
                        ) {
                            self.active_section = Section::Effects;
                        }
                    });

                egui::CentralPanel::default().show(ui, |ui| {
                    if self.active_section == Section::Effects {
                        // Живой телеметрии нет ни для одного источника — та же
                        // ветка ActiveGame::None, что и ниже в основном
                        // dispatch'е секций (см. её комментарий). Все графики
                        // источника корректно показывают lbl_fx_no_signal.
                        let active_ids_guard = self.active_custom_ids.lock();
                        let mut ectx = effects_editor::EditorCtx {
                            effects: &self.custom_fx,
                            active_ids: active_ids_guard.as_slice(),
                            live: None,
                            active_game: ag,
                            t,
                            lang: self.lang,
                            logs: &self.logs,
                            tx_hid: &self.tx_hid,
                            preview: &self.preview_lock,
                            preview_player: &self.preview_player,
                        };
                        effects_editor::show(ui, &mut self.fx_editor, &mut ectx);
                        drop(active_ids_guard);
                    } else {
                        // Тот же нейтральный экран ожидания, что и раньше —
                        // просто теперь виден только пока не выбрана Effects.
                        ui.vertical_centered(|ui| {
                            ui.add_space(80.0);
                            ui.heading(t.heading_game_not_detected);
                            ui.add_space(8.0);
                            ui.label(t.msg_game_not_detected);
                        });
                    }
                });
            } else {
                // Общий снимок "включён/активен" для каждого эффекта — используется и
                // бейджами счётчика в навигации слева, и списком Live Monitor справа,
                // чтобы не считать дважды.
                let mon = self.config.get();
                // Единый снимок пользовательских эффектов + текущего борта на этот
                // кадр нужен ВСЕГДА: оба движка (встроенный и пользовательский) считаются
                // одновременно (см. custom_fx::overrides), поэтому
                // Live Monitor ниже сводит builtin- и custom-строки в один список, а
                // `builtin_override_mask` отмечает, какие встроенные эффекты вытеснены
                // (для пометки "заменён", а не "выключен") и переиспользуется дальше по
                // кадру карточками встроенных эффектов в CentralPanel.
                let custom_snapshot = self.custom_fx.get();
                let aircraft_snapshot = self.aircraft_title.lock().clone();
                let builtin_override_mask =
                    overrides::overridden_builtins(&custom_snapshot, ag, &aircraft_snapshot);
                // Порядок элементов соответствует новой группировке по разделам
                // (Aerodynamics / Taxi / Engines / Gears) — см. диапазоны ниже.
                // Четвёртое поле — текущая интенсивность эффекта в процентах (та же
                // формула val/native_max*100, что использует слайдер самой карточки),
                // None — для triggered-по-порогу эффектов без единого "уровня"
                // (Gear Transit, а также ЛЮБОЙ пользовательский эффект — у них нет
                // единого "уровня", кривая+форма произвольные). Пятое поле — вытеснен ли
                // этот встроенный эффект пользовательским на текущем кадре (задача 1в):
                // всегда `false` для самих пользовательских строк, добавленных ниже.
                // War Thunder (ag == ActiveGame::Wt, встроенный движок): builtin-список —
                // 6 эффектов этого режима, MSFS-эффекты в этот момент не считаются (см.
                // гейт в sim/worker.rs), показывать их в Live Monitor было бы вводящим
                // в заблуждение.
                let mut rows: Vec<(&str, bool, bool, Option<f32>, bool)> = if ag == ActiveGame::Wt {
                    vec![
                        (
                            t.name_wt_weapon1,
                            mon.wt.weapon1_enabled,
                            self.effects.wt_weapon1_active.load(Ordering::Relaxed),
                            None,
                            builtin_override_mask.wt_weapon1,
                        ),
                        (
                            t.name_wt_weapon2,
                            mon.wt.weapon2_enabled,
                            self.effects.wt_weapon2_active.load(Ordering::Relaxed),
                            None,
                            builtin_override_mask.wt_weapon2,
                        ),
                        (
                            t.name_flaps,
                            mon.wt.flaps_enabled,
                            self.effects.flaps_bump_active.load(Ordering::Relaxed),
                            Some((mon.wt.flaps_peak / 255.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.wt_flaps,
                        ),
                        (
                            t.lbl_gear_transit,
                            mon.wt.gear_transit_enabled,
                            self.effects.gear_transit_active.load(Ordering::Relaxed),
                            None,
                            builtin_override_mask.wt_gear_transit,
                        ),
                        (
                            t.name_wt_stall,
                            mon.wt.stall_enabled,
                            self.effects.stall_active.load(Ordering::Relaxed),
                            Some((mon.wt.stall_ceiling / 255.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.wt_stall,
                        ),
                        (
                            t.name_wt_engine_start,
                            mon.wt.engine_start_enabled,
                            self.effects.engine_start_active.load(Ordering::Relaxed),
                            Some((mon.wt.engine_start_peak / 255.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.wt_engine_start,
                        ),
                    ]
                } else {
                    vec![
                        (
                            t.overspeed_effect_name,
                            mon.overspeed_enabled,
                            self.effects.overspeed_active.load(Ordering::Relaxed),
                            Some((mon.overspeed_intensity / 255.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.overspeed,
                        ),
                        (
                            t.name_stall,
                            mon.stall_enabled,
                            self.effects.stall_active.load(Ordering::Relaxed),
                            Some((mon.stall_ceiling / 255.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.stall,
                        ),
                        (
                            t.name_spoilers,
                            mon.spoilers_enabled,
                            self.effects.spoilers_active.load(Ordering::Relaxed),
                            Some((mon.spoilers_intensity / 250.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.spoilers,
                        ),
                        (
                            t.name_flaps,
                            mon.flaps_enabled,
                            self.effects.flaps_bump_active.load(Ordering::Relaxed),
                            Some((mon.flaps_peak / 255.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.flaps,
                        ),
                        (
                            t.lbl_bank_turb,
                            mon.bank_enabled,
                            self.effects.bank_active.load(Ordering::Relaxed),
                            Some((mon.bank_intensity / 200.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.bank,
                        ),
                        (
                            t.name_engine_start,
                            mon.enable_engine_start,
                            self.effects.engine_start_active.load(Ordering::Relaxed),
                            Some((mon.engine_start_strength / 255.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.engine_start,
                        ),
                        (
                            t.name_ground_roll,
                            mon.ground_enabled,
                            self.effects.ground_active.load(Ordering::Relaxed)
                                || self.effects.ground_thump_active.load(Ordering::Relaxed),
                            Some((mon.ground_roll / 50.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.ground,
                        ),
                        (
                            t.name_left_peak,
                            mon.gear_comp_left_enabled,
                            self.effects.gear_comp_left_active.load(Ordering::Relaxed),
                            Some((mon.gear_comp_left_peak / 55.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.gear_comp,
                        ),
                        (
                            t.name_nose_peak,
                            mon.gear_comp_nose_enabled,
                            self.effects.gear_comp_nose_active.load(Ordering::Relaxed),
                            Some((mon.gear_comp_nose_peak / 55.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.gear_comp,
                        ),
                        (
                            t.name_right_peak,
                            mon.gear_comp_right_enabled,
                            self.effects.gear_comp_right_active.load(Ordering::Relaxed),
                            Some((mon.gear_comp_right_peak / 55.0 * 100.0).clamp(0.0, 100.0)),
                            builtin_override_mask.gear_comp,
                        ),
                        (
                            t.lbl_gear_transit,
                            mon.gear_transit_enabled,
                            self.effects.gear_transit_active.load(Ordering::Relaxed),
                            None,
                            builtin_override_mask.gear_transit,
                        ),
                    ]
                };
                // Пользовательские эффекты теперь ВСЕГДА показываются рядом со
                // встроенными (не вместо них, см. doc-комментарий выше) — задача 1в.
                // Гвард мьютекса живёт только на время этого блока: чуть ниже по кадру
                // (CentralPanel, Section::Effects) тот же self.active_custom_ids снова
                // блокируется, а parking_lot::Mutex не реентерабелен даже в одном потоке.
                {
                    let active_guard = self.active_custom_ids.lock();
                    for e in &custom_snapshot {
                        let active = active_guard.iter().any(|id| id == &e.id);
                        rows.push((e.name.as_str(), e.enabled, active, None, false));
                    }
                }
                let nav_panel_width = if self.lang == Lang::Ru { 206.0 } else { 188.0 };
                egui::Panel::left("nav_panel")
                    .resizable(false)
                    .exact_size(nav_panel_width)
                    .frame(
                        egui::Frame::side_top_panel(ui.style())
                            .fill(palette::get().bg_sidebar)
                            .inner_margin(egui::Margin::same(8)),
                    )
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        // Кириллические подписи заметно длиннее английских, поэтому кнопка
                        // должна переноситься на 2 строки, а не выходить за границы панели.
                        // Это простой список разделов без индикации срабатывания эффектов —
                        // такая индикация есть только в карточках эффектов и Live Monitor.
                        // Раньше здесь ещё был параметр `muted`, приглушавший встроенные
                        // разделы целиком, пока активны пользовательские эффекты.
                        // Глобального переключателя движков больше нет: движки работают
                        // вместе, вытеснение точечное и видно в самой карточке
                        // (см. custom_fx::overrides и `overridden_by_note` ниже),
                        // поэтому разделы навигации всегда полноценные.

                        if ag == ActiveGame::Wt {
                            if nav_item(
                                ui,
                                self.active_section == Section::Wt,
                                NavIcon::Weapon,
                                t.nav_wt,
                            ) {
                                self.active_section = Section::Wt;
                            }
                        } else {
                            if nav_item(
                                ui,
                                self.active_section == Section::Rumble,
                                NavIcon::Aero,
                                t.nav_rumble,
                            ) {
                                self.active_section = Section::Rumble;
                            }
                            if nav_item(
                                ui,
                                self.active_section == Section::Taxi,
                                NavIcon::Taxi,
                                t.nav_taxi,
                            ) {
                                self.active_section = Section::Taxi;
                            }
                            if nav_item(
                                ui,
                                self.active_section == Section::Engines,
                                NavIcon::Engine,
                                t.nav_engines,
                            ) {
                                self.active_section = Section::Engines;
                            }
                            if nav_item(
                                ui,
                                self.active_section == Section::Gear,
                                NavIcon::Gear,
                                t.nav_gear,
                            ) {
                                self.active_section = Section::Gear;
                            }
                        }
                        ui.separator();
                        if nav_item(
                            ui,
                            self.active_section == Section::Telemetry,
                            NavIcon::Telemetry,
                            t.nav_telemetry,
                        ) {
                            self.active_section = Section::Telemetry;
                        }
                        // Как и Telemetry, виден во ВСЕХ играх — конструктор
                        // не привязан к конкретному конвейеру телеметрии.
                        if nav_item(
                            ui,
                            self.active_section == Section::Effects,
                            NavIcon::Editor,
                            t.nav_effects,
                        ) {
                            self.active_section = Section::Effects;
                        }
                    });

                // Компактный, всегда развёрнутый Live Monitor: раньше сворачивался в
                // узкую полоску безымянных точек по умолчанию — теперь фиксированные
                // 160/220px (EN/RU) с именем + реальным %, никакого режима "просто точки".
                let live_monitor_width = if self.monitor_collapsed {
                    24.0
                } else if self.lang == Lang::Ru {
                    270.0
                } else {
                    250.0
                };
                // Редактору эффектов нужна вся ширина под графики шагов, а
                // свою живую картинку он показывает сам.
                if self.active_section != Section::Effects {
                    egui::Panel::right("live_monitor_panel")
                        .resizable(false)
                        .exact_size(live_monitor_width)
                        .frame(
                            egui::Frame::side_top_panel(ui.style())
                                .fill(palette::get().bg_sidebar)
                                .inner_margin(egui::Margin::same(10)),
                        )
                        .show(ui, |ui| {
                            if self.monitor_collapsed {
                                if ui
                                    .button("◀")
                                    .on_hover_text(t.hover_monitor_expand)
                                    .clicked()
                                {
                                    self.monitor_collapsed = false;
                                    self.save_global_settings();
                                }
                                return;
                            }

                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(t.heading_live_monitor)
                                        .family(palette::semibold())
                                        .size(14.5),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .button("▶")
                                            .on_hover_text(t.hover_monitor_collapse)
                                            .clicked()
                                        {
                                            self.monitor_collapsed = true;
                                            self.save_global_settings();
                                        }
                                    },
                                );
                            });
                            ui.separator();

                            let [stick, throttle_left, throttle_right] =
                                self.motor_history.motors();
                            motor_trace(
                                ui,
                                t.lbl_fx_out_joystick,
                                palette::get().accent_live,
                                stick,
                            );
                            motor_trace(
                                ui,
                                t.lbl_fx_out_throttle_left,
                                palette::get().accent_primary,
                                throttle_left,
                            );
                            motor_trace(
                                ui,
                                t.lbl_fx_out_throttle_right,
                                palette::get().accent_primary,
                                throttle_right,
                            );
                            ui.separator();

                            let disabled_count =
                                rows.iter().filter(|(_, enabled, ..)| !*enabled).count();
                            // Только то, что работает сейчас. Эффект держится в списке
                            // через live_level: иначе строки прыгали бы на каждый тик
                            // мотора. Вытесненные эффекты не показываем, их состояние
                            // написано на самой строке эффекта.
                            let mut any_firing = false;
                            for (name, enabled, active, pct, replaced) in &rows {
                                let key = ui.id().with(("monitor", *name));
                                let level = live_level(ui, key, *enabled && *active && !*replaced);
                                if level <= 0.0 {
                                    continue;
                                }
                                any_firing = true;
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(*name).size(12.0))
                                            .truncate(),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let value = match pct {
                                                Some(p) => format!("{p:.0}%"),
                                                None => t.status_active.to_string(),
                                            };
                                            ui.colored_label(
                                                palette::get().accent_live,
                                                RichText::new(value).monospace().size(11.5),
                                            );
                                        },
                                    );
                                });
                                let (bar, _) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 3.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter()
                                    .rect_filled(bar, 1u8, palette::get().surface_control);
                                let filled = pct.unwrap_or(100.0) / 100.0 * level;
                                let mut lit = bar;
                                lit.set_width(bar.width() * filled.clamp(0.0, 1.0));
                                ui.painter()
                                    .rect_filled(lit, 1u8, palette::get().accent_live);
                            }
                            if !any_firing {
                                ui.weak(t.lbl_no_active_effects);
                            }

                            if disabled_count > 0 {
                                ui.add_space(4.0);
                                let label = i18n::lbl_disabled_count(self.lang, disabled_count);
                                if ui
                                    .selectable_label(
                                        self.monitor_show_disabled,
                                        RichText::new(label).size(10.0).weak(),
                                    )
                                    .clicked()
                                {
                                    self.monitor_show_disabled = !self.monitor_show_disabled;
                                }
                                if self.monitor_show_disabled {
                                    for (name, _enabled, _active, _pct, _replaced) in
                                        rows.iter().filter(|(_, enabled, ..)| !*enabled)
                                    {
                                        ui.horizontal(|ui| {
                                            dot_indicator(
                                                ui,
                                                palette::get().text_disabled,
                                                false,
                                                8.0,
                                            );
                                            ui.add_enabled(
                                                false,
                                                egui::Label::new(RichText::new(*name).size(10.0))
                                                    .wrap(),
                                            );
                                        });
                                    }
                                }
                            }
                        });
                }

                egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::CollapsingHeader::new(t.heading_aircraft_profiles)
                            .default_open(false)
                            .show(ui, |ui| {
                                let (profiles_snapshot, active_match) = {
                                    let ap = self.aircraft_profiles.lock();
                                    (ap.profiles.clone(), ap.active_match.clone())
                                };
                                if profiles_snapshot.is_empty() {
                                    ui.label(t.empty_profiles_hint);
                                }
                                let mut delete: Option<usize> = None;
                                let mut apply: Option<usize> = None;
                                for (i, p) in profiles_snapshot.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(&p.match_substring);
                                        if active_match.as_deref()
                                            == Some(p.match_substring.as_str())
                                        {
                                            ui.label(
                                                RichText::new("✔").color(palette::get().accent_primary),
                                            )
                                            .on_hover_text(t.hover_active_profile);
                                        }
                                        if ui
                                            .button("⬇")
                                            .on_hover_text(t.hover_apply_profile)
                                            .clicked()
                                        {
                                            apply = Some(i);
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
                                if let Some(i) = apply
                                    && let Some(p) = profiles_snapshot.get(i)
                                {
                                    self.config.set(p.config.clone());
                                    self.profile_state.lock().force_recheck();
                                    self.logs.push(format!(
                                        "Aircraft profile: copied settings from '{}' into current config (unsaved)",
                                        p.match_substring
                                    ));
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

                        // Имена пользовательских эффектов, вытеснивших каждый встроенный
                        // на этом кадре — считаем ДО with_mut, потому что self.config.
                        // with_mut(|cfg| ...) ниже занимает `self` через захват других
                        // полей (active_section, effects, logs, tx_hid...) внутри
                        // замыкания, доступа к self.custom_fx/self.aircraft_title там
                        // больше нет. builtin_override_mask/custom_snapshot/
                        // aircraft_snapshot уже посчитаны выше для Live Monitor —
                        // переиспользуем тот же снимок кадра, а не считаем ещё раз.
                        let overspeed_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.overspeed,
                            builtin_override_mask.overspeed,
                        );
                        let stall_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.stall,
                            builtin_override_mask.stall,
                        );
                        let spoilers_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.spoilers,
                            builtin_override_mask.spoilers,
                        );
                        let flaps_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.flaps,
                            builtin_override_mask.flaps,
                        );
                        let bank_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.bank,
                            builtin_override_mask.bank,
                        );
                        let taxi_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.taxi,
                            builtin_override_mask.taxi,
                        );
                        let ground_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.ground,
                            builtin_override_mask.ground,
                        );
                        let engine_start_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.engine_start,
                            builtin_override_mask.engine_start,
                        );
                        let gear_comp_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.gear_comp,
                            builtin_override_mask.gear_comp,
                        );
                        let gear_transit_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.gear_transit,
                            builtin_override_mask.gear_transit,
                        );
                        let wt_weapon1_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.wt_weapon1,
                            builtin_override_mask.wt_weapon1,
                        );
                        let wt_weapon2_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.wt_weapon2,
                            builtin_override_mask.wt_weapon2,
                        );
                        let wt_stall_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.wt_stall,
                            builtin_override_mask.wt_stall,
                        );
                        let wt_overspeed_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.wt_overspeed,
                            builtin_override_mask.wt_overspeed,
                        );
                        let wt_gear_overspeed_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.wt_gear_overspeed,
                            builtin_override_mask.wt_gear_overspeed,
                        );
                        let wt_flaps_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.wt_flaps,
                            builtin_override_mask.wt_flaps,
                        );
                        let wt_gear_transit_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.wt_gear_transit,
                            builtin_override_mask.wt_gear_transit,
                        );
                        let wt_engine_start_overridden = overriding_effect_name(
                            &custom_snapshot,
                            ag,
                            &aircraft_snapshot,
                            |m| m.wt_engine_start,
                            builtin_override_mask.wt_engine_start,
                        );

                        self.config.with_mut(|cfg| {
                            cfg.split_touchdown = split_touchdown_auto;
                            cfg.joystick_hw_connected = joystick_hw_connected;
                            cfg.throttle_hw_connected = throttle_hw_connected;
                            match self.active_section {
                                Section::Rumble => {
                                    ui.heading(t.nav_rumble);
                                    ui.add_space(4.0);
                                    effects_legend(ui, t, true);
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
                                                overspeed_enabled && overspeed_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = overspeed_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    UiState::overspeed_row(
                                                        ui,
                                                        &mut overspeed_enabled,
                                                        overspeed_threshold_kn,
                                                        active,
                                                        &mut _changed,
                                                        &mut overspeed_override,
                                                        &mut overspeed_manual_kn,
                                                        &mut cfg.overspeed_intensity,
                                                        &mut cfg.device_targets.overspeed,
                                                        t,
                                                    );
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
                                                stall_enabled && stall_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = stall_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.name_stall,
                                                        &mut cfg.stall_ceiling,
                                                        10.0, // жёсткий потолок — см. STALL_CEILING_HARD_CAP в rumble.rs
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
                                                spoilers_enabled && spoilers_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = spoilers_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
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
                                                        ui.add_space(PARAM_INDENT);
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
                                                flaps_enabled && flaps_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = flaps_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
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
                                            UiState::effect_card(
                                                col,
                                                bank_enabled && bank_overridden.is_none(),
                                                active,
                                                |ui| {
                                                if let Some(name) = bank_overridden {
                                                    overridden_by_note(ui, t, name);
                                                }
                                                UiState::effect_row_percent_hinted(
                                                    ui,
                                                    t.lbl_bank_turb,
                                                    &mut cfg.bank_intensity,
                                                    200.0,
                                                    &mut bank_enabled,
                                                    active,
                                                    &mut _changed,
                                                    Some(t.hover_bank_intensity),
                                                    &mut cfg.device_targets.bank,
                                                    t,
                                                );

                                                ui.horizontal(|ui| {
                                                    ui.add_space(PARAM_INDENT);
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
                                    effects_legend(ui, t, true);
                                    ui.vertical(|ui| {
                                        // Taxi Thump Bounds (Start + End + advanced Period curve)
                                        {
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                taxi_overridden.is_none(),
                                                taxi_start_crossed || taxi_end_crossed,
                                                |ui| {
                                                    if let Some(name) = taxi_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
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
                                                        ui.add_space(PARAM_INDENT);
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
                                                ground_enabled && ground_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = ground_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
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
                                    effects_legend(ui, t, false);
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
                                                engine_start_enabled
                                                    && engine_start_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = engine_start_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    UiState::effect_row_percent(
                                                        ui,
                                                        t.name_engine_start,
                                                        &mut cfg.engine_start_strength,
                                                        255.0,
                                                        &mut engine_start_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(engine_start_hint),
                                                        None,
                                                        t,
                                                    );
                                                    cfg.enable_engine_start = engine_start_enabled;

                                                    ui.horizontal(|ui| {
                                                        ui.add_space(PARAM_INDENT);
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
                                    effects_legend(ui, t, true);
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
                                                    left_enabled && gear_comp_overridden.is_none(),
                                                    active,
                                                    |ui| {
                                                        if let Some(name) = gear_comp_overridden {
                                                            overridden_by_note(ui, t, name);
                                                        }
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
                                                gear_transit_enabled
                                                    && gear_transit_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = gear_transit_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    if UiState::effect_line(ui, t.lbl_gear_transit, None, &mut gear_transit_enabled, active, t, |ui, _| {
                                                                Self::device_target_toggle(
                                                                    ui,
                                                                    &mut cfg
                                                                        .device_targets
                                                                        .gear_transit,
                                                                    &mut _changed,
                                                                    t,
                                                                );
                                                            }) { _changed = true; }
                                                },
                                            );
                                            cfg.gear_transit_enabled = gear_transit_enabled;
                                        }
                                    });
                                }

                                Section::Wt => {
                                    ui.heading(t.nav_wt);
                                    ui.add_space(4.0);
                                    effects_legend(ui, t, true);
                                    ui.vertical(|ui| {
                                        // Weapon1 — маршрутизация зафиксирована (только
                                        // джойстик), поэтому здесь нет ни слайдера силы,
                                        // ни переключателя устройства (см. wt_link::rumble).
                                        {
                                            let mut enabled = cfg.wt.weapon1_enabled;
                                            let active = self
                                                .effects
                                                .wt_weapon1_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                enabled && wt_weapon1_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = wt_weapon1_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    if UiState::effect_line(ui, t.name_wt_weapon1, Some(t.hover_wt_weapon1), &mut enabled, active, t, |_, _| {}) { _changed = true; }
                                                },
                                            );
                                            cfg.wt.weapon1_enabled = enabled;
                                        }

                                        // Weapon2 — маршрутизация зафиксирована (только
                                        // РУД, оба мотора).
                                        {
                                            let mut enabled = cfg.wt.weapon2_enabled;
                                            let active = self
                                                .effects
                                                .wt_weapon2_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                enabled && wt_weapon2_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = wt_weapon2_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    if UiState::effect_line(ui, t.name_wt_weapon2, Some(t.hover_wt_weapon2), &mut enabled, active, t, |_, _| {}) { _changed = true; }
                                                },
                                            );
                                            cfg.wt.weapon2_enabled = enabled;
                                        }

                                        // Stall/буффет срыва потока — v1: только Bf 109 F-4,
                                        // на остальных бортах эффект молчит (см. hover).
                                        {
                                            let mut stall_enabled = cfg.wt.stall_enabled;
                                            let active =
                                                self.effects.stall_active.load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                stall_enabled && wt_stall_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = wt_stall_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.name_wt_stall,
                                                        &mut cfg.wt.stall_ceiling,
                                                        80.0, // жёсткий потолок — см. WT_STALL_CEILING_HARD_CAP в wt_link/rumble.rs
                                                        &mut stall_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(t.hover_wt_stall),
                                                        &mut cfg.wt.device_targets.stall,
                                                        t,
                                                    );
                                                },
                                            );
                                            cfg.wt.stall_enabled = stall_enabled;
                                        }

                                        // Overspeed (Vne) — таблица порогов на ~1300 бортов,
                                        // см. wt_link::overspeed_profiles.
                                        {
                                            let mut overspeed_enabled = cfg.wt.overspeed_enabled;
                                            let active = self
                                                .effects
                                                .wt_overspeed_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                overspeed_enabled
                                                    && wt_overspeed_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = wt_overspeed_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.name_wt_overspeed,
                                                        &mut cfg.wt.overspeed_ceiling,
                                                        80.0,
                                                        &mut overspeed_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(t.hover_wt_overspeed),
                                                        &mut cfg.wt.device_targets.overspeed,
                                                        t,
                                                    );
                                                },
                                            );
                                            cfg.wt.overspeed_enabled = overspeed_enabled;
                                        }

                                        // Gear overspeed — отдельный от Vne-эффекта, окно 20 км/ч,
                                        // порог из той же таблицы data/flap_gear_break.csv.
                                        {
                                            let mut gear_overspeed_enabled =
                                                cfg.wt.gear_overspeed_enabled;
                                            let active = self
                                                .effects
                                                .wt_gear_overspeed_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                gear_overspeed_enabled
                                                    && wt_gear_overspeed_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) =
                                                        wt_gear_overspeed_overridden
                                                    {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.name_wt_gear_overspeed,
                                                        &mut cfg.wt.gear_overspeed_ceiling,
                                                        80.0,
                                                        &mut gear_overspeed_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(t.hover_wt_gear_overspeed),
                                                        &mut cfg.wt.device_targets.gear_overspeed,
                                                        t,
                                                    );
                                                },
                                            );
                                            cfg.wt.gear_overspeed_enabled = gear_overspeed_enabled;
                                        }

                                        // Flaps
                                        {
                                            let mut flaps_enabled = cfg.wt.flaps_enabled;
                                            let active = self
                                                .effects
                                                .flaps_bump_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                flaps_enabled && wt_flaps_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = wt_flaps_overridden {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.name_flaps,
                                                        &mut cfg.wt.flaps_peak,
                                                        255.0,
                                                        &mut flaps_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(t.hover_wt_flaps),
                                                        &mut cfg.wt.device_targets.flaps,
                                                        t,
                                                    );
                                                },
                                            );
                                            cfg.wt.flaps_enabled = flaps_enabled;
                                        }

                                        // Gear Transit & Doors
                                        {
                                            let mut gear_transit_enabled =
                                                cfg.wt.gear_transit_enabled;
                                            let active = self
                                                .effects
                                                .gear_transit_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                gear_transit_enabled
                                                    && wt_gear_transit_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = wt_gear_transit_overridden
                                                    {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.lbl_gear_transit,
                                                        &mut cfg.wt.gear_peak,
                                                        255.0,
                                                        &mut gear_transit_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(t.hover_wt_gear_transit),
                                                        &mut cfg.wt.device_targets.gear_transit,
                                                        t,
                                                    );
                                                },
                                            );
                                            cfg.wt.gear_transit_enabled = gear_transit_enabled;
                                        }

                                        // Engine Start/Stop — моно (без раскладки по
                                        // двигателям/сторонам), см. hover.
                                        {
                                            let mut engine_start_enabled =
                                                cfg.wt.engine_start_enabled;
                                            let active = self
                                                .effects
                                                .engine_start_active
                                                .load(Ordering::Relaxed);
                                            let col = &mut *ui;
                                            UiState::effect_card(
                                                col,
                                                engine_start_enabled
                                                    && wt_engine_start_overridden.is_none(),
                                                active,
                                                |ui| {
                                                    if let Some(name) = wt_engine_start_overridden
                                                    {
                                                        overridden_by_note(ui, t, name);
                                                    }
                                                    UiState::effect_row_percent_hinted(
                                                        ui,
                                                        t.name_wt_engine_start,
                                                        &mut cfg.wt.engine_start_peak,
                                                        255.0,
                                                        &mut engine_start_enabled,
                                                        active,
                                                        &mut _changed,
                                                        Some(t.hover_wt_engine_start),
                                                        &mut cfg.wt.device_targets.engine_start,
                                                        t,
                                                    );
                                                },
                                            );
                                            cfg.wt.engine_start_enabled = engine_start_enabled;
                                        }
                                    });
                                }

                                Section::Telemetry => {}
                                // Собственная отрисовка — см. else-if чуть ниже (после
                                // закрытия этого with_mut): конструктору эффектов не
                                // нужен cfg (RumbleConfig), а нужен доступ к десятку
                                // других полей self одновременно, который проще
                                // получить уже после того, как self.config
                                // разблокируется.
                                Section::Effects => {}
                            }

                            if _changed {
                                // Конфиг уже обновлен через with_mut
                            }
                        });

                        ui.add_space(8.0);
                        if self.active_section != Section::Effects && ui.button(t.btn_reset_defaults).clicked() {
                            // Только сбрасывает ЖИВОЙ конфиг — на диск ничего не пишет,
                            // как и любое другое изменение. Нажмите Save (дискета в
                            // верхней панели), чтобы зафиксировать сброс.
                            self.config.set(RumbleConfig::default());
                        }
                        ui.add_space(8.0);
                        ui.separator();

                        if self.active_section == Section::Telemetry && ag == ActiveGame::Wt {
                            ui.heading(t.heading_wt_telemetry);
                            egui::Grid::new("wt_telemetry")
                                .num_columns(2)
                                .spacing(Vec2::new(20.0, 4.0))
                                .show(ui, |ui| {
                                    let v = self.last_wt_vars.lock().clone();
                                    match v {
                                        Some(v) => {
                                            let status_text = if !v.in_mission {
                                                t.wt_status_menu
                                            } else {
                                                t.wt_status_in_battle
                                            };
                                            ui.label(t.nav_wt);
                                            ui.label(status_text);
                                            ui.end_row();

                                            ui.label(t.lbl_wt_vehicle_type);
                                            ui.label(if v.vehicle_type.is_empty() {
                                                t.lbl_wt_vehicle_type_unknown.to_string()
                                            } else {
                                                v.vehicle_type.clone()
                                            });
                                            ui.end_row();

                                            ui.label(t.lbl_wt_speed_kt);
                                            ui.label(format!("{:.0}", v.speed_kt));
                                            ui.end_row();

                                            ui.label(t.lbl_wt_altitude_ft);
                                            ui.label(format!("{:.0}", v.altitude_ft));
                                            ui.end_row();

                                            ui.label(t.name_wt_weapon1);
                                            ui.label(v.weapon1_firing.to_string());
                                            ui.end_row();

                                            ui.label(t.lbl_wt_weapon1_ammo);
                                            ui.label(match v.weapon1_ammo {
                                                Some(n) => format!("{n:.0}"),
                                                None => t.lbl_wt_ammo_unknown.to_string(),
                                            });
                                            ui.end_row();

                                            ui.label(t.name_wt_weapon2);
                                            ui.label(v.weapon2_firing.to_string());
                                            ui.end_row();

                                            ui.label(t.lbl_wt_weapon2_ammo);
                                            ui.label(match v.weapon2_ammo {
                                                Some(n) => format!("{n:.0}"),
                                                None => t.lbl_wt_ammo_unknown.to_string(),
                                            });
                                            ui.end_row();

                                            ui.label(t.lbl_wt_flaps_pct);
                                            ui.label(format!("{:.0}", v.flaps_pct));
                                            ui.end_row();

                                            ui.label(t.lbl_wt_gear_pct);
                                            ui.label(format!("{:.0}", v.gear_pct));
                                            ui.end_row();

                                            ui.label(t.lbl_wt_aoa_deg);
                                            ui.label(format!("{:.1}", v.aoa_deg));
                                            ui.end_row();

                                            ui.label(t.lbl_wt_wx_deg_s);
                                            ui.label(format!("{:.1}", v.wx_deg_s));
                                            ui.end_row();

                                            ui.label(t.lbl_wt_rpm1);
                                            ui.label(format!("{:.0}", v.rpm_1));
                                            ui.end_row();
                                        }
                                        None => {
                                            ui.label(t.nav_wt);
                                            ui.label(t.wt_status_disconnected);
                                            ui.end_row();
                                        }
                                    }
                                });
                        } else if self.active_section == Section::Telemetry {
                            ui.columns(2, |columns| {
                                // --- Левая колонка: общая телеметрия борта ---
                                let ui = &mut columns[0];
                                ui.heading(t.heading_live_aircraft_data);

                                egui::Grid::new("aircraft_data")
                                    .num_columns(2)
                                    .spacing(Vec2::new(20.0, 4.0))
                                    .show(ui, |ui| {
                                        // FlightVars больше не Copy (добавлен
                                        // словарь lvars) — явный .clone().
                                        let v = self.last_vars.lock().clone();
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
                                        let v = self.last_vars.lock().clone();
                                        match v {
                                            Some(v) => {
                                                let combustion_label =
                                                    |ui: &mut egui::Ui, active: bool| {
                                                        if active {
                                                            ui.colored_label(
                                                                palette::get().status_ok,
                                                                t.val_on,
                                                            );
                                                        } else {
                                                            ui.colored_label(
                                                                palette::get().text_disabled,
                                                                t.val_off,
                                                            );
                                                        }
                                                    };

                                                ui.label(RichText::new(t.eng1_header).strong());
                                                ui.label("");
                                                ui.end_row();

                                                ui.label(t.lbl_n2);
                                                ui.horizontal(|ui| {
                                                    ui.label(format!("{:.1}%", v.eng1_n2_percent));
                                                    if ui
                                                        .small_button(t.btn_set_n2_idle)
                                                        .on_hover_text(t.hover_set_n2_idle)
                                                        .clicked()
                                                    {
                                                        let new_val =
                                                            (v.eng1_n2_percent - 1.5).clamp(10.0, 100.0) as f32;
                                                        self.config.with_mut(|cfg| {
                                                            cfg.engine_idle_n2 = new_val;
                                                        });
                                                        self.logs.push(
                                                            t.log_n2_idle_set.replace("{val:.1}", &format!("{:.1}", new_val)),
                                                        );
                                                    }
                                                });
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
                        } else if self.active_section == Section::Effects {
                            // Живой снимок телеметрии для конструктора — та же пара
                            // last_vars/last_wt_vars, что и у обычной секции Telemetry
                            // чуть выше, просто завёрнутая в TelemetryFrame, который
                            // понимает custom_fx::sources::read.
                            let live = match ag {
                                ActiveGame::Wt => {
                                    self.last_wt_vars.lock().clone().map(TelemetryFrame::Wt)
                                }
                                ActiveGame::None => None,
                                ActiveGame::Msfs | ActiveGame::Xplane => self
                                    .last_vars
                                    .lock()
                                    .clone()
                                    .map(TelemetryFrame::Flight),
                            };
                            let active_ids_guard = self.active_custom_ids.lock();
                            let mut ectx = effects_editor::EditorCtx {
                                effects: &self.custom_fx,
                                active_ids: active_ids_guard.as_slice(),
                                live,
                                active_game: ag,
                                t,
                                lang: self.lang,
                                logs: &self.logs,
                                tx_hid: &self.tx_hid,
                                preview: &self.preview_lock,
                                preview_player: &self.preview_player,
                            };
                            effects_editor::show(ui, &mut self.fx_editor, &mut ectx);
                            drop(active_ids_guard);
                        }
                    });
            });
            }
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
                        // Visible(true) — окно могло быть спрятано в трей
                        // (close_to_tray), а не свёрнуто; см. tray::bring_main_to_front.
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
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

#[cfg(test)]
mod palette_tests {
    use super::palette::MONO;
    use egui::Color32;

    fn luminance(c: Color32) -> f32 {
        let channel = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn high_contrast_palette_has_no_color() {
        for c in [
            MONO.bg_app,
            MONO.bg_card,
            MONO.surface_control_hover,
            MONO.surface_control_active,
            MONO.accent_live,
            MONO.accent_primary,
            MONO.status_bad,
            MONO.status_ok,
            MONO.status_warn,
            MONO.status_attention,
            MONO.danger,
            MONO.selected_bg,
            MONO.text_primary,
            MONO.text_secondary,
            MONO.text_disabled,
            MONO.rail,
        ] {
            assert!(c.r() == c.g() && c.g() == c.b(), "{c:?}");
        }
    }

    #[test]
    fn high_contrast_text_meets_wcag_aaa() {
        for text in [MONO.text_primary, MONO.text_secondary, MONO.text_disabled] {
            assert!(contrast(text, MONO.bg_app) >= 7.0);
            assert!(contrast(text, MONO.bg_card) >= 7.0);
        }
        assert!(contrast(MONO.selected_fg, MONO.selected_bg) >= 7.0);
        assert!(contrast(MONO.on_danger, MONO.danger) >= 7.0);
        // Рельс должен быть различим и на чёрном фоне, и рядом с белой заливкой.
        assert!(contrast(MONO.rail, MONO.bg_app) >= 3.0);
        assert!(contrast(MONO.rail, MONO.selected_bg) >= 3.0);
    }

    /// Windows в светлом режиме. eframe сообщает системную тему, и egui берёт
    /// светлый стиль: тёмный текст на тёмных карточках, которые красятся
    /// палитрой напрямую. Проверяем оба порядка: тема известна до apply и
    /// приходит уже после него, как на первом кадре.
    #[test]
    fn theme_stays_dark_when_windows_uses_light_mode() {
        let light = || egui::RawInput {
            system_theme: Some(egui::Theme::Light),
            ..Default::default()
        };
        for light_known_before_apply in [false, true] {
            let ctx = egui::Context::default();
            if light_known_before_apply {
                let _ = ctx.run_ui(light(), |_| {});
            }
            super::palette::apply(&ctx, false);
            let _ = ctx.run_ui(light(), |_| {});
            let style = ctx.global_style();
            assert!(style.visuals.dark_mode, "{light_known_before_apply}");
            assert_eq!(
                style.visuals.panel_fill,
                super::palette::DARK.bg_app,
                "{light_known_before_apply}"
            );
        }
    }
}
