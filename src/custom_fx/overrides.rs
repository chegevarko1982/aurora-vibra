//! Таблица вытеснения: какой встроенный эффект (`RumbleConfig`/`WtConfig`,
//! `types.rs`) гасится, когда пользователь завёл ВКЛЮЧЁННЫЙ кастомный
//! эффект на источнике телеметрии, который для этого встроенного эффекта —
//! ОСНОВНОЙ вход. Встроенные и пользовательские эффекты работают
//! ОДНОВРЕМЕННО: оба движка считаются каждый тик, сведение их выходов —
//! обычный `.max()` по каждому мотору, тот же приём, что уже сводит между
//! собой восемь встроенных эффектов внутри `rumble.rs`.
//! Вытеснение — единственный способ, которым один эффект "выключает" другой.
//!
//! Соответствие "источник -> вытесняемый встроенный" НАМЕРЕННО неполное:
//! многие источники (`FlightGroundSpeedKn`, `FlightOnGround`, `WtInMission`,
//! пороговые "расписания" вроде `FlightOverspeedBarberPoleKn`...) питают
//! сразу НЕСКОЛЬКО встроенных эффектов как вспомогательное условие (гейт /
//! динамический порог), не будучи ГЛАВНЫМ входом ни для одного — для них
//! здесь `None`, вытеснения нет. Канонический пример из задачи: положение
//! закрылков (`FlightFlapsPct`/`WtFlapsPct`) читает и сам эффект закрылков
//! (motor-hum), и (в WT) расписание порога Overspeed
//! (`overspeed_profiles::effective_limit_kmh`) — вытесняется ТОЛЬКО эффект
//! закрылков, Overspeed продолжает работать и продолжает читать закрылки
//! как вспомогательный вход. Та же логика применена к `WtGearPct` (основной
//! для `WtGearTransit`, но также гейтит `WtGearOverspeed`/`WtEngineStart`) и
//! к `WtIasKmh` (основной для `WtOverspeed`, но также гейтит порог
//! `WtGearOverspeed`). Лучше `None`, чем натянутая связь и неожиданно
//! погасший встроенный эффект — см. `primary_builtin_for` ниже.

use crate::custom_fx::engine::aircraft_matches;
use crate::custom_fx::model::CustomEffect;
use crate::custom_fx::sources::{self, SourceId};
use crate::types::{ActiveGame, RumbleConfig, WtConfig};

/// Один встроенный эффект, который может быть вытеснен пользовательским.
/// Один вариант на каждый top-level флаг `*_enabled` в `RumbleConfig`/
/// `WtConfig` (types.rs) — намеренно ИСКЛЮЧЕНИЕ: `enable_engine_start` в
/// `RumbleConfig` (Engine Spool-up & Ignition, MSFS/X-Plane) сюда не входит,
/// его нет и в задаче — поле называется `enable_*`, а не `*_enabled`, и ни
/// один `SourceId` не читается этим MSFS-эффектом напрямую так же явно, как
/// `WtRpm1`/`WtPower1Hp` читаются `WtEngineStart`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinEffect {
    // RumbleConfig (MSFS/X-Plane)
    Overspeed,
    GearComp,
    GearTransit,
    Bank,
    Taxi,
    Ground,
    Flaps,
    Gear,
    Stall,
    Spoilers,
    /// Запуск двигателей MSFS. Флаг называется `enable_engine_start`, а не
    /// `*_enabled`, как у соседей — не переименовывай его тут по аналогии.
    EngineStart,
    // WtConfig (War Thunder)
    WtWeapon1,
    WtWeapon2,
    WtFlaps,
    WtGearTransit,
    WtStall,
    WtEngineStart,
    WtOverspeed,
    WtGearOverspeed,
}

/// Набор вытесненных встроенных эффектов на текущий тик — плоский набор
/// bool-полей (не `HashSet`), чтобы `apply_to_rumble_config`/
/// `apply_to_wt_config` были простыми прямыми присваиваниями без матчинга
/// по enum на каждый флаг.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BuiltinMask {
    pub overspeed: bool,
    pub gear_comp: bool,
    pub gear_transit: bool,
    pub bank: bool,
    pub taxi: bool,
    pub ground: bool,
    pub flaps: bool,
    pub gear: bool,
    pub stall: bool,
    pub spoilers: bool,
    pub engine_start: bool,

    pub wt_weapon1: bool,
    pub wt_weapon2: bool,
    pub wt_flaps: bool,
    pub wt_gear_transit: bool,
    pub wt_stall: bool,
    pub wt_engine_start: bool,
    pub wt_overspeed: bool,
    pub wt_gear_overspeed: bool,
}

