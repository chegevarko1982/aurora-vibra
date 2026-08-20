//! Таблица источников телеметрии для конструктора кастомных эффектов:
//! перечисление `SourceId`, их метаданные (единицы измерения, диапазон для
//! оси X в UI, к какой игре относится) и единая точка чтения текущего
//! значения из уже готового тика телеметрии (`FlightVars` или `WtVars`).
//! Список — подмножество полей, которые уже существуют в `types.rs`/
//! `wt_link/vars.rs`; здесь ничего заново не считается и не хранится.

use crate::custom_fx::model::GameMask;
use crate::types::FlightVars;
use crate::wt_link::vars::WtVars;
use serde::{Deserialize, Serialize};

/// Идентификатор одного источника телеметрии. Префикс `Flight*` — общие поля
/// MSFS/X-Plane (оба конвейера сходятся в `FlightVars`), `Wt*` — War Thunder
/// (`WtVars`, другой протокол и набор единиц измерения).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceId {
    FlightAirspeedKn,
    FlightGroundSpeedKn,
    FlightBankDeg,
    FlightFlapsPct,
    FlightSlatsPct,
    FlightSpoilersPct,
    FlightGearHandle,
    FlightGearCompNose,
    FlightGearCompLeft,
    FlightGearCompRight,
    FlightEng1N2Pct,
    FlightEng2N2Pct,
    FlightEng1Rpm,
    FlightEng2Rpm,
    FlightProp1Rpm,
    FlightProp2Rpm,
    FlightOnGround,
    FlightStalled,
    FlightOverspeedWarning,
    FlightOverspeedBarberPoleKn,

    WtIasKmh,
    WtSpeedKt,
    WtAltitudeFt,
    WtAoaDeg,
    WtWxDegS,
    WtRpm1,
    WtPower1Hp,
    WtFlapsPct,
    WtGearPct,
    WtWeapon1Firing,
    WtWeapon2Firing,
    WtInMission,

    /// Пользовательская MSFS LVAR — имя и единица лежат НЕ здесь, а на самом
    /// эффекте (`model::CustomEffect::lvar`, см. doc-комментарий там). Юнит-
    /// вариант без данных: `SourceId` обязан оставаться `Copy` (см. весь
    /// остальной код, который копирует его по значению), а имя переменной —
    /// это `String`, которая `Copy` не бывает. Значение достаётся отдельной
    /// функцией `read_lvar`, а не через общий `read` — см. её doc-комментарий.
    Lvar,
}

/// Continuous — произвольное число (узлы, градусы, %...), Boolean — источник
/// физически 0.0/1.0 (флаг), UI должен предлагать под него триггер `IsTrue`
/// вместо слайдеров `Above`/`Between`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Continuous,
    Boolean,
}

/// Статические метаданные одного источника — то, что UI показывает в списке
/// выбора и использует как стартовые границы оси X кривой отклика.
#[derive(Debug, Clone, Copy)]
pub struct SourceDef {
    pub id: SourceId,
    pub label_en: &'static str,
    pub label_ru: &'static str,
    /// Единица измерения для подписи оси; "" у булевых источников.
    pub unit: &'static str,
    pub games: GameMask,
    pub kind: SourceKind,
    pub default_range: (f64, f64),
}

// Две готовые маски вместо повтора `GameMask { .. }` в каждой строке таблицы
// ниже: FlightVars — общий конвейер MSFS+X-Plane, WtVars — только War Thunder.
const FLIGHT_ONLY: GameMask = GameMask {
    msfs: true,
    xplane: true,
    wt: false,
};
const WT_ONLY: GameMask = GameMask {
    msfs: false,
    xplane: false,
    wt: true,
};
// Пользовательская LVAR читается только конвейером MSFS (см.
// FlightVars::lvars в types.rs) — у X-Plane и War Thunder своей системы
// L-var нет и не планируется.
const MSFS_ONLY: GameMask = GameMask {
    msfs: true,
    xplane: false,
    wt: false,
};

