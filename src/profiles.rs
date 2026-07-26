// Встроенные профили самолётов: у некоторых бортов телеметрия SimConnect
// или физическое поведение отличается от "стандартного" настолько, что
// глобальные дефолты RumbleConfig дают неверный эффект. Профиль применяется
// автоматически по подстроке в aircraft title (см. sim/worker.rs) — без
// участия пользователя, "из коробки".
use crate::{ConfigShared, LogBuffer, RumbleConfig};

#[derive(Debug, Clone, Copy, Default)]
pub struct AircraftOverrides {
    pub spoilers_threshold_pct: Option<f64>,
    pub engine_idle_n2: Option<f32>,
    pub flaps_track_slats: Option<bool>,
    pub overspeed_lear_horn_enabled: Option<bool>,
}

const BUILT_IN_PROFILES: &[(&str, AircraftOverrides)] = &[
    (
        "MADDOG",
        AircraftOverrides {
            // В убранном положении спойлеров телеметрия держит ложные ~10%
            // вместо фактических 0% — порог берём с запасом (12%) на шум
            // телеметрии, чтобы эффект не срабатывал на самом деле убранных
            // спойлерах.
            spoilers_threshold_pct: Some(12.0),
            // MADDOG выходит на Idle при N2 ≈ 57.8%, а не при дефолтных 60%.
            engine_idle_n2: Some(57.0),
            // На MADDOG предкрылки убираются не одновременно с закрылками —
            // они держатся выпущенными до ПОСЛЕДНЕГО щелчка ручки закрылков
            // (когда flaps_pct уже 0), и только тогда падают в 0. Если
            // ловить движение только по flaps_pct, это последнее движение
            // ручки (реальная работа мотора) останется без эффекта — поэтому
            // для этого борта дополнительно следим за slats_pct.
            flaps_track_slats: Some(true),
            overspeed_lear_horn_enabled: None,
        },
    ),
    (
        "LEARJET",
        AircraftOverrides {
            spoilers_threshold_pct: None,
            engine_idle_n2: None,
            flaps_track_slats: None,
            // Экспериментально: на Flysimware Learjet 35A дополнительно
            // используем L:XMLSND75 (клаксон "overspeed / mach trim" из
            // sound.xml аддона) как альтернативный триггер эффекта Overspeed
            // — см. rumble.rs. Включается ТОЛЬКО для этого борта (подстрока
            // "LEARJET" в title), на остальных самолётах этот L-var не
            // определён и был бы бессмысленным сигналом.
            overspeed_lear_horn_enabled: Some(true),
        },
    ),
];

/// Ищет встроенный профиль по подстроке (регистронезависимо) в aircraft title.
fn find_built_in(title: &str) -> Option<(&'static str, &'static AircraftOverrides)> {
    let upper = title.to_uppercase();
    BUILT_IN_PROFILES
        .iter()
        .find(|(name, _)| upper.contains(name))
        .map(|(name, overrides)| (*name, overrides))
}

/// true, если для этого борта (по подстроке в title) есть встроенный профиль
/// с кастомной логикой эффектов (MADDOG, LEARJET, ...). Используется в UI,
/// чтобы визуально отметить название самолёта, для которого что-то отличается
/// от общих дефолтов.
pub fn has_built_in_profile(title: &str) -> bool {
    find_built_in(title).is_some()
}

// Подстроки (регистронезависимо), по которым борт считается PMDG — только для
// UI-индикатора (см. is_pmdg_aircraft ниже). TITLE из SimConnect у части
// ливрей/вариантов не содержит слова "PMDG" вообще (например "Boeing 737-800
// WestJet...", "B737-700"), поэтому детекция идёт и по бренду, и по типовым
// обозначениям семейств 737/777, которые в MSFS на практике встречаются
// только у PMDG (других payware 737/777 нет).
const PMDG_TITLE_MARKERS: &[&str] = &[
    "PMDG",
    "737-600", "737-700", "737-800", "737-900",
    "7376", "7377", "7378", "7379",
    "B736", "B737", "B738", "B739",
    "B737-6", "B737-7", "B737-8", "B737-9",
    "777-200", "777-300", "777F",
    "777-200ER", "777-200LR", "777-300ER",
    "B772", "B773", "B77W", "B77F", "B77L",
];

/// true, если aircraft title (регистронезависимо) соответствует одному из
/// маркеров PMDG. В отличие от has_built_in_profile, НЕ завязано на таблицу
/// AircraftOverrides (у PMDG нет переопределений RumbleConfig) — сама
/// кастомная логика (pre-spool разгон по L:EngineStart1b/2b_Ext) в rumble.rs
/// работает через самонейтрализующиеся L-vars и не нуждается в этой функции;
/// она нужна только для UI-индикатора "у этого борта не дефолтная логика".
pub fn is_pmdg_aircraft(title: &str) -> bool {
    let upper = title.to_uppercase();
    PMDG_TITLE_MARKERS.iter().any(|marker| upper.contains(marker))
}

