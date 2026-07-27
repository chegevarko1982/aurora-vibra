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
    // Момент, когда ГАРАНТИРОВАННО отгремели удары обжатия ВСЕХ стоек (нос +
    // лево + право), которые успели сработать к этому моменту. Обновляется
    // через .max() при каждом новом срабатывании стойки (t0 + её длительность
    // импульса), поэтому всегда указывает на конец САМОГО ПОЗДНЕГО из ударов.
    // Ground Roll использует его (а не сырой touchdown_time_s) как точку
    // отсчёта для fade-in — так удар стоек НИКОГДА не смешивается с гулом
    // стыков плит, даже если стойки коснулись не одновременно (нос/лево/право
    // порознь на "трёхточечной" посадке).
    touchdown_settle_until: f64,
    // Флаги "стойка уже коснулась ХОТЯ БЫ РАЗ с момента этого захода на
    // посадку" (сбрасываются на фронте airborne -> on_ground, см. рядом с
    // touchdown_time_s). Нужны, чтобы Ground Roll НЕ начинал fade-in, пока
    // основные стойки уже обжаты, а носовая ЕЩЁ в воздухе (типичная ситуация:
    // самолёт садится на два колеса, затем ещё 1-3с доопускает нос) — иначе
    // гул стыков плит успевал бы разогнаться ДО касания носа и маскировать
    // её отдельный удар. См. GROUND_ROLL_WAIT_TIMEOUT_S в step() — таймаут-
    // предохранитель на случай, если у борта нет рабочей телеметрии по
    // носовой стойке (не даёт Ground Roll молчать вечно).
    nose_touched_since_air: bool,
    left_touched_since_air: bool,
    right_touched_since_air: bool,
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
    // PMDG (737/777): раньше здесь был отдельный трекер "запуска" по
    // L:EngineStart1b/2b_Ext (started_at/ignition_locked/prev_starter_ext),
    // который считал real_n2 > 0 маркером воспламенения. РЕАЛЬНЫЙ ЗАХВАТ
    // ТЕЛЕМЕТРИИ (engine_start_probe, PMDG 737 NG3, MSFS2024) показал, что это
    // предположение неверно вдвойне: (1) real TURB ENG N2 растёт плавно с
    // САМОГО момента включения стартера, а не держится на 0 до воспламенения
    // — значит engine_spool_term уже сам по себе даёт корректную кривую без
    // всякой PMDG-специфики; (2) поэтому "real_n2 > 0" фактически срабатывал
    // на 1-й же кадр после включения стартера, а НЕ в момент реального
    // воспламенения (~5с спустя, real N2≈15%) — 500-мс удар воспламенения
    // стрелял сразу при включении стартера ("толчок"), а на настоящем
    // воспламенении не срабатывал вовсе (ignition_locked уже true) —
    // ощущалось как "толчок и тишина". Трекер удалён; PMDG теперь идёт по
    // ТОЙ ЖЕ ветке, что и любой другой джет (см. is_jet ниже) — удар
    // триггерится обычным фронтом GENERAL ENG COMBUSTION, который на PMDG
    // NG3 подтверждён надёжным и точным по времени.
    // Starter-only ramp (JET, ДО воспламенения): вместо завязки на N2 (тот
    // же N2, что раньше вызывал "толчок и тишина") амплитуда пока крутит
    // ТОЛЬКО стартер (GENERAL ENG STARTER == true, combustion ещё false)
    // считается по РЕАЛЬНОМУ времени с момента переднего фронта стартера —
    // линейная рампа 0..STARTER_ONLY_AMPLITUDE_CAP за STARTER_ONLY_RAMP_SECS,
    // подобрана и подтверждена на живом железе (test_hold, Throttle И
    // Joystick, 0..10 за 12с). Универсальный сигнал (GENERAL ENG STARTER),
    // работает на любом джете, не только PMDG.
    prev_eng1_starter: bool,
    prev_eng2_starter: bool,
    prev_eng3_starter: bool,
    prev_eng4_starter: bool,
    eng1_starter_engaged_at: Option<Instant>,
    eng2_starter_engaged_at: Option<Instant>,
    eng3_starter_engaged_at: Option<Instant>,
    eng4_starter_engaged_at: Option<Instant>,
    // Флаг "двигатель хоть раз воспламенялся в текущей попытке запуска" —
    // выставляется на фронте воспламенения, сбрасывается, когда N2 падает
    // ниже ENGINE_SHUTDOWN_N2_THRESHOLD (турбина полностью остановилась).
    // Нужен, чтобы отличить СТАДИЮ 1 (ещё ни разу не воспламенялся, крутит
    // только стартер — амплитуда по синтетической рампе, см. комментарий у
    // starter_only_amplitude) от СТАДИИ 4 (уже воспламенялся и сейчас
    // ОСТАНАВЛИВАЕТСЯ — турбина ещё физически раскручена по инерции,
    // амплитуда берётся из настоящей N2-кривой). Без этого флага после
    // остановки двигателя (combustion → false, но N2 ещё далеко не 0)
    // эффект ошибочно уходил в СТАДИЮ 1 и мгновенно замолкал.
    eng1_has_ignited: bool,
    eng2_has_ignited: bool,
    eng3_has_ignited: bool,
    eng4_has_ignited: bool,
    // Starter-only abort fade: если стартер выключился ДО воспламенения
    // (неудачная/отменённая попытка запуска), амплитуда не должна замирать
    // на достигнутом уровне рампы — плавно гаснет к нулю за
    // STARTER_ABORT_FADE_SECS от значения, которое было в момент отключения
    // стартера (см. вызов в step()). Подтверждено на живом железе (test_hold
    // fadejerk, без микро-рывков в хвосте).
    eng1_abort_fade_started_at: Option<Instant>,
    eng2_abort_fade_started_at: Option<Instant>,
    eng3_abort_fade_started_at: Option<Instant>,
    eng4_abort_fade_started_at: Option<Instant>,
    eng1_abort_fade_start_value: f64,
    eng2_abort_fade_start_value: f64,
    eng3_abort_fade_start_value: f64,
    eng4_abort_fade_start_value: f64,
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

// Длительность и потолок starter-only рампы (JET, ДО воспламенения) — см.
// комментарий у eng1..4_starter_engaged_at в RumbleState выше. Подтверждены
// на живом железе (test_hold, Throttle и Joystick, 0..10 за 12с).
const STARTER_ONLY_RAMP_SECS: f64 = 12.0;
const STARTER_ONLY_AMPLITUDE_CAP: f64 = 10.0;

/// Линейная рампа 0..STARTER_ONLY_AMPLITUDE_CAP за STARTER_ONLY_RAMP_SECS с
/// момента переднего фронта стартера (см. вызов в step()). None → 0.0
/// (стартер ещё не включался в этом заходе — self-neutralizing).
fn starter_only_amplitude(engaged_at: Option<Instant>) -> f64 {
    match engaged_at {
        Some(t0) => (t0.elapsed().as_secs_f64() / STARTER_ONLY_RAMP_SECS * STARTER_ONLY_AMPLITUDE_CAP)
            .min(STARTER_ONLY_AMPLITUDE_CAP),
        None => 0.0,
    }
}

