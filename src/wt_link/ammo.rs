//! Некоторые борта продолжают слать `weapon1`/`weapon2` = 1.0, пока зажат
//! спуск, даже когда боекомплект уже кончился — этот флаг в API значит
//! "спуск нажат", а не "снаряд реально вылетел" (баг замечен пользователем
//! вживую). Там, где для оружия есть счётчик боеприпасов (`ammo_counterN` в
//! `/indicators` — не на всех бортах он есть), гасим эффект стрельбы при
//! нуле патронов, вместо того чтобы слепо доверять сырому флагу.
//!
//! Какие именно `ammo_counterN` относятся к weapon1, а какие к weapon2, в
//! API не указано напрямую (разные борты — разная раскладка стволов по
//! счётчикам, подтверждено записанными сессиями: на одном борту weapon1 —
//! это counter2+3, weapon2 — counter1+4, на другом наоборот и по другим
//! индексам). Определяем это адаптивно: если поле стабильно убывает, пока
//! стреляет конкретное оружие, закрепляем счётчик за ним на всю сессию.
//!
//! Игрок иногда жмёт оба спуска одновременно ("огонь всем бортовым
//! оружием"), и даже при разжатии одного из них API отпускает флаг на 1-2
//! тика раньше, чем реально долетевший снаряд спишется со счётчика —
//! короткий шум на границе тиков, не настоящее "стреляет только одно
//! оружие". Поэтому засчитываем связку счётчик→оружие, только если это
//! оружие стреляло СОЛО (без второго) минимум `MIN_SOLO_TICKS` тиков
//! подряд — подтверждено разбором реальной сессии 2026-07-29 (fw-190a-4):
//! длинная очередь с обоими зажатыми спусками содержала именно такой
//! 2-тиковый шумовой провал у одного из флагов, из-за которого при пороге
//! в 1 тик счётчик пушки (ammo_counter1/4) ошибочно приписывался
//! пулемётам (weapon1), хотя те в этот момент не стреляли; порог в 3 тика
//! эту сессию учит чисто (weapon1→{counter2,3}, weapon2→{counter1,4}).

use std::collections::{HashMap, HashSet};

use serde_json::Value;

/// Сколько подряд тиков оружие должно стрелять соло (без второго), прежде
/// чем убывающий в это время счётчик считается надёжно его собственным.
const MIN_SOLO_TICKS: u32 = 3;

#[derive(Debug, Default)]
pub struct AmmoTracker {
    last_values: HashMap<String, f64>,
    weapon1_keys: HashSet<String>,
    weapon2_keys: HashSet<String>,
    weapon1_solo_ticks: u32,
    weapon2_solo_ticks: u32,
}

