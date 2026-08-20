//! Движок пользовательских эффектов вибрации: превращает список
//! `CustomEffect` (модель — см. `super::model`) и текущий тик телеметрии
//! (`super::sources::TelemetryFrame`) в интенсивность трёх моторов.
//!
//! Взаимоисключающий движок относительно встроенного `RumbleEngine`
//! (см. `crate::rumble`) — вызывается только когда `settings::effect_mode()
//! == EffectMode::Custom`. Многие приёмы намеренно скопированы оттуда и из
//! `crate::wt_link::rumble` (см. комментарии по месту) — задача явно
//! запрещала трогать эти файлы, поэтому переиспользование только через
//! копирование логики, не через shared-код.

use std::collections::{HashMap, HashSet};

use crate::custom_fx::model::{CustomEffect, MAX_SHAPE_FREQ_HZ, MixMode, Shape, Trigger};
use crate::custom_fx::sources::{self, SourceId, TelemetryFrame};
use crate::types::ActiveGame;

/// Итог одного тика движка — три канала + список id эффектов, которые сейчас
/// реально куда-то отдают ненулевую интенсивность (для Live Monitor: без
/// этого списка UI не смог бы подсветить, какой именно эффект сейчас "жив").
#[derive(Debug, Clone, Default)]
pub struct CustomFxOutput {
    pub joystick: u8,
    pub throttle_left: u8,
    pub throttle_right: u8,
    pub active_ids: Vec<String>,
}

/// Минимальный xorshift32 для джиттера амплитуды `Shape::Pulse` — та же
/// реализация, что в `wt_link::rumble::Xorshift32` (см. `gun_pulse`/
/// `GunState`), скопирована копией по прямому указанию задачи: чужой модуль
/// не трогаем, а криптостойкость тут не нужна.
#[derive(Debug)]
struct Xorshift32(u32);