pub const SOURCES: &[SourceDef] = &[
    SourceDef {
        id: SourceId::FlightAirspeedKn,
        label_en: "Airspeed (IAS)",
        label_ru: "Приборная скорость (IAS)",
        unit: "kn",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 400.0),
    },
    SourceDef {
        id: SourceId::FlightGroundSpeedKn,
        label_en: "Ground speed",
        label_ru: "Путевая скорость",
        unit: "kn",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 200.0),
    },
    SourceDef {
        id: SourceId::FlightBankDeg,
        label_en: "Bank angle",
        label_ru: "Угол крена",
        unit: "deg",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (-90.0, 90.0),
    },
    SourceDef {
        id: SourceId::FlightFlapsPct,
        label_en: "Flaps position",
        label_ru: "Положение закрылков",
        unit: "%",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 100.0),
    },
    SourceDef {
        id: SourceId::FlightSlatsPct,
        label_en: "Slats position",
        label_ru: "Положение предкрылков",
        unit: "%",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 100.0),
    },
    SourceDef {
        id: SourceId::FlightSpoilersPct,
        label_en: "Spoilers position",
        label_ru: "Положение спойлеров",
        unit: "%",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 100.0),
    },
    SourceDef {
        id: SourceId::FlightGearHandle,
        label_en: "Gear handle",
        label_ru: "Рычаг шасси",
        unit: "",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 1.0),
    },
    SourceDef {
        id: SourceId::FlightGearCompNose,
        label_en: "Nose gear compression",
        label_ru: "Обжатие носовой стойки",
        unit: "%",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 100.0),
    },
    SourceDef {
        id: SourceId::FlightGearCompLeft,
        label_en: "Left gear compression",
        label_ru: "Обжатие левой стойки",
        unit: "%",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 100.0),
    },
    SourceDef {
        id: SourceId::FlightGearCompRight,
        label_en: "Right gear compression",
        label_ru: "Обжатие правой стойки",
        unit: "%",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 100.0),
    },
    SourceDef {
        id: SourceId::FlightEng1N2Pct,
        label_en: "Engine 1 N2",
        label_ru: "Двигатель 1, N2",
        unit: "%",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 105.0),
    },
    SourceDef {
        id: SourceId::FlightEng2N2Pct,
        label_en: "Engine 2 N2",
        label_ru: "Двигатель 2, N2",
        unit: "%",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 105.0),
    },
    SourceDef {
        id: SourceId::FlightEng1Rpm,
        label_en: "Engine 1 RPM",
        label_ru: "Двигатель 1, обороты",
        unit: "RPM",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 3000.0),
    },
    SourceDef {
        id: SourceId::FlightEng2Rpm,
        label_en: "Engine 2 RPM",
        label_ru: "Двигатель 2, обороты",
        unit: "RPM",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 3000.0),
    },
    SourceDef {
        id: SourceId::FlightProp1Rpm,
        label_en: "Prop 1 RPM",
        label_ru: "Винт 1, обороты",
        unit: "RPM",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 3000.0),
    },
    SourceDef {
        id: SourceId::FlightProp2Rpm,
        label_en: "Prop 2 RPM",
        label_ru: "Винт 2, обороты",
        unit: "RPM",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 3000.0),
    },
    SourceDef {
        id: SourceId::FlightOnGround,
        label_en: "On ground",
        label_ru: "На земле",
        unit: "",
        games: FLIGHT_ONLY,
        kind: SourceKind::Boolean,
        default_range: (0.0, 1.0),
    },
    SourceDef {
        id: SourceId::FlightStalled,
        label_en: "Stall warning",
        label_ru: "Срыв потока",
        unit: "",
        games: FLIGHT_ONLY,
        kind: SourceKind::Boolean,
        default_range: (0.0, 1.0),
    },
    SourceDef {
        id: SourceId::FlightOverspeedWarning,
        label_en: "Overspeed warning",
        label_ru: "Превышение скорости",
        unit: "",
        games: FLIGHT_ONLY,
        kind: SourceKind::Boolean,
        default_range: (0.0, 1.0),
    },
    SourceDef {
        id: SourceId::FlightOverspeedBarberPoleKn,
        label_en: "Overspeed threshold (Vmo/Mmo)",
        label_ru: "Порог Vmo/Mmo",
        unit: "kn",
        games: FLIGHT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 400.0),
    },
    SourceDef {
        id: SourceId::WtIasKmh,
        label_en: "Airspeed (IAS)",
        label_ru: "Приборная скорость (IAS)",
        unit: "km/h",
        games: WT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 900.0),
    },
    SourceDef {
        id: SourceId::WtSpeedKt,
        label_en: "Airspeed (IAS, kn)",
        label_ru: "Приборная скорость (узлы)",
        unit: "kn",
        games: WT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 600.0),
    },
    SourceDef {
        id: SourceId::WtAltitudeFt,
        label_en: "Altitude",
        label_ru: "Высота",
        unit: "ft",
        games: WT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 40000.0),
    },
    SourceDef {
        id: SourceId::WtAoaDeg,
        label_en: "Angle of attack",
        label_ru: "Угол атаки",
        unit: "deg",
        games: WT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 25.0),
    },
    SourceDef {
        id: SourceId::WtWxDegS,
        label_en: "Roll rate",
        label_ru: "Угловая скорость по крену",
        unit: "deg/s",
        games: WT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (-180.0, 180.0),
    },
    SourceDef {
        id: SourceId::WtRpm1,
        label_en: "Engine RPM",
        label_ru: "Обороты двигателя",
        unit: "RPM",
        games: WT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 3500.0),
    },
    SourceDef {
        id: SourceId::WtPower1Hp,
        label_en: "Engine power",
        label_ru: "Мощность двигателя",
        unit: "hp",
        games: WT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 2500.0),
    },
    SourceDef {
        id: SourceId::WtFlapsPct,
        label_en: "Flaps position",
        label_ru: "Положение закрылков",
        unit: "%",
        games: WT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 100.0),
    },
    SourceDef {
        id: SourceId::WtGearPct,
        label_en: "Gear position",
        label_ru: "Положение шасси",
        unit: "%",
        games: WT_ONLY,
        kind: SourceKind::Continuous,
        default_range: (0.0, 100.0),
    },
    SourceDef {
        id: SourceId::WtWeapon1Firing,
        label_en: "Weapon 1 firing",
        label_ru: "Стрельба, оружие 1",
        unit: "",
        games: WT_ONLY,
        kind: SourceKind::Boolean,
        default_range: (0.0, 1.0),
    },
    SourceDef {
        id: SourceId::WtWeapon2Firing,
        label_en: "Weapon 2 firing",
        label_ru: "Стрельба, оружие 2",
        unit: "",
        games: WT_ONLY,
        kind: SourceKind::Boolean,
        default_range: (0.0, 1.0),
    },
    SourceDef {
        id: SourceId::WtInMission,
        label_en: "In mission",
        label_ru: "В бою",
        unit: "",
        games: WT_ONLY,
        kind: SourceKind::Boolean,
        default_range: (0.0, 1.0),
    },
    SourceDef {
        id: SourceId::Lvar,
        label_en: "Custom variable (L:...)",
        label_ru: "Своя переменная (L:...)",
        // Настоящая единица — на самом эффекте (CustomEffect::lvar::unit),
        // здесь пусто: таблица SOURCES статическая и не знает имени/единицы
        // конкретной переменной пользователя.
        unit: "",
        games: MSFS_ONLY,
        kind: SourceKind::Continuous,
        // Нейтральный диапазон-заглушка для стартовых границ оси X кривой —
        // реальный диапазон значений LVAR пользователь задаёт сам через
        // ResponseCurve, т.к. таблица не может знать его заранее.
        default_range: (0.0, 1.0),
    },
];

