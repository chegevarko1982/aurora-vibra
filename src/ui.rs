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

use windows::Win32::Foundation::HWND;

use crate::{
    ActiveGame, ConfigShared, EffectDeviceTarget, EffectsShared, FlightVars, GameOverride, HidCmd,
    LogBuffer, RumbleConfig, SimStatus, UiCmd,
    aircraft_profiles::{self, AircraftProfile, AircraftProfiles},
    custom_fx::{
        model::{CustomEffect, new_effect},
        overrides::{self, BuiltinEffect, BuiltinMask},
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
pub(crate) mod palette {
    use egui::Color32;

    // Фоны. Раньше BG_APP был почти чистый чёрный (#0B0E14) — на неоткалиброванных
    // мониторах он сливался с текстом egui по умолчанию (gray(140)). Подняли всю
    // лестницу поверхностей, чтобы карточка/панель/фон различались и на плохом
    // экране, и чтобы у текста была нормальная подложка.
    pub const BG_APP: Color32 = Color32::from_rgb(0x0F, 0x14, 0x1C);
    pub const BG_SIDEBAR: Color32 = Color32::from_rgb(0x15, 0x1B, 0x25);
    pub const BG_CARD: Color32 = Color32::from_rgb(0x1E, 0x26, 0x32);
    pub const BG_CARD_DISABLED: Color32 = Color32::from_rgb(0x17, 0x1E, 0x28);

    /// Подложка кнопок/полей в покое. Ключевой момент доступности: без неё
    /// невыбранные пункты навигации не имели вообще никакого фона и «появлялись»
    /// только под курсором (жалоба тестировщика).
    pub const SURFACE_CONTROL: Color32 = Color32::from_rgb(0x2A, 0x33, 0x44);
    pub const SURFACE_CONTROL_HOVER: Color32 = Color32::from_rgb(0x38, 0x44, 0x5A);
    pub const SURFACE_CONTROL_ACTIVE: Color32 = Color32::from_rgb(0x46, 0x55, 0x6F);

    pub const BORDER_DEFAULT: Color32 = Color32::from_rgb(0x3A, 0x46, 0x5A);
    pub const BORDER_ACTIVE: Color32 = Color32::from_rgb(0x5A, 0x6E, 0x8C);

    pub const ACCENT_PRIMARY: Color32 = Color32::from_rgb(0x7D, 0xB3, 0xFF);
    pub const ACCENT_LIVE: Color32 = Color32::from_rgb(0x22, 0xD3, 0xEE);

    /// Статусы в верхней панели. Красный был #C83C3C — всего ~3.6:1 на BG_APP,
    /// то есть самая важная строка («не подключено») читалась хуже остальных.
    pub const STATUS_BAD: Color32 = Color32::from_rgb(0xFF, 0x6B, 0x6B);
    pub const STATUS_WARN: Color32 = Color32::from_rgb(0xDC, 0xB4, 0x28);
    pub const STATUS_OK: Color32 = Color32::from_rgb(0x3E, 0xC9, 0x74);
    pub const STATUS_ATTENTION: Color32 = Color32::from_rgb(0xE6, 0x82, 0x1E);

    /// Подложка выбранного пункта — непрозрачная тёмно-синяя, а не
    /// полупрозрачный акцент поверх фона: раньше выбранный пункт был синим
    /// текстом на синей заливке (~3:1) и читался хуже невыбранного.
    pub const SELECTED_BG: Color32 = Color32::from_rgb(0x1E, 0x3A, 0x5F);
    pub const SELECTED_FG: Color32 = Color32::from_rgb(0x93, 0xC5, 0xFD);
    /// Тинт иконки устройства во включённом состоянии: там под ней светлая
    /// заливка (#CCCEFF), поэтому сама иконка должна быть тёмной.
    pub const DEVICE_ON_FG: Color32 = Color32::from_rgb(0x1E, 0x3A, 0x5F);

    /// Базовый цвет текста. egui::Visuals::dark() ставит gray(140) — формально
    /// это ~5.7:1 на нашем фоне, но на практике тонкий шрифт такого тона
    /// читается как «серо-чёрный по чёрному». Держим ~15:1.
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xE8, 0xEE, 0xF7);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0xB4, 0xC0, 0xD0);
    pub const TEXT_DISABLED: Color32 = Color32::from_rgb(0x8C, 0x9A, 0xAE);

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
        visuals.selection.bg_fill = SELECTED_BG;
        visuals.selection.stroke.color = SELECTED_FG;
        visuals.hyperlink_color = ACCENT_PRIMARY;

        // Текст. Все три «тона» задаём явно, иначе egui считает weak-текст как
        // base * 0.6 alpha (≈2.4:1 на тёмном фоне — нечитаемо).
        visuals.weak_text_color = Some(TEXT_SECONDARY);
        // disabled_alpha перемножается с override_text_color карточки и с
        // add_enabled_ui(false) у слайдеров: 0.5 давало ~1.5:1 у выключенного
        // эффекта. 0.8 оставляет выключенное заметно тусклее включённого, но
        // всё ещё читаемым.
        visuals.disabled_alpha = 0.8;

        let w = &mut visuals.widgets;
        // noninteractive — обычные ui.label(); bg_stroke здесь же красит
        // ui.separator() и линии отступов.
        w.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);
        w.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER_DEFAULT);
        w.noninteractive.weak_bg_fill = BG_APP;
        w.noninteractive.bg_fill = BG_APP;

        w.inactive.weak_bg_fill = SURFACE_CONTROL;
        w.inactive.bg_fill = SURFACE_CONTROL;
        w.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER_DEFAULT);
        w.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);

        w.hovered.weak_bg_fill = SURFACE_CONTROL_HOVER;
        w.hovered.bg_fill = SURFACE_CONTROL_HOVER;
        w.hovered.bg_stroke = egui::Stroke::new(1.0, BORDER_ACTIVE);
        w.hovered.fg_stroke = egui::Stroke::new(1.5, Color32::WHITE);

        w.active.weak_bg_fill = SURFACE_CONTROL_ACTIVE;
        w.active.bg_fill = SURFACE_CONTROL_ACTIVE;
        w.active.bg_stroke = egui::Stroke::new(1.0, ACCENT_PRIMARY);
        w.active.fg_stroke = egui::Stroke::new(2.0, Color32::WHITE);

        w.open.weak_bg_fill = SURFACE_CONTROL;
        w.open.bg_fill = SURFACE_CONTROL;
        w.open.bg_stroke = egui::Stroke::new(1.0, BORDER_DEFAULT);
        w.open.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);

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