impl Xorshift32 {
    fn seeded(salt: u32) -> Self {
        let nanos = std::time::Instant::now().elapsed().subsec_nanos();
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

/// Приватное закрытое состояние ОДНОГО эффекта — гистерезис триггера, EMA,
/// огибающие `Pulse`/`OneShot`. Живёт в `CustomFxEngine::states` по `id`
/// эффекта, а не пересоздаётся каждый тик — иначе, например, `OneShot`
/// никогда бы не увидел фронт активации (была бы всегда "новая" стартовая
/// точка), а слайдер в UI дёргал бы фазу `Pulse` при каждой правке.
///
/// Поля используются "плоско" (не enum по типу триггера/формы): пользователь
/// может поменять тип триггера/формы эффекта с тем же `id` (правка в UI), и
/// плоская структура просто не трогает неиспользуемые сейчас поля вместо
/// паники/пересоздания состояния на несовпадении варианта.
struct EffectState {
    // Above/Below: защёлка гистерезиса.
    latched: bool,
    // Changed: последнее ПОДТВЕРЖДЁННОЕ значение и до какого t держится активность.
    changed_last_confirmed: f64,
    changed_has_value: bool,
    changed_active_until: f64,
    // EMA сглаженной интенсивности (curve.eval, ДО shape/strength_pct).
    ema: f64,
    // Фронт активности триггера "молчал -> активен" — общий якорь и для
    // attack_ms у Pulse, и для старта импульса OneShot.
    was_active: bool,
    active_since_t: f64,
    // OneShot: момент последнего фронта, на который взведён импульс. None —
    // ни разу не срабатывал (self-neutralizing: shape просто молчит).
    oneshot_fired_at: Option<f64>,
    // Pulse: джиттер амплитуды по циклам несущей — тот же приём, что
    // GunState::step в wt_link/rumble.rs (пересчитывается один раз на новый
    // цикл несущей, не на каждый тик, иначе джиттер "мерцал" бы внутри
    // одного и того же импульса).
    pulse_last_cycle_idx: i64,
    pulse_cycle_jitter_mul: f64,
    pulse_rng: Xorshift32,
}

impl EffectState {
    fn new() -> Self {
        Self {
            latched: false,
            changed_last_confirmed: 0.0,
            changed_has_value: false,
            // -inf, а не 0.0: пустое состояние не должно читаться как "активен
            // до момента времени 0" при t < 0 (in vitro/тестовые сценарии).
            changed_active_until: f64::NEG_INFINITY,
            ema: 0.0,
            was_active: false,
            active_since_t: 0.0,
            oneshot_fired_at: None,
            pulse_last_cycle_idx: -1,
            pulse_cycle_jitter_mul: 1.0,
            // Фиксированная соль вместо соли по id: время старта (subsec_nanos)
            // и так расходится между эффектами, создаваемыми в разные моменты;
            // константа тут — просто "поменять x на что-то ненулевое" в духе
            // исходного Xorshift32::seeded(salt) из wt_link/rumble.rs.
            pulse_rng: Xorshift32::seeded(0x9E37_79B9),
        }
    }
}

/// Подстрочный фильтр по борту — РОВНО та же семантика, что
/// `aircraft_profiles::find_matching_index` (регистронезависимый `contains`,
/// пустая/отсутствующая подстрока = совпадает с любым бортом).
fn aircraft_matches(pattern: &Option<String>, aircraft: &str) -> bool {
    match pattern {
        None => true,
        Some(p) if p.trim().is_empty() => true,
        Some(p) => aircraft.to_uppercase().contains(&p.to_uppercase()),
    }
}

/// Определяет "активен ли сейчас триггер", попутно продвигая его состояние
/// (гистерезис/окно удержания Changed). `t` — единое время движка (тот же
/// sim-time, что и во всём остальном коде, не Instant::now()), нужно для
/// Changed::hold_s.
fn trigger_active(trigger: &Trigger, x: f64, t: f64, state: &mut EffectState) -> bool {
    match trigger {
        Trigger::Always => true,
        Trigger::Above { value, hysteresis } => {
            if state.latched {
                if x < value - hysteresis {
                    state.latched = false;
                }
            } else if x > *value {
                state.latched = true;
            }
            state.latched
        }
        Trigger::Below { value, hysteresis } => {
            if state.latched {
                if x > value + hysteresis {
                    state.latched = false;
                }
            } else if x < *value {
                state.latched = true;
            }
            state.latched
        }
        Trigger::Between { lo, hi } => x >= *lo && x <= *hi,
        Trigger::IsTrue => x > 0.5,
        Trigger::Changed { eps, hold_s } => {
            // Тот же приём, что slats_pct_changed в rumble.rs: первый кадр
            // только фиксирует стартовое значение (не считается изменением),
            // иначе самый первый тик эффекта всегда бы взводился.
            if !state.changed_has_value {
                state.changed_last_confirmed = x;
                state.changed_has_value = true;
            } else if (x - state.changed_last_confirmed).abs() > *eps {
                state.changed_last_confirmed = x;
                state.changed_active_until = t + hold_s;
            }
            t < state.changed_active_until
        }
    }
}

/// Множитель формы 0..1 на момент времени `t`. Частота ВСЕГДА зажимается до
/// `MAX_SHAPE_FREQ_HZ` здесь (не только в `Shape::clamp_freq`, который UI
/// вызывает лишь после ручного ввода) — эффект мог прийти из руками
/// отредактированного JSON (import).
fn shape_multiplier(shape: &Shape, t: f64, state: &mut EffectState) -> f64 {
    match shape {
        Shape::Constant => 1.0,
        Shape::Pulse {
            freq_hz,
            duty_pct,
            jitter_pct,
            floor_pct,
            attack_ms,
        } => {
            // Нижняя граница 0.05 Гц — не в спеке, но не даёт периоду
            // разъехаться до бесконечности на freq_hz == 0.0, который
            // технически проходит serde (пользователь мог вбить 0 руками).
            let freq = (*freq_hz).clamp(0.05, MAX_SHAPE_FREQ_HZ) as f64;
            let duty = (*duty_pct as f64 / 100.0).clamp(0.0, 1.0);
            let floor = (*floor_pct as f64 / 100.0).clamp(0.0, 1.0);
            let attack_s = (*attack_ms as f64 / 1000.0).max(0.0);

            let period = 1.0 / freq;
            let phase = (t / period).rem_euclid(1.0);
            let cycle_idx = (t / period).floor() as i64;
            if cycle_idx != state.pulse_last_cycle_idx {
                state.pulse_last_cycle_idx = cycle_idx;
                let jitter = (*jitter_pct as f64 / 100.0).clamp(0.0, 1.0);
                state.pulse_cycle_jitter_mul = 1.0 - jitter * state.pulse_rng.next_unit();
            }

            let since_active = (t - state.active_since_t).max(0.0);
            let attack_ramp = if attack_s <= 0.0 {
                1.0
            } else {
                (since_active / attack_s).clamp(0.0, 1.0)
            };

            if phase < duty {
                (state.pulse_cycle_jitter_mul * attack_ramp).clamp(0.0, 1.0)
            } else {
                floor
            }
        }
        Shape::OneShot {
            attack_ms,
            hold_ms,
            decay_ms,
            decay_exp,
        } => match state.oneshot_fired_at {
            Some(t0) => oneshot_envelope(t - t0, *attack_ms, *hold_ms, *decay_ms, *decay_exp),
            None => 0.0,
        },
        Shape::Sine { freq_hz, depth_pct } => {
            let freq = (*freq_hz).clamp(0.0, MAX_SHAPE_FREQ_HZ) as f64;
            let depth = (*depth_pct as f64 / 100.0).clamp(0.0, 1.0);
            let wave = 0.5 * (1.0 + (2.0 * std::f64::consts::PI * freq * t).sin());
            (1.0 - depth + depth * wave).clamp(0.0, 1.0)
        }
        Shape::Sawtooth { freq_hz, depth_pct } => {
            let freq = (*freq_hz).clamp(0.0, MAX_SHAPE_FREQ_HZ) as f64;
            let depth = (*depth_pct as f64 / 100.0).clamp(0.0, 1.0);
            let wave = if freq <= 0.0 {
                0.0
            } else {
                (t * freq).rem_euclid(1.0)
            };
            (1.0 - depth + depth * wave).clamp(0.0, 1.0)
        }
    }
}

/// Attack -> hold -> степенной спад, ровно та же форма, что `bump_envelope` в
/// `rumble.rs` (Touchdown), но нормированная на 0..1 (там — сразу 0..255,
/// потому что пик всегда жёстко зафиксирован в 255; здесь пик — множитель
/// поверх `curve_value`, абсолютную силу задаёт `strength_pct`).
fn oneshot_envelope(
    elapsed_s: f64,
    attack_ms: f32,
    hold_ms: f32,
    decay_ms: f32,
    decay_exp: i32,
) -> f64 {
    if elapsed_s < 0.0 {
        return 0.0;
    }
    let attack_s = (attack_ms as f64 / 1000.0).max(0.0);
    let hold_s = (hold_ms as f64 / 1000.0).max(0.0);
    // Минимум > 0, иначе деление на 0 при decay_ms == 0.0 (руками введённый JSON).
    let decay_s = (decay_ms as f64 / 1000.0).max(0.0001);
    let exp = decay_exp.max(1);

    if elapsed_s < attack_s {
        if attack_s <= 0.0 {
            1.0
        } else {
            (elapsed_s / attack_s).clamp(0.0, 1.0)
        }
    } else if elapsed_s < attack_s + hold_s {
        1.0
    } else if elapsed_s < attack_s + hold_s + decay_s {
        let p = (elapsed_s - attack_s - hold_s) / decay_s;
        (1.0 - p).powi(exp).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn combine(acc: f64, v: f64, mix: MixMode) -> f64 {
    match mix {
        MixMode::Max => acc.max(v),
        MixMode::Add => acc + v,
    }
}

pub struct CustomFxEngine {
    states: HashMap<String, EffectState>,
    last_rev: u64,
}

impl Default for CustomFxEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomFxEngine {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            last_rev: 0,
        }
    }

    /// Полный сброс — смена режима эффектов (Custom -> BuiltIn -> Custom) или
    /// смена активной игры. Без него, например, при переключении обратно в
    /// Custom EMA/гистерезис остались бы от совсем другого полёта.
    pub fn reset(&mut self) {
        self.states.clear();
        self.last_rev = 0;
    }

    /// Один тик движка. `t` — единое время конвейера (тот же sim-time, что
    /// приходит в `TelemetryFrame`), `aircraft` — текущий title борта (для
    /// `aircraft_match`), `hold` — глобальная пауза HID-канала.
    #[allow(clippy::too_many_arguments)] // сигнатура зафиксирована контрактом задачи, см. вызовы у воркеров/UI
    pub fn step(
        &mut self,
        frame: &TelemetryFrame,
        t: f64,
        effects: &[CustomEffect],
        rev: u64,
        aircraft: &str,
        game: ActiveGame,
        hold: bool,
        max_output: u8,
    ) -> CustomFxOutput {
        if hold {
            // Тот же приём, что RumbleEngine::step: hold — это пауза HID-канала
            // (сим на паузе), а не "перемотка часов эффектов". Состояния
            // (EMA/фаза Pulse/гистерезис) НЕ трогаем — иначе после снятия hold
            // они дёрнулись бы со сбитой фазой/скачком EMA.
            return CustomFxOutput::default();
        }

        // Пересборка карты состояний при смене rev — НЕ полный сброс: id,
        // которые остались в списке, сохраняют состояние (иначе правка одного
        // слайдера в UI сбрасывала бы фазу/EMA ВСЕХ эффектов на каждый кадр
        // редактирования). Удаляются только записи, чьего id больше нет в
        // списке — иначе HashMap рос бы бесконечно при удалении/пересоздании
        // эффектов в UI.
        if rev != self.last_rev {
            let live_ids: HashSet<&str> = effects.iter().map(|e| e.id.as_str()).collect();
            self.states.retain(|id, _| live_ids.contains(id.as_str()));
            self.last_rev = rev;
        }

        let max_output_f = max_output as f64;
        // [joystick, throttle_left, throttle_right]
        let mut channels = [0f64; 3];
        let mut active_ids = Vec::new();

        for effect in effects {
            if !effect.enabled {
                continue;
            }
            if !effect.games.allows(game) {
                continue;
            }
            // SourceId::Lvar осмыслен ТОЛЬКО в MSFS: X-Plane конвертируется в
            // тот же FlightVars (см. xp_link::vars::to_flight_vars), но его
            // словарь lvars всегда пуст — read_lvar и так вернул бы None, и
            // эффект бы самонейтрализовался. Проверка здесь дублирует это
            // явно, а не полагается на "пустой словарь -> None" по цепочке:
            // так поведение видно прямо в месте, где уже фильтруется по игре
            // (games.allows), а не размазано между двумя модулями.
            if effect.source == SourceId::Lvar && game != ActiveGame::Msfs {
                continue;
            }
            if !aircraft_matches(&effect.aircraft_match, aircraft) {
                continue;
            }
            // None = источник не из текущего конвейера телеметрии (см.
            // sources::read) — эффект просто самонейтрализуется, молчит.
            // SourceId::Lvar — отдельная ветка: имя переменной живёт на самом
            // эффекте (model::CustomEffect::lvar), а не в таблице SOURCES
            // (см. doc-комментарий read_lvar), поэтому значение достаётся
            // отдельной функцией. Эффект без заполненного lvar (пользователь
            // выбрал источник, но ещё не вписал имя) молчит — как будто нет
            // сигнала, а не паникует/падает на дефолт.
            let Some(raw) = (if effect.source == SourceId::Lvar {
                effect
                    .lvar
                    .as_ref()
                    .and_then(|spec| sources::read_lvar(&spec.name, frame))
            } else {
                sources::read(effect.source, frame)
            }) else {
                continue;
            };

            let state = self
                .states
                .entry(effect.id.clone())
                .or_insert_with(EffectState::new);

            let is_active = trigger_active(&effect.trigger, raw, t, state);
            if is_active && !state.was_active {
                state.active_since_t = t;
                state.oneshot_fired_at = Some(t);
            }
            state.was_active = is_active;

            let raw_curve = if is_active {
                effect.curve.eval(raw)
            } else {
                0.0
            };
            let alpha = (effect.smoothing_alpha as f64).clamp(0.0, 1.0);
            state.ema = state.ema * alpha + raw_curve * (1.0 - alpha);

            let shape_mul = shape_multiplier(&effect.shape, t, state);
            let intensity = (state.ema * shape_mul * (effect.strength_pct as f64 / 100.0) * 255.0)
                .clamp(0.0, max_output_f);

            if intensity > 0.0 {
                let mut contributed = false;
                if effect.out.joystick {
                    channels[0] = combine(channels[0], intensity, effect.mix);
                    contributed = true;
                }
                if effect.out.throttle_left {
                    channels[1] = combine(channels[1], intensity, effect.mix);
                    contributed = true;
                }
                if effect.out.throttle_right {
                    channels[2] = combine(channels[2], intensity, effect.mix);
                    contributed = true;
                }
                if contributed {
                    active_ids.push(effect.id.clone());
                }
            }
        }

        let clamp_u8 = |v: f64| v.clamp(0.0, max_output_f) as u8;
        CustomFxOutput {
            joystick: clamp_u8(channels[0]),
            throttle_left: clamp_u8(channels[1]),
            throttle_right: clamp_u8(channels[2]),
            active_ids,
        }
    }
}

/// Общий тик для preview_waveform/preview_level — БЕЗ состояния движка
/// (`CustomFxEngine`): `EffectState` создаётся с нуля внутри самого вызова
/// preview-функции и живёт только на время одной симуляции. Триггер считается
/// взведшимся в t=0 (эмулируем "источник только что выдал raw_value") — без
/// гистерезиса и истории изменений: для осциллографа в UI важна ФОРМА
/// сигнала, а не точное воспроизведение поведения гистерезиса/Changed без
/// истории кадров, которой у превью попросту нет.
fn preview_trigger_active(trigger: &Trigger, x: f64, t: f64) -> bool {
    match trigger {
        Trigger::Always => true,
        Trigger::Above { value, .. } => x > *value,
        Trigger::Below { value, .. } => x < *value,
        Trigger::Between { lo, hi } => x >= *lo && x <= *hi,
        Trigger::IsTrue => x > 0.5,
        // Changed не хранит историю — считаем, что "изменение" произошло
        // ровно в t=0, поэтому окно активности — [0, hold_s).
        Trigger::Changed { hold_s, .. } => t < *hold_s,
    }
}

fn preview_tick(effect: &CustomEffect, raw_value: f64, t: f64, state: &mut EffectState) -> f64 {
    let is_active = preview_trigger_active(&effect.trigger, raw_value, t);
    if is_active && !state.was_active {
        state.active_since_t = t;
        state.oneshot_fired_at = Some(t);
    }
    state.was_active = is_active;

    let raw_curve = if is_active {
        effect.curve.eval(raw_value)
    } else {
        0.0
    };
    let alpha = (effect.smoothing_alpha as f64).clamp(0.0, 1.0);
    state.ema = state.ema * alpha + raw_curve * (1.0 - alpha);

    let shape_mul = shape_multiplier(&effect.shape, t, state);
    (state.ema * shape_mul * (effect.strength_pct as f64 / 100.0) * 255.0).clamp(0.0, 255.0)
}

/// Одна точка формы во времени для осциллографа в UI: чистая функция без
/// состояния движка — считает `samples` точек равномерно на окне
/// `[0, duration_s)`, начиная с t=0 как момента активации триггера.
pub fn preview_waveform(
    effect: &CustomEffect,
    raw_value: f64,
    duration_s: f64,
    samples: usize,
) -> Vec<f32> {
    let samples = samples.max(1);
    let dt = duration_s.max(0.0) / samples as f64;
    let mut state = EffectState::new();
    let mut out = Vec::with_capacity(samples);
    for i in 0..samples {
        let t = i as f64 * dt;
        out.push(preview_tick(effect, raw_value, t, &mut state) as f32);
    }
    out
}

/// Итоговая интенсивность эффекта в конкретный момент `t` при заданном сыром
/// значении источника — для ручного слайдера предпросмотра и оффлайн-прогона
/// записи. Прогоняет ту же симуляцию, что и `preview_waveform` (шагом 1/120с
/// от t=0 до `t`), чтобы EMA/фаза Pulse "устаканились" так же, как выглядели
/// бы в живом движке — иначе, например, EMA с сильным сглаживанием давала бы
/// в превью мгновенный скачок, которого в реальности никогда не будет.
pub fn preview_level(effect: &CustomEffect, raw_value: f64, t: f64) -> (u8, u8, u8) {
    const STEP: f64 = 1.0 / 120.0;
    // Потолок итераций — защита от произвольно большого t (не ожидается в
    // UI-сценариях, но preview_level — публичный API, ронять его не должны).
    const MAX_STEPS: usize = 12_000;
    let t = t.max(0.0);
    let steps = ((t / STEP).ceil() as usize).clamp(1, MAX_STEPS);

    let mut state = EffectState::new();
    let mut intensity = 0.0;
    for i in 0..steps {
        let tt = if i + 1 == steps { t } else { i as f64 * STEP };
        intensity = preview_tick(effect, raw_value, tt, &mut state);
    }

    let v = intensity.clamp(0.0, 255.0) as u8;
    (
        if effect.out.joystick { v } else { 0 },
        if effect.out.throttle_left { v } else { 0 },
        if effect.out.throttle_right { v } else { 0 },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_fx::model::{
        CurvePoint, MixMode, OutputRouting, ResponseCurve, Shape, Trigger, new_effect,
    };
    use crate::custom_fx::sources::SourceId;
    use crate::types::FlightVars;

    fn flat_curve(y: f64) -> ResponseCurve {
        ResponseCurve {
            points: vec![CurvePoint { x: 0.0, y }, CurvePoint { x: 1.0, y }],
        }
    }

    #[test]
    fn above_trigger_has_hysteresis() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::FlightAirspeedKn);
        effect.trigger = Trigger::Above {
            value: 100.0,
            hysteresis: 10.0,
        };
        effect.curve = flat_curve(1.0);
        effect.smoothing_alpha = 0.0;
        effect.shape = Shape::Constant;
        effect.strength_pct = 100.0;
        let effects = vec![effect];

        let mk = |speed: f64| {
            TelemetryFrame::Flight(FlightVars {
                airspeed_indicated: speed,
                ..Default::default()
            })
        };

        let out = engine.step(
            &mk(50.0),
            0.0,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(out.joystick, 0, "ниже порога — молчит");

        let out = engine.step(
            &mk(150.0),
            1.0,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert!(out.joystick > 0, "выше порога — взвёлся");

        let out = engine.step(
            &mk(95.0),
            2.0,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert!(
            out.joystick > 0,
            "внутри полосы гистерезиса (90..100) — не снялся"
        );

        let out = engine.step(
            &mk(85.0),
            3.0,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(out.joystick, 0, "ниже полосы гистерезиса — снялся");
    }

    #[test]
    fn changed_trigger_holds_and_ignores_dither() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::FlightFlapsPct);
        effect.trigger = Trigger::Changed {
            eps: 5.0,
            hold_s: 1.0,
        };
        effect.curve = ResponseCurve::linear(0.0, 100.0);
        effect.smoothing_alpha = 0.0;
        effect.shape = Shape::Constant;
        effect.strength_pct = 100.0;
        let effects = vec![effect];

        let mk = |pct: f64| {
            TelemetryFrame::Flight(FlightVars {
                flaps_pct: pct,
                ..Default::default()
            })
        };

        let out = engine.step(
            &mk(50.0),
            0.0,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(
            out.joystick, 0,
            "первый кадр — фиксация стартового значения, не изменение"
        );

        let out = engine.step(
            &mk(52.0),
            0.5,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(out.joystick, 0, "дребезг меньше eps — не взводит");

        let out = engine.step(
            &mk(70.0),
            1.0,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert!(out.joystick > 0, "изменение больше eps — взвело");

        let out = engine.step(
            &mk(70.0),
            1.5,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert!(out.joystick > 0, "внутри hold_s — держится");

        let out = engine.step(
            &mk(70.0),
            2.5,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(out.joystick, 0, "hold_s истёк — погасло");
    }

    #[test]
    fn oneshot_fires_once_per_edge_not_while_held() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::FlightOnGround);
        effect.trigger = Trigger::IsTrue;
        effect.curve = flat_curve(1.0);
        effect.smoothing_alpha = 0.0;
        effect.shape = Shape::OneShot {
            attack_ms: 0.0,
            hold_ms: 0.0,
            decay_ms: 100.0,
            decay_exp: 1,
        };
        effect.strength_pct = 100.0;
        let effects = vec![effect];

        let mk = |on: bool| {
            TelemetryFrame::Flight(FlightVars {
                on_ground: on,
                ..Default::default()
            })
        };

        let out = engine.step(
            &mk(true),
            0.0,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(out.joystick, 255, "фронт активации — импульс на пике");

        let out = engine.step(
            &mk(true),
            0.2,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(
            out.joystick, 0,
            "триггер держится дальше без нового фронта — импульс НЕ перезапустился, decay уже отгремел"
        );

        let out = engine.step(
            &mk(false),
            0.3,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(out.joystick, 0);

        let out = engine.step(
            &mk(true),
            0.4,
            &effects,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(out.joystick, 255, "новый фронт — новый импульс");
    }

    #[test]
    fn pulse_has_on_and_off_phase_with_floor() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::FlightOnGround);
        effect.trigger = Trigger::Always;
        effect.curve = flat_curve(1.0);
        effect.smoothing_alpha = 0.0;
        effect.shape = Shape::Pulse {
            freq_hz: 2.0,
            duty_pct: 50.0,
            jitter_pct: 0.0,
            floor_pct: 20.0,
            attack_ms: 0.0,
        };
        effect.strength_pct = 100.0;
        let effects = vec![effect];
        let frame = TelemetryFrame::Flight(FlightVars::default());

        // freq=2Гц -> период 0.5с, duty 50% -> "включено" первые 0.25с цикла.
        let on = engine.step(&frame, 0.1, &effects, 1, "", ActiveGame::Msfs, false, 255);
        assert!(on.joystick > 200, "фаза включено, jitter=0 — полная сила");

        let off = engine.step(&frame, 0.4, &effects, 1, "", ActiveGame::Msfs, false, 255);
        assert!(off.joystick > 0, "пауза, но floor_pct=20% — не ноль");
        assert!(off.joystick < on.joystick);
    }

    #[test]
    fn mixing_max_takes_maximum_and_add_sums_with_clamp() {
        let mk_effect = |mix: MixMode, y: f64| {
            let mut e = new_effect("Test".into(), SourceId::FlightOnGround);
            e.trigger = Trigger::Always;
            e.curve = flat_curve(y);
            e.smoothing_alpha = 0.0;
            e.shape = Shape::Constant;
            e.strength_pct = 100.0;
            e.mix = mix;
            e.out = OutputRouting {
                joystick: true,
                throttle_left: false,
                throttle_right: false,
            };
            e
        };
        let frame = TelemetryFrame::Flight(FlightVars::default());

        let mut engine = CustomFxEngine::new();
        let effects_max = vec![mk_effect(MixMode::Max, 0.5), mk_effect(MixMode::Max, 0.8)];
        let out = engine.step(
            &frame,
            0.0,
            &effects_max,
            1,
            "",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(
            out.joystick,
            (0.8 * 255.0) as u8,
            "Max — максимум, не сумма"
        );

        let mut engine2 = CustomFxEngine::new();
        let effects_add = vec![mk_effect(MixMode::Add, 0.5), mk_effect(MixMode::Add, 0.8)];
        let out = engine2.step(
            &frame,
            0.0,
            &effects_add,
            1,
            "",
            ActiveGame::Msfs,
            false,
            200,
        );
        assert_eq!(out.joystick, 200, "Add — сумма, зажатая max_output");
    }

    #[test]
    fn hold_returns_zero_on_all_channels() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::FlightOnGround);
        effect.trigger = Trigger::Always;
        effect.curve = flat_curve(1.0);
        effect.out = OutputRouting {
            joystick: true,
            throttle_left: true,
            throttle_right: true,
        };
        let effects = vec![effect];
        let frame = TelemetryFrame::Flight(FlightVars::default());

        let out = engine.step(&frame, 0.0, &effects, 1, "", ActiveGame::Msfs, true, 255);
        assert_eq!(out.joystick, 0);
        assert_eq!(out.throttle_left, 0);
        assert_eq!(out.throttle_right, 0);
        assert!(out.active_ids.is_empty());
    }

    #[test]
    fn rev_change_with_same_ids_preserves_state() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::FlightOnGround);
        effect.trigger = Trigger::Always;
        effect.curve = flat_curve(1.0);
        // Сильное сглаживание — EMA не долетает до 1.0 за один тик, поэтому
        // видно, продолжает ли она расти или обнулилась при смене rev.
        effect.smoothing_alpha = 0.9;
        let effects = vec![effect];
        let frame = TelemetryFrame::Flight(FlightVars::default());

        let out1 = engine.step(&frame, 0.0, &effects, 1, "", ActiveGame::Msfs, false, 255);
        let out2 = engine.step(&frame, 0.1, &effects, 2, "", ActiveGame::Msfs, false, 255);

        assert!(
            out2.joystick > out1.joystick,
            "rev поменялся, но id тот же — EMA продолжает расти, а не начинает с нуля"
        );
    }

    #[test]
    fn lvar_effect_reads_value_by_name_from_flight_vars() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::Lvar);
        effect.lvar = Some(crate::custom_fx::model::LvarSpec {
            name: "L:A320_Gear_Nose".into(),
            unit: "Number".into(),
        });
        effect.trigger = Trigger::Always;
        effect.curve = ResponseCurve::linear(0.0, 1000.0);
        effect.smoothing_alpha = 0.0;
        effect.shape = Shape::Constant;
        effect.strength_pct = 100.0;
        let effects = vec![effect];

        let mut fv = FlightVars::default();
        fv.lvars.insert("L:A320_Gear_Nose".to_string(), 1000.0);
        let frame = TelemetryFrame::Flight(fv);

        let out = engine.step(&frame, 0.0, &effects, 1, "", ActiveGame::Msfs, false, 255);
        assert_eq!(
            out.joystick, 255,
            "значение есть в словаре — эффект даёт выход"
        );
    }

    #[test]
    fn lvar_effect_silent_when_name_not_in_dict() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::Lvar);
        effect.lvar = Some(crate::custom_fx::model::LvarSpec {
            name: "L:Unknown_Var".into(),
            unit: "Number".into(),
        });
        effect.trigger = Trigger::Always;
        effect.curve = flat_curve(1.0);
        effect.smoothing_alpha = 0.0;
        effect.shape = Shape::Constant;
        effect.strength_pct = 100.0;
        let effects = vec![effect];

        // Словарь непустой, но не содержит запрошенное имя — read_lvar
        // возвращает None, эффект молчит.
        let mut fv = FlightVars::default();
        fv.lvars.insert("L:Something_Else".to_string(), 42.0);
        let frame = TelemetryFrame::Flight(fv);

        let out = engine.step(&frame, 0.0, &effects, 1, "", ActiveGame::Msfs, false, 255);
        assert_eq!(out.joystick, 0, "имени нет в словаре — молчит");
    }

    #[test]
    fn lvar_effect_silent_when_lvar_not_configured() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::Lvar);
        // effect.lvar остался None — пользователь выбрал источник, но ещё не
        // вписал имя переменной.
        effect.trigger = Trigger::Always;
        effect.curve = flat_curve(1.0);
        effect.smoothing_alpha = 0.0;
        effect.shape = Shape::Constant;
        effect.strength_pct = 100.0;
        let effects = vec![effect];

