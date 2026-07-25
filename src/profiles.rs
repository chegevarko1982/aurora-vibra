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
    base: Option<(f64, f32, bool)>,
}

impl ProfileState {
    pub fn new() -> Self {
        Self::default()
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
                if let Some((spoilers_threshold_pct, engine_idle_n2, flaps_track_slats)) =
                    self.base.take()
                {
                    config.with_mut(|cfg| {
                        cfg.spoilers_threshold_pct = spoilers_threshold_pct;
                        cfg.engine_idle_n2 = engine_idle_n2;
                        cfg.flaps_track_slats = flaps_track_slats;
                    });
                    logs.push("Aircraft profile: left known aircraft, restored base config".to_string());
                }
            }
        }
    }
}
