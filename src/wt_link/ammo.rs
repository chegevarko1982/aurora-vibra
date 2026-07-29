//! Некоторые борта продолжают слать `weapon1`/`weapon2` = 1.0, пока зажат
//! спуск, даже когда боекомплект уже кончился — этот флаг в API значит
//! "спуск нажат", а не "снаряд реально вылетел" (баг замечен пользователем
//! вживую). Там, где для оружия есть счётчик боеприпасов (`ammo_counterN` в
//! `/indicators` — не на всех бортах он есть), гасим эффект стрельбы при
//! нуле патронов, вместо того чтобы слепо доверять сырому флагу.
//!
//! Какие именно `ammo_counterN` относятся к weapon1, а какие к weapon2, в
//! API не указано напрямую (разные борты — разная раскладка стволов по
//! счётчикам). Определяем это адаптивно: если поле убыло в тот же тик, когда
//! стреляло конкретное оружие, закрепляем счётчик за ним на всю сессию.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

#[derive(Debug, Default)]
pub struct AmmoTracker {
    last_values: HashMap<String, f64>,
    weapon1_keys: HashSet<String>,
    weapon2_keys: HashSet<String>,
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
                if weapon1_firing {
                    self.weapon1_keys.insert(key.clone());
                }
                if weapon2_firing {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn learns_association_from_decrease_during_firing_and_gates_at_zero() {
        let mut t = AmmoTracker::new();
        t.observe(&json!({"ammo_counter1": 60}), false, false);
        t.observe(&json!({"ammo_counter1": 59}), true, false);
        assert!(!t.weapon1_empty(&json!({"ammo_counter1": 59})));
        assert!(t.weapon1_empty(&json!({"ammo_counter1": 0})));
    }

    #[test]
    fn unknown_ammo_field_never_gates() {
        let mut t = AmmoTracker::new();
        t.observe(&json!({}), true, false);
        assert!(!t.weapon1_empty(&json!({})));
    }

    #[test]
    fn does_not_confuse_weapon2_counter_with_weapon1() {
        let mut t = AmmoTracker::new();
        t.observe(&json!({"ammo_counter1": 60, "ammo_counter5": 1000}), false, false);
        t.observe(&json!({"ammo_counter1": 59, "ammo_counter5": 1000}), true, false);
        t.observe(&json!({"ammo_counter1": 59, "ammo_counter5": 999}), false, true);
        // weapon1's own counter hitting 0 gates weapon1 regardless of weapon2's counter
        assert!(t.weapon1_empty(&json!({"ammo_counter1": 0, "ammo_counter5": 500})));
        assert!(!t.weapon2_empty(&json!({"ammo_counter1": 0, "ammo_counter5": 500})));
    }

    #[test]
    fn reset_forgets_learned_association() {
        let mut t = AmmoTracker::new();
        t.observe(&json!({"ammo_counter1": 60}), false, false);
        t.observe(&json!({"ammo_counter1": 59}), true, false);
        t.reset();
        assert!(!t.weapon1_empty(&json!({"ammo_counter1": 0})));
    }
}
