// Пользовательские профили конфига по самолётам. В отличие от
// src/profiles.rs (захардкоженные, code-only оверрайды нескольких полей для
// MADDOG/LEARJET), это полноценные снимки RumbleConfig, которые пользователь
// сам создаёт/переименовывает/удаляет через UI и сохраняет на диск явной
// кнопкой (см. ui.rs) — структура RumbleConfig остаётся единой для всех
// самолётов, отличается только набор сохранённых значений.
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::{profiles::ProfileState, settings, ConfigShared, LogBuffer, RumbleConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AircraftProfile {
    pub match_substring: String,
    pub config: RumbleConfig,
}

/// Ищет первый профиль, чья match_substring содержится (регистронезависимо)
/// в aircraft title — та же логика, что и profiles::find_built_in.
pub fn find_matching_index(profiles: &[AircraftProfile], title: &str) -> Option<usize> {
    let upper = title.to_uppercase();
    profiles.iter().position(|p| {
        !p.match_substring.trim().is_empty() && upper.contains(&p.match_substring.to_uppercase())
    })
}

/// Живое состояние набора профилей: то, что сейчас сохранено на диске (или
/// будет при следующем Save), плюс какой профиль сейчас активен для текущего
/// самолёта.
#[derive(Default)]
pub struct AircraftProfiles {
    pub default: RumbleConfig,
    pub profiles: Vec<AircraftProfile>,
    // match_substring активного именного профиля; None = активен default.
    pub active_match: Option<String>,
    // ConfigShared::current_rev(), снятый сразу после того, как apply_for_aircraft
    // в последний раз подставил в живой конфиг то, что реально лежит на диске
    // (либо только что было туда сохранено). UI сравнивает с текущим rev, чтобы
    // показать индикатор "есть несохранённые изменения" — это НЕ то же самое,
    // что просто "рёв поменялся с прошлого кадра", т.к. смена самолёта тоже
    // меняет rev, но это не пользовательское изменение.
    pub loaded_rev: u64,
}

/// Подбирает конфиг для нового самолёта (именной профиль по подстроке, иначе
/// default), применяет его к ЖИВОМУ ConfigShared и заново прогоняет встроенный
/// оверлей (profiles.rs) поверх — используется и при смене борта, и при
/// ручной перезагрузке с диска (кнопка Load), поэтому вынесено в общую функцию.
pub fn apply_for_aircraft(
    ap: &mut AircraftProfiles,
    config: &ConfigShared,
    profile_state: &mut ProfileState,
    title: &str,
    logs: &LogBuffer,
) {
    let new_match = find_matching_index(&ap.profiles, title).map(|i| ap.profiles[i].match_substring.clone());
    let new_cfg = match &new_match {
        Some(m) => ap
            .profiles
            .iter()
            .find(|p| &p.match_substring == m)
            .expect("just resolved by find_matching_index")
            .config
            .clone(),
        None => ap.default.clone(),
    };
    config.set(new_cfg);
    if new_match != ap.active_match {
        logs.push(match &new_match {
            Some(m) => format!("Aircraft profile: loaded saved profile '{}' for '{}'", m, title),
            None => format!("Aircraft profile: no saved profile for '{}', loaded default", title),
        });
    }
    ap.active_match = new_match;
    // base=None заставляет on_aircraft_changed заново снять "базовые" значения
    // с только что загруженного конфига, прежде чем (при необходимости) снова
    // наложить встроенные оверрайды MADDOG/LEARJET.
    profile_state.force_recheck();
    profile_state.on_aircraft_changed(config, title, logs);
    ap.loaded_rev = config.current_rev();
}

/// Сохраняет ЖИВОЙ (уже очищенный от built-in оверрайдов, см.
/// ProfileState::sanitize_for_save) конфиг в активный на данный момент профиль
/// (именной или default) и пишет весь набор профилей на диск.
///
/// `also_default` — чекбокс в UI: если включён, конфиг ДОПОЛНИТЕЛЬНО
/// записывается и в default (даже когда активен именной профиль текущего
/// борта), т.е. станет применяться ко всем самолётам без своего профиля.
/// Если именного профиля сейчас нет, конфиг и так уходит в default —
/// параметр в этом случае не даёт дублирующего эффекта.
pub fn save_active(
    shared: &Mutex<AircraftProfiles>,
    sanitized_cfg: RumbleConfig,
    also_default: bool,
) -> std::io::Result<PathBuf> {
    let mut ap = shared.lock();
    match ap.active_match.clone() {
        Some(m) => {
            if let Some(p) = ap.profiles.iter_mut().find(|p| p.match_substring == m) {
                p.config = sanitized_cfg.clone();
            }
            if also_default {
                ap.default = sanitized_cfg;
            }
        }
        None => ap.default = sanitized_cfg,
    }
    settings::save(&settings::SettingsFile {
        default: ap.default.clone(),
        profiles: ap.profiles.clone(),
    })
}
