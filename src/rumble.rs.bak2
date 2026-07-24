use crate::{EffectsSnapshot, FlightVars, RumbleConfig};

#[derive(Debug, Clone, Copy, Default)]
pub struct RumbleState {
    prev_gear: f64,
    gear_t0: f64,
    gear_t1: f64,
    gear_peak: f64,
    bg_smoothed: f64,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RumbleOutput {
    pub intensity: u8,
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
            return RumbleOutput { intensity: 0, effects };
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
            let ramp_up_time_s = 2.0; // Время раскрутки ~2 секунды
            let step_up = max_amplitude * (dt_clamped / ramp_up_time_s);

            // Плавно прибавляем силу
            s.current_flaps_amplitude = (s.current_flaps_amplitude + step_up).min(max_amplitude);
        } else {
            // Плавно глушим мотор при остановке
            let ramp_down_time_s = 1.0; // Время затухания ~1 секунда (быстрее, чем раскрутка)
            let step_down = max_amplitude * (dt_clamped / ramp_down_time_s);

            // Плавно убавляем силу до нуля
            s.current_flaps_amplitude = (s.current_flaps_amplitude - step_down).max(0.0);
        }

        // 2. Применяем эффект, если амплитуда больше минимального порога
        // Так как RumbleOutput поддерживает только одну общую интенсивность (intensity: u8),
        // а s.current_flaps_amplitude — это duty cycle (0.0 .. 0.8),
        // мы должны сами сформировать вибрацию (программный ШИМ) на частоте 25 Гц.
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

        let mut ground_term: f64 = 0.0;
        let mut air_term: f64 = 0.0;
        let mut transients: f64 = 0.0;
        let mut bank_term: f64 = 0.0;
        let mut spoilers_term: f64 = 0.0;

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
                let raw_term = thump_amplitude * amplitude_curve;
                ground_term = raw_term.clamp(GROUND_THUMP_PEAK_MIN, GROUND_THUMP_PEAK_MAX);
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
                air_term += intensity * (0.7 + 0.3 * oscillation);
                effects.overspeed_active = true;
            }
        }

        if cfg.bank_enabled && !fv.on_ground {
            let bank_abs = fv.bank_deg.abs();
            if bank_abs > bank_threshold_deg {
                let raw_norm = ((bank_abs - bank_threshold_deg) / (90.0 - bank_threshold_deg)).clamp(0.0, 1.0);
                if (fv.sim_time_s % 0.15) < (0.15 * raw_norm) {
                    bank_term = cfg.bank_intensity as f64;
                }
            }
        }

        if spoilers_active {
            let min_pct = cfg.spoilers_threshold_pct;
            let defl_norm = ((fv.spoilers_pct - min_pct) / (100.0 - min_pct)).clamp(0.0, 1.0);
            let base_spoilers_intensity = 1.0 + defl_norm * ((cfg.spoilers_intensity as f64) - 1.0);
            let speed_factor = (fv.airspeed_indicated / 300.0).clamp(0.0, 1.2);
            let oscillation = (2.0 * std::f64::consts::PI * 25.0 * fv.sim_time_s).sin() * 0.4 + 0.6;
            spoilers_term = base_spoilers_intensity * speed_factor * oscillation;
        }

        if cfg.stall_enabled && fv.stalled {
            transients = transients.max(cfg.stall_ceiling as f64);
        }

        if cfg.gear_enabled {
            let gear_active = fv.sim_time_s >= s.gear_t0 && fv.sim_time_s <= s.gear_t1 && s.gear_peak > 0.0;
            if gear_active {
                let p = ((fv.sim_time_s - s.gear_t0) / (s.gear_t1 - s.gear_t0)).clamp(0.0, 1.0);
                transients += s.gear_peak * (std::f64::consts::PI * p).sin();
            }
            effects.gear_bump_active = gear_active;
        }

        if cfg.gear_comp_enabled {
            let nose_active = cfg.gear_comp_nose_enabled && fv.sim_time_s >= s.gear_comp_nose_t0 && fv.sim_time_s <= s.gear_comp_nose_t0 + GEAR_COMP_BUMP_DURATION;
            if nose_active {
                let p = ((fv.sim_time_s - s.gear_comp_nose_t0) / GEAR_COMP_BUMP_DURATION).clamp(0.0, 1.0);
                transients += s.gear_comp_nose_dyn_peak * (1.0 - p).powi(3);
            }
            effects.gear_comp_nose_active = nose_active;

            let left_active = cfg.gear_comp_left_enabled && fv.sim_time_s >= s.gear_comp_left_t0 && fv.sim_time_s <= s.gear_comp_left_t0 + GEAR_COMP_BUMP_DURATION;
            if left_active {
                let p = ((fv.sim_time_s - s.gear_comp_left_t0) / GEAR_COMP_BUMP_DURATION).clamp(0.0, 1.0);
                transients += s.gear_comp_left_dyn_peak * (1.0 - p).powi(3);
            }
            effects.gear_comp_left_active = left_active;

            let right_active = cfg.gear_comp_right_enabled && fv.sim_time_s >= s.gear_comp_right_t0 && fv.sim_time_s <= s.gear_comp_right_t0 + GEAR_COMP_BUMP_DURATION;
            if right_active {
                let p = ((fv.sim_time_s - s.gear_comp_right_t0) / GEAR_COMP_BUMP_DURATION).clamp(0.0, 1.0);
                transients += s.gear_comp_right_dyn_peak * (1.0 - p).powi(3);
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

        transients += gear_transit_term;
        // ----------------------------------------------

        // ВАЖНО: ground_term (толчки от стыков плит) НЕ должен идти через
        // bg_smoothed ниже — экспоненциальное сглаживание с маленьким alpha
        // размазывает короткий резкий импульс по времени и гасит его пик,
        // из-за чего вместо чётких толчков ощущается смазанная вибрация.
        // Сглаживание оставляем только для air_term (Overspeed — он должен
        // быть плавным фоном), а ground_term подмешиваем напрямую в total.
        let bg = air_term;
        if cfg_rev != s.last_cfg_rev {
            s.bg_smoothed = bg;
            s.last_cfg_rev = cfg_rev;
        } else {
            s.bg_smoothed += (cfg.smoothing_alpha.clamp(0.0, 1.0) as f64) * (bg - s.bg_smoothed);
        }

        // Подмешиваем вибрацию закрылков в transients (чтобы она не сглаживалась)
        transients += flaps_term;

        let mut total = s.bg_smoothed + ground_term + transients + bank_term + spoilers_term;
        if cfg.stall_enabled && fv.stalled {
            total = total.max(cfg.stall_ceiling as f64);
        }

        // 1. Применяем лимиты из ползунков программы для обычных эффектов
        let mut final_intensity = total.clamp(0.0, cfg.max_output as f64);

        // 2. АБСОЛЮТНЫЙ ОВЕРРАЙД: Удар створок шасси (длительность 1 секунда, макс 255)
        // Выполняется ПОСЛЕ clamp, поэтому пробивает любые ограничения конфигурации
        if s.gear_doors_closed_t0 > 0.0 && fv.sim_time_s <= s.gear_doors_closed_t0 + 1.0 {
            let p = (fv.sim_time_s - s.gear_doors_closed_t0) / 1.0;
            // Удар силой 255 с квадратичным затуханием
            let slam = 255.0 * (1.0 - p).powi(2);
            final_intensity = final_intensity.max(slam);
        }

        RumbleOutput {
            intensity: final_intensity.clamp(0.0, 255.0).round() as u8,
            effects,
        }
    } // Конец метода step
} // Конец impl RumbleEngine