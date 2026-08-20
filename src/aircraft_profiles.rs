// Пользовательские профили конфига по самолётам. В отличие от
// src/profiles.rs (захардкоженные, code-only оверрайды нескольких полей для
// MADDOG/LEARJET), это полноценные снимки RumbleConfig, которые пользователь
// сам создаёт/переименовывает/удаляет через UI и сохраняет на диск явной
// кнопкой (см. ui.rs) — структура RumbleConfig остаётся единой для всех
// самолётов, отличается только набор сохранённых значений.
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::{ConfigShared, LogBuffer, RumbleConfig, profiles::ProfileState, settings};

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
/// default), применяет его к ЖИВОМУ ConfigShared — используется и при смене
/// борта, и при ручной перезагрузке с диска (кнопка Load), поэтому вынесено в
/// общую функцию.
///
/// Встроенный оверлей (profiles.rs, MADDOG/LEARJET/Fenix CFM/IAE и т.п.)
/// накатывается поверх ТОЛЬКО когда для борта нет именного сохранённого
/// профиля (new_match.is_none(), т.е. используется общий ap.default). Если
/// именной профиль есть — он уже содержит финальные значения для этого борта
/// (см. ui.rs::Save: для именного профиля конфиг пишется БЕЗ sanitize_for_save,
/// т.е. как есть, включая захардкоженные оверрайды или ручные правки
/// пользователя поверх них) — повторно накатывать встроенный оверлей нельзя,
/// иначе явно сохранённое пользователем значение (например Engine Idle N2%
/// для конкретной ливреи Fenix) тут же затрётся обратно захардкоженным
/// дефолтом из profiles.rs.
pub fn apply_for_aircraft(
    ap: &mut AircraftProfiles,
    config: &ConfigShared,
    profile_state: &mut ProfileState,
    title: &str,
    logs: &LogBuffer,
) {
    let new_match =
        find_matching_index(&ap.profiles, title).map(|i| ap.profiles[i].match_substring.clone());
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
            Some(m) => format!(
                "Aircraft profile: loaded saved profile '{}' for '{}'",
                m, title
            ),
            None => format!(
                "Aircraft profile: no saved profile for '{}', loaded default",
                title
            ),
        });
    }
    ap.active_match = new_match.clone();
    // base=None заставляет on_aircraft_changed (если вызван) заново снять
    // "базовые" значения с только что загруженного конфига, а не повторно
    // использовать базу от предыдущего борта.
    profile_state.force_recheck();
    if new_match.is_none() {
        profile_state.on_aircraft_changed(config, title, logs);
    }
    ap.loaded_rev = config.current_rev();
}