fn apply(cfg: &mut RumbleConfig, overrides: &AircraftOverrides) {
    if let Some(v) = overrides.spoilers_threshold_pct {
        cfg.spoilers_threshold_pct = v;
    }
    if let Some(v) = overrides.engine_idle_n2 {
        cfg.engine_idle_n2 = v;
    }
    if let Some(v) = overrides.flaps_track_slats {
        cfg.flaps_track_slats = v;
    }
    if let Some(v) = overrides.overspeed_lear_horn_enabled {
        cfg.overspeed_lear_horn_enabled = v;
    }
}

/// Отслеживает переключения между самолётами и накатывает/откатывает
/// встроенные оверрайды поверх ЖИВОГО ConfigShared, не трогая то, что
/// пользователь сохранил на диск (settings.rs) как свои базовые настройки.
///
/// Без этого отката значения, зашитые под конкретный борт (например MADDOG),
/// утекли бы в settings.json (автосохранение по rev, см. ui.rs) и стали бы
/// дефолтом для ВСЕХ остальных самолётов после первого же вылета на MADDOG.
#[derive(Default)]
pub struct ProfileState {
    // Значения полей ДО применения текущего оверрайда — восстанавливаются,
    // когда борт больше не подпадает ни под один встроенный профиль.
    base: Option<(f64, f32, bool, bool)>,
}

impl ProfileState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Сбрасывает запомненный "базовый" снимок, чтобы следующий вызов
    /// on_aircraft_changed заново снял базу с ТЕКУЩЕГО состояния ConfigShared
    /// (например, после того как aircraft_profiles.rs только что подставил
    /// туда другой профиль/default) вместо повторного использования базы от
    /// предыдущего борта.
    pub fn force_recheck(&mut self) {
        self.base = None;
    }

    /// Возвращает копию cfg, где поля, которые мог переписать встроенный
    /// оверлей (spoilers_threshold_pct, engine_idle_n2, flaps_track_slats,
    /// overspeed_lear_horn_enabled), возвращены к "базовым" значениям, если
    /// такой оверлей сейчас активен. Вызывается перед сохранением на диск
    /// (см. aircraft_profiles::save_active), чтобы значения, зашитые под
    /// конкретный борт, не утекли в сохранённый профиль/default (см. ремарку
    /// у ProfileState выше).
    pub fn sanitize_for_save(&self, cfg: &RumbleConfig) -> RumbleConfig {
        let mut out = cfg.clone();
        if let Some((
            spoilers_threshold_pct,
            engine_idle_n2,
            flaps_track_slats,
            overspeed_lear_horn_enabled,
        )) = self.base
        {
            out.spoilers_threshold_pct = spoilers_threshold_pct;
            out.engine_idle_n2 = engine_idle_n2;
            out.flaps_track_slats = flaps_track_slats;
            out.overspeed_lear_horn_enabled = overspeed_lear_horn_enabled;
        }
        out
    }

    /// Вызывать при каждой смене aircraft_title (не на каждый тик!).
    pub fn on_aircraft_changed(&mut self, config: &ConfigShared, title: &str, logs: &LogBuffer) {
        match find_built_in(title) {
            Some((name, overrides)) => {
                config.with_mut(|cfg| {
                    if self.base.is_none() {
                        self.base = Some((
                            cfg.spoilers_threshold_pct,
                            cfg.engine_idle_n2,
                            cfg.flaps_track_slats,
                            cfg.overspeed_lear_horn_enabled,
                        ));
                    }
                    apply(cfg, overrides);
                });
                logs.push(format!(
                    "Aircraft profile: applied built-in overrides for '{}' (matched '{}')",
                    title, name
                ));
            }
            None => {
                if let Some((
                    spoilers_threshold_pct,
                    engine_idle_n2,
                    flaps_track_slats,
                    overspeed_lear_horn_enabled,
                )) = self.base.take()
                {
                    config.with_mut(|cfg| {
                        cfg.spoilers_threshold_pct = spoilers_threshold_pct;
                        cfg.engine_idle_n2 = engine_idle_n2;
                        cfg.flaps_track_slats = flaps_track_slats;
                        cfg.overspeed_lear_horn_enabled = overspeed_lear_horn_enabled;
                    });
                    logs.push("Aircraft profile: left known aircraft, restored base config".to_string());
                }
            }
        }
    }
}
