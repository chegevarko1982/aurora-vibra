//! Арбитраж владения HID-каналом/GUI между MSFS- и WT-конвейерами. Оба
//! воркера (`sim::sim_worker`, `wt_link::wt_worker`) сами решают, живы ли
//! они, и заявляют/освобождают слот через `GameSlot` — липкое владение,
//! первый заявивший игру владеет ей, пока сам не отпустит (см. "Правило
//! арбитража" в плане фичи). Чистая логика без платформенных зависимостей —
//! юнит-тестируема на любой платформе, отсюда безусловный `pub mod
//! game_state;` в lib.rs (без cfg(windows)).

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::types::ActiveGame;

/// Общий слот владения активной игрой. Клонируется в оба воркера (каждый
/// получает свою копию — общий `Arc<Mutex<ActiveGame>>` внутри).
#[derive(Clone)]
pub struct GameSlot {
    active: Arc<Mutex<ActiveGame>>,
}

impl GameSlot {
    pub fn new(active: Arc<Mutex<ActiveGame>>) -> Self {
        Self { active }
    }

    /// Липкое владение: если слот занят ДРУГОЙ игрой — false. Своей игрой —
    /// идемпотентно true. Свободный слот — заявляет и возвращает true.
    pub fn try_claim(&self, who: ActiveGame) -> bool {
        let mut g = self.active.lock();
        if *g == who {
            return true;
        }
        if *g == ActiveGame::None {
            *g = who;
            return true;
        }
        false
    }

    /// Освобождает слот, только если он всё ещё принадлежит `who` (иначе
    /// no-op — чужой слот трогать нельзя).
    pub fn release_if_owned(&self, who: ActiveGame) {
        let mut g = self.active.lock();
        if *g == who {
            *g = ActiveGame::None;
        }
    }
}

/// Дебаунс "жив ли воркер": переживает кратковременные провалы опроса
/// (например, долгий чёрный экран загрузки миссии в WT) в течение
/// `grace_after_first`, не объявляя игру пропавшей мгновенно.
pub struct Liveness {
    last_ok: Option<Instant>,
    grace_after_first: Duration,
}

impl Liveness {
    pub fn new(grace_after_first: Duration) -> Self {
        Self {
            last_ok: None,
            grace_after_first,
        }
    }

    /// `now` передаётся параметром (не `Instant::now()` внутри) — так юнит-тесты
    /// могут прогонять синтетическую шкалу времени без реальных sleep.
    pub fn observe(&mut self, ok: bool, now: Instant) -> bool {
        if ok {
            self.last_ok = Some(now);
            return true;
        }
        match self.last_ok {
            None => false, // ни одного успеха ещё не было
            Some(t) => {
                let alive = now.duration_since(t) < self.grace_after_first;
                if !alive {
                    self.last_ok = None;
                }
                alive
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_slot_first_claim_owns() {
        let active = Arc::new(Mutex::new(ActiveGame::None));
        let slot = GameSlot::new(active.clone());
        assert!(slot.try_claim(ActiveGame::Msfs));
        assert_eq!(*active.lock(), ActiveGame::Msfs);
    }

    #[test]
    fn second_claim_by_other_game_fails_while_first_alive() {
        let active = Arc::new(Mutex::new(ActiveGame::None));
        let msfs = GameSlot::new(active.clone());
        let wt = GameSlot::new(active.clone());
        assert!(msfs.try_claim(ActiveGame::Msfs));
        assert!(!wt.try_claim(ActiveGame::Wt));
        assert_eq!(*active.lock(), ActiveGame::Msfs);
    }

    #[test]
    fn release_by_non_owner_is_noop() {
        let active = Arc::new(Mutex::new(ActiveGame::None));
        let msfs = GameSlot::new(active.clone());
        let wt = GameSlot::new(active.clone());
        assert!(msfs.try_claim(ActiveGame::Msfs));
        wt.release_if_owned(ActiveGame::Wt);
        assert_eq!(*active.lock(), ActiveGame::Msfs);
    }

    #[test]
    fn release_by_owner_frees_slot() {
        let active = Arc::new(Mutex::new(ActiveGame::None));
        let msfs = GameSlot::new(active.clone());
        assert!(msfs.try_claim(ActiveGame::Msfs));
        msfs.release_if_owned(ActiveGame::Msfs);
        assert_eq!(*active.lock(), ActiveGame::None);
    }

    #[test]
    fn liveness_false_before_first_success() {
        let mut l = Liveness::new(Duration::from_millis(2500));
        let t0 = Instant::now();
        assert!(!l.observe(false, t0));
    }

    #[test]
    fn liveness_true_on_success() {
        let mut l = Liveness::new(Duration::from_millis(2500));
        let t0 = Instant::now();
        assert!(l.observe(true, t0));
    }

    #[test]
    fn liveness_survives_failure_within_grace() {
        let mut l = Liveness::new(Duration::from_millis(2500));
        let t0 = Instant::now();
        assert!(l.observe(true, t0));
        assert!(l.observe(false, t0 + Duration::from_millis(1000)));
    }

    #[test]
    fn liveness_dies_after_grace_expires() {
        let mut l = Liveness::new(Duration::from_millis(2500));
        let t0 = Instant::now();
        assert!(l.observe(true, t0));
        assert!(!l.observe(false, t0 + Duration::from_millis(3000)));
    }

    #[test]
    fn liveness_recovers_after_failure() {
        let mut l = Liveness::new(Duration::from_millis(2500));
        let t0 = Instant::now();
        assert!(l.observe(true, t0));
        assert!(!l.observe(false, t0 + Duration::from_millis(3000)));
        assert!(l.observe(true, t0 + Duration::from_millis(3100)));
    }
}