impl AmmoTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Скармливаем сырой `/indicators` каждого тика вместе с уже
    /// разобранными флагами стрельбы этого же тика.
    pub fn observe(&mut self, indicators: &Value, weapon1_firing: bool, weapon2_firing: bool) {
        self.weapon1_solo_ticks = if weapon1_firing && !weapon2_firing {
            self.weapon1_solo_ticks + 1
        } else {
            0
        };
        self.weapon2_solo_ticks = if weapon2_firing && !weapon1_firing {
            self.weapon2_solo_ticks + 1
        } else {
            0
        };

        let Some(obj) = indicators.as_object() else {
            return;
        };
        for (key, value) in obj {
            if !key.starts_with("ammo_counter") || key.ends_with("_lamp") {
                continue;
            }
            let Some(n) = value.as_f64() else { continue };
            let prev = self.last_values.insert(key.clone(), n);
            if let Some(prev) = prev
                && n < prev - 0.5
            {
                if self.weapon1_solo_ticks >= MIN_SOLO_TICKS {
                    self.weapon1_keys.insert(key.clone());
                } else if self.weapon2_solo_ticks >= MIN_SOLO_TICKS {
                    self.weapon2_keys.insert(key.clone());
                }
            }
        }
    }

    fn is_empty(&self, indicators: &Value, keys: &HashSet<String>) -> bool {
        if keys.is_empty() {
            // Ни разу не видели убывания при стрельбе этим оружием — либо
            // борт без счётчика, либо ещё не стреляли. В обоих случаях
            // лучше не гейтить вслепую, чем ошибочно заглушить эффект.
            return false;
        }
        let Some(obj) = indicators.as_object() else {
            return false;
        };
        keys.iter()
            .all(|k| obj.get(k).and_then(Value::as_f64).unwrap_or(1.0) <= 0.5)
    }

    pub fn weapon1_empty(&self, indicators: &Value) -> bool {
        self.is_empty(indicators, &self.weapon1_keys)
    }

    pub fn weapon2_empty(&self, indicators: &Value) -> bool {
        self.is_empty(indicators, &self.weapon2_keys)
    }

    /// Суммарный остаток по всем закреплённым за оружием счётчикам (сумма,
    /// а не минимум — так телеметрия показывает то же "общее число
    /// патронов", что и HUD самой игры для синхронизированной группы
    /// стволов). `None`, пока для оружия не выучено ни одного счётчика —
    /// либо борт без телеметрии боеприпасов, либо оружие ещё не стреляло.
    fn remaining(&self, indicators: &Value, keys: &HashSet<String>) -> Option<f64> {
        if keys.is_empty() {
            return None;
        }
        let obj = indicators.as_object()?;
        Some(
            keys.iter()
                .filter_map(|k| obj.get(k).and_then(Value::as_f64))
                .sum(),
        )
    }

    pub fn weapon1_ammo(&self, indicators: &Value) -> Option<f64> {
        self.remaining(indicators, &self.weapon1_keys)
    }

    pub fn weapon2_ammo(&self, indicators: &Value) -> Option<f64> {
        self.remaining(indicators, &self.weapon2_keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Кормит трекер N тиками, где убывает только `key`, соло для `w1`/`w2`
    /// (сколько раз нужно, чтобы перескочить `MIN_SOLO_TICKS`).
    fn fire_solo(t: &mut AmmoTracker, key: &str, mut n: f64, ticks: u32, w1: bool, w2: bool) -> f64 {
        for _ in 0..ticks {
            n -= 1.0;
            t.observe(&json!({ key: n }), w1, w2);
        }
        n
    }

    #[test]
    fn learns_association_from_decrease_during_firing_and_gates_at_zero() {
        let mut t = AmmoTracker::new();
        t.observe(&json!({"ammo_counter1": 60}), false, false);
        fire_solo(&mut t, "ammo_counter1", 60.0, MIN_SOLO_TICKS, true, false);
        assert!(!t.weapon1_empty(&json!({"ammo_counter1": 30})));
        assert!(t.weapon1_empty(&json!({"ammo_counter1": 0})));
    }

    #[test]
    fn unknown_ammo_field_never_gates() {
        let mut t = AmmoTracker::new();
        t.observe(&json!({}), true, false);
        assert!(!t.weapon1_empty(&json!({})));
    }

    #[test]
    fn single_solo_tick_is_not_enough_to_learn() {
        // Порог MIN_SOLO_TICKS защищает как раз от однотикового шума —
        // одного тика соло-стрельбы недостаточно, чтобы закрепить счётчик.
        let mut t = AmmoTracker::new();
        t.observe(&json!({"ammo_counter1": 60}), false, false);
        t.observe(&json!({"ammo_counter1": 59}), true, false);
        assert!(!t.weapon1_empty(&json!({"ammo_counter1": 0})));
    }

    #[test]
    fn does_not_confuse_weapon2_counter_with_weapon1() {
        let mut t = AmmoTracker::new();
        t.observe(&json!({"ammo_counter1": 60, "ammo_counter5": 1000}), false, false);
        fire_solo(&mut t, "ammo_counter1", 60.0, MIN_SOLO_TICKS, true, false);
        t.observe(&json!({"ammo_counter1": 57, "ammo_counter5": 1000}), false, false);
        fire_solo(&mut t, "ammo_counter5", 1000.0, MIN_SOLO_TICKS, false, true);
        // weapon1's own counter hitting 0 gates weapon1 regardless of weapon2's counter
        assert!(t.weapon1_empty(&json!({"ammo_counter1": 0, "ammo_counter5": 500})));
        assert!(!t.weapon2_empty(&json!({"ammo_counter1": 0, "ammo_counter5": 500})));
    }

    #[test]
    fn simultaneous_trigger_press_does_not_contaminate_association() {
        // Повторяет реальный случай из записанной сессии: weapon1 и weapon2
        // сперва учат каждый свой счётчик соло, потом игрок жмёт оба
        // спуска разом на несколько тиков, пока фактически стреляет только
        // "чужой" для weapon1 счётчик (ammo_counter1, на деле weapon2) —
        // это не должно приписать ammo_counter1 к weapon1.
        let mut t = AmmoTracker::new();
        t.observe(&json!({"ammo_counter1": 250, "ammo_counter2": 900}), false, false);
        fire_solo(&mut t, "ammo_counter2", 900.0, MIN_SOLO_TICKS, true, false);
        t.observe(&json!({"ammo_counter1": 250, "ammo_counter2": 897}), false, false);
        fire_solo(&mut t, "ammo_counter1", 250.0, MIN_SOLO_TICKS, false, true);
        // оба спуска зажаты несколько тиков, стреляет фактически только counter1
        for n in (240..247).rev() {
            t.observe(&json!({"ammo_counter1": n as f64, "ammo_counter2": 897}), true, true);
        }
        // weapon1's real counter (2) still has ammo -> must not be gated
        assert!(!t.weapon1_empty(&json!({"ammo_counter1": 0, "ammo_counter2": 897})));
        // and counter1 must not have leaked into weapon1's learned keys
        assert!(t.weapon2_empty(&json!({"ammo_counter1": 0, "ammo_counter2": 897})));
    }

    #[test]
    fn reports_remaining_ammo_once_learned_none_before() {
        let mut t = AmmoTracker::new();
        assert_eq!(t.weapon1_ammo(&json!({"ammo_counter1": 60})), None);
        t.observe(&json!({"ammo_counter1": 60}), false, false);
        fire_solo(&mut t, "ammo_counter1", 60.0, MIN_SOLO_TICKS, true, false);
        assert_eq!(t.weapon1_ammo(&json!({"ammo_counter1": 40})), Some(40.0));
    }

    #[test]
    fn reset_forgets_learned_association() {
        let mut t = AmmoTracker::new();
        t.observe(&json!({"ammo_counter1": 60}), false, false);
        fire_solo(&mut t, "ammo_counter1", 60.0, MIN_SOLO_TICKS, true, false);
        t.reset();
        assert!(!t.weapon1_empty(&json!({"ammo_counter1": 0})));
    }
}