/// Заливка под «включённой» иконкой устройства — одна константа на кнопку в
/// карточке и на образец в легенде, чтобы они выглядели одинаково.
const DEVICE_ON_BG: Color32 = Color32::from_rgb(0xCC, 0xCE, 0xFF);

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
            .fill(DEVICE_ON_BG)
            .corner_radius(3u8)
            .inner_margin(egui::Margin::same(2))
            .show(ui, |ui| {
                ui.add(device_icon_image(icon, palette::DEVICE_ON_FG, 14.0));
            });
        ui.label(
            RichText::new(label)
                .small()
                .strong()
                .color(palette::TEXT_SECONDARY),
        );
    }

    fn note(ui: &mut egui::Ui, text: &str) {
        ui.label(RichText::new(text).small().color(palette::TEXT_DISABLED));
    }

    egui::Frame::new()
        .fill(palette::BG_CARD_DISABLED)
        .stroke(egui::Stroke::new(1.0, palette::BORDER_DEFAULT))
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
                            .color(palette::TEXT_SECONDARY),
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
                        .color(palette::TEXT_SECONDARY),
                );

                ui.label(
                    RichText::new(t.status_off)
                        .small()
                        .color(palette::TEXT_DISABLED),
                );
                note(ui, t.legend_state_off);
                ui.add_space(6.0);

                dot_indicator(ui, palette::BORDER_ACTIVE, false, 8.0);
                ui.label(
                    RichText::new(t.status_idle)
                        .small()
                        .color(palette::TEXT_SECONDARY),
                );
                note(ui, t.legend_state_idle);
                ui.add_space(6.0);

                dot_indicator(ui, Color32::WHITE, true, 8.0);
                ui.label(RichText::new(t.status_active).small().color(Color32::WHITE));
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
        .color(palette::STATUS_ATTENTION)
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
                .color(palette::TEXT_DISABLED),
        );
        return;
    }
    let (dot, text, color) = if active {
        (Color32::WHITE, t.status_active, Color32::WHITE)
    } else {
        (
            palette::BORDER_ACTIVE,
            t.status_idle,
            palette::TEXT_SECONDARY,
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
        SimStatus::Disconnected => (t.disconnected, palette::STATUS_BAD, false),
        SimStatus::Connected => (t.connected, palette::STATUS_WARN, false),
        SimStatus::InFlight => (t.in_flight, palette::STATUS_OK, true),
        SimStatus::SimConnectMissing => (t.simconnect_missing, palette::STATUS_ATTENTION, true),
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

fn controller_badge_dot(ui: &mut egui::Ui, label: &str, connected: bool, t: &Strings) {
    let (color, filled) = if connected {
        (palette::STATUS_OK, true)
    } else {
        (palette::STATUS_BAD, false)
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
        crate::settings::set_game_override(self.game_override);
        let ap = self.aircraft_profiles.lock();
        let _ = crate::settings::save(&crate::settings::SettingsFile {
            default: ap.default.clone(),
            profiles: ap.profiles.clone(),
            lang: self.lang,
            close_to_tray: self.close_to_tray,
            simconnect_dll_path: crate::settings::simconnect_dll_path(),
            monitor_collapsed: self.monitor_collapsed,
            // wt_enabled — legacy-поле, производное от game_override (см.
            // комментарий у поля в SettingsFile): единственный источник
            // истины теперь game_override, wt_enabled существует только для
            // миграции файлов, сохранённых сборкой до этой фичи.
            wt_enabled: self.game_override == GameOverride::ForceWt,
            game_override: self.game_override,
        });
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
        // Во включённом состоянии под иконкой светлая заливка (DEVICE_ON_BG),
        // поэтому тинт там должен быть ТЁМНЫМ — раньше он брался из
        // selection.stroke.color, и после подъёма акцента до светло-голубого
        // иконка стала бы светлой по светлому.
        let tint = if selected {
            palette::DEVICE_ON_FG
        } else {
            ui.visuals().text_color()
        };
        let image = device_icon_image(icon, tint, 19.2);
        let mut button = egui::Button::new(image).selected(selected);
        if selected {
            // Явный fill вместо стандартной полупрозрачной selection.bg_fill —
            // просили конкретную подложку под включённой иконкой устройства,
            // не трогая глобальный акцент выбора (он используется и в других
            // местах — nav, чекбоксы).
            button = button.fill(DEVICE_ON_BG);
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
                .stop_preview(&self.tx_hid, &self.preview_lock);
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
                        // пустая строка -> отступ, всё остальное -> абзац/пункт
                        // с переносом по ширине окна.
                        for line in t.help_text.lines() {
                            if let Some(h) = line.strip_prefix("## ") {
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

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let st = *self.status.lock();
                status_badge(ui, &st, t);
                // Верхняя панель рисуется раньше блока show_main (где для
                // шага секций уже читается active_game в отдельную `ag`) —
                // читаем Mutex здесь же отдельно, а не полагаемся на порядок
                // между двумя блоками кода (дешёвая lock, не проблема).
                let top_ag = *self.active_game.lock();
                game_badge(ui, top_ag, t);
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

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(4.0);

                // "Check for updates" перенесена в оверфлоу-меню "..." → Options
                // (см. ниже) — вместо отдельной кнопки в тулбаре.

                // Stop/Resume перенесены в левую колонку навигации, под пункт
                // Telemetry — см. nav_panel ниже.

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ru_btn = ui.add(egui::Button::new("RU").selected(self.lang == Lang::Ru));
                    let en_btn = ui.add(egui::Button::new("EN").selected(self.lang == Lang::En));

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
                let nav_panel_width = if self.lang == Lang::Ru { 190.0 } else { 150.0 };
                egui::Panel::left("nav_panel")
                    .resizable(false)
                    .exact_size(nav_panel_width)
                    .frame(egui::Frame::side_top_panel(ui.style()).fill(palette::BG_SIDEBAR))
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        let nav_item_disabled = |ui: &mut egui::Ui, label: &str| {
                            let w = ui.available_width();
                            ui.add_enabled(
                                false,
                                egui::Button::new(label).wrap().min_size(Vec2::new(w, 0.0)),
                            );
                        };
                        nav_item_disabled(ui, t.nav_rumble);
                        nav_item_disabled(ui, t.nav_taxi);
                        nav_item_disabled(ui, t.nav_engines);
                        nav_item_disabled(ui, t.nav_gear);
                        ui.separator();
                        nav_item_disabled(ui, t.nav_telemetry);
                        let w = ui.available_width();
                        if ui
                            .add(
                                egui::Button::new(t.nav_effects)
                                    .selected(self.active_section == Section::Effects)
                                    .wrap()
                                    .min_size(Vec2::new(w, 0.0)),
                            )
                            .clicked()
                        {
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
                let nav_panel_width = if self.lang == Lang::Ru { 190.0 } else { 150.0 };
                egui::Panel::left("nav_panel")
                    .resizable(false)
                    .exact_size(nav_panel_width)
                    .frame(egui::Frame::side_top_panel(ui.style()).fill(palette::BG_SIDEBAR))
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        // Кириллические подписи заметно длиннее английских, поэтому кнопка
                        // должна переноситься на 2 строки, а не выходить за границы панели.
                        // Это простой список разделов без индикации срабатывания эффектов —
                        // такая индикация есть только в карточках эффектов и Live Monitor.
                        // Button::new(..).selected(..) вместо Button::selectable(..):
                        // у selectable невыбранная кнопка рисуется вообще без рамки
                        // и заливки, поэтому пункты навигации проявлялись только под
                        // курсором. Здесь рамка есть всегда, на всю ширину панели.
                        // Раньше здесь ещё был параметр `muted`, приглушавший встроенные
                        // разделы целиком, пока активны пользовательские эффекты.
                        // Глобального переключателя движков больше нет: движки работают
                        // вместе, вытеснение точечное и видно в самой карточке
                        // (см. custom_fx::overrides и `overridden_by_note` ниже),
                        // поэтому разделы навигации всегда полноценные.
                        let nav_item = |ui: &mut egui::Ui, selected: bool, label: &str| -> bool {
                            let w = ui.available_width();
                            ui.add(
                                egui::Button::new(RichText::new(label))
                                    .selected(selected)
                                    .wrap()
                                    .min_size(Vec2::new(w, 0.0)),
                            )
                            .clicked()
                        };
                        if ag == ActiveGame::Wt {
                            if nav_item(ui, self.active_section == Section::Wt, t.nav_wt) {
                                self.active_section = Section::Wt;
                            }
                        } else {
                            if nav_item(ui, self.active_section == Section::Rumble, t.nav_rumble) {
                                self.active_section = Section::Rumble;
                            }
                            if nav_item(ui, self.active_section == Section::Taxi, t.nav_taxi) {
                                self.active_section = Section::Taxi;
                            }
                            if nav_item(ui, self.active_section == Section::Engines, t.nav_engines)
                            {
                                self.active_section = Section::Engines;
                            }
                            if nav_item(ui, self.active_section == Section::Gear, t.nav_gear) {
                                self.active_section = Section::Gear;
                            }
                        }
                        ui.separator();
                        if nav_item(
                            ui,
                            self.active_section == Section::Telemetry,
                            t.nav_telemetry,
                        ) {
                            self.active_section = Section::Telemetry;
                        }
                        // Как и Telemetry, виден во ВСЕХ играх — конструктор
                        // не привязан к конкретному конвейеру телеметрии.
                        if nav_item(ui, self.active_section == Section::Effects, t.nav_effects) {
                            self.active_section = Section::Effects;
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
                let live_monitor_width = if self.monitor_collapsed {
                    24.0
                } else if self.lang == Lang::Ru {
                    220.0
                } else {
                    160.0
                };
                egui::Panel::right("live_monitor_panel")
                    .resizable(false)
                    .exact_size(live_monitor_width)
                    .frame(egui::Frame::side_top_panel(ui.style()).fill(palette::BG_SIDEBAR))
                    .show(ui, |ui| {
                        ui.add_space(4.0);
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
                            ui.label(RichText::new(t.heading_live_monitor).strong());
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

                        let enabled_rows: Vec<_> =
                            rows.iter().filter(|(_, enabled, ..)| *enabled).collect();
                        let disabled_count = rows.len() - enabled_rows.len();

                        if enabled_rows.is_empty() {
                            ui.weak(t.lbl_no_active_effects);
                        }
                        for (name, enabled, active, pct, replaced) in enabled_rows {
                            ui.horizontal(|ui| {
                                // Задача 1в: вытесненный встроенный эффект помечается
                                // отдельным состоянием "заменён", а не сливается с обычным
                                // idle/active — иначе выглядел бы так, будто может
                                // сработать, хотя вытеснение гарантирует, что он молчит.
                                let (dot_color, filled) = if *replaced {
                                    (palette::TEXT_DISABLED, false)
                                } else if *enabled && *active {
                                    (palette::ACCENT_LIVE, true)
                                } else {
                                    (palette::BORDER_ACTIVE, false)
                                };
                                // Значение резервирует место первым (right_to_left), затем
                                // точка + имя получают оставшуюся ширину и переносятся, если
                                // русская подпись не помещается в одну строку.
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let value_text = if *replaced {
                                            t.lbl_replaced_by_custom.to_string()
                                        } else if *active {
                                            match pct {
                                                Some(p) => format!("{p:.0}%"),
                                                None => t.status_active.to_string(),
                                            }
                                        } else {
                                            "—".to_string()
                                        };
                                        let color = if *replaced {
                                            palette::TEXT_DISABLED
                                        } else if *active {
                                            palette::ACCENT_LIVE
                                        } else {
                                            palette::TEXT_SECONDARY
                                        };
                                        ui.colored_label(
                                            color,
                                            RichText::new(value_text).size(10.0),
                                        );
                                        ui.with_layout(
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                dot_indicator(ui, dot_color, filled, 8.0);
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(*name).size(10.0),
                                                    )
                                                    .wrap(),
                                                );
                                            },
                                        );
                                    },
                                );
                            });
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
                                        dot_indicator(ui, palette::TEXT_DISABLED, false, 8.0);
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
                                                RichText::new("✔").color(palette::ACCENT_PRIMARY),
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
                                                    ui.horizontal(|ui| {
                                                        if ui.checkbox(&mut enabled, "").changed()
                                                        {
                                                            _changed = true;
                                                        }
                                                        ui.label(
                                                            RichText::new(t.name_wt_weapon1)
                                                                .strong(),
                                                        )
                                                        .on_hover_text(t.hover_wt_weapon1);
                                                        ui.with_layout(
                                                            egui::Layout::right_to_left(
                                                                egui::Align::Center,
                                                            ),
                                                            |ui| {
                                                                effect_status_badge(
                                                                    ui, enabled, active, t,
                                                                );
                                                            },
                                                        );
                                                    });
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
                                                    ui.horizontal(|ui| {
                                                        if ui.checkbox(&mut enabled, "").changed()
                                                        {
                                                            _changed = true;
                                                        }
                                                        ui.label(
                                                            RichText::new(t.name_wt_weapon2)
                                                                .strong(),
                                                        )
                                                        .on_hover_text(t.hover_wt_weapon2);
                                                        ui.with_layout(
                                                            egui::Layout::right_to_left(
                                                                egui::Align::Center,
                                                            ),
                                                            |ui| {
                                                                effect_status_badge(
                                                                    ui, enabled, active, t,
                                                                );
                                                            },
                                                        );
                                                    });
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
                        if ui.button(t.btn_reset_defaults).clicked() {
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
                                                                palette::STATUS_OK,
                                                                t.val_on,
                                                            );
                                                        } else {
                                                            ui.colored_label(
                                                                palette::TEXT_DISABLED,
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