impl BuiltinMask {
    fn insert(&mut self, effect: BuiltinEffect) {
        match effect {
            BuiltinEffect::Overspeed => self.overspeed = true,
            BuiltinEffect::GearComp => self.gear_comp = true,
            BuiltinEffect::GearTransit => self.gear_transit = true,
            BuiltinEffect::Bank => self.bank = true,
            BuiltinEffect::Taxi => self.taxi = true,
            BuiltinEffect::Ground => self.ground = true,
            BuiltinEffect::Flaps => self.flaps = true,
            BuiltinEffect::Gear => self.gear = true,
            BuiltinEffect::Stall => self.stall = true,
            BuiltinEffect::Spoilers => self.spoilers = true,
            BuiltinEffect::EngineStart => self.engine_start = true,
            BuiltinEffect::WtWeapon1 => self.wt_weapon1 = true,
            BuiltinEffect::WtWeapon2 => self.wt_weapon2 = true,
            BuiltinEffect::WtFlaps => self.wt_flaps = true,
            BuiltinEffect::WtGearTransit => self.wt_gear_transit = true,
            BuiltinEffect::WtStall => self.wt_stall = true,
            BuiltinEffect::WtEngineStart => self.wt_engine_start = true,
            BuiltinEffect::WtOverspeed => self.wt_overspeed = true,
            BuiltinEffect::WtGearOverspeed => self.wt_gear_overspeed = true,
        }
    }

    /// Применяет маску к КОПИИ `RumbleConfig` (sim/worker.rs, xp_link/worker.rs)
    /// — гасит ровно вытесненные встроенные эффекты MSFS/X-Plane, остальные
    /// поля не трогает. Движок вибрации (`rumble.rs`) не меняется вообще —
    /// подавление целиком происходит здесь, до вызова `RumbleEngine::step`.
    pub fn apply_to_rumble_config(&self, cfg: &mut RumbleConfig) {
        if self.overspeed {
            cfg.overspeed_enabled = false;
        }
        if self.gear_comp {
            cfg.gear_comp_enabled = false;
        }
        if self.gear_transit {
            cfg.gear_transit_enabled = false;
        }
        if self.bank {
            cfg.bank_enabled = false;
        }
        if self.taxi {
            // Один пункт таблицы = два флага: "окно" таксирования задают
            // ОБА конца (start/end), приглушать имеет смысл только целиком.
            cfg.taxi_start_enabled = false;
            cfg.taxi_end_enabled = false;
        }
        if self.ground {
            cfg.ground_enabled = false;
        }
        if self.engine_start {
            cfg.enable_engine_start = false;
        }
        if self.flaps {
            cfg.flaps_enabled = false;
        }
        if self.gear {
            cfg.gear_enabled = false;
        }
        if self.stall {
            cfg.stall_enabled = false;
        }
        if self.spoilers {
            cfg.spoilers_enabled = false;
        }
    }

    /// Применяет маску к КОПИИ `WtConfig` (wt_link/worker.rs) — гасит ровно
    /// вытесненные встроенные эффекты War Thunder, остальные поля не трогает.
    pub fn apply_to_wt_config(&self, cfg: &mut WtConfig) {
        if self.wt_weapon1 {
            cfg.weapon1_enabled = false;
        }
        if self.wt_weapon2 {
            cfg.weapon2_enabled = false;
        }
        if self.wt_flaps {
            cfg.flaps_enabled = false;
        }
        if self.wt_gear_transit {
            cfg.gear_transit_enabled = false;
        }
        if self.wt_stall {
            cfg.stall_enabled = false;
        }
        if self.wt_engine_start {
            cfg.engine_start_enabled = false;
        }
        if self.wt_overspeed {
            cfg.overspeed_enabled = false;
        }
        if self.wt_gear_overspeed {
            cfg.gear_overspeed_enabled = false;
        }
    }
}

