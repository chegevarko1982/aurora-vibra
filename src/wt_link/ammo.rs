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
//!
//! ## Fallback для бортов без единого ключа `weapon1..weapon4`
//!
//! На части бортов (подтверждено записями) API вообще не шлёт ни одного
//! булевого ключа `weapon1..weapon4` — единственный сигнал стрельбы это
//! убывание похожих на боеприпасы полей (`infer_firing_from_ammo_sum`).
//! Учитель `weapon1_keys`/`weapon2_keys` выше для этой ветки не годится
//! в принципе: он заполняется в `observe()` по соло-тикам от УЖЕ готовых
//! булевых флагов `weapon1_firing`/`weapon2_firing`, а раз таких ключей в
//! API нет, эти флаги никогда не становятся `true`, и счётчики соло-тиков
//! (`weapon1_solo_ticks`/`weapon2_solo_ticks`) никогда не растут.
//!
//! Поэтому `infer_firing_from_ammo_sum` ведёт свой собственный, полностью
//! автономный 2-кластерный учитель — без внешней истины о том, кто
//! стреляет, определяем это по паттерну совместного убывания ключей друг с
//! другом. Первое же убывание любых полей сразу (без задержки — иначе
//! борт с одним-единственным типом боеприпасов вообще никогда бы не
//! репортил стрельбу) становится "базовым" кластером (`fallback_bucket_a`,
//! репортится как weapon1 по умолчанию). Если позже появляется набор
//! ключей, убывающий БЕЗ участия базового кластера несколько тиков подряд
//! (`MIN_SOLO_TICKS`, тот же порог и то же обоснование, что выше), это
//! закрепляется как второй кластер (`fallback_bucket_b`, weapon2). Ключи,
//! убывающие ВМЕСТЕ с уже известным кластером, сразу присоединяются к нему
//! (тот же ствол/группа стволов), а не считаются кандидатом на отдельное
//! оружие.
//!
//! `set_weapon_capacity_hint` — необязательная подсказка ожидаемой ёмкости
//! боекомплекта по борту (из статической таблицы `weapon_profiles`,
//! перенесённой из датамайн-CSV). Она НЕ создаёт кластеры и не подменяет
//! живой сигнал — только один раз, когда оба кластера уже сформированы,
//! выбирает, какой из них назвать weapon1, а какой weapon2, по ближайшему
//! совпадению стартовой суммы кластера с ожидаемой ёмкостью. На бортах без
//! единого поля боеприпасов вообще (например, известный пробел телеметрии
//! A6M3 Zero) кластеры не формируются никогда — никакая CSV-подсказка это
//! не исправит, сигнала для неё просто нет.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

/// Сколько подряд тиков оружие должно стрелять соло (без второго), прежде
/// чем убывающий в это время счётчик считается надёжно его собственным.
const MIN_SOLO_TICKS: u32 = 3;

/// Ключевые слова для отлова "похожих на боеприпасы" полей на бортах, где
/// `weapon1..weapon4` в `/indicators` не встречаются вообще (см.
/// `infer_firing_from_ammo_sum`) — шире, чем подтверждённый живыми сессиями
/// префикс `ammo_counter`, на случай другой раскладки полей на
/// неисследованных бортах.
const AMMO_KEYWORDS: [&str; 5] = ["ammo", "bullets", "rounds", "mg", "cannon"];
/// Патроны убывают целыми числами — этого порога достаточно, чтобы не
/// принять погрешность округления f64 за реальное расходование.
const SUM_DECREASE_EPS: f64 = 0.5;

fn is_ammo_like_key(key: &str) -> bool {
    // Ключи `/indicators` у War Thunder всегда в нижнем snake_case
    // (подтверждено записанными сессиями) — приведение к нижнему регистру
    // не нужно и лишь аллоцировало бы новую строку на каждый ключ каждого
    // тика (~100-200 ключей * 20 Гц).
    if key.ends_with("_lamp") {
        return false;
    }
    AMMO_KEYWORDS.iter().any(|kw| key.contains(kw))
}

/// Результат `infer_firing_from_ammo_sum` за один тик — оба поля
/// независимы и могут быть `true` одновременно (игрок реально жмёт оба
/// спуска разом, это подтверждено записанными сессиями).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FallbackFiring {
    pub weapon1: bool,
    pub weapon2: bool,
}

