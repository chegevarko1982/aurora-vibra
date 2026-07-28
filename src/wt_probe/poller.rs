//! Потоки опроса: один поток на эндпоинт, каждый со своим ритмом, не
//! блокирующие друг друга. Таймаут одного запроса задаётся в самом
//! `WtClient` (короткий, 100-200 мс) — потеря одного тика не останавливает
//! цикл, ошибка просто уходит в канал вместе с успешными результатами.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use serde_json::Value;

use crate::wt_probe::http::{FetchError, WtClient, max_id_in_array};
use crate::wt_probe::model::{Endpoint, PollOutcome, RawRecord};

/// Общий каркас тайминга: держит целевую частоту, но не копит очередь
/// пропущенных тиков, если игра тормозит с ответом дольше периода. Сон
/// режем на кусочки по 100 мс, чтобы Ctrl+C отрабатывал быстро даже на
/// редких эндпоинтах (1 Гц).
fn run_loop<F: FnMut()>(hz: f64, shutdown: &AtomicBool, mut tick: F) {
    let interval = Duration::from_secs_f64(1.0 / hz.max(0.001));
    let mut next = Instant::now();
    while !shutdown.load(Ordering::Relaxed) {
        tick();
        next += interval;
        let now = Instant::now();
        let mut remaining = next.saturating_duration_since(now);
        if remaining.is_zero() {
            next = now;
        }
        while remaining > Duration::ZERO {
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            let chunk = remaining.min(Duration::from_millis(100));
            thread::sleep(chunk);
            remaining = remaining.saturating_sub(chunk);
        }
    }
}

fn send(tx: &Sender<PollOutcome>, endpoint: Endpoint, t: f64, result: Result<Value, FetchError>) {
    let outcome = match result {
        Ok(parsed) => PollOutcome::Ok(RawRecord {
            t,
            endpoint,
            parsed,
        }),
        Err(e) => PollOutcome::Err {
            endpoint,
            message: e.to_string(),
        },
    };
    // Получатель (writer) мог уже закрыть канал при завершении — это не
    // повод паниковать, просто теряем последний в полёте результат.
    let _ = tx.send(outcome);
}

/// Опрос простых (без курсора) эндпоинтов: /state, /indicators,
/// /map_obj.json, /mission.json, /map_info.json.
pub fn spawn_fixed(
    client: WtClient,
    endpoint: Endpoint,
    hz: f64,
    session_start: Instant,
    tx: Sender<PollOutcome>,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        run_loop(hz, &shutdown, || {
            let t = session_start.elapsed().as_secs_f64();
            let result = match endpoint {
                Endpoint::State => client.state(),
                Endpoint::Indicators => client.indicators(),
                Endpoint::MapObj => client.map_obj(),
                Endpoint::Mission => client.mission(),
                Endpoint::MapInfo => client.map_info(),
                Endpoint::HudMsg => {
                    debug_assert!(false, "hudmsg must use spawn_hudmsg (курсорная модель)");
                    return;
                }
            };
            send(&tx, endpoint, t, result);
        });
    })
}

/// Опрос /hudmsg — курсорная модель по lastEvt/lastDmg. Начинаем с -1/-1
/// (получить всё, что накопилось с начала сессии), дальше двигаем курсор по
/// максимальному "id", встреченному в массивах "events"/"damage" ответа.
pub fn spawn_hudmsg(
    client: WtClient,
    hz: f64,
    session_start: Instant,
    tx: Sender<PollOutcome>,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut last_evt: i64 = -1;
        let mut last_dmg: i64 = -1;
        run_loop(hz, &shutdown, || {
            let t = session_start.elapsed().as_secs_f64();
            match client.hudmsg(last_evt, last_dmg) {
                Ok(parsed) => {
                    if let Some(max_evt) = max_id_in_array(&parsed, "events") {
                        last_evt = max_evt;
                    }
                    if let Some(max_dmg) = max_id_in_array(&parsed, "damage") {
                        last_dmg = max_dmg;
                    }
                    let _ = tx.send(PollOutcome::Ok(RawRecord {
                        t,
                        endpoint: Endpoint::HudMsg,
                        parsed,
                    }));
                }
                Err(e) => {
                    let _ = tx.send(PollOutcome::Err {
                        endpoint: Endpoint::HudMsg,
                        message: e.to_string(),
                    });
                }
            }
        });
    })
}
