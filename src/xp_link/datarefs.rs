//! Единый источник правды для каталога X-Plane datarefs, на которые
//! подписывается RREF-клиент (`super::rref`).
//!
//! Тот же приём, что в `crate::sim::elem_idx`: если завести список имён для
//! регистрации подписки и отдельный список для разбора значений, они рано
//! или поздно разойдутся (см. историю `elem_idx.rs` — там расхождение
//! молча сдвинуло чтение всех полей после `L:I_PFD_VMAX` на одну позицию).
//! Макрос `dataref_idx!` генерирует enum и таблицу `DEFS` из одного
//! упорядоченного списка, поэтому дискриминант варианта (Rust присваивает
//! его по порядку объявления, начиная с 0) всегда совпадает с позицией в
//! `DEFS`.
//!
//! Это совпадение важно не только для чтения: дискриминант `DrIdx` — это
//! ровно тот `index`, который уходит в RREF-запрос подписки
//! (`sim/...` + этот индекс), и ровно тот слот в массиве значений, куда
//! X-Plane потом кладёт число в ответных пакетах. Один список — одно место,
//! где что-то добавить/переставить, без второго списка, который можно
//! забыть обновить.
//!
//! X-Plane при подписке на несуществующий (например, из-за опечатки)
//! dataref НЕ возвращает ошибку — он просто молча его игнорирует, и слот
//! в массиве значений навсегда остаётся с `seen == false`. Поэтому опечатка
//! в имени не роняет транспорт и не портит остальные значения — её ловит
//! только self-check воркера (следующий этап), который сверяет, что все
//! ожидаемые индексы когда-нибудь стали `seen`.

/// Частота подписки для "быстрых" datarefs (полёт, управление) — раз в кадр
/// физики примерно эквивалентно 20 Гц у SimConnect-конвейера MSFS.
const FAST: u32 = 20;
/// Частота подписки для "медленных" datarefs (метаданные борта, пауза) —
/// меняются редко, опрашивать чаще незачем.
const SLOW: u32 = 1;

macro_rules! dataref_idx {
    ( $( $(#[$meta:meta])* $variant:ident => ($dataref: expr, $freq:expr) ),+ $(,)? ) => {
        #[repr(usize)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum DrIdx {
            $( $(#[$meta])* $variant, )+
        }

        impl DrIdx {
            /// (имя dataref, частота подписки в Гц), в порядке RREF-подписки
            /// == порядке слотов в массиве значений воркера.
            pub const DEFS: &'static [(&'static str, u32)] = &[
                $( ($dataref, $freq), )+
            ];

            /// Число зарегистрированных datarefs == требуемая длина буферов
            /// `values`/`seen` у воркера.
            pub const COUNT: usize = Self::DEFS.len();

            /// Имя dataref — для логов self-check (какой индекс ни разу не
            /// пришёл от сима).
            pub fn name(self) -> &'static str {
                Self::DEFS[self as usize].0
            }
        }
    };
}