#[derive(Debug)]
pub struct AmmoTracker {
    last_values: HashMap<String, f64>,
    weapon1_keys: HashSet<String>,
    weapon2_keys: HashSet<String>,
    weapon1_solo_ticks: u32,
    weapon2_solo_ticks: u32,
    previous_ammo_sum: Option<f64>,

    // Состояние автономного 2-кластерного учителя fallback-ветки
    // (`infer_firing_from_ammo_sum`) — намеренно отдельное от полей выше,
    // чтобы не путать два независимых механизма (см. doc-комментарий
    // модуля).
    fallback_last_values: HashMap<String, f64>,
    fallback_bucket_a: HashSet<String>,
    fallback_bucket_b: HashSet<String>,
    fallback_pending: HashSet<String>,
    fallback_pending_ticks: u32,
    fallback_bucket_a_starting_total: Option<f64>,
    fallback_bucket_b_starting_total: Option<f64>,
    /// Какой из двух кластеров сейчас репортится как weapon1. По умолчанию
    /// `true` (кластер A = weapon1, "первый увиденный" — соглашение,
    /// сохраняющее старое мгновенное поведение для борта с одним типом
    /// боеприпасов). Меняется не более одного раза за сессию, см.
    /// `set_weapon_capacity_hint`.
    bucket_a_is_weapon1: bool,
    weapon_capacity_hint_applied: bool,
    weapon_capacity_hint: (Option<f64>, Option<f64>),
}

impl Default for AmmoTracker {
    fn default() -> Self {
        Self {
            last_values: HashMap::new(),
            weapon1_keys: HashSet::new(),
            weapon2_keys: HashSet::new(),
            weapon1_solo_ticks: 0,
            weapon2_solo_ticks: 0,
            previous_ammo_sum: None,
            fallback_last_values: HashMap::new(),
            fallback_bucket_a: HashSet::new(),
            fallback_bucket_b: HashSet::new(),
            fallback_pending: HashSet::new(),
            fallback_pending_ticks: 0,
            fallback_bucket_a_starting_total: None,
            fallback_bucket_b_starting_total: None,
            // Не через #[derive(Default)] нарочно: derive дал бы `false`,
            // а нужно "кластер A по умолчанию — weapon1".
            bucket_a_is_weapon1: true,
            weapon_capacity_hint_applied: false,
            weapon_capacity_hint: (None, None),
        }
    }
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

    /// Однократная (за сессию) подсказка ожидаемой ёмкости боекомплекта по
    /// weapon1/weapon2 из статической таблицы (`weapon_profiles`). Нужна
    /// только чтобы выбрать, какой из двух уже самостоятельно выученных
    /// `infer_firing_from_ammo_sum`-кластеров назвать weapon1, а какой —
    /// weapon2 — никогда не подменяет и не опережает живой сигнал. Вызов
    /// после того, как подсказка уже применена в этой сессии — no-op.
    pub fn set_weapon_capacity_hint(&mut self, weapon1_capacity: Option<f64>, weapon2_capacity: Option<f64>) {
        if self.weapon_capacity_hint_applied {
            return;
        }
        self.weapon_capacity_hint = (weapon1_capacity, weapon2_capacity);
    }

