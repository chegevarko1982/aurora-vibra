//! Поток ручного предпросмотра пользовательского эффекта на устройстве
//! (кнопка «Запустить на устройстве» в конструкторе, `ui::effects_editor`).
//!
//! Раньше отправка `HidCmd::SendIntensity` во время предпросмотра шла прямо
//! из кадра egui: гейт `if now - preview_last_sent >= PREVIEW_SEND_INTERVAL`
//! срабатывал на ближайшем кадре ПОСЛЕ порога (кадры egui приходят по vsync,
//! ~16.7 мс на 60 Гц — не совпадает ни с чем), а `t` бралось из настенных
//! часов (`Instant::elapsed()`), а не с точной сетки. В сумме — отсчёты формы
//! ложились на нецелое число периода, и фронты импульсов гуляли на ±кадр.
//! Диагностический стенд `src/bin/test_pattern_uniformity.rs` доказал на
//! живом железе, что ТА ЖЕ форма, отправленная по абсолютным дедлайнам на
//! точной сетке 20 мс, даёт идеально ровный паттерн — этот модуль переносит
//! ровно тот приём в продовый путь предпросмотра.
//!
//! `PreviewPlayer` — тонкая ручка (клонируемый `Arc` внутри, но сам тип не
//! `Clone`: единственный владелец живёт в `UiState`/`App`, редактор получает
//! `&PreviewPlayer`) вокруг фонового потока `"fx-preview"`. GUI-поток кладёт
//! в неё "что играть" через `play`/`set_effect`/`set_value`, поток-плеер
//! читает это на каждом тике 20 мс, крутит `custom_fx::engine::PreviewRunner`
//! и шлёт `HidCmd::SendIntensity`. Обратно в GUI уходит только последний
//! уровень (`levels()`, упакован в один `AtomicU32`) — для индикатора рядом с
//! кнопкой, больше GUI-потоку ничего из плеера не нужно.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::custom_fx::engine::{PreviewRunner, preview_playback_time_s};
use crate::custom_fx::model::CustomEffect;
use crate::types::HidCmd;

/// Что сейчас проигрывает поток — эффект + пробное сырое значение источника
/// (слайдер шага «Предпросмотр»). `Clone`, потому что GUI-поток и
/// поток-плеер видят его через разные снимки одного и того же `Mutex`:
/// плеер читает свежий снимок на КАЖДОМ тике 20 мс, а не держит блокировку
/// на всё время тика (иначе `set_value`/`set_effect` из GUI-потока ждали бы
/// освобождения мьютекса до 20 мс).
#[derive(Clone)]
struct Job {
    effect: CustomEffect,
    test_value: f64,
}

struct Inner {
    job: Mutex<Option<Job>>,
    /// Поднимается на каждом `play()` — поток видит новое значение и
    /// перезапускает и сетку тиков, и состояние `PreviewRunner` (фаза формы,
    /// EMA) с нуля. `set_effect`/`set_value` НЕ трогают это поле — правки
    /// параметров на лету не должны рвать фазу уже идущего паттерна.
    generation: AtomicU64,
    /// Последние отправленные три канала, упакованные как
    /// `(joystick<<16)|(throttle_left<<8)|throttle_right` — одно атомарное
    /// слово вместо мьютекса, потому что GUI читает его каждый кадр только
    /// для индикатора, а поток-плеер пишет раз в 20 мс.
    levels: AtomicU32,
    shutdown: AtomicBool,
}

/// Точка входа плеера — см. doc-комментарий модуля.
pub struct PreviewPlayer {
    inner: Arc<Inner>,
}

fn pack_levels(joystick: u8, throttle_left: u8, throttle_right: u8) -> u32 {
    ((joystick as u32) << 16) | ((throttle_left as u32) << 8) | (throttle_right as u32)
}

fn unpack_levels(bits: u32) -> (u8, u8, u8) {
    (
        ((bits >> 16) & 0xFF) as u8,
        ((bits >> 8) & 0xFF) as u8,
        (bits & 0xFF) as u8,
    )
}

/// Время тика `tick_index` в секундах, ТОЧНОЕ кратное такту отправки
/// (`hid::protocol::SEND_INTERVAL_S`) — вынесено из тела цикла отдельной
/// функцией специально ради юнит-тестов на арифметику сетки (сам тайминг
/// потока юнит-тестом не проверить, а вот то, что `t` не дрейфует и паттерн
/// замыкается по фазе — вполне).
pub(crate) fn tick_time_s(tick_index: u64) -> f64 {
    tick_index as f64 * crate::hid::protocol::SEND_INTERVAL_S
}

