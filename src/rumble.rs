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
    // Touchdown fade-in tracking: момент касания земли (переход airborne -> on_ground),
    // используется, чтобы Ground Roll плавно нарастал и не маскировал резкий
    // удар обжатия стоек в момент касания.
    prev_on_ground: bool,
    touchdown_time_s: f64,
    // Universal Engine Start tracking (Starter + Combustion boolean model,
    // одинаково работает на поршневых и турбинных двигателях).
    // Каждый двигатель отслеживается НЕЗАВИСИМО (свой prev_combusting и свой
    // combustion_timer) — это принципиально важно для 4-моторного режима:
    // если групповой edge-триггер завязать на "сторону" целиком, то у
    // ВТОРОГО двигателя пары воспламенение вообще не будет замечено, т.к.
    // "сторона" уже считается воспламенившейся из-за первого двигателя.
    // Комбинирование по сторонам (left = Eng1[, Eng2], right = Eng2/Eng3[, Eng4])
    // происходит уже ПОСЛЕ вычисления каждого двигателя отдельно, см. step().
    prev_combusting: [bool; 4],
    // Таймер импульса воспламенения на каждый двигатель — считается по
    // fv.sim_time_s (НЕ Instant::now()), единый источник времени со всем
    // остальным движком. Обнуляется на переднем фронте GENERAL ENG COMBUSTION
    // (false → true) и растёт, пока двигатель горит, до потолка в 5.0 с.
    combustion_timer: [f64; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RumbleOutput {
    pub joystick_intensity: u8,
    pub throttle_left_intensity: u8,
    pub throttle_right_intensity: u8,
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
                touchdown_time_s: -1000.0,
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
                throttle_left_intensity: 0,
                throttle_right_intensity: 0,
                effects,
            };
        }

        let s = &mut self.state;
        let mut dt = fv.sim_time_s - s.prev_sim_time_s;
        if s.prev_sim_time_s < 0.0 {
            dt = 0.0;
        }

        // Touchdown fade-in tracking: фиксируем момент перехода airborne -> on_ground,
        // чтобы Ground Roll (гул рулёжки/пробега) мог плавно нарастать после этого
        // момента и не маскировал резкий импульс обжатия стоек при касании.
        if fv.on_ground && !s.prev_on_ground {
            s.touchdown_time_s = fv.sim_time_s;
        }
        s.prev_on_ground = fv.on_ground;

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
        // flaps_term_j — немодифицированный термин для джойстика (не делится на лево/право).
        // flaps_term_t_left/right — термин для РУД, чередующийся между каналами каждые 500 мс.
        let mut flaps_term_j: f64 = 0.0;
        let mut flaps_term_t_left: f64 = 0.0;
        let mut flaps_term_t_right: f64 = 0.0;
        if cfg.flaps_enabled && s.current_flaps_amplitude > 0.01 {
            let fixed_period = 0.04; // 0.04 с = 25 Гц
            let cycle = (fv.sim_time_s / fixed_period).fract();

            // Создаем пульсацию (от 0.0 до 1.0) в виде полуволн синуса
            let oscillation = (std::f64::consts::PI * cycle).sin();

            // Преобразуем duty cycle в силу вибрации (0 .. 255)
            let flaps_term = s.current_flaps_amplitude * 255.0 * oscillation;
            flaps_term_j = flaps_term;

            // Чередуем канал (лево/право) каждые 500 мс.
            // Примечание: dt_.flaps.enable_throttle (маршрутизация на РУД) проверяется
            // позже, в месте подмешивания flaps_term_t_left/right в transients_t_left/right,
            // т.к. cfg.device_targets (dt_) ещё не определён на этом этапе step().
            let is_left_phase = (fv.sim_time_s / 0.5).floor() as i64 % 2 == 0;

            if is_left_phase {
                flaps_term_t_left = flaps_term;
            } else {
                flaps_term_t_right = flaps_term;
            }

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
            let amplitude_curve = 0.3 + 0.7 * speed_progress.powf(1.4);

            // 4. Логика перезапуска цикла импульса (стык плиты позади — ждём следующий).
            let time_since_last_thump = fv.sim_time_s - s.thump_last_time_s;
            if time_since_last_thump >= target_period_s {
                s.thump_last_time_s = fv.sim_time_s;
            }
            let time_since_last_thump = fv.sim_time_s - s.thump_last_time_s;

            // 5. Окно удара. Если период короче длительности импульса — удары сливаются
            // в сплошной гул (актуально на высоких скоростях рулёжки/разбега).
            if time_since_last_thump < thump_duration_s || target_period_s <= thump_duration_s {
                // Fade-in после касания: 0.0 сразу в момент touchdown -> 1.0 через 750мс.
                // Это "разводит" по времени резкий импульс обжатия стоек (сразу, полная
                // сила) и фоновый гул стыков плит (нарастает плавно), чтобы они не
                // маскировали друг друга тактильно в момент посадки.
                let time_since_touchdown = fv.sim_time_s - s.touchdown_time_s;
                let touchdown_fade_in = (time_since_touchdown / 0.75).clamp(0.0, 1.0);

                let raw_term = (thump_amplitude * amplitude_curve * touchdown_fade_in)
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
                if cfg.split_touchdown {
                    // SPLIT: левая основная стойка — эксклюзивно на РУД (левая рука),
                    // независимо от чекбоксов dt_.gear_comp_left.
                    transients_t += term;
                } else {
                    if dt_.gear_comp_left.enable_joystick { transients_j += term; }
                    if dt_.gear_comp_left.enable_throttle { transients_t += term; }
                }
            }
            effects.gear_comp_left_active = left_active;

            let right_active = cfg.gear_comp_right_enabled && fv.sim_time_s >= s.gear_comp_right_t0 && fv.sim_time_s <= s.gear_comp_right_t0 + GEAR_COMP_BUMP_DURATION;
            if right_active {
                let p = ((fv.sim_time_s - s.gear_comp_right_t0) / GEAR_COMP_BUMP_DURATION).clamp(0.0, 1.0);
                let term = s.gear_comp_right_dyn_peak * (1.0 - p).powi(3);
                if cfg.split_touchdown {
                    // SPLIT: правая основная стойка — эксклюзивно на джойстик (правая рука),
                    // независимо от чекбоксов dt_.gear_comp_right.
                    transients_j += term;
                } else {
                    if dt_.gear_comp_right.enable_joystick { transients_j += term; }
                    if dt_.gear_comp_right.enable_throttle { transients_t += term; }
                }
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

            // Индикатор активности в UI: горит пока стойки физически движутся
            // ИЛИ пока идёт 1-секундный удар фиксации на замке (см. оверрайд
            // final_throttle/final_joystick ниже, использующий то же окно).
            let slam_active =
                s.gear_doors_closed_t0 > 0.0 && fv.sim_time_s <= s.gear_doors_closed_t0 + 1.0;
            effects.gear_transit_active = moving_count > 0 || slam_active;
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
        // БЛОК ЗАПУСКА ДВИГАТЕЛЯ (UNIVERSAL ENGINE START) — модель Starter + Combustion
        // =========================================================================
        // Универсальная модель, одинаково работающая на поршневых и турбинных
        // двигателях: вместо N2 (которое на поршневых обычно недоступно) она
        // опирается ТОЛЬКО на два булевых флага SimConnect:
        //   • GENERAL ENG STARTER:N     — стартер крутит двигатель N
        //   • GENERAL ENG COMBUSTION:N  — двигатель N воспламенился/работает
        //
        // СТАРТЕР-ФАЗА (starter == true И combustion == false): вибрация
        // прокрутки стартером, маршрутизируется СТРОГО на свой борт (см. ниже
        // Left/Right routing по cfg.four_engine_mode).
        //
        // ФАЗА ВОСПЛАМЕНЕНИЯ (combustion == true): импульс воспламенения
        // максимальной силы, ОДНОВРЕМЕННО на ОБА борта, ограниченный по
        // времени combustion_timer[N] (растёт по fv.sim_time_s, не более 5.0 с
        // на двигатель, затем эффект по этому двигателю истекает).
        //
        // РОУТИНГ ПО БОРТАМ (cfg.four_engine_mode, соответствует чекбоксу
        // "4 двигателя" в UI):
        //   • 4-моторный режим:  Left = Eng1 ∪ Eng2 (левый РУД),
        //                        Right = Eng3 ∪ Eng4 (правый РУД + джойстик)
        //   • 2-моторный режим:  Left = Eng1 (левый РУД),
        //                        Right = Eng2 (правый РУД + джойстик),
        //                        Eng3/Eng4 полностью игнорируются.
        // Агрегация внутри борта — max() по вовлечённым двигателям.
        const COMBUSTION_KICK_MAX_S: f64 = 5.0; // потолок combustion_timer на один запуск

        let starters = [fv.eng1_starter, fv.eng2_starter, fv.eng3_starter, fv.eng4_starter];
        let combusting = [
            fv.eng1_combustion > 0.5,
            fv.eng2_combustion > 0.5,
            fv.eng3_combustion > 0.5,
            fv.eng4_combustion > 0.5,
        ];

        let engine_count: usize = if cfg.four_engine_mode { 4 } else { 2 };

        let mut starter_term = [0.0f64; 4];
        let mut combustion_active = [false; 4];

        for i in 0..engine_count {
            let is_combusting = combusting[i];

            // Передний фронт воспламенения (false → true) — обнуляем таймер
            // этого конкретного двигателя. Каждый двигатель отслеживается
            // независимо (см. комментарий у RumbleState::prev_combusting),
            // иначе у второго двигателя пары в 4-моторном режиме таймер не
            // перезапустится, если "сторона" уже считается воспламенившейся.
            if is_combusting && !s.prev_combusting[i] {
                s.combustion_timer[i] = 0.0;
            }
            if is_combusting {
                // Таймер растёт по симуляционному времени (dt_clamped), а не
                // по Instant::now() — единый источник времени со всем
                // остальным движком. Потолок 5.0 с на один запуск.
                s.combustion_timer[i] = (s.combustion_timer[i] + dt_clamped).min(COMBUSTION_KICK_MAX_S);
            } else {
                // Двигатель заглох/ещё не запущен — таймер сброшен, чтобы
                // следующий передний фронт снова начинал с нуля.
                s.combustion_timer[i] = 0.0;
            }
            s.prev_combusting[i] = is_combusting;

            combustion_active[i] = is_combusting && s.combustion_timer[i] < COMBUSTION_KICK_MAX_S;

            // Стартер-фаза: стартер крутит, но воспламенения ещё не было.
            if starters[i] && !is_combusting {
                starter_term[i] = (cfg.engine_start_strength as f64).clamp(0.0, 255.0);
            }
        }
        // Игнорируемые в 2-моторном режиме Eng3/Eng4 не должны копить состояние
        // "по инерции" — если четырёхмоторный режим включат позже, они обязаны
        // начинать с чистого состояния.
        for i in engine_count..4 {
            s.combustion_timer[i] = 0.0;
            s.prev_combusting[i] = false;
        }

        let (left_engines, right_engines): (&[usize], &[usize]) = if cfg.four_engine_mode {
            (&[0, 1], &[2, 3])
        } else {
            (&[0], &[1])
        };

        // Left → ТОЛЬКО Left Throttle. Right → Right Throttle И Joystick.
        let left_starter_term = left_engines
            .iter()
            .fold(0.0f64, |m, &i| m.max(starter_term[i]));
        let right_starter_term = right_engines
            .iter()
            .fold(0.0f64, |m, &i| m.max(starter_term[i]));

        // Импульс воспламенения — глобальный эффект: срабатывает, если ХОТЬ
        // ОДИН из активных (в текущем режиме) двигателей ещё в пределах своих
        // 5.0 с после воспламенения, и одновременно перекрывает ОБА борта.
        let combustion_kick_active = (0..engine_count).any(|i| combustion_active[i]);

        effects.engine_start_active = cfg.enable_engine_start
            && (left_starter_term > 0.5 || right_starter_term > 0.5 || combustion_kick_active);

        let mut throttle_eng_vib_left: f64 = 0.0; // → ТОЛЬКО throttle_left
        let mut throttle_eng_vib_right: f64 = 0.0; // → throttle_right И joystick

        if cfg.enable_engine_start {
            throttle_eng_vib_left = left_starter_term;
            throttle_eng_vib_right = right_starter_term;
        }
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

        // Подмешиваем вибрацию закрылков в transients (чтобы она не сглаживалась).
        // РУД (throttle) разветвляется на левый/правый канал: обе копии стартуют
        // от общего transients_t, накопленного выше, а затем в каждую подмешивается
        // только "свой" flaps_term_t_left/right (чередование каждые 500 мс).
        if dt_.flaps.enable_joystick { transients_j += flaps_term_j; }

        let mut transients_t_left = transients_t;
        let mut transients_t_right = transients_t;
        if dt_.flaps.enable_throttle {
            transients_t_left += flaps_term_t_left;
            transients_t_right += flaps_term_t_right;
        }

        // Universal Engine Start (Starter phase): левый борт → ТОЛЬКО throttle_left,
        // правый борт → throttle_right И joystick одновременно (см. блок выше).
        transients_t_left += throttle_eng_vib_left;
        transients_t_right += throttle_eng_vib_right;
        transients_j += throttle_eng_vib_right;

        let mut total_j = s.bg_smoothed + ground_term_j + transients_j + bank_term_j + spoilers_term_j;
        let mut total_t_left = s.bg_smoothed_throttle + ground_term_t + transients_t_left + bank_term_t + spoilers_term_t;
        let mut total_t_right = s.bg_smoothed_throttle + ground_term_t + transients_t_right + bank_term_t + spoilers_term_t;

        if cfg.stall_enabled && fv.stalled {
            let ceiling = cfg.stall_ceiling as f64;
            if dt_.stall.enable_joystick { total_j = total_j.max(ceiling); }
            if dt_.stall.enable_throttle {
                total_t_left = total_t_left.max(ceiling);
                total_t_right = total_t_right.max(ceiling);
            }
        }

        // 1. Применяем лимиты из ползунков программы для обычных эффектов
        let mut final_joystick = total_j.clamp(0.0, cfg.max_output as f64);
        let mut final_throttle_left = total_t_left.clamp(0.0, cfg.max_output as f64);
        let mut final_throttle_right = total_t_right.clamp(0.0, cfg.max_output as f64);

        // 2. АБСОЛЮТНЫЙ ОВЕРРАЙД: Удар створок шасси (длительность 1 секунда, макс 255)
        // Выполняется ПОСЛЕ clamp, поэтому пробивает любые ограничения конфигурации.
        // Маршрутизируется по тому же тумблеру, что и "Gear Transit & Doors" в UI.
        if s.gear_doors_closed_t0 > 0.0 && fv.sim_time_s <= s.gear_doors_closed_t0 + 1.0 {
            let p = (fv.sim_time_s - s.gear_doors_closed_t0) / 1.0;
            // Удар силой 255 с квадратичным затуханием
            let slam = 255.0 * (1.0 - p).powi(2);
            if dt_.gear_transit.enable_joystick { final_joystick = final_joystick.max(slam); }
            if dt_.gear_transit.enable_throttle {
                final_throttle_left = final_throttle_left.max(slam);
                final_throttle_right = final_throttle_right.max(slam);
            }
        }

        // 3. АБСОЛЮТНЫЙ ОВЕРРАЙД: импульс воспламенения (Universal Engine Start).
        // Глобальный эффект — если воспламенился ЛЮБОЙ из активных в текущем
        // режиме двигателей и его combustion_timer ещё не дошёл до потолка в
        // 5.0 с, — перекрывает ВСЕ три канала (throttle_left, throttle_right,
        // joystick) ОДНОВРЕМЕННО, независимо от раздельной маршрутизации
        // стартер-фазы выше. По истечении 5.0 с эффект по этому двигателю
        // истекает сам собой (см. блок выше).
        if cfg.enable_engine_start && combustion_kick_active {
            let strength = (cfg.engine_start_strength as f64).clamp(0.0, 255.0);
            final_joystick = strength;
            final_throttle_left = strength;
            final_throttle_right = strength;
        }

        RumbleOutput {
            joystick_intensity: final_joystick.clamp(0.0, 255.0).round() as u8,
            throttle_left_intensity: final_throttle_left.clamp(0.0, 255.0).round() as u8,
            throttle_right_intensity: final_throttle_right.clamp(0.0, 255.0).round() as u8,
            effects,
        }
    } // Конец метода step
} // Конец impl RumbleEngine