    /// Фолбэк для бортов без единого ключа `weapon1..weapon4` в
    /// `/indicators`: суммирует все числовые поля, похожие по имени на
    /// боеприпасы (см. `is_ammo_like_key`), и по убыванию суммы между
    /// тиками определяет, какой из двух самостоятельно выученных
    /// кластеров ключей (см. doc-комментарий модуля) стрелял — а не только
    /// сам факт стрельбы, как раньше. Рост суммы (довооружение/респавн)
    /// просто переустанавливает базу, не сигнализируя стрельбу и не
    /// трогая уже выученные кластеры. Вызывать только когда вызывающий
    /// код уже убедился, что ни одного триггер-ключа нет.
    pub fn infer_firing_from_ammo_sum(&mut self, indicators: &Value) -> FallbackFiring {
        let Some(obj) = indicators.as_object() else {
            return FallbackFiring::default();
        };

        let mut current: HashMap<String, f64> = HashMap::new();
        for (key, value) in obj {
            if !is_ammo_like_key(key) {
                continue;
            }
            if let Some(n) = value.as_f64() {
                current.insert(key.clone(), n);
            }
        }
        if current.is_empty() {
            return FallbackFiring::default();
        }

        let sum: f64 = current.values().sum();
        let prev_sum = self.previous_ammo_sum;
        self.previous_ammo_sum = Some(sum);
        let sum_decreased = prev_sum.is_some_and(|prev| sum < prev - SUM_DECREASE_EPS);

        if !sum_decreased {
            // Первый тик, рост суммы (довооружение/респавн) или отсутствие
            // изменений — просто перебазируемся, кластеры не трогаем.
            self.fallback_last_values = current;
            return FallbackFiring::default();
        }

        let decreased: HashSet<String> = current
            .iter()
            .filter_map(|(k, v)| {
                let prev = self.fallback_last_values.get(k.as_str())?;
                (*v < prev - SUM_DECREASE_EPS).then(|| k.clone())
            })
            .collect();

        if decreased.is_empty() {
            // Сумма упала за счёт множества мелких дробных изменений ниже
            // порога на каждый отдельный ключ — редкий шум, не считаем это
            // стрельбой ни одного оружия.
            self.fallback_last_values = current;
            return FallbackFiring::default();
        }

        let a_hit = !decreased.is_disjoint(&self.fallback_bucket_a);
        let unassigned: HashSet<String> = decreased
            .iter()
            .filter(|k| !self.fallback_bucket_a.contains(*k) && !self.fallback_bucket_b.contains(*k))
            .cloned()
            .collect();

        if self.fallback_bucket_a.is_empty() {
            // Самое первое убывание за всю сессию — не с чем сравнивать,
            // сразу и без задержки принимаем весь убывший набор за
            // "базовый" кластер (сохраняет мгновенный отклик борта с одним
            // типом боеприпасов — как и было в старой версии, до этого
            // фикса).
            self.fallback_bucket_a = decreased.clone();
        } else if !unassigned.is_empty() {
            if a_hit {
                // Новые ключи убыли одновременно с уже известными ключами
                // bucket_a — то же оружие (например, ещё один счётчик того
                // же ствола), присоединяем сразу, без ожидания.
                self.fallback_bucket_a.extend(unassigned);
                self.fallback_pending.clear();
                self.fallback_pending_ticks = 0;
            } else {
                // Убыли без участия bucket_a — кандидат на второе оружие,
                // подтверждаем только после нескольких соло-тиков подряд
                // (та же защита от шума на границе тиков, что и у
                // основного flag-based учителя выше).
                if unassigned == self.fallback_pending {
                    self.fallback_pending_ticks += 1;
                } else {
                    self.fallback_pending = unassigned.clone();
                    self.fallback_pending_ticks = 1;
                }
                if self.fallback_pending_ticks >= MIN_SOLO_TICKS {
                    self.fallback_bucket_b = std::mem::take(&mut self.fallback_pending);
                    self.fallback_pending_ticks = 0;
                }
            }
        } else {
            self.fallback_pending.clear();
            self.fallback_pending_ticks = 0;
        }

        // Стартовую сумму кластера ловим по значениям ДО этого тика
        // (`fallback_last_values`) — ближе к неизрасходованной ёмкости,
        // чем уже уменьшившиеся значения текущего тика.
        if self.fallback_bucket_a_starting_total.is_none() && !self.fallback_bucket_a.is_empty() {
            let total: f64 = self
                .fallback_bucket_a
                .iter()
                .filter_map(|k| self.fallback_last_values.get(k))
                .sum();
            self.fallback_bucket_a_starting_total = Some(total);
        }
        if self.fallback_bucket_b_starting_total.is_none() && !self.fallback_bucket_b.is_empty() {
            let total: f64 = self
                .fallback_bucket_b
                .iter()
                .filter_map(|k| self.fallback_last_values.get(k))
                .sum();
            self.fallback_bucket_b_starting_total = Some(total);
        }

        if !self.weapon_capacity_hint_applied
            && let (Some(a_total), Some(b_total), (Some(w1), Some(w2))) = (
                self.fallback_bucket_a_starting_total,
                self.fallback_bucket_b_starting_total,
                self.weapon_capacity_hint,
            )
        {
            let dist_as_is = (a_total - w1).abs() + (b_total - w2).abs();
            let dist_swapped = (a_total - w2).abs() + (b_total - w1).abs();
            self.bucket_a_is_weapon1 = dist_as_is <= dist_swapped;
            self.weapon_capacity_hint_applied = true;
        }

        self.fallback_last_values = current;

        // "Кластер B" — единственное, что требует подтверждения; всё
        // остальное убывшее (включая ещё не подтверждённых кандидатов) по
        // умолчанию считается тем же "первым" оружием — так борт с одним
        // типом боеприпасов продолжает мгновенно репортить стрельбу, как и
        // раньше, а не ждёт появления второго типа.
        let fired_default = decreased.iter().any(|k| !self.fallback_bucket_b.contains(k));
        let fired_b = decreased.iter().any(|k| self.fallback_bucket_b.contains(k));

        if self.bucket_a_is_weapon1 {
            FallbackFiring { weapon1: fired_default, weapon2: fired_b }
        } else {
            FallbackFiring { weapon1: fired_b, weapon2: fired_default }
        }
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

    #[test]
    fn infer_firing_first_call_only_establishes_baseline() {
        let mut t = AmmoTracker::new();
        assert_eq!(
            t.infer_firing_from_ammo_sum(&json!({"cannon1_ammo": 100})),
            FallbackFiring::default()
        );
    }

    #[test]
    fn infer_firing_from_ammo_sum_decrease() {
        let mut t = AmmoTracker::new();
        t.infer_firing_from_ammo_sum(&json!({"cannon1_ammo": 100}));
        // Единственный когда-либо виденный ammo-подобный ключ — сразу и без
        // задержки становится "базовым" кластером (weapon1 по умолчанию).
        assert_eq!(
            t.infer_firing_from_ammo_sum(&json!({"cannon1_ammo": 98})),
            FallbackFiring { weapon1: true, weapon2: false }
        );
    }

    #[test]
    fn infer_firing_rearm_rebases_without_firing() {
        let mut t = AmmoTracker::new();
        t.infer_firing_from_ammo_sum(&json!({"cannon1_ammo": 10}));
        // респавн/довооружение — сумма выросла, стрельбы быть не должно
        assert_eq!(
            t.infer_firing_from_ammo_sum(&json!({"cannon1_ammo": 200})),
            FallbackFiring::default()
        );
        // и следующий тик сравнивается уже с новой базой (200), а не старой (10):
        // без изменения от новой базы стрельбы тоже быть не должно
        assert_eq!(
            t.infer_firing_from_ammo_sum(&json!({"cannon1_ammo": 200})),
            FallbackFiring::default()
        );
    }

    #[test]
    fn infer_firing_no_matching_keys_never_fires() {
        let mut t = AmmoTracker::new();
        t.infer_firing_from_ammo_sum(&json!({"speed": 500}));
        assert_eq!(
            t.infer_firing_from_ammo_sum(&json!({"speed": 100})),
            FallbackFiring::default()
        );
    }

    #[test]
    fn infer_firing_lamp_duplicates_excluded() {
        let mut t = AmmoTracker::new();
        // Только `_lamp`-ключ без основного счётчика — не считается вовсе
        // (found_any остаётся false), а не просто дублирует значение.
        t.infer_firing_from_ammo_sum(&json!({"ammo_counter1_lamp": 60}));
        assert_eq!(
            t.infer_firing_from_ammo_sum(&json!({"ammo_counter1_lamp": 0})),
            FallbackFiring::default()
        );
    }

    #[test]
    fn infer_firing_routes_second_independent_ammo_key_to_weapon2() {
        // Прямой регрессионный тест на баг: второй независимый ammo-ключ
        // после обучения должен репортиться как weapon2 — раньше это было
        // структурно невозможно (fallback всегда писал в weapon1).
        let mut t = AmmoTracker::new();
        let mut n = 100.0;
        t.infer_firing_from_ammo_sum(&json!({"ammo_a": n}));
        for _ in 0..MIN_SOLO_TICKS {
            n -= 1.0;
            let f = t.infer_firing_from_ammo_sum(&json!({"ammo_a": n}));
            assert_eq!(f, FallbackFiring { weapon1: true, weapon2: false });
        }

        // Заводим базовую точку отсчёта для ammo_b до того, как он начнёт
        // убывать (реалистично — оба счётчика телеметрии присутствуют
        // каждый тик, меняется только один).
        let mut m = 50.0;
        t.infer_firing_from_ammo_sum(&json!({"ammo_a": n, "ammo_b": m}));

        let mut last = FallbackFiring::default();
        for _ in 0..MIN_SOLO_TICKS {
            m -= 1.0;
            last = t.infer_firing_from_ammo_sum(&json!({"ammo_a": n, "ammo_b": m}));
        }
        assert_eq!(last, FallbackFiring { weapon1: false, weapon2: true });
    }

    #[test]
    fn infer_firing_keeps_co_decreasing_multi_counter_group_together() {
        let mut t = AmmoTracker::new();
        let (mut n1, mut n2) = (100.0, 200.0);
        t.infer_firing_from_ammo_sum(&json!({"ammo_a1": n1, "ammo_a2": n2}));
        for _ in 0..MIN_SOLO_TICKS {
            n1 -= 1.0;
            n2 -= 1.0;
            let f = t.infer_firing_from_ammo_sum(&json!({"ammo_a1": n1, "ammo_a2": n2}));
            assert_eq!(f, FallbackFiring { weapon1: true, weapon2: false });
        }
    }

    #[test]
    fn infer_firing_reports_simultaneous_fire_on_both_weapons() {
        let mut t = AmmoTracker::new();
        let mut n = 100.0;
        t.infer_firing_from_ammo_sum(&json!({"ammo_a": n}));
        for _ in 0..MIN_SOLO_TICKS {
            n -= 1.0;
            t.infer_firing_from_ammo_sum(&json!({"ammo_a": n}));
        }
        let mut m = 50.0;
        t.infer_firing_from_ammo_sum(&json!({"ammo_a": n, "ammo_b": m}));
        for _ in 0..MIN_SOLO_TICKS {
            m -= 1.0;
            t.infer_firing_from_ammo_sum(&json!({"ammo_a": n, "ammo_b": m}));
        }

        n -= 1.0;
        m -= 1.0;
        let f = t.infer_firing_from_ammo_sum(&json!({"ammo_a": n, "ammo_b": m}));
        assert_eq!(f, FallbackFiring { weapon1: true, weapon2: true });
    }

    #[test]
    fn infer_firing_capacity_hint_relabels_when_closer_match() {
        let mut t = AmmoTracker::new();
        // bucket A (первый увиденный) на деле — низкоёмкое оружие (~200),
        // bucket B — высокоёмкое (~2000).
        let mut a = 200.0;
        t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a}));
        for _ in 0..MIN_SOLO_TICKS {
            a -= 1.0;
            t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a}));
        }
        // подсказка: weapon1 = 2000-патронный пулемёт, weapon2 = 200-патронная пушка
        t.set_weapon_capacity_hint(Some(2000.0), Some(200.0));

        let mut b = 2000.0;
        t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a, "mg_ammo": b}));
        let mut last = FallbackFiring::default();
        for _ in 0..MIN_SOLO_TICKS {
            b -= 1.0;
            last = t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a, "mg_ammo": b}));
        }
        // bucket A (cannon_ammo, ~200) теперь должен маркироваться как
        // weapon2 — его стартовая сумма ближе к подсказанной ёмкости
        // weapon2 (200), чем weapon1 (2000).
        assert_eq!(last, FallbackFiring { weapon1: true, weapon2: false });

        // и стрельба по cannon_ammo (bucket A) в одиночку теперь должна
        // репортиться как weapon2, а не weapon1
        a -= 1.0;
        let f = t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a, "mg_ammo": b}));
        assert_eq!(f, FallbackFiring { weapon1: false, weapon2: true });
    }

    #[test]
    fn infer_firing_no_capacity_hint_keeps_first_observed_as_weapon1() {
        let mut t = AmmoTracker::new();
        let mut a = 200.0;
        t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a}));
        for _ in 0..MIN_SOLO_TICKS {
            a -= 1.0;
            t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a}));
        }
        let mut b = 2000.0;
        t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a, "mg_ammo": b}));
        let mut last = FallbackFiring::default();
        for _ in 0..MIN_SOLO_TICKS {
            b -= 1.0;
            last = t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a, "mg_ammo": b}));
        }
        // без подсказки порядок по умолчанию сохраняется: bucket A (cannon,
        // первый увиденный) остаётся weapon1, bucket B (mg) — weapon2.
        assert_eq!(last, FallbackFiring { weapon1: false, weapon2: true });
    }

    #[test]
    fn infer_firing_capacity_hint_with_missing_slot_does_not_relabel() {
        let mut t = AmmoTracker::new();
        let mut a = 200.0;
        t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a}));
        for _ in 0..MIN_SOLO_TICKS {
            a -= 1.0;
            t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a}));
        }
        t.set_weapon_capacity_hint(None, Some(200.0));
        let mut b = 2000.0;
        t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a, "mg_ammo": b}));
        let mut last = FallbackFiring::default();
        for _ in 0..MIN_SOLO_TICKS {
            b -= 1.0;
            last = t.infer_firing_from_ammo_sum(&json!({"cannon_ammo": a, "mg_ammo": b}));
        }
        assert_eq!(last, FallbackFiring { weapon1: false, weapon2: true });
    }
}
