//! Эффекты этапа 1 поддержки War Thunder: Weapon1/Weapon2 (стрельба), Flaps,
//! Gear Transit & Doors. Маленький аналог `crate::rumble::RumbleEngine`, но
//! без сложностей MSFS-модели (несколько стоек шасси, 4 двигателя, пауза
//! симулятора и т.д.) — у War Thunder ровно 4 сигнала на входе (см.
//! `wt_link::vars::WtVars`).
//!
//! Weapon1/Weapon2 используют текстуру "гул с джиттером амплитуды",
//! подобранную и подтверждённую на живом железе (сессия калибровки
//! `src/bin/test_gun1.rs`, 2026-07-29) — см. `GunPreset` в `crate::types` и
//! его дефолты для weapon1/weapon2 в `WtConfig::default`. Маршрутизация
//! ЖЁСТКО зафиксирована в этом файле (не через `device_targets`): weapon1 →
//! только джойстик, weapon2 → только РУД (оба мотора одновременно) — так
//! подтверждено пользователем, разведение по рукам и есть смысл эффекта.
//! Несущая weapon1 (изначально подобранные 12.5 Гц) в `WtConfig::default`
//! понижена до 6.5 Гц: 12.5 Гц даёт пик короче интервала отправки штатного
//! HID-канала (`hid_worker`, 20 Гц/50мс) — алиасинг; 6.5 Гц проходит с
//! запасом (см. `thump_min_period_s` в `crate::rumble` — та же граница).
//! Flaps и Gear Transit & Doors переиспользуют ту же математику, что и
//! одноимённые MSFS-эффекты в `crate::rumble`.

use std::f64::consts::PI;
use std::time::Instant;

use crate::rumble::RumbleOutput;
use crate::types::{GunPreset, WtConfig};
use crate::EffectsSnapshot;

use super::vars::WtVars;

/// Минимальный xorshift32 — та же реализация, что и в test_gun1.rs (только
/// для джиттера амплитуды текстуры, криптостойкость не нужна).
#[derive(Debug)]
struct Xorshift32(u32);

impl Xorshift32 {
    fn seeded(salt: u32) -> Self {
        let nanos = Instant::now().elapsed().subsec_nanos();
        Self(((nanos ^ salt) | 1).max(1))
    }

    fn next_unit(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f64) / (u32::MAX as f64 + 1.0)
    }
}

fn gun_cycle_index(t: f64, carrier_freq_hz: f64) -> i64 {
    (t / (1.0 / carrier_freq_hz)).floor() as i64
}

/// Огибающая одного цикла несущей: скважность `preset.duty_pct` на пике (с
/// джиттером амплитуды и attack-рампой от начала очереди), иначе — нижний
/// уровень `preset.floor` (текстура "несколько стволов, не в идеальный
/// унисон"). `floor` дополнительно подрезается до `peak`, если тот вдруг
/// меньше (защита на случай, если пользователь когда-нибудь получит доступ
/// к этим полям в UI и поставит peak ниже floor).
fn gun_pulse(t: f64, fire_started_t: f64, preset: &GunPreset, cycle_jitter_mul: f64) -> f64 {
    let peak = preset.peak as f64;
    if peak <= 0.0 {
        return 0.0;
    }
    let carrier_freq_hz = (preset.carrier_freq_hz as f64).max(0.1);
    let duty = (preset.duty_pct as f64 / 100.0).clamp(0.0, 1.0);
    let attack_s = (preset.attack_ms as f64 / 1000.0).max(0.0);

    let phase = (t / (1.0 / carrier_freq_hz)).fract();
    let since_fire = (t - fire_started_t).max(0.0);
    let attack_ramp = if attack_s <= 0.0 {
        1.0
    } else {
        (since_fire / attack_s).clamp(0.0, 1.0)
    };
    if phase < duty {
        (peak * cycle_jitter_mul * attack_ramp).clamp(0.0, 255.0)
    } else {
        (preset.floor as f64).min(peak)
    }
}

#[derive(Debug)]
struct GunState {
    was_firing: bool,
    fire_started_t: f64,
    last_cycle_idx: i64,
    cycle_jitter_mul: f64,
    rng: Xorshift32,
}

impl GunState {
    fn new(salt: u32) -> Self {
        Self {
            was_firing: false,
            fire_started_t: 0.0,
            last_cycle_idx: -1,
            cycle_jitter_mul: 1.0,
            rng: Xorshift32::seeded(salt),
        }
    }

