use std::time::{Duration, Instant};

use crate::{EffectsSnapshot, FlightVars, RumbleConfig};

#[derive(Debug, Clone, Copy, Default)]
pub struct RumbleState {
    prev_gear: f64,
    gear_t0: f64,
    gear_t1: f64,
    gear_peak: f64,
    bg_smoothed: f64,
    bg_smoothed_throttle: f64,
    last_cfg_rev: u64,
    // Gear Strut Compression (Touchdown) tracking
    prev_sim_time_s: f64,
    prev_gear_comp_nose: f64,
    prev_gear_comp_left: f64,
    prev_gear_comp_right: f64,
    gear_comp_nose_t0: f64,
    gear_comp_left_t0: f64,
    gear_comp_right_t0: f64,
    gear_comp_nose_dyn_peak: f64,
    gear_comp_left_dyn_peak: f64,
    gear_comp_right_dyn_peak: f64,
    // Gear Transit tracking
    prev_gear_nose: f64,
    prev_gear_left: f64,
    prev_gear_right: f64,
    gear_doors_closed_t0: f64,
    // Flaps Motor Hum tracking
    last_flaps_percent: f64,
    current_flaps_amplitude: f64,
    // Ground Roll (физическая модель удара о стыки плит) tracking
    thump_last_time_s: f64,
    // Engine Spool-up & Ignition tracking
    prev_eng1_combustion: bool,
    prev_eng2_combustion: bool,
    // Таймер удара воспламенения — реальный wall-clock Instant (не sim_time_s),
    // как явно требуется для этого конкретного эффекта.
    eng1_kick_started_at: Option<Instant>,
    eng2_kick_started_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RumbleOutput {
    pub joystick_intensity: u8,
    pub throttle_intensity: u8,
    pub effects: EffectsSnapshot,
}

pub struct RumbleEngine {
    state: RumbleState,
}

impl Default for RumbleEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RumbleEngine {
    pub fn new() -> Self {
        Self {
            state: RumbleState {
                gear_t0: -1.0,
                gear_t1: -1.0,
                gear_comp_nose_t0: -1.0,
                gear_comp_left_t0: -1.0,
                gear_comp_right_t0: -1.0,
                prev_sim_time_s: -1.0,
                prev_gear_nose: 0.0,
                prev_gear_left: 0.0,
                prev_gear_right: 0.0,
                gear_doors_closed_t0: -1.0,
                last_flaps_percent: 0.0,
                current_flaps_amplitude: 0.0,
                thump_last_time_s: -1000.0,
                ..Default::default()
            },
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn step(
        &mut self,
        fv: &FlightVars,
        cfg: &RumbleConfig,
        cfg_rev: u64,
        hold: bool,
    ) -> RumbleOutput {
        let gs = fv.ground_speed_kt;
        let start = if cfg.taxi_start_enabled { cfg.taxi_start_kn.min(cfg.taxi_end_kn - 0.1) } else { 0.0 };
        let end = if cfg.taxi_end_enabled { cfg.taxi_end_kn.max(start + 0.1) } else { 9999.0 };

        // Минимальный порог скорости 1.0 узел для предотвращения вибрации в статике при отключенных чекбоксах
        let start_active = start.max(1.0);

        let in_thump_band = cfg.ground_enabled && fv.on_ground && gs >= start_active && gs < end;
        let at_or_above_end = cfg.ground_enabled && cfg.taxi_end_enabled && fv.on_ground && gs >= end;
        let at_or_above_start = cfg.taxi_start_enabled && fv.on_ground && gs >= start;

        let overspeed_threshold_kn = cfg.overspeed_threshold_kn as f64;
        let bank_threshold_deg = cfg.bank_threshold_deg as f64;

        let spoilers_active = cfg.spoilers_enabled
            && fv.spoilers_pct > cfg.spoilers_threshold_pct
            && fv.airspeed_indicated > 20.0;

        let mut effects = EffectsSnapshot {
            taxi_start_crossed: at_or_above_start,
            taxi_end_crossed: at_or_above_end,
            ground_thump_active: in_thump_band,
            ground_active: at_or_above_end,
            stall_active: fv.stalled,
            bank_active: !fv.on_ground && fv.bank_deg.abs() > bank_threshold_deg,
            spoilers_active,
            overspeed_active: false, // Will be set below if overspeed is active
            ..Default::default()
        };

        if fv.paused || hold {
            return RumbleOutput {
                joystick_intensity: 0,
                throttle_intensity: 0,
                effects,
            };
        }

        let s = &mut self.state;
        let mut dt = fv.sim_time_s - s.prev_sim_time_s;
        if s.prev_sim_time_s < 0.0 {
            dt = 0.0;
        }

        // Gear Strut Compression / Touchdown Detection
        const GEAR_COMP_TOUCHDOWN_THRESHOLD: f64 = 50.1;
        const GEAR_COMP_BUMP_DURATION: f64 = 0.15; // Резкий импульс за 0.15с

        // ═══════════════════════════════════════════════════════════════════
        // РЕМАРКА ДЛЯ РУЧНОЙ НАСТРОЙКИ (диапазоны силы эффектов):
        //
        // • Сжатие стоек шасси (touchdown bump). Слайдер gear_comp_*_peak в UI
        //   имеет диапазон 0..55 — это НЕ сырая сила, а "запас сверху" над
        //   обязательным полом в 200. Слайдер=0 → всегда строго 200 (мягкий
        //   предел мотора при любой посадке). Слайдер=55 → от 200 (мягкая
        //   посадка) до 255 (жёсткая, severity на максимуме). Итоговая сила
        //   физически не может выйти за пределы [200..255].
        //
        // • Удар о стыки плит (ground_roll). Слайдер в UI — 0..50, это и есть
        //   итоговый потолок силы (amplitude_curve лишь масштабирует от 0 до
        //   этого значения). Должен оставаться мягким фоновым эффектом и НЕ
        //   соперничать по ощущению с ударом сжатия стоек (200-255).
        // ═══════════════════════════════════════════════════════════════════
        const GEAR_COMP_PEAK_MIN: f64 = 200.0;
        const GEAR_COMP_PEAK_MAX: f64 = 255.0;
        const GEAR_COMP_HEADROOM_MAX: f64 = 55.0; // слайдер gear_comp_*_peak: 0..55
        const GROUND_THUMP_PEAK_MIN: f64 = 0.0;
        const GROUND_THUMP_PEAK_MAX: f64 = 50.0;

        if cfg.gear_comp_enabled && dt > 0.0 {
            // Nose Gear
            if cfg.gear_comp_nose_enabled && fv.gear_comp_nose >= GEAR_COMP_TOUCHDOWN_THRESHOLD && s.prev_gear_comp_nose < GEAR_COMP_TOUCHDOWN_THRESHOLD {
                s.gear_comp_nose_t0 = fv.sim_time_s;
                let comp_rate = (fv.gear_comp_nose - s.prev_gear_comp_nose) / dt;
                let severity = (comp_rate / 100.0).clamp(0.3, 2.5);
                let intensity_frac = ((severity - 0.3) / (2.5 - 0.3)).clamp(0.0, 1.0);
                let headroom = (cfg.gear_comp_nose_peak as f64).clamp(0.0, GEAR_COMP_HEADROOM_MAX);
                let raw_peak = GEAR_COMP_PEAK_MIN + headroom * intensity_frac;
                s.gear_comp_nose_dyn_peak = raw_peak.clamp(GEAR_COMP_PEAK_MIN, GEAR_COMP_PEAK_MAX);
            }
            s.prev_gear_comp_nose = fv.gear_comp_nose;

            // Left Gear
            if cfg.gear_comp_left_enabled && fv.gear_comp_left >= GEAR_COMP_TOUCHDOWN_THRESHOLD && s.prev_gear_comp_left < GEAR_COMP_TOUCHDOWN_THRESHOLD {
                s.gear_comp_left_t0 = fv.sim_time_s;
                let comp_rate = (fv.gear_comp_left - s.prev_gear_comp_left) / dt;
                let severity = (comp_rate / 100.0).clamp(0.3, 2.5);
                let intensity_frac = ((severity - 0.3) / (2.5 - 0.3)).clamp(0.0, 1.0);
                let headroom = (cfg.gear_comp_left_peak as f64).clamp(0.0, GEAR_COMP_HEADROOM_MAX);
                let raw_peak = GEAR_COMP_PEAK_MIN + headroom * intensity_frac;
                s.gear_comp_left_dyn_peak = raw_peak.clamp(GEAR_COMP_PEAK_MIN, GEAR_COMP_PEAK_MAX);
            }
            s.prev_gear_comp_left = fv.gear_comp_left;

            // Right Gear
            if cfg.gear_comp_right_enabled && fv.gear_comp_right >= GEAR_COMP_TOUCHDOWN_THRESHOLD && s.prev_gear_comp_right < GEAR_COMP_TOUCHDOWN_THRESHOLD {
                s.gear_comp_right_t0 = fv.sim_time_s;
                let comp_rate = (fv.gear_comp_right - s.prev_gear_comp_right) / dt;
                let severity = (comp_rate / 100.0).clamp(0.3, 2.5);
                let intensity_frac = ((severity - 0.3) / (2.5 - 0.3)).clamp(0.0, 1.0);
                let headroom = (cfg.gear_comp_right_peak as f64).clamp(0.0, GEAR_COMP_HEADROOM_MAX);
                let raw_peak = GEAR_COMP_PEAK_MIN + headroom * intensity_frac;
                s.gear_comp_right_dyn_peak = raw_peak.clamp(GEAR_COMP_PEAK_MIN, GEAR_COMP_PEAK_MAX);
            }
            s.prev_gear_comp_right = fv.gear_comp_right;
        } else {
            s.prev_gear_comp_nose = fv.gear_comp_nose;
            s.prev_gear_comp_left = fv.gear_comp_left;
            s.prev_gear_comp_right = fv.gear_comp_right;
        }
        s.prev_sim_time_s = fv.sim_time_s;

        // =========================================================================
        // БЛОК ЗАКРЫЛКОВ (FLAPS MOTOR HUM)
        // =========================================================================

        // 1. Проверяем, движутся ли физически закрылки
        let flaps_delta = (fv.flaps_pct - s.last_flaps_percent).abs();
        let flaps_is_moving = flaps_delta > 0.01; // Переименовали, чтобы не затенять closure ниже

        // Целевая рабочая мощность (0.8 — это примерно 200 из 255)
        let max_amplitude = cfg.flaps_duty.clamp(0.01, 0.8);

        // Ограничиваем dt, чтобы при лагах/паузах симулятора не было резкого скачка амплитуды
        let dt_clamped = dt.min(0.1);

        if flaps_is_moving {
            // ----------------------------------------------------------------------
            // НАСТРОЙКА ВРЕМЕНИ РАСКРУТКИ МОТОРА ЗАКРЫЛКОВ
            // Теперь не зависит от FPS, используем реальное время dt_clamped.
            // ----------------------------------------------------------------------
            let ramp_up_time_s = 5.0; // Время раскрутки ~5 секунд
            let step_up = max_amplitude * (dt_clamped / ramp_up_time_s);

            // Плавно прибавляем силу
            s.current_flaps_amplitude = (s.current_flaps_amplitude + step_up).min(max_amplitude);
        } else {
            // Плавно глушим мотор при остановке
            let ramp_down_time_s = 5.0; // Время затухания ~5 секунд
            let step_down = max_amplitude * (dt_clamped / ramp_down_time_s);

            // Плавно убавляем силу до нуля
            s.current_flaps_amplitude = (s.current_flaps_amplitude - step_down).max(0.0);
        }

        // 2. Применяем эффект, если амплитуда больше минимального порога.
        // s.current_flaps_amplitude — это duty cycle (0.0 .. 0.8), поэтому
        // мы сами формируем вибрацию (программный ШИМ) на частоте 25 Гц;
        // маршрутизация на джойстик/РУД применяется позже, при подмешивании
        // flaps_term в transients_j/transients_t.
        let mut flaps_term: f64 = 0.0;
        if cfg.flaps_enabled && s.current_flaps_amplitude > 0.01 {
            let fixed_period = 0.04; // 0.04 с = 25 Гц
            let cycle = (fv.sim_time_s / fixed_period).fract();

            // Создаем пульсацию (от 0.0 до 1.0) в виде полуволн синуса
            let oscillation = (std::f64::consts::PI * cycle).sin();

            // Преобразуем duty cycle в силу вибрации (0 .. 255)
            flaps_term = s.current_flaps_amplitude * 255.0 * oscillation;
            effects.flaps_bump_active = true;
        } else {
            effects.flaps_bump_active = false;
        }

        // 3. Запоминаем позицию закрылков для следующего кадра
        s.last_flaps_percent = fv.flaps_pct;

        // =========================================================================
        // БЛОК ВЫПУСКА/УБОРКИ ШАССИ (Gear Handle Bump)
        // =========================================================================

        if (fv.gear_handle - s.prev_gear).abs() >= 0.5 {
            s.gear_t0 = fv.sim_time_s;
            s.gear_t1 = fv.sim_time_s + cfg.gear_bump_duration_s;
            s.gear_peak = cfg.gear_peak as f64;
        }
        s.prev_gear = fv.gear_handle;

        // Каждый термин теперь считается сразу в двух каналах — джойстик (_j)
        // и РУД (_t) — согласно cfg.device_targets для соответствующего
        // эффекта. Это позволяет независимо маршрутизировать каждый эффект
        // на одно/оба/ни одного устройства.
        let dt_ = &cfg.device_targets;
        let mut ground_term_j: f64 = 0.0;
        let mut ground_term_t: f64 = 0.0;
        let mut air_term_j: f64 = 0.0;
        let mut air_term_t: f64 = 0.0;
        let mut transients_j: f64 = 0.0;
        let mut transients_t: f64 = 0.0;
        let mut bank_term_j: f64 = 0.0;
        let mut bank_term_t: f64 = 0.0;
        let mut spoilers_term_j: f64 = 0.0;
        let mut spoilers_term_t: f64 = 0.0;

        // Ground Roll effect — стук о стыки бетонных плит на рулении/разбеге.
        // ФИЗИЧЕСКАЯ МОДЕЛЬ: период удара = длина_плиты / скорость (t = S / V).
        // Время — fv.sim_time_s (НЕ Instant::now()), чтобы корректно работать
        // на паузе симулятора и при ускорении времени (time acceleration).
        if cfg.ground_enabled && fv.on_ground && gs >= start_active {

            // ═══════════════════════════════════════════════════════════════════
            //  ПАРАМЕТРЫ ЭФФЕКТА «СТУК О СТЫКИ ПЛИТ» — правь здесь при тестах
            // ═══════════════════════════════════════════════════════════════════

            // Длина одной бетонной плиты ВПП в метрах — из настроек программы.
            let slab_length_m = cfg.runway_slab_length_m.max(0.5) as f64;

            // Длительность одного импульса (удара) в секундах — из настроек программы.
            let thump_duration_s = (cfg.thump_duration_ms.max(1.0) / 1000.0) as f64;

            // Сила удара — пиковая амплитуда (0..255) — из настроек программы.
            let thump_amplitude: f64 = cfg.ground_roll as f64;

            // ═══════════════════════════════════════════════════════════════════

            // 1. Переводим текущую GS из узлов в метры в секунду (1 узел = 0.514444 м/с)
            let speed_mps = gs * 0.514444;

            // 2а. Прогресс скорости от 0.0 до 1.0 в диапазоне [0 .. taxi_end_kn].
            // Используется и для амплитуды, и для кривизны нарастания частоты периода.
            let speed_progress = (gs / cfg.taxi_end_kn.max(0.1)).clamp(0.0, 1.0);

            // 2б. "Чистый" физический период по формуле t = S / V, зажатый в безопасные
            // для HID-канала границы [thump_min_period_s .. thump_max_period_s].
            let physical_period_s = (slab_length_m / speed_mps)
                .clamp(cfg.thump_min_period_s, cfg.thump_max_period_s);

            // 2в. КОЭФФИЦИЕНT КРИВИЗНЫ (cfg.thump_period_curve): управляет тем, КАК БЫСТРО
            // период сокращается (паузы между ударами укорачиваются) по мере роста скорости.
            // Чистая физика (t = S/V) сокращает период очень резко уже на малых скоростях —
            // этот коэффициент позволяет растянуть переход.
            //   1.0 — без изменений (как физика просчитала)
            //   >1.0 — период дольше остаётся близким к максимуму, резкое сокращение паузы
            //          сдвигается к более высоким скоростям (плавнее на старте)
            //   <1.0 — наоборот, период сокращается ещё быстрее, чем по чистой физике
            let period_curve_exp = (cfg.thump_period_curve as f64).max(0.1);
            let period_progress = speed_progress.powf(period_curve_exp);
            let target_period_s = cfg.thump_max_period_s
                - (cfg.thump_max_period_s - physical_period_s) * period_progress;

            // 3. Нелинейное нарастание амплитуды (более резкий рост к верхней границе).
            let amplitude_curve = speed_progress.powf(1.4);

            // 4. Логика перезапуска цикла импульса (стык плиты позади — ждём следующий).
            let time_since_last_thump = fv.sim_time_s - s.thump_last_time_s;
            if time_since_last_thump >= target_period_s {
                s.thump_last_time_s = fv.sim_time_s;
            }
            let time_since_last_thump = fv.sim_time_s - s.thump_last_time_s;

            // 5. Окно удара. Если период короче длительности импульса — удары сливаются
            // в сплошной гул (актуально на высоких скоростях рулёжки/разбега).
            if time_since_last_thump < thump_duration_s || target_period_s <= thump_duration_s {
                let raw_term = (thump_amplitude * amplitude_curve)
                    .clamp(GROUND_THUMP_PEAK_MIN, GROUND_THUMP_PEAK_MAX);
                if dt_.ground_roll.enable_joystick { ground_term_j = raw_term; }
                if dt_.ground_roll.enable_throttle { ground_term_t = raw_term; }
            }
        } else {
            // Эффект неактивен (в воздухе/стоит/выключен) — сбрасываем фазу,
            // чтобы при следующем рулении удар не "досчитывал" старый интервал.
            s.thump_last_time_s = fv.sim_time_s - 1000.0;
        }

        // Базовый фон полёта удалён по требованию пользователя
        // const BASE_RUMBLE_MAGNITUDE: f64 = 40.0;
        // if cfg.base_enabled && !fv.on_ground && fv.airspeed_indicated > cfg.base_airspeed {
        //     let excess = fv.airspeed_indicated - cfg.base_airspeed;
        //     let ratio = (excess / 60.0).clamp(0.0, 1.0);
        //     air_term += ratio * BASE_RUMBLE_MAGNITUDE;
        // }

        if cfg.overspeed_enabled {
            if !fv.on_ground && fv.airspeed_indicated > overspeed_threshold_kn {
                let overspeed = fv.airspeed_indicated - overspeed_threshold_kn;
                let ratio = (overspeed / 120.0).clamp(0.0, 1.0);
                let intensity = ratio * (cfg.overspeed_intensity as f64);
                let oscillation = (2.0 * std::f64::consts::PI * (5.0 + ratio * 15.0) * fv.sim_time_s).sin() * 0.5 + 0.5;
                let term = intensity * (0.7 + 0.3 * oscillation);
                if dt_.overspeed.enable_joystick { air_term_j += term; }
                if dt_.overspeed.enable_throttle { air_term_t += term; }
                effects.overspeed_active = true;
            }
        }

        if cfg.bank_enabled && !fv.on_ground {
            let bank_abs = fv.bank_deg.abs();
            if bank_abs > bank_threshold_deg {
                let raw_norm = ((bank_abs - bank_threshold_deg) / (90.0 - bank_threshold_deg)).clamp(0.0, 1.0);
                if (fv.sim_time_s % 0.15) < (0.15 * raw_norm) {
                    let term = cfg.bank_intensity as f64;
                    if dt_.bank.enable_joystick { bank_term_j = term; }
                    if dt_.bank.enable_throttle { bank_term_t = term; }
                }
            }
        }

        if spoilers_active {
            let min_pct = cfg.spoilers_threshold_pct;
            let defl_norm = ((fv.spoilers_pct - min_pct) / (100.0 - min_pct)).clamp(0.0, 1.0);
            let base_spoilers_intensity = 1.0 + defl_norm * ((cfg.spoilers_intensity as f64) - 1.0);
            let speed_factor = (fv.airspeed_indicated / 300.0).clamp(0.0, 1.2);
            let oscillation = (2.0 * std::f64::consts::PI * 25.0 * fv.sim_time_s).sin() * 0.4 + 0.6;
            let term = base_spoilers_intensity * speed_factor * oscillation;
            if dt_.spoilers.enable_joystick { spoilers_term_j = term; }
            if dt_.spoilers.enable_throttle { spoilers_term_t = term; }
        }

        if cfg.stall_enabled && fv.stalled {
            let ceiling = cfg.stall_ceiling as f64;
            if dt_.stall.enable_joystick { transients_j = transients_j.max(ceiling); }
            if dt_.stall.enable_throttle { transients_t = transients_t.max(ceiling); }
        }

        if cfg.gear_enabled {
            let gear_active = fv.sim_time_s >= s.gear_t0 && fv.sim_time_s <= s.gear_t1 && s.gear_peak > 0.0;
            if gear_active {
                let p = ((fv.sim_time_s - s.gear_t0) / (s.gear_t1 - s.gear_t0)).clamp(0.0, 1.0);
                let term = s.gear_peak * (std::f64::consts::PI * p).sin();
                if dt_.gear_bump.enable_joystick { transients_j += term; }
                if dt_.gear_bump.enable_throttle { transients_t += term; }
            }
            effects.gear_bump_active = gear_active;
        }

        if cfg.gear_comp_enabled {
            let nose_active = cfg.gear_comp_nose_enabled && fv.sim_time_s >= s.gear_comp_nose_t0 && fv.sim_time_s <= s.gear_comp_nose_t0 + GEAR_COMP_BUMP_DURATION;
            if nose_active {
                let p = ((fv.sim_time_s - s.gear_comp_nose_t0) / GEAR_COMP_BUMP_DURATION).clamp(0.0, 1.0);
                let term = s.gear_comp_nose_dyn_peak * (1.0 - p).powi(3);
                if dt_.gear_comp_nose.enable_joystick { transients_j += term; }
                if dt_.gear_comp_nose.enable_throttle { transients_t += term; }
            }
            effects.gear_comp_nose_active = nose_active;

            let left_active = cfg.gear_comp_left_enabled && fv.sim_time_s >= s.gear_comp_left_t0 && fv.sim_time_s <= s.gear_comp_left_t0 + GEAR_COMP_BUMP_DURATION;
            if left_active {
                let p = ((fv.sim_time_s - s.gear_comp_left_t0) / GEAR_COMP_BUMP_DURATION).clamp(0.0, 1.0);
                let term = s.gear_comp_left_dyn_peak * (1.0 - p).powi(3);
                if dt_.gear_comp_left.enable_joystick { transients_j += term; }
                if dt_.gear_comp_left.enable_throttle { transients_t += term; }
            }
            effects.gear_comp_left_active = left_active;

            let right_active = cfg.gear_comp_right_enabled && fv.sim_time_s >= s.gear_comp_right_t0 && fv.sim_time_s <= s.gear_comp_right_t0 + GEAR_COMP_BUMP_DURATION;
            if right_active {
                let p = ((fv.sim_time_s - s.gear_comp_right_t0) / GEAR_COMP_BUMP_DURATION).clamp(0.0, 1.0);
                let term = s.gear_comp_right_dyn_peak * (1.0 - p).powi(3);
                if dt_.gear_comp_right.enable_joystick { transients_j += term; }
                if dt_.gear_comp_right.enable_throttle { transients_t += term; }
            }
            effects.gear_comp_right_active = right_active;
        }

        // --- БЛОК: ЭФФЕКТ ДВИЖЕНИЯ ШАССИ (Gear Transit + Gear Doors Closed) ---
        // Раньше не был привязан ни к одному чекбоксу и работал постоянно.
        // Теперь оба под общим cfg.gear_transit_enabled.
        // Переименовали closure, чтобы избежать конфликта с переменной закрылков
        let gear_is_moving = |pos: f64, prev: f64| -> bool {
            pos > 0.0 && pos < 50.0 && (pos - prev).abs() >= 0.001
        };

        let mut gear_transit_term: f64 = 0.0;

        if cfg.gear_transit_enabled {
            // Использует переменные анимации шасси из FlightVars
            let moving_count = gear_is_moving(fv.gear_comp_nose, s.prev_gear_nose) as i32
                             + gear_is_moving(fv.gear_comp_left, s.prev_gear_left) as i32
                             + gear_is_moving(fv.gear_comp_right, s.prev_gear_right) as i32;

            if moving_count > 0 {
                let multiplier = match moving_count {
                    3 => 1.0,
                    2 => 0.75,
                    1 => 0.5,
                    _ => 0.0,
                };

                let beat_duration = 60.0 / 80.0;
                let current_beat = fv.sim_time_s / beat_duration;
                let beat_phase = current_beat.fract();
                let beat_index = (current_beat.floor() as i64) % 3;

                if beat_index == 0 {
                    if beat_phase < 0.35 { gear_transit_term += 40.0 * multiplier; }
                } else {
                    if beat_phase < 0.15 { gear_transit_term += 15.0 * multiplier; }
                }
            }

            // Детекция финала уборки (все стойки в 0.0)
            let all_up_now = fv.gear_comp_nose <= 0.0 && fv.gear_comp_left <= 0.0 && fv.gear_comp_right <= 0.0;
            let not_all_up_prev = s.prev_gear_nose > 0.0 || s.prev_gear_left > 0.0 || s.prev_gear_right > 0.0;

            // Детекция финала выпуска (все стойки в 50.0)
            let all_down_now = fv.gear_comp_nose >= 49.9 && fv.gear_comp_left >= 49.9 && fv.gear_comp_right >= 49.9;
            let not_all_down_prev = s.prev_gear_nose < 49.9 || s.prev_gear_left < 49.9 || s.prev_gear_right < 49.9;

            // Если сработал любой триггер (последняя стойка встала на замок)
            if (all_up_now && not_all_up_prev) || (all_down_now && not_all_down_prev) {
                s.gear_doors_closed_t0 = fv.sim_time_s;
            }
        }

        // Обновляем состояния для следующего кадра (независимо от чекбокса,
        // иначе при включении эффекта в середине движения сработает ложный триггер)
        s.prev_gear_nose = fv.gear_comp_nose;
        s.prev_gear_left = fv.gear_comp_left;
        s.prev_gear_right = fv.gear_comp_right;

        if dt_.gear_transit.enable_joystick { transients_j += gear_transit_term; }
        if dt_.gear_transit.enable_throttle { transients_t += gear_transit_term; }
        // ----------------------------------------------

        // =========================================================================
        // БЛОК ЗАПУСКА ДВИГАТЕЛЯ (ENGINE SPOOL-UP & IGNITION) — 3-стадийная модель
        // =========================================================================
        // СТАДИЯ 1 (Pre-Combustion Spool-up): базовая амплитуда вибрации по N2
        // для каждого двигателя — гладкая треугольная огибающая по шкале 0..255
        // (дэдзона N2<1.0 → 0, минимум 1 при N2>=1.0, пик 255 у 20% N2, спад к
        // ~0 у 60% N2/Idle). Маршрутизация: ПОКА combustion == false у данного
        // двигателя — его N2-вибрация идёт ТОЛЬКО на свой борт:
        //   • Двигатель 1 (левый)  → ТОЛЬКО РУД (Throttle)
        //   • Двигатель 2 (правый) → ТОЛЬКО джойстик (Joystick)
        //
        // СТАДИЯ 2 (Combustion Kick, глобальный): в момент перехода
        // GENERAL ENG COMBUSTION false → true у ЛЮБОГО из двигателей — 500-мс
        // удар максимальной силы (таймер на реальном std::time::Instant),
        // ОДНОВРЕМЕННО перекрывающий ОБА канала (РУД + джойстик).
        //
        // СТАДИЯ 3 (Post-Combustion N2 Continuation, глобальный): после
        // истечения 500 мс удара оверрайд снимается. Если двигатель уже
        // работает (combustion == true), его N2-вибрация (из Стадии 1)
        // применяется теперь к ОБОИМ каналам одновременно — работающий
        // двигатель трясёт всю кабину, а не только свой борт.
        // Порог Idle N2 теперь настраивается пользователем (config.engine_idle_n2),
        // поскольку разные самолёты выходят на Idle при разных значениях N2.
        let engine_spool_n2_max: f64 = (cfg.engine_idle_n2 as f64).max(ENGINE_SPOOL_DEADZONE_N2 + 0.001);
        let engine_spool_peak_n2: f64 = engine_spool_n2_max / 3.0;
        const ENGINE_SPOOL_DEADZONE_N2: f64 = 1.0; // N2 < 1.0 — двигатель считается выключенным (PWM = 0)
        const ENGINE_SPOOL_MIN_AMPLITUDE: f64 = 1.0; // минимум по ШИМ (0..255) при N2 >= 1.0
        const ENGINE_IGNITION_KICK_DURATION: Duration = Duration::from_millis(500);

        // Фиксируем момент воспламенения (передний фронт) для каждого двигателя —
        // на реальном wall-clock Instant::now(), как требуется для этого эффекта.
        let is_combusting_1 = fv.eng1_combustion > 0.5;
        if is_combusting_1 && !s.prev_eng1_combustion {
            s.eng1_kick_started_at = Some(Instant::now());
        }

        let is_combusting_2 = fv.eng2_combustion > 0.5;
        if is_combusting_2 && !s.prev_eng2_combustion {
            s.eng2_kick_started_at = Some(Instant::now());
        }

        // Обновляем предыдущее состояние зажигания для следующего кадра.
        s.prev_eng1_combustion = is_combusting_1;
        s.prev_eng2_combustion = is_combusting_2;

        // ШАГ 1: ВСЕГДА считаем базовую вибрацию раскрутки по N2 — независимо
        // от того, активен ли сейчас удар воспламенения. Именно поэтому после
        // истечения 500 мс переход происходит бесшовно, а не через провал в 0.
        // Дэдзона: N2 < 1.0 → 0 (двигатель выключен). При N2 в [1.0 .. engine_idle_n2):
        // минимум 1, линейный рост к пику 255 у engine_idle_n2/3, ПЛАВНЫЙ
        // НЕлинейный (smoothstep-ease) спад к строго 0 у engine_idle_n2 (Idle).
        // Раньше спад был линейным ("(max - n2) / (max - peak)"), из-за чего
        // вибрация ощущалась как резкий обрыв прямо на пороге Idle. Кубический
        // smoothstep имеет нулевую производную на обоих концах интервала спада,
        // поэтому затухание получается мягким и естественным, а не "срезанным".
        let engine_spool_term = |n2_percent: f64| -> f64 {
            if n2_percent < ENGINE_SPOOL_DEADZONE_N2 || n2_percent >= engine_spool_n2_max {
                // Дэдзона (двигатель выключен) ИЛИ N2 достиг/превысил Idle —
                // амплитуда строго 0.0 (п.1 и п.5 требований).
                0.0
            } else if n2_percent <= engine_spool_peak_n2 {
                // Плавный рост от минимума ШИМ=1 до пика 255 ровно на engine_spool_peak_n2
                // (peak_n2 = engine_idle_n2 / 3.0, п.2 требований).
                let rise_progress = (n2_percent / engine_spool_peak_n2).clamp(0.0, 1.0);
                (rise_progress * 255.0).max(ENGINE_SPOOL_MIN_AMPLITUDE)
            } else {
                // Non-linear ease-out спад: decay_progress идёт от 0 (у пика)
                // до 1 (точно у engine_idle_n2/Idle).
                let decay_progress = ((n2_percent - engine_spool_peak_n2)
                    / (engine_spool_n2_max - engine_spool_peak_n2))
                    .clamp(0.0, 1.0);
                // Кубический smoothstep: eased(0) = 0, eased(1) = 1, производная
                // на обоих концах равна нулю — мягкое, "дышащее" затухание.
                let eased = decay_progress * decay_progress * (3.0 - 2.0 * decay_progress);
                let amplitude_factor = (1.0 - eased).clamp(0.0, 1.0);
                // Без .max(ENGINE_SPOOL_MIN_AMPLITUDE) здесь: амплитуда должна
                // дойти РОВНО до 0.0 у engine_idle_n2, а не застрять на полу.
                (amplitude_factor * 255.0).clamp(0.0, 255.0)
            }
        };

        // Маршрутизация базовой раскрутки зависит от состояния combustion
        // каждого двигателя (СТАДИЯ 1 vs СТАДИЯ 3):
        //   combustion == false → только свой борт (Eng1→Throttle, Eng2→Joystick)
        //   combustion == true  → ОБА борта одновременно (двигатель уже работает)
        let mut throttle_eng_vib: f64 = 0.0;
        let mut joystick_eng_vib: f64 = 0.0;

        if cfg.enable_engine_start {
            let eng1_n2_term = engine_spool_term(fv.eng1_n2_percent);
            let eng2_n2_term = engine_spool_term(fv.eng2_n2_percent);

            // Двигатель 1 (левый)
            if is_combusting_1 {
                // Работающий двигатель трясёт всю кабину — оба канала.
                throttle_eng_vib += eng1_n2_term;
                joystick_eng_vib += eng1_n2_term;
            } else {
                // Ещё не воспламенился — только свой борт (РУД).
                throttle_eng_vib += eng1_n2_term;
            }

            // Двигатель 2 (правый)
            if is_combusting_2 {
                throttle_eng_vib += eng2_n2_term;
                joystick_eng_vib += eng2_n2_term;
            } else {
                // Ещё не воспламенился — только свой борт (джойстик).
                joystick_eng_vib += eng2_n2_term;
            }
        }

        // ШАГ 2: Проверяем активность 500-мс удара воспламенения по реальному
        // Instant::now().duration_since(t0) — таймер не зависит от sim_time_s.
        let eng1_kick_active = s
            .eng1_kick_started_at
            .map(|t0| t0.elapsed() < ENGINE_IGNITION_KICK_DURATION)
            .unwrap_or(false);
        let eng2_kick_active = s
            .eng2_kick_started_at
            .map(|t0| t0.elapsed() < ENGINE_IGNITION_KICK_DURATION)
            .unwrap_or(false);
        // Удар — глобальный эффект: срабатывает от ЛЮБОГО из двигателей и
        // применяется одновременно к обоим каналам (см. оверрайд ниже).
        let combustion_kick_active = eng1_kick_active || eng2_kick_active;

        effects.engine_start_active = cfg.enable_engine_start
            && (throttle_eng_vib.abs() > 0.5 || joystick_eng_vib.abs() > 0.5 || combustion_kick_active);

        // Подмешиваем базовую раскрутку в соответствующие каналы через transients
        // (чтобы не гаситься экспоненциальным сглаживанием air_term ниже). Пока
        // удар воспламенения активен, ниже (после clamp) он ПОЛНОСТЬЮ перекроет
        // итоговое значение максимальной силой; как только 500 мс истекут —
        // оверрайд снимается и итог автоматически возвращается к этой базе.
        transients_t += throttle_eng_vib;
        transients_j += joystick_eng_vib;
        // ----------------------------------------------

        // ВАЖНО: ground_term (толчки от стыков плит) НЕ должен идти через
        // bg_smoothed ниже — экспоненциальное сглаживание с маленьким alpha
        // размазывает короткий резкий импульс по времени и гасит его пик,
        // из-за чего вместо чётких толчков ощущается смазанная вибрация.
        // Сглаживание оставляем только для air_term (Overspeed — он должен
        // быть плавным фоном), а ground_term подмешиваем напрямую в total.
        // Сглаживание (только для air_term/Overspeed) считаем отдельно на
        // каждый канал (джойстик/РУД), т.к. у них может быть разный набор
        // включённых эффектов и, соответственно, разное "фоновое" значение.
        if cfg_rev != s.last_cfg_rev {
            s.bg_smoothed = air_term_j;
            s.bg_smoothed_throttle = air_term_t;
            s.last_cfg_rev = cfg_rev;
        } else {
            let alpha = cfg.smoothing_alpha.clamp(0.0, 1.0) as f64;
            s.bg_smoothed += alpha * (air_term_j - s.bg_smoothed);
            s.bg_smoothed_throttle += alpha * (air_term_t - s.bg_smoothed_throttle);
        }

        // Подмешиваем вибрацию закрылков в transients (чтобы она не сглаживалась)
        if dt_.flaps.enable_joystick { transients_j += flaps_term; }
        if dt_.flaps.enable_throttle { transients_t += flaps_term; }

        let mut total_j = s.bg_smoothed + ground_term_j + transients_j + bank_term_j + spoilers_term_j;
        let mut total_t = s.bg_smoothed_throttle + ground_term_t + transients_t + bank_term_t + spoilers_term_t;

        if cfg.stall_enabled && fv.stalled {
            let ceiling = cfg.stall_ceiling as f64;
            if dt_.stall.enable_joystick { total_j = total_j.max(ceiling); }
            if dt_.stall.enable_throttle { total_t = total_t.max(ceiling); }
        }

        // 1. Применяем лимиты из ползунков программы для обычных эффектов
        let mut final_joystick = total_j.clamp(0.0, cfg.max_output as f64);
        let mut final_throttle = total_t.clamp(0.0, cfg.max_output as f64);

        // 2. АБСОЛЮТНЫЙ ОВЕРРАЙД: Удар створок шасси (длительность 1 секунда, макс 255)
        // Выполняется ПОСЛЕ clamp, поэтому пробивает любые ограничения конфигурации.
        // Маршрутизируется по тому же тумблеру, что и "Gear Transit & Doors" в UI.
        if s.gear_doors_closed_t0 > 0.0 && fv.sim_time_s <= s.gear_doors_closed_t0 + 1.0 {
            let p = (fv.sim_time_s - s.gear_doors_closed_t0) / 1.0;
            // Удар силой 255 с квадратичным затуханием
            let slam = 255.0 * (1.0 - p).powi(2);
            if dt_.gear_transit.enable_joystick { final_joystick = final_joystick.max(slam); }
            if dt_.gear_transit.enable_throttle { final_throttle = final_throttle.max(slam); }
        }

        // 3. АБСОЛЮТНЫЙ ОВЕРРАЙД: "Удар воспламенения" двигателя (500 мс, макс. сила).
        // Глобальный эффект — если воспламенился ЛЮБОЙ из двигателей (Eng1 ИЛИ Eng2),
        // перекрывает ОБА канала (РУД + джойстик) ОДНОВРЕМЕННО, независимо от
        // раздельной маршрутизации раскрутки выше.
        if cfg.enable_engine_start && combustion_kick_active {
            let strength = (cfg.engine_start_strength as f64).clamp(0.0, 255.0);
            final_joystick = strength;
            final_throttle = strength;
        }

        RumbleOutput {
            joystick_intensity: final_joystick.clamp(0.0, 255.0).round() as u8,
            throttle_intensity: final_throttle.clamp(0.0, 255.0).round() as u8,
            effects,
        }
    } // Конец метода step
} // Конец impl RumbleEngine