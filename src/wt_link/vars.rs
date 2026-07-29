//! Минимальный набор телеметрии War Thunder, нужный для этапа 1 (см. план):
//! Weapon1/Weapon2, Flaps, Gear Transit & Doors. НЕ путать с
//! `wt_probe::model`/`schema` — та машинерия для recon (детект незнакомой
//! схемы, автопоиск счётчиков боеприпасов и т.д.), здесь же — точечное
//! чтение 4 конкретных, уже подтверждённых живым захватом полей.

use serde_json::Value;

/// Один тик телеметрии War Thunder, разобранный из `/state` + `/indicators`.
#[derive(Debug, Clone, Default, PartialEq)]
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
    /// Остаток патронов, если для этого оружия удалось выучить счётчик
    /// (см. `wt_link::ammo::AmmoTracker`) — заполняется в `worker.rs`
    /// ПОСЛЕ `parse`, не самим `parse` (это не поле `/state`/`/indicators`
    /// напрямую, а результат отдельного адаптивного трекера с состоянием
    /// на всю сессию, не тик). `None` — борт без телеметрии боеприпасов
    /// или оружие ещё не стреляло.
    pub weapon1_ammo: Option<f64>,
    pub weapon2_ammo: Option<f64>,
    /// `/indicators`.type — название техники (например, "fw190a4"). Пустая
    /// строка, если поле отсутствует (борт без этой телеметрии/ещё не
    /// определилось после захода в бой).
    pub vehicle_type: String,
    /// `/state`."IAS, km/h", переведено в узлы — для отображения в панели
    /// телеметрии рядом с остальными полями, которые уже в узлах (см.
    /// `FlightVars::airspeed_indicated` в MSFS-конвейере).
    pub speed_kt: f64,
    /// `/state`."H, m", переведено в футы — та же причина, что и для
    /// `speed_kt` (единая единица измерения с MSFS-панелью).
    pub altitude_ft: f64,
    /// `/state`."IAS, km/h", БЕЗ конвертации (в отличие от `speed_kt`) — нужна
    /// в исходных км/ч для сравнения с порогами `aero_profiles::StallProfile`
    /// (`ias_safety_cutoff_kmh`), которые заданы в км/ч.
    pub ias_kmh: f64,
    /// `/state`."AoA, deg" — угол атаки, для эффекта срыва потока/сваливания
    /// (см. `wt_link::aero_profiles`, `wt_link::rumble::StallState`).
    pub aoa_deg: f64,
    /// `/state`."Wx, deg/s" — угловая скорость по крену (град/с). Единственная
    /// реально подтверждённая угловая скорость в API War Thunder (нет Wy/Wz) —
    /// см. память `wt_link_wx_angular_rate_field_found` — потенциальный
    /// сигнал для будущего детекта штопора, пока только отображается в
    /// панели телеметрии.
    pub wx_deg_s: f64,
}

const KMH_TO_KT: f64 = 1.0 / 1.852;
const M_TO_FT: f64 = 3.280_839_895_013_123;

fn as_bool_flag(v: Option<&Value>) -> bool {
    v.and_then(Value::as_f64).unwrap_or(0.0) > 0.5
}

fn as_f64(v: Option<&Value>) -> f64 {
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
        flaps_pct: as_f64(state.get("flaps, %")),
        gear_pct: as_f64(state.get("gear, %")),
        weapon1_ammo: None,
        weapon2_ammo: None,
        vehicle_type: indicators
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        speed_kt: as_f64(state.get("IAS, km/h")) * KMH_TO_KT,
        altitude_ft: as_f64(state.get("H, m")) * M_TO_FT,
        ias_kmh: as_f64(state.get("IAS, km/h")),
        aoa_deg: as_f64(state.get("AoA, deg")),
        wx_deg_s: as_f64(state.get("Wx, deg/s")),
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
    fn converts_speed_and_altitude_units() {
        let state = json!({"IAS, km/h": 370.4, "H, m": 1000.0});
        let vars = parse(0.0, &state, &json!({}));
        assert!((vars.speed_kt - 200.0).abs() < 0.01);
        assert!((vars.altitude_ft - 3280.84).abs() < 0.01);
    }

    #[test]
    fn missing_fields_self_neutralize() {
        let vars = parse(0.0, &json!({}), &json!({}));
        assert_eq!(vars, WtVars::default());
    }

    #[test]
    fn parses_aoa_and_ias_from_real_recorded_body() {
        // Тело реального `/state` из wt_probe_sessions/session_20260728_204134.jsonl
        // (Bf 109 E-3), строка 9 — используется как фикстура для срыва потока.
        let state = json!({
            "AoA, deg": 0.2, "AoS, deg": -0.0, "H, m": 2519, "IAS, km/h": 380,
            "M": 0.36, "Mfuel, kg": 90, "Mfuel0, kg": 303, "Ny": 1.0,
            "RPM 1": 2301, "RPM throttle 1, %": 64, "TAS, km/h": 431,
            "Vy, m/s": 2.2, "Wx, deg/s": -0.0, "aileron, %": -0.0,
            "efficiency 1, %": 86, "elevator, %": -4, "flaps, %": 0, "gear, %": 0,
            "magneto 1": 3, "manifold pressure 1, atm": 1.28, "oil temp 1, C": 53,
            "pitch 1, deg": 36.4, "power 1, hp": 974.2, "radiator 1, %": 0,
            "rudder, %": -8, "throttle 1, %": 100, "thrust 1, kgs": 535,
            "valid": true, "water temp 1, C": 55
        });
        let vars = parse(0.1, &state, &json!({}));
        assert_eq!(vars.ias_kmh, 380.0);
        assert_eq!(vars.aoa_deg, 0.2);
        assert_eq!(vars.wx_deg_s, -0.0);
    }
}