/// Источник -> ОСНОВНОЙ встроенный эффект, который он вытесняет. Матч
/// исчерпывающий (без `_ =>`) НАМЕРЕННО: новый вариант `SourceId` без записи
/// здесь обязан ломать сборку, а не тихо утекать в `None` — та же защита,
/// что уже применена к `sources::def`/`sources::read` в sources.rs.
fn primary_builtin_for(source: SourceId) -> Option<BuiltinEffect> {
    use BuiltinEffect::*;
    use SourceId::*;
    match source {
        FlightAirspeedKn => Some(Overspeed),
        FlightBankDeg => Some(Bank),
        FlightFlapsPct => Some(Flaps),
        FlightSpoilersPct => Some(Spoilers),
        FlightGearHandle => Some(Gear),
        FlightGearCompNose | FlightGearCompLeft | FlightGearCompRight => Some(GearComp),
        FlightStalled => Some(Stall),
        // Раскрутка/обороты — основной вход эффекта запуска двигателей
        // (rumble.rs берёт max(N2, % от макс. оборотов), поэтому и турбинные,
        // и поршневые источники ведут к одному и тому же эффекту).
        FlightEng1N2Pct | FlightEng2N2Pct | FlightEng1Rpm | FlightEng2Rpm | FlightProp1Rpm
        | FlightProp2Rpm => Some(EngineStart),

        WtIasKmh => Some(WtOverspeed),
        WtAoaDeg => Some(WtStall),
        WtRpm1 | WtPower1Hp => Some(WtEngineStart),
        WtFlapsPct => Some(WtFlaps),
        WtGearPct => Some(WtGearTransit),
        WtWeapon1Firing => Some(WtWeapon1),
        WtWeapon2Firing => Some(WtWeapon2),

        // Явно НЕ основные ни для одного встроенного эффекта — см.
        // doc-комментарий модуля про "вспомогательный вход у нескольких
        // эффектов сразу" (FlightGroundSpeedKn/FlightOnGround/WtInMission/...)
        // и про поля, которые встроенный движок вообще не читает
        // (FlightSlatsPct — см. её doc-комментарий в types.rs;
        // FlightOverspeedWarning — тоже только для отображения в UI,
        // не вход rumble.rs). Инженерные обороты/N2 MSFS (Flight*Rpm/N2Pct)
        // не сопоставлены ни с чем — соответствующего варианта
        // `BuiltinEffect` для MSFS engine-start нет (см. doc-комментарий на
        // enum). Ground speed/Vmo-порог/охват — вспомогательные условия сразу
        // нескольких эффектов, не основные ни для одного.
        FlightGroundSpeedKn
        | FlightSlatsPct
        | FlightOnGround
        | FlightOverspeedWarning
        | FlightOverspeedBarberPoleKn
        | WtSpeedKt
        | WtAltitudeFt
        | WtWxDegS
        | WtInMission
        | Lvar => None,
    }
}

/// Собирает маску вытеснения из списка пользовательских эффектов для
/// текущей игры/борта. Учитывает только ВКЛЮЧЁННЫЕ (`enabled`) эффекты,
/// разрешённые для `game` и по `GameMask` эффекта, и по борту
/// (`aircraft_match`, та же подстрочная семантика, что и у самого движка —
/// см. `custom_fx::engine::aircraft_matches`).
///
/// Дополнительно проверяет собственный игровой охват ИСТОЧНИКА
/// (`sources::def(id).games`), а не только `CustomEffect::games`: у
/// `GameMask` пользователя по умолчанию разрешены все три игры (см.
/// `GameMask::default`), но источник вроде `FlightAirspeedKn` физически не
/// существует в WT-телеметрии — `CustomFxEngine::step` в этом случае
/// самонейтрализуется (`sources::read` возвращает `None` для чужого кадра
/// телеметрии), и вытеснение обязано вести себя так же: не гасить WT-эффект
/// по MSFS-источнику только потому, что пользователь не снял галку `wt` в
/// маске своего эффекта.
pub fn overridden_builtins(
    effects: &[CustomEffect],
    game: ActiveGame,
    aircraft: &str,
) -> BuiltinMask {
    let mut mask = BuiltinMask::default();
    for effect in effects {
        if !effect.enabled {
            continue;
        }
        if !effect.games.allows(game) {
            continue;
        }
        if !sources::def(effect.source).games.allows(game) {
            continue;
        }
        if !aircraft_matches(&effect.aircraft_match, aircraft) {
            continue;
        }
        if let Some(builtin) = primary_builtin_for(effect.source) {
            mask.insert(builtin);
        }
    }
    mask
}