/// Сохраняет живой конфиг в активный на данный момент профиль (именной или
/// default) и пишет весь набор профилей на диск.
///
/// `named_cfg` и `default_cfg` намеренно РАЗНЫЕ снимки одного и того же живого
/// конфига:
/// - `named_cfg` — конфиг КАК ЕСТЬ (см. ui.rs::Save), без sanitize_for_save.
///   Именной профиль привязан к конкретному борту (match_substring), поэтому
///   значения встроенного оверлея (MADDOG/LEARJET/Fenix CFM/IAE) или ручные
///   правки пользователя поверх них законно становятся частью ЭТОГО профиля —
///   именно так пользовательское значение Engine Idle N2%, к примеру,
///   переживает Save/Load для этого борта (см. apply_for_aircraft).
/// - `default_cfg` — очищенный ProfileState::sanitize_for_save снимок.
///   default общий для ВСЕХ самолётов без своего именного профиля, поэтому
///   значения, зашитые под конкретный борт, здесь по-прежнему обязаны
///   откатываться к базовым — иначе они утекли бы в дефолт для всех
///   остальных самолётов после первого же вылета на, скажем, MADDOG.
///
/// `also_default` — чекбокс в UI: если включён, `default_cfg` ДОПОЛНИТЕЛЬНО
/// записывается в default (даже когда активен именной профиль текущего
/// борта). Если именного профиля сейчас нет, конфиг и так уходит в default —
/// параметр в этом случае не даёт дублирующего эффекта.
pub fn save_active(
    shared: &Mutex<AircraftProfiles>,
    named_cfg: RumbleConfig,
    default_cfg: RumbleConfig,
    also_default: bool,
) -> std::io::Result<PathBuf> {
    let mut ap = shared.lock();
    match ap.active_match.clone() {
        Some(m) => {
            if let Some(p) = ap.profiles.iter_mut().find(|p| p.match_substring == m) {
                p.config = named_cfg;
            }
            if also_default {
                ap.default = default_cfg;
            }
        }
        None => ap.default = default_cfg,
    }
    let game_override = settings::game_override();
    settings::save(&settings::SettingsFile {
        default: ap.default.clone(),
        profiles: ap.profiles.clone(),
        lang: crate::i18n::get(),
        close_to_tray: settings::close_to_tray(),
        simconnect_dll_path: settings::simconnect_dll_path(),
        monitor_collapsed: settings::monitor_collapsed(),
        // wt_enabled — производное от game_override, см. комментарий у поля
        // в SettingsFile (settings.rs): единственный источник истины теперь
        // game_override, wt_enabled существует только для миграции старых файлов.
        wt_enabled: game_override == crate::types::GameOverride::ForceWt,
        game_override,
        effect_mode: settings::effect_mode(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogBuffer;

    #[test]
    fn no_named_profile_gets_built_in_override_from_shared_default() {
        // Борт есть в profiles.rs (Fenix CFM), но именного сохранённого
        // профиля для него нет — конфиг берётся из ap.default, и встроенный
        // оверлей (engine_idle_n2 = 59.0) обязан накатиться поверх.
        let mut ap = AircraftProfiles::default();
        let config = ConfigShared::new();
        let mut state = ProfileState::new();
        let logs = LogBuffer::default();

        apply_for_aircraft(&mut ap, &config, &mut state, "Fenix A320 CFM56", &logs);

        assert_eq!(config.get().engine_idle_n2, 59.0);
        assert!(ap.active_match.is_none());
    }

    #[test]
    fn named_profile_wins_over_built_in_override() {
        // Пользователь ранее сохранил именной профиль для этого конкретного
        // борта с engine_idle_n2 = 68.0 (не совпадает ни с дефолтным 60.0, ни
        // с захардкоженным Fenix CFM оверрайдом 59.0). При следующей загрузке
        // этого борта должно применяться ИМЕННО сохранённое значение — а не
        // повторно накатываемый встроенный оверлей.
        let mut ap = AircraftProfiles::default();
        let custom_cfg = RumbleConfig {
            engine_idle_n2: 68.0,
            ..Default::default()
        };
        ap.profiles.push(AircraftProfile {
            match_substring: "Fenix A320 CFM56".to_string(),
            config: custom_cfg,
        });
        let config = ConfigShared::new();
        let mut state = ProfileState::new();
        let logs = LogBuffer::default();

        apply_for_aircraft(&mut ap, &config, &mut state, "Fenix A320 CFM56", &logs);

        assert_eq!(config.get().engine_idle_n2, 68.0);
        assert_eq!(ap.active_match.as_deref(), Some("Fenix A320 CFM56"));
    }

    #[test]
    fn switching_from_named_profile_to_unmatched_aircraft_uses_default_plus_override() {
        let mut ap = AircraftProfiles::default();
        let custom_cfg = RumbleConfig {
            engine_idle_n2: 68.0,
            ..Default::default()
        };
        ap.profiles.push(AircraftProfile {
            match_substring: "Fenix A320 CFM56".to_string(),
            config: custom_cfg,
        });
        let config = ConfigShared::new();
        let mut state = ProfileState::new();
        let logs = LogBuffer::default();

        apply_for_aircraft(&mut ap, &config, &mut state, "Fenix A320 CFM56", &logs);
        assert_eq!(config.get().engine_idle_n2, 68.0);

        // Смена на борт с другим встроенным оверлеем (Fenix IAE) без своего
        // именного профиля — должен применяться ЕГО оверлей (70.0), а не
        // застрявшее значение от предыдущего борта.
        apply_for_aircraft(&mut ap, &config, &mut state, "Fenix A320 IAE V2500", &logs);
        assert_eq!(config.get().engine_idle_n2, 70.0);
    }
}