    /// Возвращает силу эффекта на этот тик (0.0, если оружие не стреляет).
    fn step(&mut self, t: f64, firing: bool, preset: &GunPreset) -> f64 {
        if !firing {
            self.was_firing = false;
            return 0.0;
        }
        if !self.was_firing {
            self.fire_started_t = t;
            self.last_cycle_idx = -1;
        }
        self.was_firing = true;

        let carrier_freq_hz = (preset.carrier_freq_hz as f64).max(0.1);
        let jitter = (preset.jitter_pct as f64 / 100.0).clamp(0.0, 1.0);
        let cycle_idx = gun_cycle_index(t, carrier_freq_hz);
        if cycle_idx != self.last_cycle_idx {
            self.last_cycle_idx = cycle_idx;
            self.cycle_jitter_mul = 1.0 - jitter * self.rng.next_unit();
        }
        gun_pulse(t, self.fire_started_t, preset, self.cycle_jitter_mul)
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Flaps — та же математика, что и MSFS-эффект в crate::rumble (округление
// до целого %, детект изменения кадр-к-кадру, короткое окно активности,
// 25 Гц полу-синусный ШИМ, чередование throttle-каналов раз в 500мс).
// ═══════════════════════════════════════════════════════════════════════
const FLAPS_BUMP_DURATION_S: f64 = 0.15;

// ═══════════════════════════════════════════════════════════════════════
// Gear Transit & Doors — гул движения (та же ритмика, что и в MSFS-версии,
// см. crate::rumble) + удар фиксации на замке при достижении 0%/100%
// (та же attack/decay синус-огибающая, что и у "Gear Bump" в MSFS, сила —
// cfg.gear_peak, длительность — GEAR_LOCK_BUMP_DURATION_S).
// ═══════════════════════════════════════════════════════════════════════
const GEAR_LOCK_BUMP_DURATION_S: f64 = 0.8;
const GEAR_LOCKED_THRESHOLD_PCT: f64 = 0.5; // клиренс вокруг 0%/100% — считается "на замке"

#[derive(Debug)]
pub struct WtRumbleState {
    weapon1: GunState,
    weapon2: GunState,

    last_flaps_pct_rounded: i32,
    flaps_active_until_t: f64,

    gear_initialized: bool,
    prev_gear_pct: f64,
    gear_lock_t0: f64,
    gear_lock_t1: f64,
    gear_lock_peak: f64,
}

impl Default for WtRumbleState {
    fn default() -> Self {
        Self::new()
    }
}

impl WtRumbleState {
    pub fn new() -> Self {
        Self {
            weapon1: GunState::new(0x9E3779B9),
            weapon2: GunState::new(0x85EBCA6B),
            last_flaps_pct_rounded: i32::MIN,
            flaps_active_until_t: -1.0,
            gear_initialized: false,
            prev_gear_pct: 0.0,
            gear_lock_t0: -1.0,
            gear_lock_t1: -1.0,
            gear_lock_peak: 0.0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn step(&mut self, vars: &WtVars, cfg: &WtConfig, hold: bool) -> RumbleOutput {
        let mut effects = EffectsSnapshot::default();

        if hold || !vars.in_mission {
            return RumbleOutput {
                joystick_intensity: 0,
                throttle_left_intensity: 0,
                throttle_right_intensity: 0,
                effects,
            };
        }

        let t = vars.t;
        let dt_ = &cfg.device_targets;

        let mut joystick: f64 = 0.0;
        let mut throttle_left: f64 = 0.0;
        let mut throttle_right: f64 = 0.0;

        // --- Weapon1 / Weapon2 ---
        // Маршрутизация жёсткая (см. комментарий в шапке файла): weapon1 →
        // ТОЛЬКО джойстик, weapon2 → ТОЛЬКО РУД, сразу оба мотора. Не через
        // cfg.device_targets — WtDeviceTargets этих полей больше не содержит.
        let w1_term = if cfg.weapon1_enabled {
            self.weapon1.step(t, vars.weapon1_firing, &cfg.weapon1_gun)
        } else {
            0.0
        };
        let w2_term = if cfg.weapon2_enabled {
            self.weapon2.step(t, vars.weapon2_firing, &cfg.weapon2_gun)
        } else {
            0.0
        };
        effects.wt_weapon1_active = cfg.weapon1_enabled && vars.weapon1_firing;
        effects.wt_weapon2_active = cfg.weapon2_enabled && vars.weapon2_firing;

        joystick += w1_term;
        throttle_left += w2_term;
        throttle_right += w2_term;

        // --- Flaps ---
        let flaps_pct_rounded = vars.flaps_pct.round() as i32;
        let pct_changed = self.last_flaps_pct_rounded != i32::MIN
            && self.last_flaps_pct_rounded != flaps_pct_rounded;
        self.last_flaps_pct_rounded = flaps_pct_rounded;
        if pct_changed {
            self.flaps_active_until_t = t + FLAPS_BUMP_DURATION_S;
        }
        let flaps_is_moving = t < self.flaps_active_until_t;

        if cfg.flaps_enabled && flaps_is_moving {
            let max_amplitude = (cfg.flaps_peak as f64 / 255.0).clamp(0.01, 0.8);
            let fixed_period = 0.04; // 25 Гц
            let cycle = (t / fixed_period).fract();
            let oscillation = (PI * cycle).sin();
            let flaps_term = max_amplitude * 255.0 * oscillation;

            // Чередуем throttle-канал каждые 500мс — та же текстура, что и в
            // MSFS-версии эффекта (см. crate::rumble).
            let is_left_phase = (t / 0.5).floor() as i64 % 2 == 0;

            if dt_.flaps.enable_joystick {
                joystick += flaps_term;
            }
            if dt_.flaps.enable_throttle {
                if is_left_phase {
                    throttle_left += flaps_term;
                } else {
                    throttle_right += flaps_term;
                }
            }
            effects.flaps_bump_active = true;
        } else {
            effects.flaps_bump_active = false;
        }

        // --- Gear Transit & Doors ---
        if !self.gear_initialized {
            // Первый кадр: не считаем стартовое положение шасси "движением"
            // или "переходом на замок" — иначе борт, заспавнившийся сразу с
            // убранным/выпущенным шасси, дал бы ложный удар фиксации.
            self.prev_gear_pct = vars.gear_pct;
            self.gear_initialized = true;
        }

        let gear_is_moving = vars.gear_pct > GEAR_LOCKED_THRESHOLD_PCT
            && vars.gear_pct < 100.0 - GEAR_LOCKED_THRESHOLD_PCT
            && (vars.gear_pct - self.prev_gear_pct).abs() >= 0.01;

        let now_locked = vars.gear_pct <= GEAR_LOCKED_THRESHOLD_PCT
            || vars.gear_pct >= 100.0 - GEAR_LOCKED_THRESHOLD_PCT;
        let prev_locked = self.prev_gear_pct <= GEAR_LOCKED_THRESHOLD_PCT
            || self.prev_gear_pct >= 100.0 - GEAR_LOCKED_THRESHOLD_PCT;

        let mut gear_term: f64 = 0.0;
        if cfg.gear_transit_enabled {
            if now_locked && !prev_locked {
                self.gear_lock_t0 = t;
                self.gear_lock_t1 = t + GEAR_LOCK_BUMP_DURATION_S;
                self.gear_lock_peak = cfg.gear_peak as f64;
            }

            if gear_is_moving {
                // Тот же ритм гула, что и у MSFS Gear Transit (см. crate::rumble):
                // редкие "тяжёлые" толчки раз в 3 такта, лёгкие — между ними.
                let beat_duration = 60.0 / 80.0;
                let current_beat = t / beat_duration;
                let beat_phase = current_beat.fract();
                let beat_index = (current_beat.floor() as i64) % 3;
                if beat_index == 0 {
                    if beat_phase < 0.35 {
                        gear_term += 40.0;
                    }
                } else if beat_phase < 0.15 {
                    gear_term += 15.0;
                }
            }

            let lock_active =
                t >= self.gear_lock_t0 && t <= self.gear_lock_t1 && self.gear_lock_peak > 0.0;
            if lock_active {
                let p = ((t - self.gear_lock_t0) / (self.gear_lock_t1 - self.gear_lock_t0))
                    .clamp(0.0, 1.0);
                gear_term += self.gear_lock_peak * (PI * p).sin();
            }

            effects.gear_transit_active = gear_is_moving || lock_active;

            if dt_.gear_transit.enable_joystick {
                joystick += gear_term;
            }
            if dt_.gear_transit.enable_throttle {
                throttle_left += gear_term;
                throttle_right += gear_term;
            }
        } else {
            effects.gear_transit_active = false;
        }

        self.prev_gear_pct = vars.gear_pct;

        RumbleOutput {
            joystick_intensity: joystick.clamp(0.0, 255.0).round() as u8,
            throttle_left_intensity: throttle_left.clamp(0.0, 255.0).round() as u8,
            throttle_right_intensity: throttle_right.clamp(0.0, 255.0).round() as u8,
            effects,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WtConfig {
        WtConfig::default()
    }

    fn vars() -> WtVars {
        WtVars {
            t: 0.0,
            in_mission: true,
            weapon1_firing: false,
            weapon2_firing: false,
            flaps_pct: 0.0,
            gear_pct: 100.0,
        }
    }

    #[test]
    fn silent_when_not_in_mission() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        v.in_mission = false;
        v.weapon1_firing = true;
        let out = engine.step(&v, &cfg(), false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn silent_on_hold() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        v.weapon1_firing = true;
        let out = engine.step(&v, &cfg(), true);
        assert_eq!(out.joystick_intensity, 0);
    }

    #[test]
    fn weapon1_firing_routes_to_joystick_only() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        v.weapon1_firing = true;
        // Несколько тиков (20 Гц), чтобы пройти attack-рампу (default attack_ms=41).
        let mut out = engine.step(&v, &cfg(), false);
        for i in 1..10 {
            v.t = i as f64 * 0.02;
            out = engine.step(&v, &cfg(), false);
        }
        assert!(out.joystick_intensity > 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn weapon2_firing_routes_to_both_throttle_motors_only() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        v.weapon2_firing = true;
        let mut out = engine.step(&v, &cfg(), false);
        for i in 1..10 {
            v.t = i as f64 * 0.02;
            out = engine.step(&v, &cfg(), false);
        }
        assert_eq!(out.joystick_intensity, 0);
        assert!(out.throttle_left_intensity > 0);
        assert!(out.throttle_right_intensity > 0);
    }

    #[test]
    fn weapon_disabled_stays_silent_even_while_firing() {
        let mut engine = WtRumbleState::new();
        let mut c = cfg();
        c.weapon1_enabled = false;
        c.weapon2_enabled = false;
        let mut v = vars();
        v.weapon1_firing = true;
        v.weapon2_firing = true;
        let out = engine.step(&v, &c, false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn flaps_change_triggers_bump_then_settles() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        engine.step(&v, &cfg(), false); // baseline at 0%, t=0.0
        v.flaps_pct = 30.0;
        v.t = 0.05;
        let out = engine.step(&v, &cfg(), false);
        assert!(out.joystick_intensity > 0 || out.throttle_left_intensity > 0);

        // После окончания короткого окна (FLAPS_BUMP_DURATION_S=0.15с) без
        // нового изменения — эффект должен смолкнуть.
        v.t = 0.05 + FLAPS_BUMP_DURATION_S + 0.05;
        let out2 = engine.step(&v, &cfg(), false);
        assert_eq!(out2.joystick_intensity, 0);
        assert_eq!(out2.throttle_left_intensity, 0);
        assert_eq!(out2.throttle_right_intensity, 0);
    }

    #[test]
    fn gear_does_not_lock_bump_on_first_frame() {
        // Первый кадр уже приходит с gear_pct=100 (выпущено с начала полёта) —
        // не должно считаться переходом "на замок".
        let mut engine = WtRumbleState::new();
        let v = vars(); // gear_pct = 100.0
        let out = engine.step(&v, &cfg(), false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn gear_retraction_then_lock_produces_bump() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        v.gear_pct = 100.0;
        engine.step(&v, &cfg(), false); // init at 100 (down & locked), t=0.0
        v.gear_pct = 50.0;
        v.t = 0.1;
        engine.step(&v, &cfg(), false); // in transit
        v.gear_pct = 0.0;
        v.t = 0.2;
        engine.step(&v, &cfg(), false); // edge tick: attack of the lock-bump sine starts at 0
        // Пик attack/decay-огибающей приходится не на сам фронт, а чуть позже —
        // проверяем следующий тик внутри окна удара (GEAR_LOCK_BUMP_DURATION_S=0.8с).
        v.t = 0.3;
        let locked_out = engine.step(&v, &cfg(), false);
        assert!(
            locked_out.joystick_intensity > 0
                || locked_out.throttle_left_intensity > 0
                || locked_out.throttle_right_intensity > 0
        );
    }

    #[test]
    fn gear_transit_disabled_never_bumps() {
        let mut engine = WtRumbleState::new();
        let mut c = cfg();
        c.gear_transit_enabled = false;
        let mut v = vars();
        v.gear_pct = 100.0;
        engine.step(&v, &c, false);
        v.gear_pct = 0.0;
        let out = engine.step(&v, &c, false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }
}