/// Метаданные конкретного источника. Паникует, если `id` не заведён в
/// `SOURCES` — это баг таблицы (новый вариант `SourceId` без записи), а не
/// штатная ситуация рантайма, поэтому `expect`, а не `Option`.
pub fn def(id: SourceId) -> &'static SourceDef {
    SOURCES
        .iter()
        .find(|s| s.id == id)
        .expect("SourceId без записи в таблице SOURCES")
}

/// Один тик телеметрии, приведённый к общему виду для `read` — либо
/// MSFS/X-Plane (общий `FlightVars`), либо War Thunder (`WtVars`, другой
/// протокол и единицы измерения, отдельный вариант).
///
/// `FlightVars` заметно крупнее `WtVars` (добавился словарь `lvars`), из-за
/// чего clippy предлагает `Box<FlightVars>` — сознательно не делаем: этот
/// вариант конструируется и разбирается по значению/ссылке в добром десятке
/// мест (`sim/worker.rs`, `xp_link/worker.rs`, `ui.rs`, `ui/effects_editor.rs`,
/// `recorder.rs`, `custom_fx/player.rs`...), боксинг потянул бы правку всех
/// них ради разницы в пару сотен байт на структуре, которая живёт по одной
/// копии за тик, а не в горячем цикле с миллионами аллокаций.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum TelemetryFrame {
    Flight(FlightVars),
    Wt(WtVars),
}