/// Сведение выхода встроенного и пользовательского движков по ОДНОМУ каналу
/// мотора — тот же приём, что уже сводит между собой восемь встроенных
/// эффектов в `rumble.rs` (`.max()` по каждому мотору, см.
/// `final_joystick.max(...)` и соседние там же): несколько источников
/// вибрации на одном моторе не складываются, слышен только самый сильный.
/// `.min(max_output)` — оба слагаемых и так уже держатся в этих границах
/// каждый своим движком, но явный зажим здесь документирует инвариант в
/// самой точке сведения, а не только неявно через два разных движка.
pub fn combine_channel(builtin: u8, custom: u8, max_output: u8) -> u8 {
    builtin.max(custom).min(max_output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_fx::model::new_effect;

    fn ias_effect(enabled: bool) -> CustomEffect {
        let mut e = new_effect("Test".into(), SourceId::FlightAirspeedKn);
        e.enabled = enabled;
        e
    }

    #[test]
    fn enabled_effect_on_ias_overrides_overspeed() {
        let effects = vec![ias_effect(true)];
        let mask = overridden_builtins(&effects, ActiveGame::Msfs, "");
        assert!(mask.overspeed);
        assert!(!mask.stall, "не должно вытеснять ничего постороннего");
    }

    #[test]
    fn disabled_effect_does_not_override() {
        let effects = vec![ias_effect(false)];
        let mask = overridden_builtins(&effects, ActiveGame::Msfs, "");
        assert!(!mask.overspeed);
    }

    #[test]
    fn effect_for_other_game_does_not_override() {
        let mut e = ias_effect(true);
        e.games.msfs = false;
        e.games.xplane = false;
        // wt остаётся true, но источник FlightAirspeedKn физически не
        // существует в WT-телеметрии — проверка охвата источника обязана
        // отсечь его так же, как самонейтрализовался бы сам движок.
        let mask_wt = overridden_builtins(std::slice::from_ref(&e), ActiveGame::Wt, "");
        assert!(!mask_wt.overspeed, "источник вне игры не должен вытеснять");

        let mask_msfs = overridden_builtins(&[e], ActiveGame::Msfs, "");
        assert!(
            !mask_msfs.overspeed,
            "games.msfs=false — эффект выключен для MSFS"
        );
    }

    #[test]
    fn wrong_aircraft_does_not_override() {
        let mut e = ias_effect(true);
        e.aircraft_match = Some("A320".into());
        let effects = vec![e];
        let mask = overridden_builtins(&effects, ActiveGame::Msfs, "Boeing 737");
        assert!(!mask.overspeed);

        let mask2 = overridden_builtins(&effects, ActiveGame::Msfs, "Airbus A320neo");
        assert!(mask2.overspeed);
    }

    #[test]
    fn multiple_effects_accumulate_into_one_mask() {
        let mut bank = new_effect("Bank".into(), SourceId::FlightBankDeg);
        bank.enabled = true;
        let mut stall = new_effect("Stall".into(), SourceId::FlightStalled);
        stall.enabled = true;
        let effects = vec![ias_effect(true), bank, stall];

        let mask = overridden_builtins(&effects, ActiveGame::Msfs, "");
        assert!(mask.overspeed);
        assert!(mask.bank);
        assert!(mask.stall);
        assert!(!mask.flaps);
    }

    #[test]
    fn wt_source_overrides_only_wt_side() {
        let mut e = new_effect("Test".into(), SourceId::WtAoaDeg);
        e.enabled = true;
        let effects = vec![e];
        let mask = overridden_builtins(&effects, ActiveGame::Wt, "");
        assert!(mask.wt_stall);
        assert!(!mask.stall, "WT-вариант не должен трогать MSFS-флаг");
    }

    #[test]
    fn flaps_source_overrides_flaps_only_not_overspeed_schedule() {
        // Канонический пример из задачи: положение закрылков кормит и эффект
        // закрылков, и (в WT) расписание порога overspeed — вытесняется
        // ТОЛЬКО эффект закрылков.
        let mut e = new_effect("Test".into(), SourceId::WtFlapsPct);
        e.enabled = true;
        let effects = vec![e];
        let mask = overridden_builtins(&effects, ActiveGame::Wt, "");
        assert!(mask.wt_flaps);
        assert!(
            !mask.wt_overspeed,
            "flaps — вспомогательный вход overspeed-порога, не основной"
        );
    }

    #[test]
    fn apply_to_rumble_config_mutes_exactly_expected_flags() {
        let mask = BuiltinMask {
            overspeed: true,
            stall: true,
            ..Default::default()
        };

        let mut cfg = RumbleConfig::default();
        assert!(cfg.overspeed_enabled);
        assert!(cfg.stall_enabled);
        assert!(cfg.bank_enabled);
        assert!(cfg.flaps_enabled);

        mask.apply_to_rumble_config(&mut cfg);
        assert!(!cfg.overspeed_enabled);
        assert!(!cfg.stall_enabled);
        assert!(cfg.bank_enabled, "соседний флаг не должен трогаться");
        assert!(cfg.flaps_enabled, "соседний флаг не должен трогаться");
    }

    /// Флаг запуска двигателей MSFS называется `enable_engine_start`, а не
    /// `*_enabled`, как все соседние — из-за этого он однажды уже выпал из
    /// таблицы вытеснения целиком (эффект просто не вытеснялся). Тест держит
    /// оба конца цепочки: и источник -> эффект, и эффект -> флаг конфига.
    #[test]
    fn engine_spool_sources_override_msfs_engine_start() {
        for src in [
            SourceId::FlightEng1N2Pct,
            SourceId::FlightEng2N2Pct,
            SourceId::FlightEng1Rpm,
            SourceId::FlightProp1Rpm,
        ] {
            assert_eq!(
                primary_builtin_for(src),
                Some(BuiltinEffect::EngineStart),
                "{src:?} обязан вытеснять запуск двигателей"
            );
        }

        let mask = BuiltinMask {
            engine_start: true,
            ..Default::default()
        };
        let mut cfg = RumbleConfig::default();
        assert!(cfg.enable_engine_start);
        mask.apply_to_rumble_config(&mut cfg);
        assert!(!cfg.enable_engine_start);
        assert!(cfg.overspeed_enabled, "соседний флаг не должен трогаться");
    }

    #[test]
    fn apply_to_rumble_config_taxi_mutes_both_bounds() {
        let mask = BuiltinMask {
            taxi: true,
            ..Default::default()
        };
        let mut cfg = RumbleConfig::default();
        mask.apply_to_rumble_config(&mut cfg);
        assert!(!cfg.taxi_start_enabled);
        assert!(!cfg.taxi_end_enabled);
    }

    #[test]
    fn apply_to_wt_config_mutes_exactly_expected_flags() {
        let mask = BuiltinMask {
            wt_weapon1: true,
            wt_gear_transit: true,
            ..Default::default()
        };

        let mut cfg = WtConfig::default();
        assert!(cfg.weapon1_enabled);
        assert!(cfg.gear_transit_enabled);
        assert!(cfg.weapon2_enabled);
        assert!(cfg.stall_enabled);

        mask.apply_to_wt_config(&mut cfg);
        assert!(!cfg.weapon1_enabled);
        assert!(!cfg.gear_transit_enabled);
        assert!(cfg.weapon2_enabled, "соседний флаг не должен трогаться");
        assert!(cfg.stall_enabled, "соседний флаг не должен трогаться");
    }

    #[test]
    fn combine_channel_takes_max_and_clamps_to_max_output() {
        assert_eq!(combine_channel(50, 200, 255), 200);
        assert_eq!(combine_channel(200, 50, 255), 200);
        assert_eq!(combine_channel(0, 0, 255), 0);
        // max_output ниже обоих слагаемых — зажимается.
        assert_eq!(combine_channel(200, 220, 180), 180);
    }

    #[test]
    fn every_source_id_is_covered_by_the_table_without_panicking() {
        // primary_builtin_for — исчерпывающий match без `_ =>`: сам факт
        // компиляции уже гарантирует полноту, но прогон по всей таблице
        // SOURCES вдобавок документирует намерение и не даст будущему
        // рефакторингу тихо превратить исчерпывающий матч в частичный.
        for s in sources::SOURCES {
            let _ = primary_builtin_for(s.id);
        }
    }
}