impl PreviewPlayer {
    /// Поднимает фоновый поток `"fx-preview"` и возвращает ручку к нему.
    /// Живёт всё время работы приложения (создаётся один раз в `main.rs`,
    /// рядом с `PreviewLock`), останавливается через `Drop`.
    pub fn spawn(tx_hid: Sender<HidCmd>) -> Self {
        let inner = Arc::new(Inner {
            job: Mutex::new(None),
            generation: AtomicU64::new(0),
            levels: AtomicU32::new(0),
            shutdown: AtomicBool::new(false),
        });
        let worker_inner = inner.clone();
        thread::Builder::new()
            .name("fx-preview".to_string())
            .spawn(move || run(worker_inner, tx_hid))
            // Провал создания ОС-потока здесь не более вероятен, чем у любого
            // другого `thread::spawn` в проекте (см. `main.rs`) — они тоже не
            // обрабатывают эту ошибку отдельно, ресурсов на неё в рантайме
            // всё равно нет.
            .expect("не удалось создать поток fx-preview");
        Self { inner }
    }

    /// Запускает проигрывание `effect` с сырым значением `test_value` —
    /// поднимает `generation`, поэтому поток перезапустит сетку тиков и
    /// состояние формы с t=0 (фронт "молчал -> активен" для событийных
    /// эффектов, свежая фаза для периодических).
    pub fn play(&self, effect: CustomEffect, test_value: f64) {
        *self.inner.job.lock() = Some(Job { effect, test_value });
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Подменяет проигрываемый эффект БЕЗ подъёма `generation` — правки
    /// параметров в UI на лету (двигаем слайдер формы, пока идёт
    /// предпросмотр) не должны рвать фазу уже идущего паттерна. No-op, если
    /// сейчас ничего не играет.
    pub fn set_effect(&self, effect: CustomEffect) {
        if let Some(job) = self.inner.job.lock().as_mut() {
            job.effect = effect;
        }
    }

    /// Обновляет пробное сырое значение источника (слайдер «Тестовое
    /// значение»). No-op, если сейчас ничего не играет.
    pub fn set_value(&self, test_value: f64) {
        if let Some(job) = self.inner.job.lock().as_mut() {
            job.test_value = test_value;
        }
    }

    /// Снимает job — поток при следующей итерации один раз пошлёт нули на
    /// все три канала и перейдёт в лёгкий простой (без busy-wait).
    pub fn stop(&self) {
        *self.inner.job.lock() = None;
    }

    /// Последние отправленные три канала — для индикатора рядом с кнопкой.
    pub fn levels(&self) -> (u8, u8, u8) {
        unpack_levels(self.inner.levels.load(Ordering::Acquire))
    }

    /// true, если сейчас что-то проигрывается.
    pub fn is_playing(&self) -> bool {
        self.inner.job.lock().is_some()
    }
}

impl Drop for PreviewPlayer {
    fn drop(&mut self) {
        // join() не обязателен: поток видит shutdown на следующей итерации
        // (не позже 10 мс простоя или 20 мс тика) и завершается сам, а
        // приложение и так закрывается вместе с процессом.
        self.inner.shutdown.store(true, Ordering::Release);
    }
}

/// Тело потока `"fx-preview"`. Простой в отсутствие job — лёгкий цикл со
/// `thread::sleep(10мс)`, БЕЗ spin_loop (иначе поток жёг бы ядро процессора
/// круглосуточно, даже когда предпросмотр никогда не запускали). Во время
/// игры — точная сетка `sleep_until` на АБСОЛЮТНЫХ дедлайнах, никогда не
/// `next = now`: иначе ошибка планирования одного тика накапливалась бы на
/// следующий, тот самый дефект, который чинит вся эта задача.
fn run(inner: Arc<Inner>, tx_hid: Sender<HidCmd>) {
    let tick = Duration::from_secs_f64(crate::hid::protocol::SEND_INTERVAL_S);

    let mut generation = inner.generation.load(Ordering::SeqCst);
    let mut tick_index: u64 = 0;
    let mut runner = PreviewRunner::new();
    let mut was_playing = false;
    // Дедлайн следующего тика — осмыслен только пока `was_playing == true`
    // (переустанавливается на `Instant::now()` при каждом переходе из
    // простоя/перезапуске по `generation`), поэтому стартовое значение здесь
    // произвольно и никогда не используется как есть.
    let mut next = Instant::now();

    loop {
        if inner.shutdown.load(Ordering::Acquire) {
            return;
        }

        let snapshot = inner.job.lock().clone();
        let Some(job) = snapshot else {
            if was_playing {
                // Переход в простой — один раз гасим моторы, а не оставляем
                // их на последнем проигранном значении.
                let _ = tx_hid.send(HidCmd::SendIntensity {
                    joystick: 0,
                    throttle_left: 0,
                    throttle_right: 0,
                });
                inner.levels.store(0, Ordering::Release);
                was_playing = false;
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        };

        let current_gen = inner.generation.load(Ordering::SeqCst);
        if !was_playing || current_gen != generation {
            // Заново якорим сетку тиков либо на самом первом тике после
            // простоя, либо на перезапуске по generation (новый фронт
            // «Играть») — в обоих случаях первый кадр обязан уйти немедленно,
            // не дожидаясь такта, а `t` обязан начаться ровно с 0.
            generation = current_gen;
            tick_index = 0;
            runner = PreviewRunner::new();
            next = Instant::now();
        }
        was_playing = true;

        crate::timing::sleep_until(next);

        let t = tick_time_s(tick_index);
        let t = preview_playback_time_s(&job.effect, t);
        let (joystick, throttle_left, throttle_right) = runner.tick(&job.effect, job.test_value, t);
        let _ = tx_hid.send(HidCmd::SendIntensity {
            joystick,
            throttle_left,
            throttle_right,
        });
        inner.levels.store(
            pack_levels(joystick, throttle_left, throttle_right),
            Ordering::Release,
        );

        tick_index += 1;

        // Абсолютная сетка: следующий дедлайн — предыдущий + такт, НЕ
        // `Instant::now() + tick` (это и есть накопление ошибки квантования,
        // которое чинит вся задача). Если система всё же тормознула и такой
        // дедлайн уже в прошлом — подтягиваем к `now + tick`, чтобы не
        // разгонять очередь тиков вдогонку пачкой.
        next += tick;
        let now = Instant::now();
        if next < now {
            next = now + tick;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_time_fifty_ticks_is_exactly_one_second() {
        assert_eq!(
            tick_time_s(50),
            1.0,
            "50 тактов по 20мс обязаны дать РОВНО 1.0с"
        );
    }

    /// Регрессия на «паттерн замыкается»: на 5 Гц период — ровно 10 тактов
    /// (200мс / 20мс), поэтому фаза `t*freq` обязана пробегать РОВНО 10
    /// различных значений на 100 тактах (не 9 и не 11 — иначе цикл дрейфует
    /// и по мере проигрывания форма расползается), а `tick_time_s(i+10)`
    /// обязано давать фазу БЕЗ накопленной ошибки округления против
    /// `tick_time_s(i)` — ровно на 1.0 больше.
    ///
    /// Сравнение НЕ бит-в-бит: `t = i * 0.02` — уже единственное умножение
    /// на не представимую точно в двоичной дроби константу, и на разных `i`
    /// оно округляется по-своему (проверено эмпирически — без округления
    /// `.fract().to_bits()` даёт 38 "почти одинаковых" значений вместо 10).
    /// Это тот же класс шума, что и во всей остальной f64-арифметике формы в
    /// `engine.rs` (там тоже сравнивают с допуском, не бит-в-бит) — округляем
    /// до 1e-9, чтобы отличить РЕАЛЬНЫЙ дрейф фазы (десятые доли) от шума
    /// последнего бита мантиссы.
    #[test]
    fn five_hz_phase_has_exactly_ten_distinct_values_and_closes_without_drift() {
        let bucket = |v: f64| (v * 1e9).round() as i64;
        let phases: std::collections::BTreeSet<i64> = (0..100)
            .map(|i| bucket((tick_time_s(i) * 5.0).fract()))
            .collect();
        assert_eq!(
            phases.len(),
            10,
            "5 Гц на такте 20мс обязана дать ровно 10 различных фаз за 100 тактов"
        );

        for i in 0..90u64 {
            let a = tick_time_s(i) * 5.0;
            let b = tick_time_s(i + 10) * 5.0;
            assert!(
                (b - a - 1.0).abs() < 1e-9,
                "i={i}: 10 тактов на 5Гц обязаны сдвигать фазу РОВНО на 1.0 цикл без дрейфа, got {}",
                b - a
            );
        }
    }

    #[test]
    fn pack_unpack_levels_roundtrip() {
        for (j, tl, tr) in [(0u8, 0u8, 0u8), (255, 128, 1), (7, 250, 33)] {
            assert_eq!(unpack_levels(pack_levels(j, tl, tr)), (j, tl, tr));
        }
    }
}
