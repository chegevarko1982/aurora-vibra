//! Состояние, разделяемое между writer-потоком (пишет его) и главным
//! потоком, который на нём рисует дашборд (читает его). Один `Mutex` —
//! данные маленькие (JSON-снапшоты одного тика, счётчики, VecDeque на 5
//! строк), удерживать блокировку долго негде.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

use serde_json::Value;

use crate::wt_probe::model::{ConnStatus, Endpoint};

const MAX_RECENT_EVENTS: usize = 5;
const MAX_OWN_HITS: usize = 5;

pub struct Shared {
    pub conn: ConnStatus,
    pub vehicle_type: Option<String>,
    /// Последний известный плоский снапшот значений по эндпоинту — на нём
    /// дашборд ищет "интересные величины" (interesting::match_categories).
    pub last_flat: BTreeMap<Endpoint, BTreeMap<String, Value>>,
    /// Суммарное число успешных ответов по каждому эндпоинту с начала сессии
    /// — дашборд сам считает дельту между кадрами, чтобы получить Гц.
    pub success_total: BTreeMap<Endpoint, u64>,
    pub recent_events: VecDeque<String>,
    /// Собственный позывной (задаётся один раз при старте сессии, дальше не
    /// меняется) — используется для сопоставления с /hudmsg.damage[].msg в
    /// hits::classify_own_hit, т.к. API не даёт другого способа понять, что
    /// урон нанесён именно тебе (см. src/wt_probe/hits.rs).
    pub own_callsign: String,
    /// Найденные попадания ПО собственному самолёту (не все damage-события
    /// матча — только те, где жертва == own_callsign).
    pub own_hits: VecDeque<String>,
    pub lines_written: u64,
    pub bytes_written: u64,
    pub last_error: Option<String>,
    pub started_at: Instant,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            conn: ConnStatus::Disconnected,
            vehicle_type: None,
            last_flat: BTreeMap::new(),
            success_total: BTreeMap::new(),
            recent_events: VecDeque::with_capacity(MAX_RECENT_EVENTS),
            own_callsign: String::new(),
            own_hits: VecDeque::with_capacity(MAX_OWN_HITS),
            lines_written: 0,
            bytes_written: 0,
            last_error: None,
            started_at: Instant::now(),
        }
    }

    pub fn push_event(&mut self, line: String) {
        if self.recent_events.len() >= MAX_RECENT_EVENTS {
            self.recent_events.pop_front();
        }
        self.recent_events.push_back(line);
    }

    pub fn push_own_hit(&mut self, line: String) {
        if self.own_hits.len() >= MAX_OWN_HITS {
            self.own_hits.pop_front();
        }
        self.own_hits.push_back(line);
    }

    pub fn bump_success(&mut self, endpoint: Endpoint) {
        *self.success_total.entry(endpoint).or_insert(0) += 1;
    }
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedHandle = std::sync::Arc<Mutex<Shared>>;