        let mut fv = FlightVars::default();
        fv.lvars.insert("L:Something".to_string(), 1.0);
        let frame = TelemetryFrame::Flight(fv);

        let out = engine.step(&frame, 0.0, &effects, 1, "", ActiveGame::Msfs, false, 255);
        assert_eq!(out.joystick, 0, "lvar не заполнен — молчит");
    }

    #[test]
    fn lvar_effect_silent_on_xplane_even_with_matching_name() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::Lvar);
        effect.lvar = Some(crate::custom_fx::model::LvarSpec {
            name: "L:A320_Gear_Nose".into(),
            unit: "Number".into(),
        });
        effect.trigger = Trigger::Always;
        effect.curve = flat_curve(1.0);
        effect.smoothing_alpha = 0.0;
        effect.shape = Shape::Constant;
        effect.strength_pct = 100.0;
        let effects = vec![effect];

        // В X-Plane словарь lvars всегда пуст, но проверяем явный gate по
        // игре, а не полагаемся на пустоту словаря.
        let mut fv = FlightVars::default();
        fv.lvars.insert("L:A320_Gear_Nose".to_string(), 1000.0);
        let frame = TelemetryFrame::Flight(fv);

        let out = engine.step(&frame, 0.0, &effects, 1, "", ActiveGame::Xplane, false, 255);
        assert_eq!(out.joystick, 0, "SourceId::Lvar осмыслен только в MSFS");
    }

    #[test]
    fn disabled_or_wrong_game_or_aircraft_stays_silent() {
        let mut engine = CustomFxEngine::new();
        let mut effect = new_effect("Test".into(), SourceId::FlightOnGround);
        effect.trigger = Trigger::Always;
        effect.curve = flat_curve(1.0);
        effect.aircraft_match = Some("A320".into());
        let effects = vec![effect];
        let frame = TelemetryFrame::Flight(FlightVars::default());

        let out = engine.step(
            &frame,
            0.0,
            &effects,
            1,
            "boeing 737",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert_eq!(out.joystick, 0, "борт не совпал по подстроке");

        let out = engine.step(
            &frame,
            0.0,
            &effects,
            1,
            "Airbus A320neo",
            ActiveGame::Msfs,
            false,
            255,
        );
        assert!(out.joystick > 0, "регистронезависимое совпадение подстроки");
    }
}