// Длительность плавного затухания при отменённом/неудавшемся запуске
// (стартер выключился ДО воспламенения) — см. eng1..4_abort_fade_* в
// RumbleState выше. Подтверждена на живом железе (test_hold fadejerk).
const STARTER_ABORT_FADE_SECS: f64 = 4.0;

/// Линейный спад от start_value к 0.0 за STARTER_ABORT_FADE_SECS с момента
/// отключения стартера БЕЗ воспламенения. None → 0.0 (фейда сейчас нет).
fn starter_abort_fade_amplitude(started_at: Option<Instant>, start_value: f64) -> f64 {
    match started_at {
        Some(t0) => {
            let elapsed = t0.elapsed().as_secs_f64();
            (start_value * (1.0 - elapsed / STARTER_ABORT_FADE_SECS)).max(0.0)
        }
        None => 0.0,
    }
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
                touchdown_settle_until: -1000.0,
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

        // Порог Overspeed приходит динамически из SimConnect (AIRSPEED BARBER
        // POLE — Vmo/Mmo, сим сам двигает её вниз при наборе высоты). Некоторые
        // сложные аддоны (напр. TFDI MADDOG) не синхронизируют эту переменную
        // с реальным прибором в кабине — для них есть ручной override.
        // 0.0 означает "порог неизвестен" — в этом случае эффект не должен
        // срабатывать (иначе сработает от IAS >= 0).
        let overspeed_threshold_kn = if cfg.overspeed_override_enabled {
            cfg.overspeed_manual_kn
        } else {
            fv.overspeed_barber_pole_kn
        };
        let overspeed_threshold_known = overspeed_threshold_kn > 0.0;
        let bank_threshold_deg = cfg.bank_threshold_deg as f64;

        // Помимо порога на min(L, R), требуем ещё и симметрию панелей. На практике
        // одного min() недостаточно: при крене (roll spoilers/спойлероны) поднятая
        // плоскость может уйти в 100%, а "спокойная" при этом не обязательно
        // возвращается к 0% (напр. TFDI MD-11 показывает L=100/R=40) — min() в
        // таком случае всё равно превышает порог и ложно активирует эффект.
        // Честный симметричный выпуск спидбрейков держит обе плоскости близко
        // друг к другу, поэтому дополнительно проверяем разницу между ними.
        const SPOILERS_SYMMETRY_TOLERANCE_PCT: f64 = 10.0;
        let spoilers_symmetry_delta = (fv.spoilers_left_pct - fv.spoilers_right_pct).abs();

        // TFDI MD-11: на этом борте даже SPOILERS LEFT/RIGHT POSITION не всегда
        // надёжны, поэтому дополнительно сверяемся с его собственными L-vars по
        // секциям (см. sim/parse.rs). Единицы измерения этих L-vars неизвестны,
        // поэтому сравниваем L и R ОТНОСИТЕЛЬНО друг друга, а не в процентах.
        // На самолётах без этих L-vars оба значения — 0.0, проверка проходит
        // тривиально (0 против 0 симметрично) и не влияет на общий результат.
        const MD11_PANEL_SYMMETRY_TOLERANCE_RATIO: f64 = 0.15;
        let md11_l = fv.spoilers_md11_left_avg;
        let md11_r = fv.spoilers_md11_right_avg;
        let md11_denom = md11_l.max(md11_r);
        let md11_panels_symmetric =
            md11_denom < 1e-6 || (md11_l - md11_r).abs() / md11_denom <= MD11_PANEL_SYMMETRY_TOLERANCE_RATIO;

        let spoilers_active = cfg.spoilers_enabled
            && fv.spoilers_pct > cfg.spoilers_threshold_pct
            && spoilers_symmetry_delta <= SPOILERS_SYMMETRY_TOLERANCE_PCT
            && md11_panels_symmetric
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
            s.nose_touched_since_air = false;
            s.left_touched_since_air = false;
            s.right_touched_since_air = false;
        }
        s.prev_on_ground = fv.on_ground;

        // Gear Strut Compression / Touchdown Detection
        const GEAR_COMP_TOUCHDOWN_THRESHOLD: f64 = 50.1;
        // Длительность резкого импульса обжатия. Основные стойки (лево/право)
        // держат чуть более длинный импульс (0.22с ≈ 4-5 отсчётов на 20 Гц HID-
        // канале, см. hid/worker.rs SEND_INTERVAL) — так удар ощущается как
        // чёткий "тук", а не как щелчок из 2-3 отсчётов. Носовая стойка
        // намеренно короче и с более резким спадом (см. GEAR_COMP_NOSE_DECAY_EXP
        // ниже) — так три стойки при близких по времени касаниях (нос почти
        // сразу за основными на "трёхточечной" посадке) ощущаются как РАЗНЫЕ
        // удары, а не как один смазанный импульс.
        const GEAR_COMP_BUMP_DURATION_MAIN: f64 = 0.22;
        const GEAR_COMP_BUMP_DURATION_NOSE: f64 = 0.12;
        const GEAR_COMP_NOSE_DECAY_EXP: i32 = 5; // круче и короче, чем основные стойки (powi(3))

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
            // gs > 0.5 (не 0.0): настоящее касание при посадке всегда
            // происходит с ненулевой путевой скоростью — при GS=0 самолёт уже
            // стоит (на стоянке/после полной остановки), и любой скачок
            // gear_comp_* (перезагрузка сценария, телепорт, вес/баланс) не
            // должен восприниматься как удар обжатия стойки. Порог не строго
            // 0.0: GROUND VELOCITY у части бортов не бывает ЛИТЕРАЛЬНО нулём
            // даже стоя на месте (шум телеметрии/дробные остатки), а на
            // кокпит-дисплее округляется до "0" — 0.5 узла отсекает этот шум,
            // не мешая реальному касанию (там GS всегда заметно выше).
            if cfg.gear_comp_nose_enabled && gs > 0.5 && fv.gear_comp_nose >= GEAR_COMP_TOUCHDOWN_THRESHOLD && s.prev_gear_comp_nose < GEAR_COMP_TOUCHDOWN_THRESHOLD {
                s.gear_comp_nose_t0 = fv.sim_time_s;
                let comp_rate = (fv.gear_comp_nose - s.prev_gear_comp_nose) / dt;
                let severity = (comp_rate / 100.0).clamp(0.3, 2.5);
                let intensity_frac = ((severity - 0.3) / (2.5 - 0.3)).clamp(0.0, 1.0);
                let headroom = (cfg.gear_comp_nose_peak as f64).clamp(0.0, GEAR_COMP_HEADROOM_MAX);
                let raw_peak = GEAR_COMP_PEAK_MIN + headroom * intensity_frac;
                s.gear_comp_nose_dyn_peak = raw_peak.clamp(GEAR_COMP_PEAK_MIN, GEAR_COMP_PEAK_MAX);
                s.touchdown_settle_until = s.touchdown_settle_until.max(fv.sim_time_s + GEAR_COMP_BUMP_DURATION_NOSE);
                s.nose_touched_since_air = true;
            }
            s.prev_gear_comp_nose = fv.gear_comp_nose;

            // Left Gear
            if cfg.gear_comp_left_enabled && gs > 0.5 && fv.gear_comp_left >= GEAR_COMP_TOUCHDOWN_THRESHOLD && s.prev_gear_comp_left < GEAR_COMP_TOUCHDOWN_THRESHOLD {
                s.gear_comp_left_t0 = fv.sim_time_s;
                let comp_rate = (fv.gear_comp_left - s.prev_gear_comp_left) / dt;
                let severity = (comp_rate / 100.0).clamp(0.3, 2.5);
                let intensity_frac = ((severity - 0.3) / (2.5 - 0.3)).clamp(0.0, 1.0);
                let headroom = (cfg.gear_comp_left_peak as f64).clamp(0.0, GEAR_COMP_HEADROOM_MAX);
                let raw_peak = GEAR_COMP_PEAK_MIN + headroom * intensity_frac;
                s.gear_comp_left_dyn_peak = raw_peak.clamp(GEAR_COMP_PEAK_MIN, GEAR_COMP_PEAK_MAX);
                s.touchdown_settle_until = s.touchdown_settle_until.max(fv.sim_time_s + GEAR_COMP_BUMP_DURATION_MAIN);
                s.left_touched_since_air = true;
            }
            s.prev_gear_comp_left = fv.gear_comp_left;

            // Right Gear
            if cfg.gear_comp_right_enabled && gs > 0.5 && fv.gear_comp_right >= GEAR_COMP_TOUCHDOWN_THRESHOLD && s.prev_gear_comp_right < GEAR_COMP_TOUCHDOWN_THRESHOLD {
                s.gear_comp_right_t0 = fv.sim_time_s;
                let comp_rate = (fv.gear_comp_right - s.prev_gear_comp_right) / dt;
                let severity = (comp_rate / 100.0).clamp(0.3, 2.5);
                let intensity_frac = ((severity - 0.3) / (2.5 - 0.3)).clamp(0.0, 1.0);
                let headroom = (cfg.gear_comp_right_peak as f64).clamp(0.0, GEAR_COMP_HEADROOM_MAX);
                let raw_peak = GEAR_COMP_PEAK_MIN + headroom * intensity_frac;
                s.gear_comp_right_dyn_peak = raw_peak.clamp(GEAR_COMP_PEAK_MIN, GEAR_COMP_PEAK_MAX);
                s.touchdown_settle_until = s.touchdown_settle_until.max(fv.sim_time_s + GEAR_COMP_BUMP_DURATION_MAIN);
                s.right_touched_since_air = true;
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

        // Целевая рабочая мощность берётся из слайдера "Flaps (bump)"
        // (0..255, как и остальные *_peak-слайдеры), переведённого в duty
        // cycle 0.01..0.8 (0.8 — это примерно 200 из 255).
        let max_amplitude = (cfg.flaps_peak as f64 / 255.0).clamp(0.01, 0.8);

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

        // Удар обжатия стоек шасси (touchdown) больше НЕ подмешивается в
        // transients_j/t (там он суммировался с прочими эффектами и мог
        // "потонуть" в их сумме или в clamp по max_output). Вместо этого
        // копится отдельно и применяется ПОСЛЕ clamp как абсолютный оверрайд
        // (см. блок оверрайдов ниже, тот же приём что и удар створок шасси /
        // воспламенение двигателя) — так удар стойки ГАРАНТИРОВАННО пробивает
        // любую сумму остальных эффектов в момент касания.
        let mut touchdown_override_j: f64 = 0.0;
        let mut touchdown_override_t_left: f64 = 0.0;
        let mut touchdown_override_t_right: f64 = 0.0;

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
                // Fade-in после касания: 0.0 пока ещё гремит хотя бы один удар обжатия
                // стойки -> 1.0 через 750мс ПОСЛЕ того, как отгремела ПОСЛЕДНЯЯ из них.
                // Точка отсчёта — s.touchdown_settle_until (максимум по всем трём
                // стойкам, см. комментарий у поля), а не сырой момент касания земли:
                // стойки касаются не одновременно (нос/лево/право порознь на
                // "трёхточечной" посадке), поэтому фиксированный рамп от общего
                // on_ground-фронта мог начать нарастать ДО того, как прогремел
                // удар последней стойки, и они смешивались. Пока settle_until в
                // будущем — разница отрицательна и clamp(0.0, 1.0) даёт строгий 0,
                // то есть Ground Roll полностью молчит на всё время ударов стоек.
                // .max(s.touchdown_time_s) — подстраховка на случай, если gear_comp
                // выключен/не сработал: тогда fade-in идёт по старой схеме от факта
                // касания земли.
                //
                // ВАЖНО: одного touchdown_settle_until недостаточно — он знает только
                // про стойки, которые УЖЕ коснулись. При посадке на два колеса нос
                // ещё 1-3с висит в воздухе (доопускание после флэра), а settle_until
                // в это время уже "протух" (обжатие основных стоек давно отгремело),
                // из-за чего Ground Roll успевал разогнаться ДО касания носа и
                // маскировать её отдельный удар — та же проблема, которую мы решали,
                // просто на уровне "нос vs основные", а не "стойки vs гул". Поэтому
                // fade-in дополнительно ждёт, пока коснутся ВСЕ включённые стойки
                // (nose_touched_since_air и т.д.), либо — предохранитель на случай
                // борта без рабочей телеметрии по носовой стойке — не более
                // GROUND_ROLL_WAIT_TIMEOUT_S с момента первого касания земли.
                const GROUND_ROLL_WAIT_TIMEOUT_S: f64 = 3.0;
                let all_struts_settled = !cfg.gear_comp_enabled
                    || ((!cfg.gear_comp_nose_enabled || s.nose_touched_since_air)
                        && (!cfg.gear_comp_left_enabled || s.left_touched_since_air)
                        && (!cfg.gear_comp_right_enabled || s.right_touched_since_air));
                let touchdown_timeout_elapsed =
                    fv.sim_time_s - s.touchdown_time_s >= GROUND_ROLL_WAIT_TIMEOUT_S;

                let fade_anchor = s.touchdown_settle_until.max(s.touchdown_time_s);
                let time_since_fade_anchor = fv.sim_time_s - fade_anchor;
                let touchdown_fade_in = if all_struts_settled || touchdown_timeout_elapsed {
                    (time_since_fade_anchor / 0.75).clamp(0.0, 1.0)
                } else {
                    0.0
                };

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
            let ias_overspeed = if overspeed_threshold_known {
                (fv.airspeed_indicated - overspeed_threshold_kn).max(0.0)
            } else {
                0.0
            };
            // Learjet 35A (Flysimware, экспериментально): дополнительный
            // триггер по L:XMLSND75 — клаксону "overspeed / mach trim" из
            // sound.xml аддона (см. profiles.rs). Включён только на этом
            // борту (overspeed_lear_horn_enabled ставится профилем по title);
            // на прочих самолётах fv.overspeed_lear_horn всегда false, и это
            // условие не меняет прежнее поведение.
            let horn_triggered = cfg.overspeed_lear_horn_enabled && fv.overspeed_lear_horn;
            if !fv.on_ground && (ias_overspeed > 0.0 || horn_triggered) {
                // Умная вибрация: сразу ощутимая (не меньше 10% силы) уже на 1
                // узле превышения красной черты (Vmo/Mmo), дальше нарастает вместе
                // с величиной превышения — чем яростнее разгон сверх лимита, тем
                // сильнее и чаще бьёт мотор. 120 узлов превышения = полная сила.
                // Если сработал только клаксон (IAS-порог неизвестен/не превышен),
                // магнитуды превышения нет — трактуем как полную силу.
                let growth = if ias_overspeed > 0.0 {
                    (ias_overspeed / 120.0).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let ratio = 0.1 + 0.9 * growth;
                let intensity = ratio * (cfg.overspeed_intensity as f64);
                let oscillation = (2.0 * std::f64::consts::PI * (5.0 + growth * 15.0) * fv.sim_time_s).sin() * 0.5 + 0.5;
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
            // SPLIT-режим (3 стойки): оба мотора РУД физически находятся в ОДНОЙ
            // руке (это один блок throttle_left+throttle_right под одной ладонью),
            // а джойстик — в другой. Поэтому две ОСНОВНЫЕ стойки — которые почти
            // всегда касаются практически одновременно (посадка на два колеса
            // сразу) — разводятся по РАЗНЫМ рукам (РУД vs джойстик), иначе они
            // сливаются в один "блок" ощущений в одной ладони — та же проблема
            // смазывания, что и с Ground Roll, только между собой. cfg.swap_hand_layout
            // определяет, какая сторона (лево/право) достаётся какой руке — та же
            // семантика, что уже используется для двигателей.
            // Носовая стойка касается заметно ПОЗЖЕ основных (самолёт доопускает
            // нос уже после обжатия главных стоек), поэтому безопасно делит руку
            // РУД со "своей" основной стойкой — на свободный второй мотор — и
            // дополнительно отличается по ощущению более резкой/короткой формой
            // импульса (GEAR_COMP_NOSE_DECAY_EXP), так что даже при редком
            // наложении по времени это не сольётся в одно и то же ощущение.
            let left_gear_on_throttle = !cfg.swap_hand_layout;

            let nose_active = cfg.gear_comp_nose_enabled && fv.sim_time_s >= s.gear_comp_nose_t0 && fv.sim_time_s <= s.gear_comp_nose_t0 + GEAR_COMP_BUMP_DURATION_NOSE;
            if nose_active {
                let p = ((fv.sim_time_s - s.gear_comp_nose_t0) / GEAR_COMP_BUMP_DURATION_NOSE).clamp(0.0, 1.0);
                let term = s.gear_comp_nose_dyn_peak * (1.0 - p).powi(GEAR_COMP_NOSE_DECAY_EXP);
                if cfg.split_touchdown {
                    // Нос всегда идёт на РУД (единственное устройство с запасным
                    // мотором) — на тот мотор, который в ЭТОМ кадре НЕ занят
                    // "своей" основной стойкой (см. left_gear_on_throttle выше).
                    if left_gear_on_throttle {
                        touchdown_override_t_right = touchdown_override_t_right.max(term);
                    } else {
                        touchdown_override_t_left = touchdown_override_t_left.max(term);
                    }
                } else {
                    if dt_.gear_comp_nose.enable_joystick { touchdown_override_j = touchdown_override_j.max(term); }
                    if dt_.gear_comp_nose.enable_throttle {
                        touchdown_override_t_left = touchdown_override_t_left.max(term);
                        touchdown_override_t_right = touchdown_override_t_right.max(term);
                    }
                }
            }
            effects.gear_comp_nose_active = nose_active;

            let left_active = cfg.gear_comp_left_enabled && fv.sim_time_s >= s.gear_comp_left_t0 && fv.sim_time_s <= s.gear_comp_left_t0 + GEAR_COMP_BUMP_DURATION_MAIN;
            if left_active {
                let p = ((fv.sim_time_s - s.gear_comp_left_t0) / GEAR_COMP_BUMP_DURATION_MAIN).clamp(0.0, 1.0);
                let term = s.gear_comp_left_dyn_peak * (1.0 - p).powi(3);
                if cfg.split_touchdown {
                    // SPLIT: левая основная стойка — эксклюзивно на РУД (мотор
                    // throttle_left), кроме случая swap_hand_layout — тогда на
                    // джойстик. Независимо от чекбоксов dt_.gear_comp_left.
                    if left_gear_on_throttle {
                        touchdown_override_t_left = touchdown_override_t_left.max(term);
                    } else {
                        touchdown_override_j = touchdown_override_j.max(term);
                    }
                } else {
                    if dt_.gear_comp_left.enable_joystick { touchdown_override_j = touchdown_override_j.max(term); }
                    if dt_.gear_comp_left.enable_throttle {
                        touchdown_override_t_left = touchdown_override_t_left.max(term);
                        touchdown_override_t_right = touchdown_override_t_right.max(term);
                    }
                }
            }
            effects.gear_comp_left_active = left_active;

            let right_active = cfg.gear_comp_right_enabled && fv.sim_time_s >= s.gear_comp_right_t0 && fv.sim_time_s <= s.gear_comp_right_t0 + GEAR_COMP_BUMP_DURATION_MAIN;
            if right_active {
                let p = ((fv.sim_time_s - s.gear_comp_right_t0) / GEAR_COMP_BUMP_DURATION_MAIN).clamp(0.0, 1.0);
                let term = s.gear_comp_right_dyn_peak * (1.0 - p).powi(3);
                if cfg.split_touchdown {
                    // SPLIT: правая основная стойка — всегда на РУКУ, противоположную
                    // левой стойке выше (гарантирует, что лево/право никогда не
                    // окажутся в одной ладони одновременно).
                    if left_gear_on_throttle {
                        touchdown_override_j = touchdown_override_j.max(term);
                    } else {
                        touchdown_override_t_right = touchdown_override_t_right.max(term);
                    }
                } else {
                    if dt_.gear_comp_right.enable_joystick { touchdown_override_j = touchdown_override_j.max(term); }
                    if dt_.gear_comp_right.enable_throttle {
                        touchdown_override_t_left = touchdown_override_t_left.max(term);
                        touchdown_override_t_right = touchdown_override_t_right.max(term);
                    }
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

        // Эффект уборки/выпуска шасси имеет смысл только пока самолёт
        // практически неподвижен (наземный тест шасси на стоянке) — при
        // GS >= 0.1 узла отключаем целиком, чтобы не путать его с шумом
        // strut-компрессии во время руления/пробега.
        if cfg.gear_transit_enabled && gs < 0.1 {
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
        } else {
            effects.gear_transit_active = false;
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
        // PMDG: real_n2 пересекает дэдзону (>1.0) только ~0.3с после включения
        // стартера (см. engine_start_probe capture), поэтому дополнительно
        // считаем джетом, если сработал L:EngineStart1b/2b_Ext — иначе эта
        // короткая щель могла бы попасть в PISTON-ветку. На не-PMDG бортах эти
        // L-vars всегда false (самонейтрализуется), поведение не меняется.
        //
        // БАГ (найден при тестировании 4-моторного режима): раньше здесь
        // проверялись ТОЛЬКО Eng1/Eng2. Если тестировать/запускать по
        // отдельности Eng3 или Eng4, пока Eng1/Eng2 ещё стоят на 0% (типичный
        // сценарий пошаговой проверки каждого двигателя), самолёт ошибочно
        // определялся как PISTON и весь эффект уходил в piston-модель
        // (5-секундный фиксированный фейд удара, без продолжающейся
        // N2-кривой) — вместо корректной JET-модели. В 4-моторном режиме
        // дополнительно учитываем N2 третьего и четвёртого двигателя.
        let is_jet = fv.eng1_n2_percent > 1.0
            || fv.eng2_n2_percent > 1.0
            || fv.eng1_pmdg_starter_ext
            || fv.eng2_pmdg_starter_ext
            || (cfg.four_engine_mode && (fv.eng3_n2_percent > 1.0 || fv.eng4_n2_percent > 1.0));

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
            // JET / TURBINE — 3-СТАДИЙНАЯ МОДЕЛЬ.
            // =====================================================================
            // СТАДИЯ 1 (Pre-Combustion Spool-up): ПОКА combustion == false —
            // амплитуда НЕ по N2 (см. starter_only_amplitude ниже: реальный N2
            // растёт медленно и непредсказуемо по бортам, линейная завязка на
            // него делает раскрутку то незаметной, то слишком резкой) — линейная
            // рампа 0..STARTER_ONLY_AMPLITUDE_CAP за STARTER_ONLY_RAMP_SECS от
            // реального времени включения стартера (GENERAL ENG STARTER),
            // подтверждённая на живом железе. Маршрутизация: ПОКА combustion ==
            // false у данного двигателя — его вибрация идёт ТОЛЬКО на свой борт:
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
            // Минимум по ШИМ (0..255) в кривой N2 (используется ТОЛЬКО ПОСЛЕ
            // воспламенения — см. starter_only_amplitude выше для ДО-воспламенительной
            // фазы). 1.0 — просто "не строго ноль, двигатель определённо крутится".
            const ENGINE_SPOOL_MIN_AMPLITUDE: f64 = 1.0;
            // Потолок ШИМ для N2-кривой ПОСЛЕ воспламенения — управляется тем же
            // слайдером "Engine Start" (cfg.engine_start_strength), который
            // раньше напрямую задавал силу удара воспламенения. По требованию
            // сам слайдер (диапазон в UI не менялся, 0..100%/0..255 raw)
            // теперь маппится в потолок кривой 1%..80% от максимума (255), а
            // не в силу удара — удар теперь фиксирован (см. ниже). НЕЗАВИСИМО
            // от того, один двигатель работает или несколько (см. клэмп
            // throttle_eng_vib/joystick_eng_vib ПОСЛЕ маршрутизации по бортам
            // ниже — иначе при одновременной работе обоих двигателей их
            // кросс-роутинг мог бы просуммироваться выше этого потолка).
            let engine_spool_max_pct =
                1.0 + (cfg.engine_start_strength as f64 / 255.0).clamp(0.0, 1.0) * 79.0; // 1%..80%
            let engine_spool_max_amplitude: f64 = engine_spool_max_pct / 100.0 * 255.0;
            // Удар воспламенения: по требованию фиксированная максимальная
            // сила (255, больше НЕ завязана на cfg.engine_start_strength —
            // тот теперь полностью отдан под потолок кривой выше; у поршневых
            // двигателей (PISTON-ветка ниже) слайдер по-прежнему задаёт силу
            // удара напрямую, как раньше) и длительность 1с (была 500мс).
            const ENGINE_IGNITION_KICK_DURATION: Duration = Duration::from_millis(1000);

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

            // PMDG (737/777) на eng1/eng2 больше не имеет отдельной ветки — см.
            // комментарий у PISTON/JET-полей в RumbleState: real GENERAL ENG
            // COMBUSTION на PMDG NG3 подтверждён надёжным и точным по времени
            // (engine_start_probe capture), поэтому удар воспламенения
            // триггерится тем же обычным фронтом, что и у любого другого джета.
            // Также сбрасываем starter_engaged_at в None на этом же фронте:
            // иначе после успешного запуска это поле навсегда остаётся
            // Some(старый_момент), и если двигатель ПОЗЖЕ заглохнет
            // (combustion → false), starter_only_amplitude() снова будет
            // вызвана с этим устаревшим таймстампом — t0.elapsed() окажется
            // намного больше STARTER_ONLY_RAMP_SECS, рампа зависнет на
            // STARTER_ONLY_AMPLITUDE_CAP (10.0) НАВСЕГДА вместо честного 0.0
            // (см. None-ветку функции). Именно так эффект "не выключался"
            // после остановки однажды запускавшегося двигателя.
            // На этом же фронте помечаем eng_has_ignited = true — это ключ к
            // корректному эффекту ОСТАНОВКИ (СТАДИЯ 4, см. комментарий у
            // eng1..4_has_ignited в RumbleState): после этого момента, пока
            // N2 не упадёт ниже ENGINE_SHUTDOWN_N2_THRESHOLD, амплитуда идёт
            // по настоящей N2-кривой, даже когда combustion уже false
            // (турбина ещё раскручена по инерции).
            if is_combusting_eng1 && !s.prev_eng1_combusting {
                s.eng1_kick_started_at = Some(Instant::now());
                s.eng1_starter_engaged_at = None;
                s.eng1_has_ignited = true;
            }
            if is_combusting_eng2 && !s.prev_eng2_combusting {
                s.eng2_kick_started_at = Some(Instant::now());
                s.eng2_starter_engaged_at = None;
                s.eng2_has_ignited = true;
            }
            if is_combusting_eng3 && !s.prev_eng3_combusting {
                s.eng3_kick_started_at = Some(Instant::now());
                s.eng3_starter_engaged_at = None;
                s.eng3_has_ignited = true;
            }
            if is_combusting_eng4 && !s.prev_eng4_combusting {
                s.eng4_kick_started_at = Some(Instant::now());
                s.eng4_starter_engaged_at = None;
                s.eng4_has_ignited = true;
            }

            // Обновляем предыдущее состояние зажигания для следующего кадра.
            s.prev_eng1_combusting = is_combusting_eng1;
            s.prev_eng2_combusting = is_combusting_eng2;
            s.prev_eng3_combusting = is_combusting_eng3;
            s.prev_eng4_combusting = is_combusting_eng4;

            // Передний фронт стартера — фиксируем момент начала прокрутки
            // (реальный wall-clock), от которого считается starter_only_amplitude
            // ниже. Сигнал = GENERAL ENG STARTER ИЛИ PMDG L:EngineStart1b/2b_Ext,
            // ЛЮБОЙ из двух. Причина: GENERAL ENG STARTER САМ ПО СЕБЕ оказался
            // на практике недостаточно надёжным сигналом (на живом тесте PMDG —
            // L-var включался, а GENERAL ENG STARTER на том же кадре нет), а
            // предыдущая версия (просто N2-кривая без всякого стартер-гейта)
            // вибрацию честно давала — поэтому здесь берём ЛЮБОЙ доступный
            // маркер стартера, а не только "универсальный" (который не всегда
            // universal на практике). На не-PMDG бортах L-var всегда false
            // (самонейтрализуется), решает GENERAL ENG STARTER как и раньше.
            let eng1_starter_signal = fv.eng1_starter || fv.eng1_pmdg_starter_ext;
            let eng2_starter_signal = fv.eng2_starter || fv.eng2_pmdg_starter_ext;
            let eng3_starter_signal = fv.eng3_starter;
            let eng4_starter_signal = fv.eng4_starter;

            // Передний фронт стартера: старт рампы, отменяем любой висящий
            // abort-фейд (новая попытка запуска перекрывает предыдущую).
            if eng1_starter_signal && !s.prev_eng1_starter {
                s.eng1_starter_engaged_at = Some(Instant::now());
                s.eng1_abort_fade_started_at = None;
            }
            if eng2_starter_signal && !s.prev_eng2_starter {
                s.eng2_starter_engaged_at = Some(Instant::now());
                s.eng2_abort_fade_started_at = None;
            }
            if eng3_starter_signal && !s.prev_eng3_starter {
                s.eng3_starter_engaged_at = Some(Instant::now());
                s.eng3_abort_fade_started_at = None;
            }
            if eng4_starter_signal && !s.prev_eng4_starter {
                s.eng4_starter_engaged_at = Some(Instant::now());
                s.eng4_abort_fade_started_at = None;
            }

            // Задний фронт стартера БЕЗ воспламенения (отменённая/неудавшаяся
            // попытка запуска) — не замираем на достигнутом уровне рампы,
            // запоминаем его и плавно гасим к нулю (starter_abort_fade_amplitude
            // ниже) вместо мгновенного обнуления или зависания.
            if !eng1_starter_signal && s.prev_eng1_starter && !is_combusting_eng1 {
                s.eng1_abort_fade_start_value = starter_only_amplitude(s.eng1_starter_engaged_at);
                s.eng1_abort_fade_started_at = Some(Instant::now());
                s.eng1_starter_engaged_at = None;
            }
            if !eng2_starter_signal && s.prev_eng2_starter && !is_combusting_eng2 {
                s.eng2_abort_fade_start_value = starter_only_amplitude(s.eng2_starter_engaged_at);
                s.eng2_abort_fade_started_at = Some(Instant::now());
                s.eng2_starter_engaged_at = None;
            }
            if !eng3_starter_signal && s.prev_eng3_starter && !is_combusting_eng3 {
                s.eng3_abort_fade_start_value = starter_only_amplitude(s.eng3_starter_engaged_at);
                s.eng3_abort_fade_started_at = Some(Instant::now());
                s.eng3_starter_engaged_at = None;
            }
            if !eng4_starter_signal && s.prev_eng4_starter && !is_combusting_eng4 {
                s.eng4_abort_fade_start_value = starter_only_amplitude(s.eng4_starter_engaged_at);
                s.eng4_abort_fade_started_at = Some(Instant::now());
                s.eng4_starter_engaged_at = None;
            }

            s.prev_eng1_starter = eng1_starter_signal;
            s.prev_eng2_starter = eng2_starter_signal;
            s.prev_eng3_starter = eng3_starter_signal;
            s.prev_eng4_starter = eng4_starter_signal;

            // ШАГ 1: ВСЕГДА считаем базовую вибрацию раскрутки по N2 — независимо
            // от того, активен ли сейчас удар воспламенения. Именно поэтому после
            // истечения удара (ENGINE_IGNITION_KICK_DURATION) переход происходит
            // бесшовно, а не через провал в 0. Дэдзона: N2 < 1.0 → 0 (двигатель
            // выключен). При N2 в [1.0 .. engine_idle_n2): минимум 1, линейный рост
            // к потолку слайдера у engine_idle_n2/3, затем степенной ease-out спад
            // (резкий сразу после пика, пологий к хвосту) к строго 0 у engine_idle_n2
            // (Idle) — см. DECAY_EXPONENT ниже.
            let engine_spool_term = |n2_percent: f64| -> f64 {
                if n2_percent < ENGINE_SPOOL_DEADZONE_N2 || n2_percent >= engine_spool_n2_max {
                    // Дэдзона (двигатель выключен) ИЛИ N2 достиг/превысил Idle —
                    // амплитуда строго 0.0 (п.1 и п.5 требований).
                    0.0
                } else if n2_percent <= engine_spool_peak_n2 {
                    // Плавный рост от минимума ШИМ=1 до потолка engine_spool_max_amplitude
                    // (1%..80% от 255, см. слайдер выше) ровно на engine_spool_peak_n2
                    // (peak_n2 = engine_idle_n2 / 3.0).
                    let rise_progress = (n2_percent / engine_spool_peak_n2).clamp(0.0, 1.0);
                    (rise_progress * engine_spool_max_amplitude).max(ENGINE_SPOOL_MIN_AMPLITUDE)
                } else {
                    // Степенной ease-out спад (по требованию/ручной правке кривой,
                    // см. рисунок): decay_progress идёт от 0 (у пика) до 1 (точно у
                    // engine_idle_n2/Idle). В отличие от прежнего smoothstep (плавный
                    // старт спада, резкий перегиб в середине) — здесь спад РЕЗКИЙ
                    // сразу после пика и всё более пологий к хвосту (степень >1 даёт
                    // нулевую производную ТОЛЬКО у самого Idle, а не у пика).
                    // Было 2.5 — на живом тесте (N2=64.4%, разные Idle) хвост
                    // оказался настолько низким, что эффект был практически
                    // неощутим уже при Idle=70 (0.5% от потолка), хотя N2 ещё
                    // далеко не дошёл до настроенного порога — снижено до 1.5:
                    // спад по-прежнему быстрее старого smoothstep у пика, но
                    // хвост держит заметно бОльшую силу дольше (см. таблицу в
                    // истории обсуждения: 1.5 даёт ~5-16% в том же диапазоне
                    // вместо ~0.3-5% у 2.5).
                    const DECAY_EXPONENT: f64 = 1.5;
                    let decay_progress = ((n2_percent - engine_spool_peak_n2)
                        / (engine_spool_n2_max - engine_spool_peak_n2))
                        .clamp(0.0, 1.0);
                    let amplitude_factor = (1.0 - decay_progress).powf(DECAY_EXPONENT).clamp(0.0, 1.0);
                    // Без .max(ENGINE_SPOOL_MIN_AMPLITUDE) здесь: амплитуда должна
                    // дойти РОВНО до 0.0 у engine_idle_n2, а не застрять на полу.
                    (amplitude_factor * engine_spool_max_amplitude).clamp(0.0, engine_spool_max_amplitude)
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
            // МЫ УЖЕ ВНУТРИ ВЕТКИ is_jet == true — то есть N2 сам по себе уже
            // подтверждённо валидный, ненулевой сигнал (это и есть критерий,
            // по которому is_jet вообще стал true). Раньше здесь дополнительно
            // брался max(N2, GENERAL ENG PCT MAX RPM) "на случай поршневых", но
            // это было ошибкой ИМЕННО в этой ветке: предположение "оба сигнала
            // одновременно ненулевыми быть не могут" не подтвердилось на живом
            // тесте (A350-900, N2 idle=65 в конфиге) — GENERAL ENG PCT MAX RPM
            // у этого борта тоже читался ненулевым и местами ВЫШЕ реального N2,
            // из-за чего eng1_effective пересекал engine_idle_n2 и эффект гас
            // раньше времени (N2=62.7%, эффект уже 0, хотя порог настроен на 65%).
            // GENERAL ENG PCT MAX RPM остаётся нужен ТОЛЬКО в PISTON-ветке ниже
            // (is_jet == false), где TURB ENG N2 гарантированно 0.
            let eng1_effective = fv.eng1_n2_percent;
            let eng2_effective = fv.eng2_n2_percent;
            let eng3_effective = fv.eng3_n2_percent;
            let eng4_effective = fv.eng4_n2_percent;

            // ДО воспламенения (СТАДИЯ 1, is_combusting == false И ни разу ещё
            // не воспламенялся в этой попытке) — starter_only_amplitude (см.
            // starter-only ramp выше), НЕ engine_spool_term(N2): реальный N2 в
            // фазе прокрутки растёт слишком непредсказуемо/медленно, чтобы
            // линейно давать ощутимую вибрацию с первых секунд.
            // ПОСЛЕ воспламенения (is_combusting == true) — обычная N2-кривая.
            // ОСТАНОВКА (СТАДИЯ 4, is_combusting == false, НО двигатель уже
            // воспламенялся раньше, eng_has_ignited == true) — турбина ещё
            // физически раскручена по инерции, поэтому продолжаем использовать
            // ту же N2-кривую (engine_spool_term), пока N2 не упадёт ниже
            // ENGINE_SHUTDOWN_N2_THRESHOLD — эффект должен выключаться именно
            // тогда, а не мгновенно в момент падения флага combustion (раньше
            // в этот момент эффект обрывался мгновенно, даже если N2 всё ещё
            // был на уровне Idle). Как только N2 уходит ниже порога — считаем
            // двигатель полностью остановленным и сбрасываем eng_has_ignited,
            // чтобы следующий запуск снова начинался с чистой СТАДИИ 1.
            //
            // Гейт по s.engN_starter_engaged_at (а не по мгновенному значению
            // сигнала стартера каждый кадр) — устойчивее к возможному
            // "дребезгу"/пропускам самого сигнала на отдельных кадрах;
            // starter_only_amplitude сама возвращает 0.0, если engaged_at
            // ещё None. .max() со starter_abort_fade_amplitude: ровно один из
            // двух источников ненулевой в любой момент (engaged_at сбрасывается
            // в None в момент старта фейда, см. задний фронт выше), так что
            // max() здесь эквивалентен сложению, но не заставляет думать о
            // порядке — рампа ИЛИ фейд отмены, никогда оба разом.
            const ENGINE_SHUTDOWN_N2_THRESHOLD: f64 = 3.0;
            let eng1_term = if is_combusting_eng1 {
                engine_spool_term(eng1_effective)
            } else if s.eng1_has_ignited && eng1_effective > ENGINE_SHUTDOWN_N2_THRESHOLD {
                engine_spool_term(eng1_effective)
            } else {
                s.eng1_has_ignited = false;
                starter_only_amplitude(s.eng1_starter_engaged_at)
                    .max(starter_abort_fade_amplitude(s.eng1_abort_fade_started_at, s.eng1_abort_fade_start_value))
            };
            let eng2_term = if is_combusting_eng2 {
                engine_spool_term(eng2_effective)
            } else if s.eng2_has_ignited && eng2_effective > ENGINE_SHUTDOWN_N2_THRESHOLD {
                engine_spool_term(eng2_effective)
            } else {
                s.eng2_has_ignited = false;
                starter_only_amplitude(s.eng2_starter_engaged_at)
                    .max(starter_abort_fade_amplitude(s.eng2_abort_fade_started_at, s.eng2_abort_fade_start_value))
            };
            let eng3_term = if is_combusting_eng3 {
                engine_spool_term(eng3_effective)
            } else if s.eng3_has_ignited && eng3_effective > ENGINE_SHUTDOWN_N2_THRESHOLD {
                engine_spool_term(eng3_effective)
            } else {
                s.eng3_has_ignited = false;
                starter_only_amplitude(s.eng3_starter_engaged_at)
                    .max(starter_abort_fade_amplitude(s.eng3_abort_fade_started_at, s.eng3_abort_fade_start_value))
            };
            let eng4_term = if is_combusting_eng4 {
                engine_spool_term(eng4_effective)
            } else if s.eng4_has_ignited && eng4_effective > ENGINE_SHUTDOWN_N2_THRESHOLD {
                engine_spool_term(eng4_effective)
            } else {
                s.eng4_has_ignited = false;
                starter_only_amplitude(s.eng4_starter_engaged_at)
                    .max(starter_abort_fade_amplitude(s.eng4_abort_fade_started_at, s.eng4_abort_fade_start_value))
            };

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

            // Удар (kick) остаётся групповым по стороне — срабатывает от КАЖДОГО
            // двигателя своей группы независимо (не только от первого
            // воспламенившегося), см. поля prev_engN_combusting в RumbleState.
            let (left_kick_active, right_kick_active) = if cfg.four_engine_mode {
                (eng1_kick_active || eng2_kick_active, eng3_kick_active || eng4_kick_active)
            } else {
                (eng1_kick_active, eng2_kick_active)
            };

            // Маршрутизация базовой раскрутки — ПОЭЛЕМЕНТНО, по СВОЕМУ состоянию
            // combustion КАЖДОГО двигателя, а не по агрегированному OR всей
            // "стороны". Раньше (баг) сторона считалась "уже воспламенившейся",
            // если ГОРЕЛ хотя бы один из пары — из-за этого в 4-моторном режиме,
            // как только первый двигатель пары (Eng1/Eng3) воспламенялся, второй
            // (Eng2/Eng4), ещё только раскручивающийся стартером, тоже начинал
            // литься в ОБА канала вместо своего борта. Чтобы сохранить прежнюю
            // "амплитуда = max по стороне" (не сумма, иначе одновременная работа
            // пары удваивала бы силу), максимум берём отдельно для двигателей
            // стороны, которые УЖЕ горят, и отдельно — для тех, что ещё стартуют:
            //   • горящие  → максимум идёт в ОБА канала (сторона уже работает)
            //   • стартующие → максимум идёт ТОЛЬКО на свой борт
            // Двигатель, который уже воспламенялся и сейчас останавливается
            // (СТАДИЯ 4, eng_has_ignited && N2 > порога — см. вычисление
            // eng1..4_term выше), для маршрутизации тоже считается "горящим":
            // раскручивающаяся по инерции турбина ощущается по всей кабине,
            // как и работающий двигатель, а не только на своей стороне.
            let eng_combusting = [
                is_combusting_eng1 || (s.eng1_has_ignited && eng1_effective > ENGINE_SHUTDOWN_N2_THRESHOLD),
                is_combusting_eng2 || (s.eng2_has_ignited && eng2_effective > ENGINE_SHUTDOWN_N2_THRESHOLD),
                is_combusting_eng3 || (s.eng3_has_ignited && eng3_effective > ENGINE_SHUTDOWN_N2_THRESHOLD),
                is_combusting_eng4 || (s.eng4_has_ignited && eng4_effective > ENGINE_SHUTDOWN_N2_THRESHOLD),
            ];
            let eng_term = [eng1_term, eng2_term, eng3_term, eng4_term];
            let side_terms = |indices: &[usize]| -> (f64, f64) {
                let mut combusting_max = 0.0_f64;
                let mut starting_max = 0.0_f64;
                for &i in indices {
                    if eng_combusting[i] {
                        combusting_max = combusting_max.max(eng_term[i]);
                    } else {
                        starting_max = starting_max.max(eng_term[i]);
                    }
                }
                (combusting_max, starting_max)
            };
            let (left_combusting_term, left_starting_term) = side_terms(left_engines);
            let (right_combusting_term, right_starting_term) = side_terms(right_engines);

            let mut throttle_eng_vib: f64 = 0.0;
            let mut joystick_eng_vib: f64 = 0.0;

            // cfg.swap_hand_layout меняет местами, какая сторона (Eng1/left или
            // Eng2/right) считается "рукой РУД", а какая "рукой джойстика" —
            // см. комментарий у поля в RumbleConfig. По умолчанию (false)
            // сохраняется исходное поведение: Eng1 → РУД, Eng2 → джойстик.
            let (throttle_combusting_term, throttle_starting_term, joystick_combusting_term, joystick_starting_term) =
                if cfg.swap_hand_layout {
                    (right_combusting_term, right_starting_term, left_combusting_term, left_starting_term)
                } else {
                    (left_combusting_term, left_starting_term, right_combusting_term, right_starting_term)
                };

            if cfg.enable_engine_start {
                // Сторона "руки РУД": уже горящие двигатели трясут оба канала,
                // ещё стартующие — только свой борт (РУД).
                throttle_eng_vib += throttle_combusting_term;
                joystick_eng_vib += throttle_combusting_term;
                throttle_eng_vib += throttle_starting_term;

                // Сторона "руки джойстика": симметрично.
                throttle_eng_vib += joystick_combusting_term;
                joystick_eng_vib += joystick_combusting_term;
                joystick_eng_vib += joystick_starting_term;
            }

            // Финальный клэмп ПОСЛЕ маршрутизации по бортам — гарантирует потолок
            // слайдера (engine_spool_max_amplitude, 1%..80%) независимо от того,
            // сколько двигателей сейчас работает: при одновременной работе ОБОИХ
            // сторон кросс-роутинг суммирует их термины в один канал, а каждый
            // term уже отдельно ограничен потолком в engine_spool_term — без
            // этого клэмпа их сумма могла бы превысить его.
            throttle_eng_vib = throttle_eng_vib.min(engine_spool_max_amplitude);
            joystick_eng_vib = joystick_eng_vib.min(engine_spool_max_amplitude);

            // Если физически подключено только ОДНО устройство (джойстик ИЛИ
            // РУД) — весь эффект запуска двигателя должен идти на него целиком,
            // а не частично теряться в канале несуществующего устройства (см.
            // cfg.joystick_hw_connected/throttle_hw_connected — живой снимок из
            // UI, обновляется автоматически, не пользовательский чекбокс).
            // Если подключены оба (или ни одного — например, сим только что
            // запущен) — раздельная маршрутизация по бортам не трогается.
            if cfg.joystick_hw_connected && !cfg.throttle_hw_connected {
                joystick_eng_vib += throttle_eng_vib;
                throttle_eng_vib = 0.0;
            } else if cfg.throttle_hw_connected && !cfg.joystick_hw_connected {
                throttle_eng_vib += joystick_eng_vib;
                joystick_eng_vib = 0.0;
            }

            // Удар — глобальный эффект: срабатывает от ЛЮБОГО двигателя (каждый
            // отслеживается независимо, см. выше) и применяется одновременно к
            // обоим каналам (см. оверрайд ниже). По требованию — фиксированная
            // максимальная сила (255), не завязана на cfg.engine_start_strength
            // (тот остаётся в ходу только у поршневых, см. PISTON-ветку ниже) —
            // в отличие от кривой ПОСЛЕ удара, сам удар в 80%-й потолок не входит.
            combustion_kick_active = left_kick_active || right_kick_active;
            combustion_kick_strength = 255.0;

            effects.engine_start_active = cfg.enable_engine_start
                && (throttle_eng_vib.abs() > 0.5 || joystick_eng_vib.abs() > 0.5 || combustion_kick_active);

            // Подмешиваем базовую раскрутку в соответствующие каналы через transients
            // (чтобы не гаситься экспоненциальным сглаживанием air_term ниже). Пока
            // удар воспламенения активен, ниже (после clamp) он ПОЛНОСТЬЮ перекроет
            // итоговое значение максимальной силой; как только ENGINE_IGNITION_KICK_DURATION
            // (1с) истечёт — оверрайд снимается и итог автоматически возвращается к этой базе.
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

        // Universal Engine Start (Starter phase, PISTON): моторы throttle_left/
        // right жёстко привязаны к Eng1/Eng2 (железо квадранта, поза за столом
        // тут ни при чём). В обычном режиме (оба устройства подключены) "чья
        // сторона" дублируется на джойстик — зависит от cfg.swap_hand_layout
        // (см. комментарий у поля и jet-ветку выше). Если подключено только
        // ОДНО устройство — весь вклад (оба борта) уходит целиком на него, а не
        // теряется наполовину в канале несуществующего устройства (см.
        // cfg.joystick_hw_connected/throttle_hw_connected — живой снимок из UI).
        if cfg.joystick_hw_connected && !cfg.throttle_hw_connected {
            transients_j += throttle_eng_vib_left + throttle_eng_vib_right;
        } else if cfg.throttle_hw_connected && !cfg.joystick_hw_connected {
            transients_t_left += throttle_eng_vib_left;
            transients_t_right += throttle_eng_vib_right;
        } else {
            transients_t_left += throttle_eng_vib_left;
            transients_t_right += throttle_eng_vib_right;
            transients_j += if cfg.swap_hand_layout { throttle_eng_vib_left } else { throttle_eng_vib_right };
        }

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

        // 3. АБСОЛЮТНЫЙ ОВЕРРАЙД: удар обжатия стоек шасси (touchdown), 0.12-0.22с.
        // Выполняется ПОСЛЕ clamp — гарантированно пробивает Ground Roll и любой
        // другой суммированный эффект в момент касания, вместо того чтобы тонуть
        // в их сумме (как было раньше, когда термин подмешивался в transients).
        // .max() на каждый канал по отдельности: нос/лево/право уже разведены
        // по трём разным моторам через touchdown_override_j/t_left/t_right
        // (см. блок детекции обжатия стоек выше), поэтому .max() здесь — просто
        // защита на случай, если несколько стоек делят один канал (split_touchdown
        // выключен, чекбоксы устройств пересекаются).
        if touchdown_override_j > 0.0 { final_joystick = final_joystick.max(touchdown_override_j); }
        if touchdown_override_t_left > 0.0 { final_throttle_left = final_throttle_left.max(touchdown_override_t_left); }
        if touchdown_override_t_right > 0.0 { final_throttle_right = final_throttle_right.max(touchdown_override_t_right); }

        // 4. АБСОЛЮТНЫЙ ОВЕРРАЙД: импульс воспламенения.
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