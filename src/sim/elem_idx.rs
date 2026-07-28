//! Single source of truth for the SimConnect "main" data-definition layout.
//!
//! Historically the registration order (`defs` in sim/worker.rs, built via
//! `SimConnect_AddToDataDefinition`) and the read order (`elem.get(N)` in
//! sim/parse.rs) were two hand-maintained lists linked only by comments
//! claiming "index N". They drifted apart (see git history) and every field
//! from `L:I_PFD_VMAX` onward silently read one slot too far.
//!
//! `elem_idx!` below generates the enum variants and the `DEFS` registration
//! table from the *same* ordered list, so a variant's discriminant (assigned
//! by Rust in declaration order, starting at 0) always matches its position
//! in `DEFS`. Inserting a variable now means adding one line in one place;
//! there is no second list to forget to update.

macro_rules! elem_idx {
    ( $( $(#[$meta:meta])* $variant:ident => ($sim_var:expr, $unit:expr) ),+ $(,)? ) => {
        #[repr(usize)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum ElemIdx {
            $( $(#[$meta])* $variant, )+
        }

        impl ElemIdx {
            /// (SimConnect variable name, unit), in `AddToDataDefinition()` registration order.
            pub const DEFS: &'static [(&'static str, &'static str)] = &[
                $( ($sim_var, $unit), )+
            ];

            /// Number of registered SimConnect variables == required `elem[]` buffer length.
            pub const COUNT: usize = Self::DEFS.len();
        }
    };
}

elem_idx! {
    AirspeedIndicated => ("AIRSPEED INDICATED", "Knots"),
    SimOnGround => ("SIM ON GROUND", "Bool"),
    PlaneBankDegrees => ("PLANE BANK DEGREES", "Degrees"),
    FlapsLeftPercent => ("TRAILING EDGE FLAPS LEFT PERCENT", "Percent"),
    FlapsRightPercent => ("TRAILING EDGE FLAPS RIGHT PERCENT", "Percent"),
    FlapsHandleIndex => ("FLAPS HANDLE INDEX", "Number"),
    GearHandlePosition => ("GEAR HANDLE POSITION", "Bool"),
    StallWarning => ("STALL WARNING", "Bool"),
    AbsoluteTime => ("ABSOLUTE TIME", "Seconds"),
    GroundVelocity => ("GROUND VELOCITY", "Knots"),
    // Строку "PAUSED" удалили из этого списка — спойлеры (следующий индекс)
    // больше не зависят от неё и сидят на фиксированной позиции.
    SpoilersHandlePosition => ("SPOILERS HANDLE POSITION", "Percent"),
    GearAnimNose => ("GEAR ANIMATION POSITION:0", "Percent"),
    GearAnimLeft => ("GEAR ANIMATION POSITION:1", "Percent"),
    GearAnimRight => ("GEAR ANIMATION POSITION:2", "Percent"),

    // Телеметрия запуска двигателей (Engine Spool-up & Ignition).
    Eng1N2 => ("TURB ENG N2:1", "Percent"),
    Eng1Combustion => ("GENERAL ENG COMBUSTION:1", "Bool"),
    Eng2N2 => ("TURB ENG N2:2", "Percent"),
    Eng2Combustion => ("GENERAL ENG COMBUSTION:2", "Bool"),

    // Двигатели 3/4 для 4-моторных самолётов (см. RumbleConfig::four_engine_mode).
    Eng3N2 => ("TURB ENG N2:3", "Percent"),
    Eng3Combustion => ("GENERAL ENG COMBUSTION:3", "Bool"),
    Eng4N2 => ("TURB ENG N2:4", "Percent"),
    Eng4Combustion => ("GENERAL ENG COMBUSTION:4", "Bool"),

    // Универсальная поддержка поршневых двигателей: GENERAL ENG PCT MAX RPM
    // работает и на турбинах (примерно повторяет N2), и на поршневых (даёт
    // % от макс. оборотов, в отличие от TURB ENG N2, которое на поршневых
    // обычно равно 0).
    Eng1PctMaxRpm => ("GENERAL ENG PCT MAX RPM:1", "Percent"),
    Eng2PctMaxRpm => ("GENERAL ENG PCT MAX RPM:2", "Percent"),
    Eng3PctMaxRpm => ("GENERAL ENG PCT MAX RPM:3", "Percent"),
    Eng4PctMaxRpm => ("GENERAL ENG PCT MAX RPM:4", "Percent"),

    // Универсальная модель запуска (Starter + Combustion): работает
    // одинаково на поршневых и турбинных двигателях.
    Eng1Starter => ("GENERAL ENG STARTER:1", "Bool"),
    Eng2Starter => ("GENERAL ENG STARTER:2", "Bool"),
    Eng3Starter => ("GENERAL ENG STARTER:3", "Bool"),
    Eng4Starter => ("GENERAL ENG STARTER:4", "Bool"),

    // Поршневые двигатели (Piston Engine Telemetry) — сырые обороты
    // коленвала и воздушного винта, для UI-инспектора телеметрии.
    Eng1Rpm => ("GENERAL ENG RPM:1", "Rpm"),
    Eng2Rpm => ("GENERAL ENG RPM:2", "Rpm"),
    Eng3Rpm => ("GENERAL ENG RPM:3", "Rpm"),
    Eng4Rpm => ("GENERAL ENG RPM:4", "Rpm"),

    // Обороты воздушного винта.
    Prop1Rpm => ("PROP RPM:1", "Rpm"),
    Prop2Rpm => ("PROP RPM:2", "Rpm"),
    Prop3Rpm => ("PROP RPM:3", "Rpm"),
    Prop4Rpm => ("PROP RPM:4", "Rpm"),

    // AIRSPEED BARBER POLE — динамическая "красная черта" (Vmo/Mmo, сим сам
    // двигает её вниз при наборе высоты), порог срабатывания эффекта
    // Overspeed, заменяет ручной слайдер в UI.
    AirspeedBarberPole => ("AIRSPEED BARBER POLE", "Knots"),

    // Предкрылки (Slats) — пока только для отображения в UI ("Live Aircraft
    // Data"), в логику эффектов не задействованы.
    SlatsLeftPercent => ("LEADING EDGE FLAPS LEFT PERCENT", "Percent"),
    SlatsRightPercent => ("LEADING EDGE FLAPS RIGHT PERCENT", "Percent"),

    // Раздельная позиция спойлеров по крыльям — нужна, чтобы отличать
    // честный симметричный выпуск спидбрейков от асимметричного подъёма
    // панелей при крене (roll spoilers/спойлероны, напр. TFDI MD-11, где
    // SPOILERS HANDLE POSITION некорректно следует за креном).
    SpoilersLeftPosition => ("SPOILERS LEFT POSITION", "Percent"),
    SpoilersRightPosition => ("SPOILERS RIGHT POSITION", "Percent"),

    // TFDI MD-11: собственные L-vars по каждой из 5 секций спойлеров на
    // крыло (SPOILERS LEFT/RIGHT POSITION выше на этом борте тоже не всегда
    // надёжны). Используются ТОЛЬКО как дополнительная проверка симметрии
    // (см. rumble.rs) — на других самолётах эти L-vars не определены,
    // SimConnect отдаёт по ним 0.0, и проверка самонейтрализуется (0 против
    // 0 — симметрично).
    Md11SpoilerL1 => ("L:MD11_EXT_L_SPOILER_1", "Number"),
    Md11SpoilerL2 => ("L:MD11_EXT_L_SPOILER_2", "Number"),
    Md11SpoilerL3 => ("L:MD11_EXT_L_SPOILER_3", "Number"),
    Md11SpoilerL4 => ("L:MD11_EXT_L_SPOILER_4", "Number"),
    Md11SpoilerL5 => ("L:MD11_EXT_L_SPOILER_5", "Number"),
    Md11SpoilerR1 => ("L:MD11_EXT_R_SPOILER_1", "Number"),
    Md11SpoilerR2 => ("L:MD11_EXT_R_SPOILER_2", "Number"),
    Md11SpoilerR3 => ("L:MD11_EXT_R_SPOILER_3", "Number"),
    Md11SpoilerR4 => ("L:MD11_EXT_R_SPOILER_4", "Number"),
    Md11SpoilerR5 => ("L:MD11_EXT_R_SPOILER_5", "Number"),

    // OVERSPEED WARNING — булев флаг сима "клацера" (overspeed clacker), для
    // сравнения с нашим порогом AIRSPEED BARBER POLE в телеметрии (пока
    // только для отображения в UI).
    OverspeedWarning => ("OVERSPEED WARNING", "Bool"),

    // Learjet 35A (Flysimware, FSW_L35A): L:XMLSND75 — тот же L-var, к
    // которому в sound.xml аддона привязан звук "overspeed / mach trim"
    // (аварийный клаксон превышения Vmo/Mmo). Экспериментальный
    // альтернативный триггер эффекта Overspeed для этого борта (см.
    // profiles.rs/rumble.rs) — на остальных самолётах L-var не определён,
    // SimConnect отдаёт 0.0 (самонейтрализуется, тот же паттерн, что у
    // L:MD11_EXT_*_SPOILER_* выше).
    OverspeedLearHorn => ("L:XMLSND75", "Bool"),

    // Fenix A320 (FenixSim): L:I_PFD_VMAX — этот аддон не держит AIRSPEED
    // BARBER POLE синхронной с реальным PFD (см. отчёты пользователей), зато
    // сам выставляет свой предел overspeed в этот L-var. Используется вместо
    // AIRSPEED BARBER POLE только когда aircraft title содержит "Fenix" (см.
    // sim/parse.rs) — на остальных бортах L-var не определён, SimConnect
    // отдаёт 0.0 (самонейтрализуется, тот же паттерн, что у L:XMLSND75 выше).
    FenixOverspeedVmax => ("L:I_PFD_VMAX", "Knots"),

    // PMDG 737 (NG3, MSFS): L:EngineStart1b/2b_Ext — true, пока стартер
    // крутит двигатель до воспламенения (взято из sound.xml самого PMDG).
    // Подтверждено тестом в симе, что это надёжный сигнал — используется в
    // rumble.rs как маркер воспламенения (момент, когда real TURB ENG N2
    // впервые > 0) и для более раннего распознавания борта как джета; сама
    // N2-кривая раскрутки идёт напрямую по real TURB ENG N2 (см. rumble.rs —
    // реальный захват телеметрии показал, что N2 у PMDG растёт плавно с
    // момента включения стартера, а не держится на 0 до воспламенения).
    // L:EngineStart1c/2c_Ext (отдельный "маркер воспламенения") ПРОВЕРЕН и
    // ОТКЛОНЁН — не сработал корректно в реальном тесте, поэтому вместо него
    // как маркер воспламенения используется момент, когда сам TURB ENG N2
    // становится > 0 (см. rumble.rs). На остальных самолётах
    // L:EngineStart1b/2b_Ext не определены, SimConnect отдаёт false
    // (самонейтрализуется).
    PmdgEngineStart1b => ("L:EngineStart1b_Ext", "Bool"),
    PmdgEngineStart2b => ("L:EngineStart2b_Ext", "Bool"),

    // GENERAL ENG STARTER ACTIVE:1/2 — пока только для телеметрии (сравнение
    // с L:EngineStart1b/2b_Ext).
    Eng1StarterActive => ("GENERAL ENG STARTER ACTIVE:1", "Bool"),
    Eng2StarterActive => ("GENERAL ENG STARTER ACTIVE:2", "Bool"),

    // Fenix A320 (FenixSim): L:A320_Gear_Nose — GEAR ANIMATION POSITION:0/1/2
    // на этом борте не отражает реальное движение стоек, зато сам аддон
    // выставляет позицию в этот L-var (0 = убрано, 1000 = выпущено). Пока
    // только для телеметрии/отладки (см. sim/parse.rs, ui.rs), на остальных
    // самолётах L-var не определён, SimConnect отдаёт 0.0
    // (самонейтрализуется).
    FenixGearNose => ("L:A320_Gear_Nose", "Number"),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defs_len_matches_count() {
        assert_eq!(ElemIdx::DEFS.len(), ElemIdx::COUNT);
    }

    #[test]
    fn variant_discriminants_match_defs_positions() {
        assert_eq!(ElemIdx::AirspeedIndicated as usize, 0);
        assert_eq!(ElemIdx::SpoilersLeftPosition as usize, 41);
        assert_eq!(ElemIdx::OverspeedWarning as usize, 53);
        assert_eq!(ElemIdx::OverspeedLearHorn as usize, 54);
        assert_eq!(ElemIdx::FenixOverspeedVmax as usize, 55);
        assert_eq!(ElemIdx::PmdgEngineStart1b as usize, 56);
        assert_eq!(ElemIdx::PmdgEngineStart2b as usize, 57);
        assert_eq!(ElemIdx::Eng1StarterActive as usize, 58);
        assert_eq!(ElemIdx::Eng2StarterActive as usize, 59);
        assert_eq!(ElemIdx::FenixGearNose as usize, ElemIdx::COUNT - 1);
        assert_eq!(ElemIdx::FenixGearNose as usize, 60);
    }

    #[test]
    fn defs_names_match_expected_variants() {
        assert_eq!(
            ElemIdx::DEFS[ElemIdx::OverspeedWarning as usize].0,
            "OVERSPEED WARNING"
        );
        assert_eq!(
            ElemIdx::DEFS[ElemIdx::FenixOverspeedVmax as usize].0,
            "L:I_PFD_VMAX"
        );
        assert_eq!(
            ElemIdx::DEFS[ElemIdx::PmdgEngineStart1b as usize].0,
            "L:EngineStart1b_Ext"
        );
        assert_eq!(
            ElemIdx::DEFS[ElemIdx::FenixGearNose as usize].0,
            "L:A320_Gear_Nose"
        );
    }
}