dataref_idx! {
    // === Быстрые (FAST, 20 Гц) ===
    Airspeed => ("sim/flightmodel/position/indicated_airspeed", FAST),
    Groundspeed => ("sim/flightmodel/position/groundspeed", FAST),
    OnGround => ("sim/flightmodel/failures/onground_any", FAST),
    BankDeg => ("sim/flightmodel/position/phi", FAST),
    StallWarning => ("sim/cockpit2/annunciators/stall_warning", FAST),
    OverspeedWarning => ("sim/cockpit2/annunciators/overspeed", FAST),
    FlapDeployRatio => ("sim/flightmodel2/controls/flap1_deploy_ratio", FAST),
    SlatDeployRatio => ("sim/flightmodel2/controls/slat1_deploy_ratio", FAST),
    SpeedbrakeRatio => ("sim/flightmodel2/controls/speedbrake_ratio", FAST),
    GearHandleDown => ("sim/cockpit2/controls/gear_handle_down", FAST),
    SimTime => ("sim/network/misc/network_time_sec", FAST),

    // Позиция и деформация стоек шасси по массиву [нос, лево, право].
    GearDeploy0 => ("sim/flightmodel2/gear/deploy_ratio[0]", FAST),
    GearDeploy1 => ("sim/flightmodel2/gear/deploy_ratio[1]", FAST),
    GearDeploy2 => ("sim/flightmodel2/gear/deploy_ratio[2]", FAST),
    GearDefl0 => ("sim/flightmodel2/gear/tire_vertical_deflection_mtr[0]", FAST),
    GearDefl1 => ("sim/flightmodel2/gear/tire_vertical_deflection_mtr[1]", FAST),
    GearDefl2 => ("sim/flightmodel2/gear/tire_vertical_deflection_mtr[2]", FAST),

    // Телеметрия двигателей 1..4 — N2/N1, горение, стартер, обороты вала и
    // винта. Массивы X-Plane 0-based, у нас 1-based имена переменных (как в
    // elem_idx.rs у MSFS).
    Eng1N2 => ("sim/flightmodel/engine/ENGN_N2_[0]", FAST),
    Eng2N2 => ("sim/flightmodel/engine/ENGN_N2_[1]", FAST),
    Eng3N2 => ("sim/flightmodel/engine/ENGN_N2_[2]", FAST),
    Eng4N2 => ("sim/flightmodel/engine/ENGN_N2_[3]", FAST),

    Eng1N1 => ("sim/flightmodel/engine/ENGN_N1_[0]", FAST),
    Eng2N1 => ("sim/flightmodel/engine/ENGN_N1_[1]", FAST),
    Eng3N1 => ("sim/flightmodel/engine/ENGN_N1_[2]", FAST),
    Eng4N1 => ("sim/flightmodel/engine/ENGN_N1_[3]", FAST),

    Eng1Burning => ("sim/flightmodel2/engines/engine_is_burning_fuel[0]", FAST),
    Eng2Burning => ("sim/flightmodel2/engines/engine_is_burning_fuel[1]", FAST),
    Eng3Burning => ("sim/flightmodel2/engines/engine_is_burning_fuel[2]", FAST),
    Eng4Burning => ("sim/flightmodel2/engines/engine_is_burning_fuel[3]", FAST),

    Eng1Starter => ("sim/cockpit2/engine/actuators/starter_hit[0]", FAST),
    Eng2Starter => ("sim/cockpit2/engine/actuators/starter_hit[1]", FAST),
    Eng3Starter => ("sim/cockpit2/engine/actuators/starter_hit[2]", FAST),
    Eng4Starter => ("sim/cockpit2/engine/actuators/starter_hit[3]", FAST),

    Eng1Rpm => ("sim/cockpit2/engine/indicators/engine_speed_rpm[0]", FAST),
    Eng2Rpm => ("sim/cockpit2/engine/indicators/engine_speed_rpm[1]", FAST),
    Eng3Rpm => ("sim/cockpit2/engine/indicators/engine_speed_rpm[2]", FAST),
    Eng4Rpm => ("sim/cockpit2/engine/indicators/engine_speed_rpm[3]", FAST),

    Prop1Rpm => ("sim/cockpit2/engine/indicators/prop_speed_rpm[0]", FAST),
    Prop2Rpm => ("sim/cockpit2/engine/indicators/prop_speed_rpm[1]", FAST),
    Prop3Rpm => ("sim/cockpit2/engine/indicators/prop_speed_rpm[2]", FAST),
    Prop4Rpm => ("sim/cockpit2/engine/indicators/prop_speed_rpm[3]", FAST),

    // === Медленные (SLOW, 1 Гц) ===
    Paused => ("sim/time/paused", SLOW),
    AcfVne => ("sim/aircraft/view/acf_Vne", SLOW),
    AcfNumEngines => ("sim/aircraft/engine/acf_num_engines", SLOW),
    AcfEnType => ("sim/aircraft/prop/acf_en_type[0]", SLOW),

    // Имя борта (acf_descrip) X-Plane отдаёт как массив из 40 байт-datarefs
    // (по одному char на индекс) — RREF не умеет отдавать dataref-строку
    // одним пакетом, поэтому подписываемся на все 40 байт по отдельности и
    // склеиваем строку на стороне воркера (следующий этап). ACF_DESCRIP_RANGE
    // ниже даёт диапазон их индексов без магических чисел.
    AcfDescrip00 => ("sim/aircraft/view/acf_descrip[0]", SLOW),
    AcfDescrip01 => ("sim/aircraft/view/acf_descrip[1]", SLOW),
    AcfDescrip02 => ("sim/aircraft/view/acf_descrip[2]", SLOW),
    AcfDescrip03 => ("sim/aircraft/view/acf_descrip[3]", SLOW),
    AcfDescrip04 => ("sim/aircraft/view/acf_descrip[4]", SLOW),
    AcfDescrip05 => ("sim/aircraft/view/acf_descrip[5]", SLOW),
    AcfDescrip06 => ("sim/aircraft/view/acf_descrip[6]", SLOW),
    AcfDescrip07 => ("sim/aircraft/view/acf_descrip[7]", SLOW),
    AcfDescrip08 => ("sim/aircraft/view/acf_descrip[8]", SLOW),
    AcfDescrip09 => ("sim/aircraft/view/acf_descrip[9]", SLOW),
    AcfDescrip10 => ("sim/aircraft/view/acf_descrip[10]", SLOW),
    AcfDescrip11 => ("sim/aircraft/view/acf_descrip[11]", SLOW),
    AcfDescrip12 => ("sim/aircraft/view/acf_descrip[12]", SLOW),
    AcfDescrip13 => ("sim/aircraft/view/acf_descrip[13]", SLOW),
    AcfDescrip14 => ("sim/aircraft/view/acf_descrip[14]", SLOW),
    AcfDescrip15 => ("sim/aircraft/view/acf_descrip[15]", SLOW),
    AcfDescrip16 => ("sim/aircraft/view/acf_descrip[16]", SLOW),
    AcfDescrip17 => ("sim/aircraft/view/acf_descrip[17]", SLOW),
    AcfDescrip18 => ("sim/aircraft/view/acf_descrip[18]", SLOW),
    AcfDescrip19 => ("sim/aircraft/view/acf_descrip[19]", SLOW),
    AcfDescrip20 => ("sim/aircraft/view/acf_descrip[20]", SLOW),
    AcfDescrip21 => ("sim/aircraft/view/acf_descrip[21]", SLOW),
    AcfDescrip22 => ("sim/aircraft/view/acf_descrip[22]", SLOW),
    AcfDescrip23 => ("sim/aircraft/view/acf_descrip[23]", SLOW),
    AcfDescrip24 => ("sim/aircraft/view/acf_descrip[24]", SLOW),
    AcfDescrip25 => ("sim/aircraft/view/acf_descrip[25]", SLOW),
    AcfDescrip26 => ("sim/aircraft/view/acf_descrip[26]", SLOW),
    AcfDescrip27 => ("sim/aircraft/view/acf_descrip[27]", SLOW),
    AcfDescrip28 => ("sim/aircraft/view/acf_descrip[28]", SLOW),
    AcfDescrip29 => ("sim/aircraft/view/acf_descrip[29]", SLOW),
    AcfDescrip30 => ("sim/aircraft/view/acf_descrip[30]", SLOW),
    AcfDescrip31 => ("sim/aircraft/view/acf_descrip[31]", SLOW),
    AcfDescrip32 => ("sim/aircraft/view/acf_descrip[32]", SLOW),
    AcfDescrip33 => ("sim/aircraft/view/acf_descrip[33]", SLOW),
    AcfDescrip34 => ("sim/aircraft/view/acf_descrip[34]", SLOW),
    AcfDescrip35 => ("sim/aircraft/view/acf_descrip[35]", SLOW),
    AcfDescrip36 => ("sim/aircraft/view/acf_descrip[36]", SLOW),
    AcfDescrip37 => ("sim/aircraft/view/acf_descrip[37]", SLOW),
    AcfDescrip38 => ("sim/aircraft/view/acf_descrip[38]", SLOW),
    AcfDescrip39 => ("sim/aircraft/view/acf_descrip[39]", SLOW),
}

/// Диапазон индексов `DrIdx`, занятых байтами имени борта (`acf_descrip`) —
/// чтобы воркер собирал из них строку по `DrIdx::DEFS[i]`/`values[i]` без
/// магических констант "0..40" или "61..101".
pub const ACF_DESCRIP_RANGE: std::ops::Range<usize> =
    DrIdx::AcfDescrip00 as usize..DrIdx::AcfDescrip39 as usize + 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defs_len_matches_count() {
        assert_eq!(DrIdx::DEFS.len(), DrIdx::COUNT);
    }

    #[test]
    fn acf_descrip_range_has_40_entries_with_expected_prefix() {
        assert_eq!(ACF_DESCRIP_RANGE.len(), 40);
        for i in ACF_DESCRIP_RANGE {
            assert!(
                DrIdx::DEFS[i]
                    .0
                    .starts_with("sim/aircraft/view/acf_descrip["),
                "unexpected dataref at index {i}: {}",
                DrIdx::DEFS[i].0
            );
        }
    }
}
