//! Минимальный набор телеметрии War Thunder, нужный для этапа 1 (см. план):
//! Weapon1/Weapon2, Flaps, Gear Transit & Doors. НЕ путать с
//! `wt_probe::model`/`schema` — та машинерия для recon (детект незнакомой
//! схемы, автопоиск счётчиков боеприпасов и т.д.), здесь же — точечное
//! чтение 4 конкретных, уже подтверждённых живым захватом полей.

use serde_json::Value;

/// Один тик телеметрии War Thunder, разобранный из `/state` + `/indicators`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WtVars {
    /// Секунды с начала опроса (аналог `FlightVars::sim_time_s`, но по
    /// wall-clock — у War Thunder нет понятия "время симулятора", см.
    /// `wt_link::worker`). Передаётся явно (а не берётся из `Instant::now()`
    /// внутри движка эффектов), чтобы `WtRumbleState::step` оставался чистой,
    /// легко тестируемой функцией — та же причина, по которой MSFS-версия
    /// берёт `sim_time_s` из `FlightVars`, а не из системных часов.
    pub t: f64,
    /// `/state`."valid" — идёт бой (в ангаре/меню всегда false, эффекты
    /// должны молчать, как `FlightVars::paused` в MSFS).
    pub in_mission: bool,
    /// `/indicators`.weapon1 — оружие первой группы стреляет ПРЯМО СЕЙЧАС
    /// (0.0/1.0 в API, подтверждено живым захватом — см. план).
    pub weapon1_firing: bool,
    pub weapon2_firing: bool,
    /// `/state`."flaps, %" — 0..100, как `FlightVars::flaps_pct` в MSFS.
    pub flaps_pct: f64,
    /// `/state`."gear, %" — 0..100, ОДНО значение на весь самолёт (в API
    /// War Thunder нет раздельных стоек, в отличие от MSFS).
    pub gear_pct: f64,
}

fn as_bool_flag(v: Option<&Value>) -> bool {
    v.and_then(Value::as_f64).unwrap_or(0.0) > 0.5
}

fn as_pct(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

/// Собирает `WtVars` из уже полученных тел `/state` и `/indicators`.
/// Отсутствующее поле (борт без второго орудия, старая версия API и т.п.)
/// самонейтрализуется в 0.0/false — тот же приём, что и в MSFS-парсинге
/// (sim/parse.rs) для L-var'ов, которых нет на конкретном борту.
pub fn parse(t: f64, state: &Value, indicators: &Value) -> WtVars {
    WtVars {
        t,
        in_mission: state.get("valid").and_then(Value::as_bool).unwrap_or(false),
        weapon1_firing: as_bool_flag(indicators.get("weapon1")),
        weapon2_firing: as_bool_flag(indicators.get("weapon2")),
        flaps_pct: as_pct(state.get("flaps, %")),
        gear_pct: as_pct(state.get("gear, %")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_known_fields() {
        let state = json!({"valid": true, "flaps, %": 33, "gear, %": 100});
        let indicators = json!({"weapon1": 1.0, "weapon2": 0.0});
        let vars = parse(1.5, &state, &indicators);
        assert_eq!(vars.t, 1.5);
        assert!(vars.in_mission);
        assert!(vars.weapon1_firing);
        assert!(!vars.weapon2_firing);
        assert_eq!(vars.flaps_pct, 33.0);
        assert_eq!(vars.gear_pct, 100.0);
    }

    #[test]
    fn missing_fields_self_neutralize() {
        let vars = parse(0.0, &json!({}), &json!({}));
        assert_eq!(vars, WtVars::default());
    }
}