fn bool_to_f64(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

/// Читает текущее значение источника из уже разобранного тика телеметрии.
/// Источник из ДРУГОЙ игры (например `Wt*` при `TelemetryFrame::Flight`) —
/// `None`, это не ошибка: так движок просто гасит эффект, если пользователь
/// оставил его настроенным на источник игры, телеметрии которой сейчас нет
/// (тот же принцип самонейтрализации, что у отсутствующих L-var в
/// `sim/parse.rs`). Булевы источники возвращают 0.0/1.0.
pub fn read(id: SourceId, frame: &TelemetryFrame) -> Option<f64> {
    use SourceId::*;
    match (id, frame) {
        (FlightAirspeedKn, TelemetryFrame::Flight(fv)) => Some(fv.airspeed_indicated),
        (FlightGroundSpeedKn, TelemetryFrame::Flight(fv)) => Some(fv.ground_speed_kt),
        (FlightBankDeg, TelemetryFrame::Flight(fv)) => Some(fv.bank_deg),
        (FlightFlapsPct, TelemetryFrame::Flight(fv)) => Some(fv.flaps_pct),
        (FlightSlatsPct, TelemetryFrame::Flight(fv)) => Some(fv.slats_pct),
        (FlightSpoilersPct, TelemetryFrame::Flight(fv)) => Some(fv.spoilers_pct),
        (FlightGearHandle, TelemetryFrame::Flight(fv)) => Some(fv.gear_handle),
        (FlightGearCompNose, TelemetryFrame::Flight(fv)) => Some(fv.gear_comp_nose),
        (FlightGearCompLeft, TelemetryFrame::Flight(fv)) => Some(fv.gear_comp_left),
        (FlightGearCompRight, TelemetryFrame::Flight(fv)) => Some(fv.gear_comp_right),
        (FlightEng1N2Pct, TelemetryFrame::Flight(fv)) => Some(fv.eng1_n2_percent),
        (FlightEng2N2Pct, TelemetryFrame::Flight(fv)) => Some(fv.eng2_n2_percent),
        (FlightEng1Rpm, TelemetryFrame::Flight(fv)) => Some(fv.eng1_rpm),
        (FlightEng2Rpm, TelemetryFrame::Flight(fv)) => Some(fv.eng2_rpm),
        (FlightProp1Rpm, TelemetryFrame::Flight(fv)) => Some(fv.prop1_rpm),
        (FlightProp2Rpm, TelemetryFrame::Flight(fv)) => Some(fv.prop2_rpm),
        (FlightOnGround, TelemetryFrame::Flight(fv)) => Some(bool_to_f64(fv.on_ground)),
        (FlightStalled, TelemetryFrame::Flight(fv)) => Some(bool_to_f64(fv.stalled)),
        (FlightOverspeedWarning, TelemetryFrame::Flight(fv)) => {
            Some(bool_to_f64(fv.overspeed_warning))
        }
        (FlightOverspeedBarberPoleKn, TelemetryFrame::Flight(fv)) => {
            Some(fv.overspeed_barber_pole_kn)
        }

        (WtIasKmh, TelemetryFrame::Wt(wv)) => Some(wv.ias_kmh),
        (WtSpeedKt, TelemetryFrame::Wt(wv)) => Some(wv.speed_kt),
        (WtAltitudeFt, TelemetryFrame::Wt(wv)) => Some(wv.altitude_ft),
        (WtAoaDeg, TelemetryFrame::Wt(wv)) => Some(wv.aoa_deg),
        (WtWxDegS, TelemetryFrame::Wt(wv)) => Some(wv.wx_deg_s),
        (WtRpm1, TelemetryFrame::Wt(wv)) => Some(wv.rpm_1),
        (WtPower1Hp, TelemetryFrame::Wt(wv)) => Some(wv.power_1_hp),
        (WtFlapsPct, TelemetryFrame::Wt(wv)) => Some(wv.flaps_pct),
        (WtGearPct, TelemetryFrame::Wt(wv)) => Some(wv.gear_pct),
        (WtWeapon1Firing, TelemetryFrame::Wt(wv)) => Some(bool_to_f64(wv.weapon1_firing)),
        (WtWeapon2Firing, TelemetryFrame::Wt(wv)) => Some(bool_to_f64(wv.weapon2_firing)),
        (WtInMission, TelemetryFrame::Wt(wv)) => Some(bool_to_f64(wv.in_mission)),

        _ => None,
    }
}

/// Читает значение пользовательской LVAR по имени, как его ввёл пользователь
/// (`model::CustomEffect::lvar::name`). Отдельная функция, а не ветка в
/// `read(id, frame)`: имя переменной живёт в `model::CustomEffect`, а
/// `model` уже зависит от `sources` (поле `source: SourceId`) — обратная
/// зависимость `sources -> model` дала бы цикл модулей. Поэтому выбор между
/// `read` и `read_lvar` делает вызывающий код (`engine.rs`), а не эта
/// функция.
///
/// `None`, если имени нет в словаре (переменная ещё не зарегистрирована/не
/// пришла от SimConnect) или кадр — `TelemetryFrame::Wt` (у War Thunder нет
/// понятия LVAR вообще, см. `MSFS_ONLY` выше).
pub fn read_lvar(name: &str, frame: &TelemetryFrame) -> Option<f64> {
    match frame {
        TelemetryFrame::Flight(fv) => fv.lvars.get(name).copied(),
        TelemetryFrame::Wt(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_returns_none_for_mismatched_game() {
        let flight_frame = TelemetryFrame::Flight(FlightVars::default());
        assert_eq!(read(SourceId::WtAoaDeg, &flight_frame), None);

        let wt_frame = TelemetryFrame::Wt(WtVars::default());
        assert_eq!(read(SourceId::FlightAirspeedKn, &wt_frame), None);
    }

    #[test]
    fn read_converts_boolean_sources_to_0_or_1() {
        let fv = FlightVars {
            stalled: true,
            ..Default::default()
        };
        let frame_true = TelemetryFrame::Flight(fv);
        assert_eq!(read(SourceId::FlightStalled, &frame_true), Some(1.0));

        let frame_false = TelemetryFrame::Flight(FlightVars::default());
        assert_eq!(read(SourceId::FlightStalled, &frame_false), Some(0.0));
    }

    #[test]
    fn read_extracts_continuous_values() {
        let fv = FlightVars {
            airspeed_indicated: 250.0,
            ..Default::default()
        };
        let frame = TelemetryFrame::Flight(fv);
        assert_eq!(read(SourceId::FlightAirspeedKn, &frame), Some(250.0));

        let wv = WtVars {
            aoa_deg: 12.5,
            ..Default::default()
        };
        let wt_frame = TelemetryFrame::Wt(wv);
        assert_eq!(read(SourceId::WtAoaDeg, &wt_frame), Some(12.5));
    }

    #[test]
    fn every_table_entry_resolves_via_def() {
        // def() паникует, если запись не найдена — прогон по всей таблице
        // гарантирует, что каждая запись SOURCES самосогласована (id внутри
        // записи находится по этому же id).
        for s in SOURCES {
            assert_eq!(def(s.id).id, s.id);
        }
    }

    #[test]
    fn read_lvar_returns_value_for_known_name() {
        let mut fv = FlightVars::default();
        fv.lvars.insert("L:A320_Gear_Nose".to_string(), 1000.0);
        let frame = TelemetryFrame::Flight(fv);
        assert_eq!(read_lvar("L:A320_Gear_Nose", &frame), Some(1000.0));
    }

    #[test]
    fn read_lvar_returns_none_for_unknown_name() {
        let frame = TelemetryFrame::Flight(FlightVars::default());
        assert_eq!(read_lvar("L:Does_Not_Exist", &frame), None);
    }

    #[test]
    fn read_lvar_returns_none_for_wt_frame() {
        let frame = TelemetryFrame::Wt(WtVars::default());
        assert_eq!(read_lvar("L:Anything", &frame), None);
    }

    #[test]
    fn lvar_source_is_registered_and_msfs_only() {
        let d = def(SourceId::Lvar);
        assert!(d.games.msfs);
        assert!(!d.games.xplane);
        assert!(!d.games.wt);
    }

    #[test]
    fn flight_and_wt_sources_are_mutually_exclusive_masks() {
        for s in SOURCES {
            assert_ne!(
                s.games,
                GameMask::default(),
                "источник не должен быть доступен во всех играх сразу"
            );
            assert!(!(s.games.wt && (s.games.msfs || s.games.xplane)));
        }
    }
}
