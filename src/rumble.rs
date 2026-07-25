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
    current_flaps_amplitude: f64,
    // Последнее значение flaps_pct, ОКРУГЛЁННОЕ до целого процента — источник
    // истины для детекции движения (см. комментарий у flaps_pct_rounded в
    // step()). i32::MIN — сентинел "ещё не видели ни одного кадра", чтобы
    // самый первый кадр не считался "изменением" от 0.
    last_flaps_pct_rounded: i32,
    // Аналогично last_flaps_pct_rounded, но для slats_pct — используется
    // только когда cfg.flaps_track_slats включён профилем самолёта.
    last_slats_pct_rounded: i32,
    flaps_active_until: f64,
    // Ground Roll (физическая модель удара о стыки плит) tracking
    thump_last_time_s: f64,
    // Touchdown fade-in tracking: момент касания земли (переход airborne -> on_ground),
    // используется, чтобы Ground Roll плавно нарастал и не маскировал резкий
    // удар обжатия стоек в момент касания.
    prev_on_ground: bool,
    touchdown_time_s: f64,
    // JET / TURBINE Engine Start tracking — ОРИГИНАЛЬНАЯ реализация,
    // намеренно не тронута (см. is_jet-ветку в step()). Таймер удара
    // воспламенения — реальный wall-clock Instant (не sim_time_s), как
    // изначально и было написано для этого эффекта.
    // Каждый двигатель отслеживается НЕЗАВИСИМО (свой prev_engN_combusting и
    // свой kick_started_at) — важно для 4-моторного режима: если групповой
    // edge-триггер завязать на "сторону" целиком, то у ВТОРОГО двигателя пары
    // удар вообще не сработает, т.к. "сторона" уже считается воспламенившейся
    // из-за первого двигателя.
    prev_eng1_combusting: bool,
    prev_eng2_combusting: bool,
    prev_eng3_combusting: bool,
    prev_eng4_combusting: bool,
    eng1_kick_started_at: Option<Instant>,
    eng2_kick_started_at: Option<Instant>,
    eng3_kick_started_at: Option<Instant>,
    eng4_kick_started_at: Option<Instant>,
    // PISTON Engine Start tracking (Starter + Combustion boolean model) —
    // используется ТОЛЬКО в ветке is_jet == false. Каждый двигатель
    // отслеживается независимо, как и в джет-модели выше.
    // combustion_timer считается по fv.sim_time_s (НЕ Instant::now()) —
    // единый источник времени со всем остальным движком. Обнуляется на
    // переднем фронте GENERAL ENG COMBUSTION (false → true) и растёт, пока
    // двигатель горит, до потолка в 5.0 с, после чего удар воспламенения
    // полностью затухает (fade-out).
    piston_prev_combusting: [bool; 4],
    piston_combustion_timer: [f64; 4],
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
                last_flaps_pct_rounded: i32::MIN,
                last_slats_pct_rounded: i32::MIN,
                current_flaps_amplitude: 0.0,
                flaps_active_until: -1.0,
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

        // Порог Overspeed теперь приходит динамически из SimConnect
        // (DESIGN SPEED VC) для текущего самолёта, а не из ручного слайдера.
        // 0.0 означает "SimConnect ещё не отдал значение" — в этом случае
        // эффект не должен срабатывать (иначе сработает от IAS >= 0).
        let overspeed_threshold_kn = fv.design_speed_vc_kn;
        let overspeed_threshold_known = overspeed_threshold_kn > 0.0;
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
        // Ограничиваем dt, чтобы при лагах/паузах симулятора не было резкого
        // скачка накопительных таймеров (например piston combustion timer ниже).
        let dt_clamped = dt.min(0.1);

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

        // 1. Проверяем, движутся ли физически закрылки.
        // FLAPS HANDLE INDEX оказался ненадёжным источником (на MADDOG он
        // дребезжит между соседними значениями даже стоя на месте, из-за чего
        // эффект не смолкал никогда — см. историю правок). Вместо него
        // сравниваем flaps_pct, ОКРУГЛЁННЫЙ до целого процента, с предыдущим
        // кадром. Округление само по себе гасит суб-процентный телеметрический
        // шум, а сравнение по проценту (не по индексу защёлки) одинаково ловит
        // и плавную анимацию, и борта, где FLAPS PERCENT прыгает целиком за
        // один тик (например MADDOG: 0 -> 27 без промежуточных значений).
        let flaps_pct_rounded = fv.flaps_pct.round() as i32;
        let pct_changed =
            s.last_flaps_pct_rounded != i32::MIN && s.last_flaps_pct_rounded != flaps_pct_rounded;

        // На некоторых бортах (см. cfg.flaps_track_slats, MADDOG — включается
        // автоматически встроенным профилем по aircraft title) предкрылки
        // убираются отдельным, последним движением ручки закрылков, когда
        // flaps_pct УЖЕ 0 — само это движение (реальная работа мотора) видно
        // только по смене slats_pct, поэтому дополнительно следим и за ним.
        let slats_pct_rounded = fv.slats_pct.round() as i32;
        let slats_changed = cfg.flaps_track_slats
            && s.last_slats_pct_rounded != i32::MIN
            && s.last_slats_pct_rounded != slats_pct_rounded;

        // Держим "мотор" включённым СТРОГО на время реального изменения
        // телеметрии + минимальный запас (cfg.flaps_bump_duration_s, по
        // умолчанию доли секунды), чтобы даже мгновенный скачок за один тик
        // (MADDOG: 0 -> 27 в один кадр) успел дать ощутимый щелчок. Раньше
        // здесь были ещё и 5-секундные плавные разгон/затухание мотора —
        // из-за них эффект звучал заметно дольше, чем реально менялось
        // значение. Убрали: включаем/выключаем практически мгновенно.
        if pct_changed || slats_changed {
            s.flaps_active_until = fv.sim_time_s + cfg.flaps_bump_duration_s.max(0.05);
        }
        s.last_flaps_pct_rounded = flaps_pct_rounded;
        s.last_slats_pct_rounded = slats_pct_rounded;

        let flaps_is_moving = fv.sim_time_s < s.flaps_active_until;

        // Целевая рабочая мощность (0.8 — это примерно 200 из 255)
        let max_amplitude = cfg.flaps_duty.clamp(0.01, 0.8);

        s.current_flaps_amplitude = if flaps_is_moving { max_amplitude } else { 0.0 };

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

        if cfg.overspeed_enabled && overspeed_threshold_known {
            if !fv.on_ground && fv.airspeed_indicated >= overspeed_threshold_kn {
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
                    // SPLIT: левая основная стойка — эксклюзивно на "руку РУД"
                    // (по умолчанию РУД, при cfg.swap_hand_layout — джойстик),
                    // независимо от чекбоксов dt_.gear_comp_left.
                    if cfg.swap_hand_layout { transients_j += term; } else { transients_t += term; }
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
                    // SPLIT: правая основная стойка — эксклюзивно на "руку джойстика"
                    // (по умолчанию джойстик, при cfg.swap_hand_layout — РУД),
                    // независимо от чекбоксов dt_.gear_comp_right.
                    if cfg.swap_hand_layout { transients_t += term; } else { transients_j += term; }
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
        // БЛОК ЗАПУСКА ДВИГАТЕЛЯ — раздельные модели JET/TURBINE и PISTON
        // =========================================================================
        // Определение типа двигателя: считаем самолёт турбинным, если ЛЮБОЙ из
        // первых двух двигателей показывает N2 > 1.0% (у поршневых этот канал
        // почти всегда 0/недоступен). Один и тот же самолёт не может внезапно
        // "переключиться" между типами в полёте, но проверка каждый кадр не
        // стоит ничего и не требует отдельного состояния.
        let is_jet = fv.eng1_n2_percent > 1.0 || fv.eng2_n2_percent > 1.0;

        let engine_count: usize = if cfg.four_engine_mode { 4 } else { 2 };
        let (left_engines, right_engines): (&[usize], &[usize]) = if cfg.four_engine_mode {
            (&[0, 1], &[2, 3])
        } else {
            (&[0], &[1])
        };

        // Общие для обеих веток выходные переменные. Left → ТОЛЬКО throttle_left.
        // Right → throttle_right И joystick (см. точку разветвления транзиентов
        // ниже и абсолютный оверрайд удара воспламенения, п.3).
        let mut throttle_eng_vib_left: f64 = 0.0;
        let mut throttle_eng_vib_right: f64 = 0.0;
        let combustion_kick_active: bool;
        let combustion_kick_strength: f64;

        if is_jet {
            // =====================================================================
            // JET / TURBINE — ОРИГИНАЛЬНАЯ 3-СТАДИЙНАЯ МОДЕЛЬ, БЕЗ ИЗМЕНЕНИЙ.
            // =====================================================================
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

            // Фиксируем момент воспламенения (передний фронт) для КАЖДОГО двигателя
            // НЕЗАВИСИМО — на реальном wall-clock Instant::now(), как требуется для
            // этого эффекта. Комбинирование по сторонам (Left/Right) происходит
            // ниже, уже после того как каждый двигатель обработан отдельно — это
            // важно, иначе у второго двигателя пары не будет своего edge-триггера
            // (см. комментарий у полей prev_engN_combusting в RumbleState).
            let is_combusting_eng1 = fv.eng1_combustion > 0.5;
            let is_combusting_eng2 = fv.eng2_combustion > 0.5;
            let is_combusting_eng3 = fv.eng3_combustion > 0.5;
            let is_combusting_eng4 = fv.eng4_combustion > 0.5;

            if is_combusting_eng1 && !s.prev_eng1_combusting {
                s.eng1_kick_started_at = Some(Instant::now());
            }
            if is_combusting_eng2 && !s.prev_eng2_combusting {
                s.eng2_kick_started_at = Some(Instant::now());
            }
            if is_combusting_eng3 && !s.prev_eng3_combusting {
                s.eng3_kick_started_at = Some(Instant::now());
            }
            if is_combusting_eng4 && !s.prev_eng4_combusting {
                s.eng4_kick_started_at = Some(Instant::now());
            }

            // Обновляем предыдущее состояние зажигания для следующего кадра.
            s.prev_eng1_combusting = is_combusting_eng1;
            s.prev_eng2_combusting = is_combusting_eng2;
            s.prev_eng3_combusting = is_combusting_eng3;
            s.prev_eng4_combusting = is_combusting_eng4;

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

            // Термин раскрутки КАЖДОГО двигателя считается отдельно, а уже ПОТОМ
            // берётся максимум по паре (если 4-моторный режим). Это принципиально:
            // если брать max(raw N2) ДО кривой раскрутки, то как только один
            // двигатель пары выходит на Idle (raw N2 становится больше, но term
            // уже 0.0 по дэдзоне), max() выбирает именно его "мёртвое" значение и
            // полностью маскирует реальную раскрутку второго, ещё запускающегося
            // двигателя. Максимум нужно брать по уже посчитанным АМПЛИТУДАМ.
            //
            // Универсальная поддержка поршневых двигателей: TURB ENG N2 на поршневых
            // самолётах обычно равно 0 (нет турбины), а GENERAL ENG PCT MAX RPM на
            // турбинах часто отсутствует/неточно. Берём максимум из двух источников —
            // на турбинах сработает N2, на поршневых сработает Pct Max RPM, и то и
            // другое одновременно ложно сработать не может (у самолёта либо одно,
            // либо другое реально ненулевое), поэтому max() здесь безопасен и не
            // подвержен той же проблеме маскировки, что и max() по паре двигателей.
            let eng1_effective = fv.eng1_n2_percent.max(fv.eng1_pct_max_rpm);
            let eng2_effective = fv.eng2_n2_percent.max(fv.eng2_pct_max_rpm);
            let eng3_effective = fv.eng3_n2_percent.max(fv.eng3_pct_max_rpm);
            let eng4_effective = fv.eng4_n2_percent.max(fv.eng4_pct_max_rpm);

            let eng1_term = engine_spool_term(eng1_effective);
            let eng2_term = engine_spool_term(eng2_effective);
            let eng3_term = engine_spool_term(eng3_effective);
            let eng4_term = engine_spool_term(eng4_effective);

            // ШАГ 2: Проверяем активность 500-мс удара воспламенения для КАЖДОГО
            // двигателя по реальному Instant::now().duration_since(t0) — таймер не
            // зависит от sim_time_s.
            let eng1_kick_active = s
                .eng1_kick_started_at
                .map(|t0| t0.elapsed() < ENGINE_IGNITION_KICK_DURATION)
                .unwrap_or(false);
            let eng2_kick_active = s
                .eng2_kick_started_at
                .map(|t0| t0.elapsed() < ENGINE_IGNITION_KICK_DURATION)
                .unwrap_or(false);
            let eng3_kick_active = s
                .eng3_kick_started_at
                .map(|t0| t0.elapsed() < ENGINE_IGNITION_KICK_DURATION)
                .unwrap_or(false);
            let eng4_kick_active = s
                .eng4_kick_started_at
                .map(|t0| t0.elapsed() < ENGINE_IGNITION_KICK_DURATION)
                .unwrap_or(false);

            // Комбинируем по сторонам ТОЛЬКО сейчас, когда каждый двигатель уже
            // посчитан независимо. В обычном режиме left = Eng1, right = Eng2
            // (без изменений). В 4-моторном режиме (cfg.four_engine_mode) left =
            // Eng1 и/или Eng2 (левое крыло), right = Eng3 и/или Eng4 (правое
            // крыло) — удар срабатывает от КАЖДОГО двигателя своей группы
            // независимо, а не только от первого воспламенившегося.
            let (is_combusting_left, is_combusting_right, left_n2_term, right_n2_term, left_kick_active, right_kick_active);
            if cfg.four_engine_mode {
                is_combusting_left = is_combusting_eng1 || is_combusting_eng2;
                is_combusting_right = is_combusting_eng3 || is_combusting_eng4;
                left_n2_term = eng1_term.max(eng2_term);
                right_n2_term = eng3_term.max(eng4_term);
                left_kick_active = eng1_kick_active || eng2_kick_active;
                right_kick_active = eng3_kick_active || eng4_kick_active;
            } else {
                is_combusting_left = is_combusting_eng1;
                is_combusting_right = is_combusting_eng2;
                left_n2_term = eng1_term;
                right_n2_term = eng2_term;
                left_kick_active = eng1_kick_active;
                right_kick_active = eng2_kick_active;
            }

            // Маршрутизация базовой раскрутки зависит от состояния combustion
            // каждой стороны (СТАДИЯ 1 vs СТАДИЯ 3):
            //   combustion == false → только свой борт (Left→Throttle, Right→Joystick)
            //   combustion == true  → ОБА борта одновременно (сторона уже работает)
            // В 4-моторном режиме "сторона" — это группа из двух двигателей (см. выше).
            let mut throttle_eng_vib: f64 = 0.0;
            let mut joystick_eng_vib: f64 = 0.0;

            // cfg.swap_hand_layout меняет местами, какая сторона (Eng1/left или
            // Eng2/right) считается "рукой РУД", а какая "рукой джойстика" —
            // см. комментарий у поля в RumbleConfig. По умолчанию (false)
            // сохраняется исходное поведение: Eng1 → РУД, Eng2 → джойстик.
            let (throttle_side_combusting, throttle_side_n2, joystick_side_combusting, joystick_side_n2) =
                if cfg.swap_hand_layout {
                    (is_combusting_right, right_n2_term, is_combusting_left, left_n2_term)
                } else {
                    (is_combusting_left, left_n2_term, is_combusting_right, right_n2_term)
                };

            if cfg.enable_engine_start {
                // Сторона "руки РУД"
                if throttle_side_combusting {
                    // Работающая сторона трясёт всю кабину — оба канала.
                    throttle_eng_vib += throttle_side_n2;
                    joystick_eng_vib += throttle_side_n2;
                } else {
                    // Ещё не воспламенилась — только свой борт (РУД).
                    throttle_eng_vib += throttle_side_n2;
                }

                // Сторона "руки джойстика"
                if joystick_side_combusting {
                    throttle_eng_vib += joystick_side_n2;
                    joystick_eng_vib += joystick_side_n2;
                } else {
                    // Ещё не воспламенилась — только свой борт (джойстик).
                    joystick_eng_vib += joystick_side_n2;
                }
            }

            // Удар — глобальный эффект: срабатывает от ЛЮБОГО двигателя (каждый
            // отслеживается независимо, см. выше) и применяется одновременно к
            // обоим каналам (см. оверрайд ниже).
            combustion_kick_active = left_kick_active || right_kick_active;
            combustion_kick_strength = (cfg.engine_start_strength as f64).clamp(0.0, 255.0);

            effects.engine_start_active = cfg.enable_engine_start
                && (throttle_eng_vib.abs() > 0.5 || joystick_eng_vib.abs() > 0.5 || combustion_kick_active);

            // Подмешиваем базовую раскрутку в соответствующие каналы через transients
            // (чтобы не гаситься экспоненциальным сглаживанием air_term ниже). Пока
            // удар воспламенения активен, ниже (после clamp) он ПОЛНОСТЬЮ перекроет
            // итоговое значение максимальной силой; как только 500 мс истекут —
            // оверрайд снимается и итог автоматически возвращается к этой базе.
            // ВАЖНО: transients_t — общий на throttle_left/right (обе копии
            // получают одно и то же значение через сплит ниже) — так исторически
            // и было устроено у джет-модели, роутинг по бортам её не касается.
            transients_t += throttle_eng_vib;
            transients_j += joystick_eng_vib;
        } else {
            // =====================================================================
            // PISTON — модель Starter + Combustion (ритмичная прокрутка + затухающий
            // удар воспламенения). Работает по булевым флагам SimConnect, т.к. N2
            // на поршневых обычно недоступно:
            //   • GENERAL ENG STARTER:N     — стартер крутит двигатель N
            //   • GENERAL ENG COMBUSTION:N  — двигатель N воспламенился/работает
            // =====================================================================
            const PISTON_KICK_MAX_S: f64 = 5.0; // потолок затухания удара воспламенения
            const PISTON_CRANK_FREQ_HZ: f64 = 3.0; // ~180 "тактов" стартера в минуту

            let starters = [fv.eng1_starter, fv.eng2_starter, fv.eng3_starter, fv.eng4_starter];
            let combusting = [
                fv.eng1_combustion > 0.5,
                fv.eng2_combustion > 0.5,
                fv.eng3_combustion > 0.5,
                fv.eng4_combustion > 0.5,
            ];

            let mut starter_term = [0.0f64; 4];
            let mut kick_term = [0.0f64; 4];

            for i in 0..engine_count {
                let is_combusting = combusting[i];

                // Передний фронт воспламенения (false → true) — обнуляем таймер
                // этого конкретного двигателя. Каждый двигатель отслеживается
                // независимо, иначе у второго двигателя пары в 4-моторном режиме
                // таймер не перезапустится, если "сторона" уже считается
                // воспламенившейся.
                if is_combusting && !s.piston_prev_combusting[i] {
                    s.piston_combustion_timer[i] = 0.0;
                }
                if is_combusting {
                    // Таймер растёт по симуляционному времени (dt_clamped), а не по
                    // Instant::now() — единый источник времени со всем остальным
                    // движком. Потолок 5.0 с на один запуск.
                    s.piston_combustion_timer[i] =
                        (s.piston_combustion_timer[i] + dt_clamped).min(PISTON_KICK_MAX_S);
                } else {
                    // Двигатель заглох/ещё не запущен — таймер сброшен, чтобы
                    // следующий передний фронт снова начинал с нуля.
                    s.piston_combustion_timer[i] = 0.0;
                }
                s.piston_prev_combusting[i] = is_combusting;

                // ФАЗА ВОСПЛАМЕНЕНИЯ: резкий удар, плавно (smoothstep) затухающий
                // до строгого нуля за PISTON_KICK_MAX_S секунд — "fading out
                // completely", а не мгновенный обрыв, как у джет-модели.
                if is_combusting {
                    let progress = (s.piston_combustion_timer[i] / PISTON_KICK_MAX_S).clamp(0.0, 1.0);
                    let eased = progress * progress * (3.0 - 2.0 * progress);
                    let decay_factor = (1.0 - eased).clamp(0.0, 1.0);
                    kick_term[i] = decay_factor * (cfg.engine_start_strength as f64).clamp(0.0, 255.0);
                }

                // СТАРТЕР-ФАЗА: стартер крутит, воспламенения ещё не было —
                // ритмичный низкочастотный пульс (имитация тактов прокрутки
                // коленвала/сжатия в цилиндрах), а не гладкая кривая раскрутки
                // турбины. Резкий фронт такта компрессии, затем спад — largo,
                // не симметричная синусоида.
                if starters[i] && !is_combusting {
                    let phase = (fv.sim_time_s * PISTON_CRANK_FREQ_HZ).rem_euclid(1.0);
                    let pulse_shape = if phase < 0.25 {
                        phase / 0.25
                    } else {
                        (1.0 - (phase - 0.25) / 0.75).max(0.0)
                    };
                    starter_term[i] = pulse_shape * (cfg.engine_start_strength as f64).clamp(0.0, 255.0);
                }
            }
            // Игнорируемые в 2-моторном режиме Eng3/Eng4 не должны копить состояние
            // "по инерции" — если четырёхмоторный режим включат позже, они обязаны
            // начинать с чистого состояния.
            for i in engine_count..4 {
                s.piston_combustion_timer[i] = 0.0;
                s.piston_prev_combusting[i] = false;
            }

            // Left → ТОЛЬКО Left Throttle. Right → Right Throttle И Joystick.
            let left_starter_term = left_engines.iter().fold(0.0f64, |m, &i| m.max(starter_term[i]));
            let right_starter_term = right_engines.iter().fold(0.0f64, |m, &i| m.max(starter_term[i]));

            // Удар воспламенения — глобальный эффект: срабатывает, если ХОТЬ ОДИН
            // из активных (в текущем режиме) двигателей ещё не полностью затух
            // (kick_term > 0), и одновременно перекрывает ОБА борта.
            let left_kick_term = left_engines.iter().fold(0.0f64, |m, &i| m.max(kick_term[i]));
            let right_kick_term = right_engines.iter().fold(0.0f64, |m, &i| m.max(kick_term[i]));
            combustion_kick_strength = left_kick_term.max(right_kick_term);
            combustion_kick_active = combustion_kick_strength > 0.01;

            effects.engine_start_active = cfg.enable_engine_start
                && (left_starter_term > 0.5 || right_starter_term > 0.5 || combustion_kick_active);

            if cfg.enable_engine_start {
                throttle_eng_vib_left = left_starter_term;
                throttle_eng_vib_right = right_starter_term;
            }
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

        // Universal Engine Start (Starter phase): моторы throttle_left/right
        // жёстко привязаны к Eng1/Eng2 (железо квадранта, поза за столом тут
        // ни при чём). А вот "чья сторона" дублируется на джойстик — зависит
        // от cfg.swap_hand_layout (см. комментарий у поля и jet-ветку выше).
        transients_t_left += throttle_eng_vib_left;
        transients_t_right += throttle_eng_vib_right;
        transients_j += if cfg.swap_hand_layout { throttle_eng_vib_left } else { throttle_eng_vib_right };

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

        // 3. АБСОЛЮТНЫЙ ОВЕРРАЙД: импульс воспламенения.
        // Глобальный эффект — если воспламенился ЛЮБОЙ из активных в текущем
        // режиме двигателей, — перекрывает ВСЕ три канала (throttle_left,
        // throttle_right, joystick) ОДНОВРЕМЕННО, независимо от раздельной
        // маршрутизации стартер-фазы/раскрутки выше. На джет/турбине —
        // фиксированная максимальная сила на 500 мс (см. is_jet-ветку выше);
        // на поршневых — сила плавно затухает до нуля за 5.0 с
        // (combustion_kick_strength уже несёт в себе эту огибающую).
        if cfg.enable_engine_start && combustion_kick_active {
            let strength = combustion_kick_strength;
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