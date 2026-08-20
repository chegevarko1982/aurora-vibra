//! Эффекты этапа 1 поддержки War Thunder: Weapon1/Weapon2 (стрельба), Flaps,
//! Gear Transit & Doors. Маленький аналог `crate::rumble::RumbleEngine`, но
//! без сложностей MSFS-модели (несколько стоек шасси, 4 двигателя, пауза
//! симулятора и т.д.) — у War Thunder ровно 4 сигнала на входе (см.
//! `wt_link::vars::WtVars`).
//!
//! Weapon1/Weapon2 используют текстуру "гул с джиттером амплитуды",
//! подобранную и подтверждённую на живом железе (сессия калибровки
//! `src/bin/test_gun1.rs`, 2026-07-29) — см. `GunPreset` в `crate::types` и
//! его дефолты для weapon1/weapon2 в `WtConfig::default`. Маршрутизация
//! ЖЁСТКО зафиксирована в этом файле (не через `device_targets`): weapon1 →
//! только джойстик, weapon2 → только РУД (оба мотора одновременно) — так
//! подтверждено пользователем, разведение по рукам и есть смысл эффекта.
//! Несущая weapon1 (изначально подобранные 12.5 Гц) в `WtConfig::default`
//! понижена до 6.5 Гц: 12.5 Гц даёт пик короче интервала отправки штатного
//! HID-канала (`hid_worker`, 20 Гц/50мс) — алиасинг; 6.5 Гц проходит с
//! запасом (см. `thump_min_period_s` в `crate::rumble` — та же граница).
//! Flaps и Gear Transit & Doors переиспользуют ту же математику, что и
//! одноимённые MSFS-эффекты в `crate::rumble`.

use std::f64::consts::PI;
use std::time::Instant;

use crate::EffectsSnapshot;
use crate::rumble::RumbleOutput;
use crate::types::{GunPreset, WtConfig};

use super::aero_profiles::{self, StallProfile};
use super::overspeed_profiles;
use super::vars::WtVars;

/// Жёсткий потолок силы break-импульса (см. `BREAK_IMPULSE_PEAK_MULTIPLIER`
/// ниже) — единственное, что теперь масштабируется через `ceiling`/
/// `cfg.stall_ceiling`. Continuous-часть (подход и сам срыв, см.
/// `StallState::step`) по запросу пользователя всегда идёт по сырой шкале
/// 0..255 напрямую, потолок её не трогает — тактильное разделение фаз
/// строится на ХАРАКТЕРЕ сигнала (ровный рост vs пила), а не на громкости.
const WT_STALL_CEILING_HARD_CAP: f64 = 80.0;
/// Ширина линейного участка нарастания баффета перед сваливанием (град AoA) —
/// взято из пользовательского примера ((AoA-15)/4.9 при пороге сваливания 19.9°).
const PRE_STALL_RAMP_WIDTH_DEG: f64 = 4.9;
/// Частота пульсации ПОСЛЕ пересечения порога сваливания (Гц) — по живому
/// тесту нужно чёткое тактильное разделение: на подходе ровный линейный рост
/// (см. `StallState::step`), а в самом срыве/штопоре — резкая пульсация на
/// полную мощность (0..255), а не дальнейший плавный рост. Держится ПОКА
/// самолёт не выйдет из срыва (AoA не опустится ниже порога), не зависит от
/// глубины срыва. Форма (пауза/скачок, см. STALL_PULSE_DUTY) по живому тесту
/// подтверждена, эту правку — сжали её вдвое по времени: период 0.4с -> 0.2с
/// (пауза 0.1с + скачок на 255 на 0.1с).
const STALL_SAWTOOTH_HZ: f64 = 5.0;
/// Скважность пульсации в срыве — доля периода (`1 / STALL_SAWTOOTH_HZ`),
/// на которую мотор ВЫКЛЮЧЕН (пауза), прежде чем резко включиться на полную
/// мощность. По живому тесту заменили плавный рост 0→255 внутри периода на
/// чёткий прямоугольный импульс: пауза и работа мотора — поровну (0.5).
const STALL_PULSE_DUTY: f64 = 0.5;
/// Длительность гладкого break-импульса ("щелчок" в момент самого отрыва
/// потока), взводится один раз на восходящем фронте пересечения порога.
const BREAK_IMPULSE_DURATION_S: f64 = 0.25;
/// Пиковая сила break-импульса как множитель текущего потолка — чуть выше
/// continuous-баффета в этот же момент, чтобы ощущаться отдельным резким
/// событием на фоне нарастающей дрожи.
const BREAK_IMPULSE_PEAK_MULTIPLIER: f64 = 1.5;
/// Запас (град AoA) НИЖЕ порога сваливания, на который AoA должен опуститься,
/// прежде чем break-импульс взведётся заново — без этого в штопоре, где AoA
/// дрожит прямо у порога, импульс ретриггерится по многу раз в секунду и
/// смазывается в гул вместо одного чёткого щелчка на отрыве потока.
const BREAK_REARM_HYSTERESIS_DEG: f64 = 3.0;

/// Минимальный xorshift32 — та же реализация, что и в test_gun1.rs (только
/// для джиттера амплитуды текстуры, криптостойкость не нужна).
#[derive(Debug)]
struct Xorshift32(u32);

impl Xorshift32 {
    fn seeded(salt: u32) -> Self {
        let nanos = Instant::now().elapsed().subsec_nanos();
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

/// Умножение на частоту, а не деление на период (`t / (1.0 / f)`): период —
/// уже округлённое значение, деление на него добавляет второе округление, и
/// на границе скважности фаза начинает прыгать вокруг порога. Ровный по
/// построению ШИМ из-за этого выходил рваным — см. подробный комментарий у
/// `Shape::Pulse` в `custom_fx::engine`, там тот же дефект был найден на
/// пользовательском эффекте 5 Гц.
fn gun_cycle_index(t: f64, carrier_freq_hz: f64) -> i64 {
    (t * carrier_freq_hz).floor() as i64
}

/// Огибающая одного цикла несущей: скважность `preset.duty_pct` на пике (с
/// джиттером амплитуды и attack-рампой от начала очереди), иначе — нижний
/// уровень `preset.floor` (текстура "несколько стволов, не в идеальный
/// унисон"). `floor` дополнительно подрезается до `peak`, если тот вдруг
/// меньше (защита на случай, если пользователь когда-нибудь получит доступ
/// к этим полям в UI и поставит peak ниже floor).
fn gun_pulse(t: f64, fire_started_t: f64, preset: &GunPreset, cycle_jitter_mul: f64) -> f64 {
    let peak = preset.peak as f64;
    if peak <= 0.0 {
        return 0.0;
    }
    let carrier_freq_hz = (preset.carrier_freq_hz as f64).max(0.1);
    let duty = (preset.duty_pct as f64 / 100.0).clamp(0.0, 1.0);
    let attack_s = (preset.attack_ms as f64 / 1000.0).max(0.0);

    // См. gun_cycle_index выше — та же причина считать умножением.
    let phase = (t * carrier_freq_hz).fract();
    let since_fire = (t - fire_started_t).max(0.0);
    let attack_ramp = if attack_s <= 0.0 {
        1.0
    } else {
        (since_fire / attack_s).clamp(0.0, 1.0)
    };
    if phase < duty {
        (peak * cycle_jitter_mul * attack_ramp).clamp(0.0, 255.0)
    } else {
        (preset.floor as f64).min(peak)
    }
}

#[derive(Debug)]
struct GunState {
    was_firing: bool,
    fire_started_t: f64,
    last_cycle_idx: i64,
    cycle_jitter_mul: f64,
    rng: Xorshift32,
}

impl GunState {
    fn new(salt: u32) -> Self {
        Self {
            was_firing: false,
            fire_started_t: 0.0,
            last_cycle_idx: -1,
            cycle_jitter_mul: 1.0,
            rng: Xorshift32::seeded(salt),
        }
    }

    /// Возвращает силу эффекта на этот тик (0.0, если оружие не стреляет).
    fn step(&mut self, t: f64, firing: bool, preset: &GunPreset) -> f64 {
        if !firing {
            self.was_firing = false;
            return 0.0;
        }
        if !self.was_firing {
            self.fire_started_t = t;
            self.last_cycle_idx = -1;
        }
        self.was_firing = true;

        let carrier_freq_hz = (preset.carrier_freq_hz as f64).max(0.1);
        let jitter = (preset.jitter_pct as f64 / 100.0).clamp(0.0, 1.0);
        let cycle_idx = gun_cycle_index(t, carrier_freq_hz);
        if cycle_idx != self.last_cycle_idx {
            self.last_cycle_idx = cycle_idx;
            self.cycle_jitter_mul = 1.0 - jitter * self.rng.next_unit();
        }
        gun_pulse(t, self.fire_started_t, preset, self.cycle_jitter_mul)
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Stall/буффет срыва потока — захардкоженный профиль порогов, пока только
// для Bf 109 F-4 (см. wt_link::aero_profiles). Тактильное разделение фаз —
// через ХАРАКТЕР сигнала, не через громкость (по живому тесту): на подходе
// ровная линейная вибрация 0..255 без джиттера, в срыве — прямоугольный
// импульс с паузой (пауза/работа мотора по 0.5 периода, см. STALL_PULSE_DUTY)
// 0..255 (см. StallState::step). Гипотеза для живой проверки — см. план.
// ═══════════════════════════════════════════════════════════════════════

#[derive(Debug)]
struct StallState {
    /// Взведён ли break-импульс на следующее пересечение порога снизу вверх.
    /// Снимается сразу после срабатывания, возвращается только когда AoA
    /// опустится хотя бы на `BREAK_REARM_HYSTERESIS_DEG` ниже порога — иначе
    /// в штопоре, где AoA дрожит прямо у порога, импульс ретриггерился бы
    /// много раз подряд (см. константу выше).
    break_armed: bool,
    /// Момент времени взвода текущего break-импульса, `< 0.0` — не взведён.
    break_impulse_t0: f64,
}

impl StallState {
    fn new() -> Self {
        Self {
            break_armed: true,
            break_impulse_t0: -1.0,
        }
    }

    /// Возвращает интенсивность баффета (0.0, если самолёт вне профиля,
    /// ниже safety cutoff по IAS или AoA ниже начала пред-срывного участка).
    ///
    /// Модель — три чётко разделённых ощущения (см. план и живой тест):
    /// РОВНЫЙ линейный AoA-зависимый рост 0..255 на подходе к срыву, резкий
    /// break-импульс ровно в момент пересечения порога, и — ПОКА самолёт в
    /// срыве/штопоре — прямоугольная пульсация пауза/полная мощность (0/255),
    /// не зависящая от глубины срыва. Разделение — по характеру сигнала
    /// (ровный рост vs пауза+скачок), обе фазы идут по полной сырой шкале 0..255.
    fn step(&mut self, t: f64, vars: &WtVars, profile: &StallProfile, ceiling: f64) -> f64 {
        if vars.ias_kmh <= profile.ias_safety_cutoff_kmh as f64 {
            // Состояние break-импульса (armed/t0) НЕ трогаем: это низкий
            // порог "стоим на месте", кратких провалов IAS в реальном полёте
            // тут почти не бывает, но если случатся — не хотим ни ложно
            // пересбрасывать взвод, ни обрубать уже идущий импульс.
            return 0.0; // safety cutoff — не дребезжать на рулёжке/стоянке
        }

        let clean = profile.clean_stall_aoa_deg as f64;
        let landing_mid =
            (profile.landing_stall_aoa_deg.0 + profile.landing_stall_aoa_deg.1) as f64 / 2.0;
        let flap_frac = (vars.flaps_pct / 100.0).clamp(0.0, 1.0);
        // Интерполяция порога сваливания между "чистым" крылом (закрылки 0%)
        // и посадочной конфигурацией (закрылки 100%) по текущим закрылкам.
        let stall_aoa = clean - (clean - landing_mid) * flap_frac;
        let pre_stall_start = stall_aoa - PRE_STALL_RAMP_WIDTH_DEG;

        let is_stalled = vars.aoa_deg >= stall_aoa;
        // Взвод break-импульса — с гистерезисом: срабатывает на пересечении
        // порога снизу вверх, только если был взведён, и сразу снимается.
        // Возвращается только когда AoA опустится заметно (на
        // BREAK_REARM_HYSTERESIS_DEG) ниже порога — иначе в штопоре, где AoA
        // колеблется прямо у порога, импульс ретриггерился бы на каждом
        // мелком колебании и смазывался в гул вместо одного чёткого щелчка.
        if vars.aoa_deg < stall_aoa - BREAK_REARM_HYSTERESIS_DEG {
            self.break_armed = true;
        }
        if is_stalled && self.break_armed {
            self.break_impulse_t0 = t;
            self.break_armed = false;
        }

        let ceiling = ceiling.min(WT_STALL_CEILING_HARD_CAP);

        // Break-импульс: гладкая (без jitter) sin-огибающая поверх
        // continuous-члена, по образцу gear-lock bump — контраст подчёркивает
        // "вот отдельное резкое событие" ровно на отрыве потока.
        let break_term = if self.break_impulse_t0 >= 0.0 {
            let p = (t - self.break_impulse_t0) / BREAK_IMPULSE_DURATION_S;
            if (0.0..=1.0).contains(&p) {
                ceiling * BREAK_IMPULSE_PEAK_MULTIPLIER * (PI * p).sin()
            } else {
                self.break_impulse_t0 = -1.0;
                0.0
            }
        } else {
            0.0
        };

        // Continuous-часть — два ПРИНЦИПИАЛЬНО разных режима, не одна общая
        // кривая (по живому тесту, чтобы разделение фаз ощущалось чётко):
        let continuous_term = if is_stalled {
            // В срыве/штопоре — прямоугольная пульсация: пауза (0), затем
            // резкий скачок на полную мощность (255), НЕ ограниченная
            // `ceiling` и не зависящая от глубины срыва — держится, пока AoA
            // не опустится обратно ниже stall_aoa. См. STALL_SAWTOOTH_HZ,
            // STALL_PULSE_DUTY.
            let phase = (t * STALL_SAWTOOTH_HZ).fract();
            if phase < STALL_PULSE_DUTY { 0.0 } else { 255.0 }
        } else if vars.aoa_deg > pre_stall_start {
            // На подходе — РОВНАЯ вибрация без джиттера/несущей, линейно
            // растущая от 0 до 255 напрямую с AoA (не ограничена `ceiling`,
            // см. константу выше) — контраст с прямоугольной пульсацией в
            // срыве строится на характере сигнала (гладкий рост vs
            // пауза+скачок), не на объёме.
            let pre_progress = (vars.aoa_deg - pre_stall_start) / (stall_aoa - pre_stall_start);
            pre_progress.clamp(0.0, 1.0) * 255.0
        } else {
            0.0
        };

        if continuous_term <= 0.0 && break_term <= 0.0 {
            return 0.0;
        }

        continuous_term + break_term
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Engine Start/Stop — пуск и останов двигателя. Эффект МОНО: читаем только
// "RPM 1", раскладки по двигателям/сторонам нет — многомоторные борта
// запускают моторы синхронно (подтверждено пользователем), поэтому один
// сигнал идёт сразу на джойстик и оба мотора РУД. На устойчиво работающем
// двигателе эффект молчит (только транзиенты пуска/останова, по запросу
// пользователя) — постоянный фоновый гул тут не строим.
//
// Форма разобрана по живому логу полного цикла пуск→останов:
// wt_probe_sessions/session_20260729_151535.jsonl (Bf 109 F-4, 20 Гц):
//   0.00–27.90с  двигатель заглушен, RPM=0
//  27.90–30.25с  прокрутка стартером, ~15 об/мин/с
//  30.30–30.45с  СХВАТЫВАНИЕ, скачок ~570 об/мин/с (RPM 35→120)
//  30.45–33.25с  неровная раскрутка/флейр (RPM 120→485, манифолд "дышит")
//  33.25–39.55с  устойчивые холостые (RPM≈485, power≈1.3 л.с.)
//  39.60с        команда "стоп": power → 0.0, RPM ещё 485
//  39.60–57.80с  выбег винта, ПОЧТИ ЛИНЕЙНЫЙ спад (~26 об/мин/с), RPM→0
//
// Ключевое инженерное решение — НЕ пытаться заранее знать "холостые обороты
// этого борта" (они разные: 485 в этой сессии, 595 в другой): во время
// разгона частота/амплитуда ведутся от МОДУЛЯ производной RPM (насколько
// резво крутится двигатель прямо сейчас), а не от доли неизвестного заранее
// целевого RPM. Это одновременно снимает проблему разных бортов и даёт
// естественно попадающую в фазы кривую: полка на 120 об/мин (drpm/dt≈0)
// сама даёт минимум частоты/амплитуды, не требуя отдельного состояния.
// На выбеге, наоборот, `rpm_ref` уже ИЗВЕСТЕН (это RPM в момент, когда
// `power 1, hp` упал в ноль) — там частота/амплитуда честно ведутся долей
// rpm/rpm_ref. Все частоты капнуты 6.5 Гц — тот же потолок Найквиста, что и
// у несущей weapon1 (см. шапку файла): HID отправляет 20 Гц/50мс, пик короче
// интервала отправки алиасится.
//
// Амплитуда выбега ЛИНЕЙНО затухает до нуля вместе с частотой — оба
// параметра честно ведутся долей `rpm/rpm_ref`. v1 держала амплитуду почти
// постоянной (0.35-0.60 от peak, чтобы не уйти ниже порога страгивания
// ERM-моторов) и несла спад только частотой — по живому тесту на железе это
// ощущалось как "останов не удался", сам выбег не чувствовался (см. правку
// v2). В конце выбега винт получает ДВА отдельных рывка подряд, не один
// импульс — по тому же живому тесту нужно однозначно читаемое "вот винт
// встал". Триггер этой пары — НЕ ENGINE_RPM_ACTIVE_EPS (см. ниже), а
// отдельный ENGINE_STOP_TRIGGER_RPM: по живому тесту пара била с опозданием
// ~500мс относительно того момента, когда винт визуально уже выглядел
// остановившимся — RPM 1 в игре ещё какое-то время честно тикает единицами
// (см. хвост живого лога: 20→15→10→5→0 об/мин занимает ~0.8с), но глазом на
// таких оборотах вращение уже неотличимо от полной остановки. Порог 15
// об/мин даёт паре сработать примерно на 500-600мс раньше буквального нуля
// (при типичном темпе выбега ~26 об/мин/с в хвосте), синхронно с картинкой.
// ═══════════════════════════════════════════════════════════════════════

/// Порог "двигатель реально крутится хоть как-то" — используется для
/// детекта пуска (Off → Start) и как нижняя граница continuous-текстуры
/// выбега. НЕ используется для триггера финальной пары рывков — см.
/// ENGINE_STOP_TRIGGER_RPM выше.
const ENGINE_RPM_ACTIVE_EPS: f64 = 2.0;
/// Порог "винт выглядит остановившимся" — см. шапку раздела. Кандидат на
/// дальнейшую калибровку по живому тесту (значение подобрано по темпу спада
/// в записанном логе, не измерено напрямую по картинке в игре).
const ENGINE_STOP_TRIGGER_RPM: f64 = 15.0;
/// Окно устаканивания телеметрии после нового захода в бой/респавна —
/// см. `EngineState::step`, ветка `!self.initialized`. Пока оно не истекло,
/// любой признак "уже летит" (RPM или убранное шасси) фиксируется
/// немедленно и окончательно; честный пуск с нуля признаётся только если
/// ни один признак не сработал за всё это время.
const ENGINE_SPAWN_SETTLE_WINDOW_S: f64 = 1.0;
/// Порог "скачка" оборотов (об/мин/с) внутри окна устаканивания спауна —
/// отличает "телеметрия ещё не устаканилась, борт уже летит" (мгновенный
/// скачок на полное значение) от честной прокрутки стартером с нуля
/// (даже самое резкое реальное схватывание — ~570 об/мин/с). С большим
/// запасом выше любой реальной динамики двигателя.
const ENGINE_SPAWN_JUMP_RPM_S: f64 = 1000.0;
/// Сглаживание производной RPM (EMA) — сырые "RPM 1" целые, при опросе 20 Гц
/// одно округление даёт скачок ~20 об/мин/с шума на пустом месте.
const ENGINE_DRPM_DT_EMA_ALPHA: f64 = 0.35;
/// "Полная шкала" производной RPM для нормировки частоты/амплитуды разгона —
/// подобрана так, чтобы пик схватывания (~570 об/мин/с по логу) уверенно
/// насыщал кривую в 1.0, а медленная прокрутка (~15 об/мин/с) была у нижней
/// границы. Кандидат на калибровку на живом железе (см. план).
const ENGINE_DRPM_DT_FULL_SCALE: f64 = 300.0;
/// Порог производной RPM, взводящий одиночный импульс "схватывания" —
/// с запасом выше прокрутки (~20 об/мин/с) и ниже реального скачка (>200
/// об/мин/с по логу).
const ENGINE_CATCH_DRPM_DT: f64 = 150.0;
const ENGINE_CATCH_IMPULSE_DURATION_S: f64 = 0.30;
/// Импульс схватывания — на полную силу (см. решение пользователя: "чем
/// передавать замедление/ускорение" — частота; но само схватывание должно
/// ощущаться отдельным чётким ударом, не частью плавной кривой).
const ENGINE_CATCH_IMPULSE_PEAK_FRAC: f64 = 1.0;
const ENGINE_START_FREQ_MIN_HZ: f64 = 1.5;
const ENGINE_START_FREQ_MAX_HZ: f64 = 6.5;
const ENGINE_START_AMP_MIN: f64 = 0.35;
const ENGINE_START_AMP_MAX: f64 = 0.85;
/// Порог "производная почти нулевая" — используется и для короткой полки
/// прокрутки на 120 об/мин (не должна ложно засчитаться как "вышли на
/// холостые"), и для реального выхода на холостые.
const ENGINE_SETTLE_DRPM_DT_EPS: f64 = 15.0;
/// Окно стабильности перед тем, как считать "двигатель вышел на холостые".
/// НАМЕРЕННО больше самой долгой промежуточной полки разгона по живому логу
/// (0.9с на 120 об/мин) — иначе эта полка сама себя гасит как "холостые" и
/// съедает флейр (32.30–33.25с, самую заметную часть раскрутки).
const ENGINE_SETTLE_STABLE_WINDOW_S: f64 = 1.2;
const ENGINE_SETTLE_FADE_S: f64 = 0.8;
const ENGINE_CLIMB_JITTER: f64 = 0.4;
/// Мощность, ниже которой считаем "двигатель больше не тянет" — переход в
/// выбег. На живых холостых `power 1, hp` держится 1.3–1.4, к нулю падает
/// РОВНО в момент команды "стоп" (см. шапку раздела) — порог 0.5 берёт это
/// с запасом, не дребезжа на самих холостых.
const ENGINE_POWER_OFF_HP: f64 = 0.5;
const ENGINE_COAST_FREQ_MIN_HZ: f64 = 0.8;
const ENGINE_COAST_FREQ_MAX_HZ: f64 = 6.0;
/// v2 (по живому тесту): амплитуда выбега честно доходит до нуля — раньше
/// держали пол в 0.35 из опасения уйти ниже порога страгивания ERM, но на
/// железе это читалось как "выбег не чувствуется", а не как "тише".
const ENGINE_COAST_AMP_MIN: f64 = 0.0;
const ENGINE_COAST_AMP_MAX: f64 = 0.60;
const ENGINE_COAST_JITTER_MAX: f64 = 0.25;
/// Длительность КАЖДОГО из двух рывков полной остановки винта (не одного
/// импульса — см. шапку раздела).
const ENGINE_STOP_IMPULSE_DURATION_S: f64 = 0.15;
/// Пауза между первым и вторым рывком.
const ENGINE_STOP_IMPULSE_GAP_S: f64 = 0.15;
const ENGINE_STOP_IMPULSE_PEAK_FRAC: f64 = 0.55;

// Проверка потолка Найквиста — на этапе компиляции, а не в рантайм-тесте
// (см. ту же границу 6.5 Гц у несущей weapon1, шапка файла).
const _: () = assert!(ENGINE_START_FREQ_MAX_HZ <= 6.5);
const _: () = assert!(ENGINE_COAST_FREQ_MAX_HZ <= 6.5);

fn lerp(a: f64, b: f64, frac: f64) -> f64 {
    a + (b - a) * frac.clamp(0.0, 1.0)
}

/// Асимметричный пилообразный импульс (быстрая атака 25% периода, долгий
/// спад 75%) — та же форма, что у поршневого кранча в MSFS-движке
/// (`crate::rumble`, PISTON_CRANK), переиспользована здесь как текстура
/// "толчок от цилиндра", а не гладкая синусоида.
fn engine_pulse_shape(t: f64, freq_hz: f64) -> f64 {
    let freq = freq_hz.max(0.1);
    let phase = (t * freq).rem_euclid(1.0);
    if phase < 0.25 {
        phase / 0.25
    } else {
        (1.0 - (phase - 0.25) / 0.75).max(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnginePhase {
    /// Двигатель заглушен, RPM около нуля — эффект молчит.
    Off,
    /// Прокрутка стартером → схватывание → раскрутка до холостых.
    Start,
    /// Устойчиво работает — эффект молчит (только транзиенты).
    Run,
    /// Выбег винта после команды "стоп" (RPM падает, мощности уже нет).
    Coast,
}

#[derive(Debug)]
struct EngineState {
    /// Решение "это пуск с нуля или борт уже летит/работает" ещё не принято
    /// окончательно — используется, чтобы НЕ принять уже работающий на споне
    /// двигатель (см. живой лог session_20260728_204134.jsonl, первый же тик
    /// RPM=2301) за только что запущенный. Решение откладывается до конца
    /// окна устаканивания телеметрии (см. `spawn_deadline`), а не
    /// принимается по одному-единственному тику — см. шапку раздела.
    initialized: bool,
    /// `< 0.0` — окно устаканивания телеметрии после нового захода в бой ещё
    /// не открыто. Иначе — дедлайн (t + ENGINE_SPAWN_SETTLE_WINDOW_S), до
    /// которого решение "пуск или уже летит" остаётся отложенным.
    spawn_deadline: f64,
    phase: EnginePhase,
    prev_rpm: f64,
    prev_t: f64,
    drpm_dt_smoothed: f64,
    catch_fired: bool,
    /// `< 0.0` — импульс схватывания не взведён/уже отыгран.
    catch_pulse_t0: f64,
    /// `< 0.0` — производная сейчас не в пределах ENGINE_SETTLE_DRPM_DT_EPS
    /// непрерывно; иначе — момент, с которого началась стабильность.
    stable_since: f64,
    /// `< 0.0` — плавный уход в тишину после выхода на холостые ещё не начат.
    settle_fade_t0: f64,
    /// RPM в момент перехода Run → Coast (единственное место, где нужен
    /// "холостой RPM этого борта" — тут он уже ИЗВЕСТЕН, а не предсказывается).
    rpm_ref: f64,
    /// `false` — в текущем Run ещё ни разу не видели `power_1_hp` выше
    /// ENGINE_POWER_OFF_HP, значит переход в Coast по этому полю пока не
    /// проверяется. Нужно, потому что при тихом усыновлении Run (см.
    /// `step()`, борт спавнится уже летящим) `power 1, hp` у телеметрии
    /// борта иногда на пару тиков отстаёт от реальной раскрутки винта и
    /// читается как 0 — без этой защёлки такой тик мгновенно и ложно
    /// засчитался бы как "мощность пропала — выбег", хотя борт на самом
    /// деле продолжает лететь на полном ходу (живой пример —
    /// session_20260729_202028.jsonl, A6M3 Zero). Взводится в `true` сразу,
    /// если мощность уже легитимна В МОМЕНТ усыновления (например, честный
    /// холостой ход, где `power_1_hp` не успевает провиснуть ни на один
    /// тик) — тогда останов, случившийся сразу же, по-прежнему ловится.
    /// Честный переход Start → Run (см. `step_start`) эту защёлку не трогает
    /// — холостые там уже подтверждены стабильностью производной.
    run_power_confirmed: bool,
    /// `< 0.0` — импульс полной остановки не взведён/уже отыгран.
    stop_pulse_t0: f64,
    last_cycle_idx: i64,
    cycle_jitter_mul: f64,
    rng: Xorshift32,
}

impl EngineState {
    fn new() -> Self {
        Self {
            initialized: false,
            spawn_deadline: -1.0,
            phase: EnginePhase::Off,
            prev_rpm: 0.0,
            prev_t: 0.0,
            drpm_dt_smoothed: 0.0,
            catch_fired: false,
            catch_pulse_t0: -1.0,
            stable_since: -1.0,
            settle_fade_t0: -1.0,
            rpm_ref: 0.0,
            run_power_confirmed: false,
            stop_pulse_t0: -1.0,
            last_cycle_idx: i64::MIN,
            cycle_jitter_mul: 1.0,
            rng: Xorshift32::seeded(0xC2B2_AE35),
        }
    }

    /// Сбрасывает детектор пуска к состоянию "решение ещё не принято" —
    /// вызывается при входе в новую сессию боя (`vars.in_mission`: false →
    /// true), в т.ч. при респавне на другом борту. Без этого борт,
    /// заспавнившийся в новом бою с уже работающим двигателем, принял бы его
    /// за только что заведённый: текущая фаза (Off/Coast) осталась бы с
    /// прошлого вылета, независимо от того, что реально происходит с новым
    /// бортом.
    fn reset_for_new_spawn(&mut self) {
        self.initialized = false;
        self.spawn_deadline = -1.0;
    }

    fn step(&mut self, t: f64, vars: &WtVars, peak: f64) -> f64 {
        let rpm = vars.rpm_1;

        if !self.initialized {
            // Решение "пуск с нуля или борт уже летит/работает" по живому
            // тесту НЕЛЬЗЯ принимать по одному тику: на самом первом ответе
            // после захода в бой RPM и шасси иногда ещё не устаканились
            // (борт/техника только грузится), и если оба в этот момент
            // случайно читаются как "ещё не летит", решение "Off" уже
            // зафиксировано бы навсегда — а на следующем тике, когда
            // реальные RPM/шасси придут, Off → Start их уже не перепроверяет
            // (см. match ниже) и ловит пуск ложно. Поэтому держим окно
            // ENGINE_SPAWN_SETTLE_WINDOW_S: пока оно не истекло, шасси уже
            // убранное ИЛИ обороты, появившиеся СКАЧКОМ (не честным разгоном
            // с нуля — см. ниже), немедленно и окончательно фиксируют "уже
            // летит" (Run) — жёстко, по требованию пользователя ("если
            // Gear=0 — эффект не включать"). Только если окно закончилось, а
            // ни один из признаков ни разу не сработал — фиксируем честный
            // пуск с нуля (Off).
            //
            // "Скачок" — обороты изменились быстрее ENGINE_SPAWN_JUMP_RPM_S
            // за секунду. Настоящая прокрутка стартером растёт плавно
            // (~15 об/мин/с в живом логе, самое резкое реальное схватывание —
            // ~570 об/мин/с), а "телеметрия ещё не устаканилась" всегда даёт
            // мгновенный скачок на полное значение (напр. 0 → 2021 за один
            // тик). Без этого различия голый rpm > EPS ловил в ловушку и
            // честный медленный пуск, случайно начавшийся в первую секунду
            // после захода в бой (см. тест engine_catch_impulse_fires_once_per_start_cycle).
            let dt_settle = (t - self.prev_t).max(1e-3);
            let jump_rpm_s = (rpm - self.prev_rpm).abs() / dt_settle;
            if self.spawn_deadline < 0.0 {
                self.spawn_deadline = t + ENGINE_SPAWN_SETTLE_WINDOW_S;
            }
            if vars.gear_pct <= GEAR_LOCKED_THRESHOLD_PCT
                || (rpm > ENGINE_RPM_ACTIVE_EPS && jump_rpm_s > ENGINE_SPAWN_JUMP_RPM_S)
            {
                self.initialized = true;
                self.phase = EnginePhase::Run;
                self.run_power_confirmed = vars.power_1_hp > ENGINE_POWER_OFF_HP;
            } else if t >= self.spawn_deadline {
                self.initialized = true;
                self.phase = EnginePhase::Off;
            }
            self.prev_rpm = rpm;
            self.prev_t = t;
            return 0.0;
        }

        let dt = (t - self.prev_t).max(1e-3);
        let drpm_dt = (rpm - self.prev_rpm) / dt;
        // Нормировка коэффициента EMA по фактическому dt (см.
        // crate::timing::ema_blend_for_dt) — без неё учащение тика движка
        // (см. wt_link/worker.rs) укорачивало бы постоянную времени этого
        // сглаживания и сдвигало бы откалиброванный на живом железе порог
        // "подхвата" ENGINE_CATCH_DRPM_DT. На опорной частоте (dt == 0.05)
        // даёт исходный ENGINE_DRPM_DT_EMA_ALPHA без изменений.
        // ВНИМАНИЕ на конвенцию: строкой ниже alpha умножается на РАЗНОСТЬ
        // (`+= alpha * (x - y)`), то есть это вес НОВОГО значения —
        // отсюда `ema_blend_for_dt`, а НЕ `ema_retain_for_dt` (та для
        // записи `y = y*alpha + x*(1-alpha)`, как в custom_fx). На опорной
        // частоте обе дают одно и то же, поэтому подмена не ловится
        // тестами со штатным шагом 50 мс — см. doc-комментарий
        // crate::timing::ema_blend_for_dt.
        let drpm_dt_alpha = crate::timing::ema_blend_for_dt(ENGINE_DRPM_DT_EMA_ALPHA, dt);
        self.drpm_dt_smoothed += drpm_dt_alpha * (drpm_dt - self.drpm_dt_smoothed);
        self.prev_rpm = rpm;
        self.prev_t = t;

        match self.phase {
            EnginePhase::Off => {
                if rpm > ENGINE_RPM_ACTIVE_EPS && vars.gear_pct > GEAR_LOCKED_THRESHOLD_PCT {
                    self.phase = EnginePhase::Start;
                    self.catch_fired = false;
                    self.catch_pulse_t0 = -1.0;
                    self.stable_since = -1.0;
                    self.settle_fade_t0 = -1.0;
                    self.last_cycle_idx = i64::MIN;
                    // Не теряем сам тик пересечения нуля — считаем разгон
                    // сразу для него же, а не со следующего опроса.
                    self.step_start(t, peak)
                } else if rpm > ENGINE_RPM_ACTIVE_EPS {
                    // Обороты появились, но шасси уже убрано — это не пуск
                    // на земле (с убранным шасси не заводятся), а телеметрия
                    // "долетела" уже после того, как окно спауна (см. выше)
                    // успело закрыться на Off — например, у некоторых бортов
                    // между входом в бой (valid=true) и реальной передачей
                    // управления проходит больше секунды в неподвижной
                    // превью-позе. Тихо принимаем, что мотор уже работал.
                    self.phase = EnginePhase::Run;
                    self.run_power_confirmed = vars.power_1_hp > ENGINE_POWER_OFF_HP;
                    0.0
                } else {
                    0.0
                }
            }
            EnginePhase::Start => self.step_start(t, peak),
            EnginePhase::Run => {
                if !self.run_power_confirmed {
                    if vars.power_1_hp > ENGINE_POWER_OFF_HP {
                        self.run_power_confirmed = true;
                    }
                    0.0
                } else if vars.power_1_hp <= ENGINE_POWER_OFF_HP {
                    self.phase = EnginePhase::Coast;
                    self.rpm_ref = rpm.max(1.0);
                    self.stop_pulse_t0 = -1.0;
                    self.last_cycle_idx = i64::MIN;
                    // Аналогично: тик, на котором мощность обнулилась, уже
                    // считаем выбегом, а не тратим впустую на переключение.
                    self.step_coast(t, rpm, peak)
                } else {
                    0.0
                }
            }
            EnginePhase::Coast => self.step_coast(t, rpm, peak),
        }
    }

    /// Прокрутка → схватывание → раскрутка до холостых. Частота/амплитуда
    /// ведутся от МОДУЛЯ сглаженной производной RPM (см. шапку раздела),
    /// импульс схватывания — отдельный одиночный полусинус поверх кривой.
    fn step_start(&mut self, t: f64, peak: f64) -> f64 {
        if !self.catch_fired && self.drpm_dt_smoothed > ENGINE_CATCH_DRPM_DT {
            self.catch_fired = true;
            self.catch_pulse_t0 = t;
        }

        let ratio = (self.drpm_dt_smoothed.abs() / ENGINE_DRPM_DT_FULL_SCALE).clamp(0.0, 1.0);
        let freq = lerp(ENGINE_START_FREQ_MIN_HZ, ENGINE_START_FREQ_MAX_HZ, ratio);
        let amp = lerp(ENGINE_START_AMP_MIN, ENGINE_START_AMP_MAX, ratio);

        let stable = self.drpm_dt_smoothed.abs() < ENGINE_SETTLE_DRPM_DT_EPS;
        if stable {
            if self.stable_since < 0.0 {
                self.stable_since = t;
            }
        } else {
            self.stable_since = -1.0;
            self.settle_fade_t0 = -1.0;
        }
        if self.settle_fade_t0 < 0.0
            && self.stable_since >= 0.0
            && t - self.stable_since >= ENGINE_SETTLE_STABLE_WINDOW_S
        {
            self.settle_fade_t0 = t;
        }

        let cycle_idx = gun_cycle_index(t, freq);
        if cycle_idx != self.last_cycle_idx {
            self.last_cycle_idx = cycle_idx;
            self.cycle_jitter_mul = 1.0 + ENGINE_CLIMB_JITTER * (self.rng.next_unit() * 2.0 - 1.0);
        }

        let climb_term = engine_pulse_shape(t, freq) * amp * peak * self.cycle_jitter_mul.max(0.0);

        let continuous_term = if self.settle_fade_t0 >= 0.0 {
            let p = (t - self.settle_fade_t0) / ENGINE_SETTLE_FADE_S;
            if p >= 1.0 {
                self.phase = EnginePhase::Run;
                0.0
            } else {
                climb_term * (1.0 - p)
            }
        } else {
            climb_term
        };

        let catch_term = if self.catch_pulse_t0 >= 0.0 {
            let p = (t - self.catch_pulse_t0) / ENGINE_CATCH_IMPULSE_DURATION_S;
            if (0.0..=1.0).contains(&p) {
                peak * ENGINE_CATCH_IMPULSE_PEAK_FRAC * (PI * p).sin()
            } else {
                self.catch_pulse_t0 = -1.0;
                0.0
            }
        } else {
            0.0
        };

        (continuous_term + catch_term).clamp(0.0, 255.0)
    }

    /// Выбег винта: частота ведётся долей `rpm/rpm_ref` (в отличие от
    /// разгона, тут целевой RPM уже известен — захвачен в момент перехода
    /// Run → Coast), амплитуда линейно затухает до нуля той же долей.
    /// Финальная пара рывков взводится на ENGINE_STOP_TRIGGER_RPM (винт
    /// визуально уже стоит), а не на буквальном RPM≈0 — см. шапку раздела.
    fn step_coast(&mut self, t: f64, rpm: f64, peak: f64) -> f64 {
        let mut term = 0.0;

        if rpm > ENGINE_STOP_TRIGGER_RPM {
            let ratio = (rpm / self.rpm_ref).clamp(0.0, 1.0);
            let freq = lerp(ENGINE_COAST_FREQ_MIN_HZ, ENGINE_COAST_FREQ_MAX_HZ, ratio);
            let amp = lerp(ENGINE_COAST_AMP_MIN, ENGINE_COAST_AMP_MAX, ratio);
            let jitter_mag = ENGINE_COAST_JITTER_MAX * (1.0 - ratio);

            let cycle_idx = gun_cycle_index(t, freq);
            if cycle_idx != self.last_cycle_idx {
                self.last_cycle_idx = cycle_idx;
                self.cycle_jitter_mul = 1.0 + jitter_mag * (self.rng.next_unit() * 2.0 - 1.0);
            }

            term = engine_pulse_shape(t, freq) * amp * peak * self.cycle_jitter_mul.max(0.0);
        } else if self.stop_pulse_t0 < 0.0 {
            self.stop_pulse_t0 = t;
        }

        if self.stop_pulse_t0 >= 0.0 {
            let elapsed = t - self.stop_pulse_t0;
            match engine_stop_double_jerk_shape(elapsed) {
                Some(shape) => term += peak * ENGINE_STOP_IMPULSE_PEAK_FRAC * shape,
                None => self.phase = EnginePhase::Off,
            }
        }

        term.clamp(0.0, 255.0)
    }
}

/// Огибающая финального "винт встал" — ДВА одинаковых полусинусных рывка
/// подряд с паузой между ними (не один импульс, см. шапку раздела: по
/// живому тесту двойной толчок читается однозначно, а один — смазанно).
/// `None` — окно обоих рывков закончилось, останов эффекта.
fn engine_stop_double_jerk_shape(elapsed: f64) -> Option<f64> {
    let d = ENGINE_STOP_IMPULSE_DURATION_S;
    let gap = ENGINE_STOP_IMPULSE_GAP_S;
    if elapsed < d {
        Some((PI * (elapsed / d)).sin())
    } else if elapsed < d + gap {
        Some(0.0)
    } else if elapsed < 2.0 * d + gap {
        Some((PI * ((elapsed - d - gap) / d)).sin())
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Flaps — та же математика, что и MSFS-эффект в crate::rumble (округление
// до целого %, детект изменения кадр-к-кадру, короткое окно активности,
// 25 Гц полу-синусный ШИМ, чередование throttle-каналов раз в 500мс).
// ═══════════════════════════════════════════════════════════════════════
const FLAPS_BUMP_DURATION_S: f64 = 0.15;

// ═══════════════════════════════════════════════════════════════════════
// Gear Transit & Doors — гул движения (та же ритмика, что и в MSFS-версии,
// см. crate::rumble) + удар фиксации на замке при достижении 0%/100%
// (та же attack/decay синус-огибающая, что и у "Gear Bump" в MSFS, сила —
// cfg.gear_peak, длительность — GEAR_LOCK_BUMP_DURATION_S).
// ═══════════════════════════════════════════════════════════════════════
const GEAR_LOCK_BUMP_DURATION_S: f64 = 0.8;
const GEAR_LOCKED_THRESHOLD_PCT: f64 = 0.5; // клиренс вокруг 0%/100% — считается "на замке"

#[derive(Debug)]
pub struct WtRumbleState {
    weapon1: GunState,
    weapon2: GunState,
    stall: StallState,
    engine: EngineState,
    /// Отслеживает фронт `vars.in_mission` (false → true) — новый заход в
    /// бой/респавн, ПОСЛЕ которого состояние двигателя с прошлого вылета
    /// (например, phase=Off, если тот борт успел заглушить мотор) больше не
    /// действительно: новый борт может спавниться с уже работающим
    /// двигателем, и это не пуск. См. EngineState::reset_for_new_spawn.
    engine_was_in_mission: bool,

    last_flaps_pct_rounded: i32,
    flaps_active_until_t: f64,

    gear_initialized: bool,
    prev_gear_pct: f64,
    gear_lock_t0: f64,
    gear_lock_t1: f64,
    gear_lock_peak: f64,
}

impl Default for WtRumbleState {
    fn default() -> Self {
        Self::new()
    }
}

impl WtRumbleState {
    pub fn new() -> Self {
        Self {
            weapon1: GunState::new(0x9E3779B9),
            weapon2: GunState::new(0x85EBCA6B),
            stall: StallState::new(),
            engine: EngineState::new(),
            engine_was_in_mission: false,
            last_flaps_pct_rounded: i32::MIN,
            flaps_active_until_t: -1.0,
            gear_initialized: false,
            prev_gear_pct: 0.0,
            gear_lock_t0: -1.0,
            gear_lock_t1: -1.0,
            gear_lock_peak: 0.0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn step(&mut self, vars: &WtVars, cfg: &WtConfig, hold: bool) -> RumbleOutput {
        let mut effects = EffectsSnapshot::default();

        // Новый заход в бой (в т.ч. респавн на другом борту) — состояние
        // двигателя с прошлого вылета не описывает текущий борт, каким бы
        // оно ни было (Off, Run, Coast). Отслеживаем ДО early-return по
        // !in_mission/hold, иначе фронт false→true никогда не поймать: пока
        // не в бою, step() выходит раньше и это поле не обновилось бы.
        if vars.in_mission && !self.engine_was_in_mission {
            self.engine.reset_for_new_spawn();
        }
        self.engine_was_in_mission = vars.in_mission;

        if hold || !vars.in_mission {
            return RumbleOutput {
                joystick_intensity: 0,
                throttle_left_intensity: 0,
                throttle_right_intensity: 0,
                effects,
            };
        }

        let t = vars.t;
        let dt_ = &cfg.device_targets;

        let mut joystick: f64 = 0.0;
        let mut throttle_left: f64 = 0.0;
        let mut throttle_right: f64 = 0.0;

        // --- Weapon1 / Weapon2 ---
        // Маршрутизация жёсткая (см. комментарий в шапке файла): weapon1 →
        // ТОЛЬКО джойстик, weapon2 → ТОЛЬКО РУД, сразу оба мотора. Не через
        // cfg.device_targets — WtDeviceTargets этих полей больше не содержит.
        let w1_term = if cfg.weapon1_enabled {
            self.weapon1.step(t, vars.weapon1_firing, &cfg.weapon1_gun)
        } else {
            0.0
        };
        let w2_term = if cfg.weapon2_enabled {
            self.weapon2.step(t, vars.weapon2_firing, &cfg.weapon2_gun)
        } else {
            0.0
        };
        effects.wt_weapon1_active = cfg.weapon1_enabled && vars.weapon1_firing;
        effects.wt_weapon2_active = cfg.weapon2_enabled && vars.weapon2_firing;

        joystick += w1_term;
        throttle_left += w2_term;
        throttle_right += w2_term;

        // --- Stall/буффет срыва потока (см. StallState выше) ---
        // Профиль ищется по имени борта каждый тик — дешёвая операция
        // (константный список из одного элемента в v1), кэш не нужен.
        let stall_term = if cfg.stall_enabled {
            aero_profiles::match_profile(&vars.vehicle_type)
                .map(|profile| self.stall.step(t, vars, profile, cfg.stall_ceiling as f64))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        effects.stall_active = stall_term > 0.0;
        if dt_.stall.enable_joystick {
            joystick += stall_term;
        }
        if dt_.stall.enable_throttle {
            throttle_left += stall_term;
            throttle_right += stall_term;
        }

        // --- Overspeed (Vne / Vfe) ---
        // Порог берётся точным совпадением по имени борта из таблицы
        // overspeed_profiles (см. модуль) — в отличие от stall, фолбэка на
        // "усреднённый" борт нет: таблица покрывает практически весь ростер,
        // отсутствие записи = эффект молчит. Если выпущены закрылки,
        // действующий предел ужесточается до их собственного порога
        // разрушения (см. overspeed_profiles::effective_limit_kmh) — без
        // отдельного тумблера, это то же превышение скорости, просто с
        // более строгим порогом. Шасси в этот предел НЕ входит — у него
        // отдельный эффект Gear overspeed ниже (другое окно нарастания,
        // свой тумблер).
        let overspeed_term = if cfg.overspeed_enabled {
            let base_vne = overspeed_profiles::vne_kmh_for(&vars.vehicle_type);
            overspeed_profiles::effective_limit_kmh(&vars.vehicle_type, base_vne, vars.flaps_pct)
                .map(|limit| {
                    overspeed_profiles::intensity(
                        vars.ias_kmh,
                        limit as f64,
                        cfg.overspeed_ceiling as f64,
                    )
                })
                .unwrap_or(0.0)
        } else {
            0.0
        };
        effects.wt_overspeed_active = overspeed_term > 0.0;
        if dt_.overspeed.enable_joystick {
            joystick += overspeed_term;
        }
        if dt_.overspeed.enable_throttle {
            throttle_left += overspeed_term;
            throttle_right += overspeed_term;
        }

        // --- Gear overspeed (Vlo) ---
        // Отдельный от общего Vne-эффекта: срабатывает только пока шасси
        // выпущено (>0.5%, тот же клиренс, что у GEAR_LOCKED_THRESHOLD_PCT),
        // порог — собственная скорость разрушения шасси борта, окно
        // нарастания шире (20 км/ч по требованию пользователя, вместо 10
        // у общего Vne).
        let gear_overspeed_term =
            if cfg.gear_overspeed_enabled && vars.gear_pct > GEAR_LOCKED_THRESHOLD_PCT {
                overspeed_profiles::gear_kmh_for(&vars.vehicle_type)
                    .map(|limit| {
                        overspeed_profiles::gear_intensity(
                            vars.ias_kmh,
                            limit as f64,
                            cfg.gear_overspeed_ceiling as f64,
                        )
                    })
                    .unwrap_or(0.0)
            } else {
                0.0
            };
        effects.wt_gear_overspeed_active = gear_overspeed_term > 0.0;
        if dt_.gear_overspeed.enable_joystick {
            joystick += gear_overspeed_term;
        }
        if dt_.gear_overspeed.enable_throttle {
            throttle_left += gear_overspeed_term;
            throttle_right += gear_overspeed_term;
        }

        // --- Engine Start/Stop ---
        // Моно (см. шапку раздела EngineState выше): одно значение сразу на
        // джойстик и оба мотора РУД — раскладки по двигателям/сторонам нет.
        //
        // Жёсткое правило по требованию пользователя (после разбора живых
        // логов 29.07.2026): весь эффект целиком отдаётся, только пока шасси
        // выпущено — `self.engine.step` всё равно вызывается каждый тик (не
        // пропускаем), чтобы внутренняя машина состояний (окно устаканивания
        // спауна, защёлка run_power_confirmed, фаза Coast) не ловила рваный
        // `dt`/скачок RPM на границе уборки-выпуска шасси — глушится только
        // возвращаемая амплитуда. Да, это заодно гасит легитимный боевой
        // выбег (сброс газа/повреждение двигателя в полёте, шасси убрано) —
        // осознанный компромисс первой версии, можно уточнить позже.
        let engine_raw = if cfg.engine_start_enabled {
            self.engine.step(t, vars, cfg.engine_start_peak as f64)
        } else {
            0.0
        };
        let engine_term = if vars.gear_pct > GEAR_LOCKED_THRESHOLD_PCT {
            engine_raw
        } else {
            0.0
        };
        effects.engine_start_active = engine_term > 0.0;
        if dt_.engine_start.enable_joystick {
            joystick += engine_term;
        }
        if dt_.engine_start.enable_throttle {
            throttle_left += engine_term;
            throttle_right += engine_term;
        }

        // --- Flaps ---
        let flaps_pct_rounded = vars.flaps_pct.round() as i32;
        let pct_changed = self.last_flaps_pct_rounded != i32::MIN
            && self.last_flaps_pct_rounded != flaps_pct_rounded;
        self.last_flaps_pct_rounded = flaps_pct_rounded;
        if pct_changed {
            self.flaps_active_until_t = t + FLAPS_BUMP_DURATION_S;
        }
        let flaps_is_moving = t < self.flaps_active_until_t;

        if cfg.flaps_enabled && flaps_is_moving {
            let max_amplitude = (cfg.flaps_peak as f64 / 255.0).clamp(0.01, 0.8);
            let fixed_period = 0.04; // 25 Гц
            let cycle = (t / fixed_period).fract();
            let oscillation = (PI * cycle).sin();
            let flaps_term = max_amplitude * 255.0 * oscillation;

            // Чередуем throttle-канал каждые 500мс — та же текстура, что и в
            // MSFS-версии эффекта (см. crate::rumble).
            let is_left_phase = (t / 0.5).floor() as i64 % 2 == 0;

            if dt_.flaps.enable_joystick {
                joystick += flaps_term;
            }
            if dt_.flaps.enable_throttle {
                if is_left_phase {
                    throttle_left += flaps_term;
                } else {
                    throttle_right += flaps_term;
                }
            }
            effects.flaps_bump_active = true;
        } else {
            effects.flaps_bump_active = false;
        }

        // --- Gear Transit & Doors ---
        if !self.gear_initialized {
            // Первый кадр: не считаем стартовое положение шасси "движением"
            // или "переходом на замок" — иначе борт, заспавнившийся сразу с
            // убранным/выпущенным шасси, дал бы ложный удар фиксации.
            self.prev_gear_pct = vars.gear_pct;
            self.gear_initialized = true;
        }

        let gear_is_moving = vars.gear_pct > GEAR_LOCKED_THRESHOLD_PCT
            && vars.gear_pct < 100.0 - GEAR_LOCKED_THRESHOLD_PCT
            && (vars.gear_pct - self.prev_gear_pct).abs() >= 0.01;

        let now_locked = vars.gear_pct <= GEAR_LOCKED_THRESHOLD_PCT
            || vars.gear_pct >= 100.0 - GEAR_LOCKED_THRESHOLD_PCT;
        let prev_locked = self.prev_gear_pct <= GEAR_LOCKED_THRESHOLD_PCT
            || self.prev_gear_pct >= 100.0 - GEAR_LOCKED_THRESHOLD_PCT;

        let mut gear_term: f64 = 0.0;
        if cfg.gear_transit_enabled {
            if now_locked && !prev_locked {
                self.gear_lock_t0 = t;
                self.gear_lock_t1 = t + GEAR_LOCK_BUMP_DURATION_S;
                self.gear_lock_peak = cfg.gear_peak as f64;
            }

            if gear_is_moving {
                // Тот же ритм гула, что и у MSFS Gear Transit (см. crate::rumble):
                // редкие "тяжёлые" толчки раз в 3 такта, лёгкие — между ними.
                let beat_duration = 60.0 / 80.0;
                let current_beat = t / beat_duration;
                let beat_phase = current_beat.fract();
                let beat_index = (current_beat.floor() as i64) % 3;
                if beat_index == 0 {
                    if beat_phase < 0.35 {
                        gear_term += 40.0;
                    }
                } else if beat_phase < 0.15 {
                    gear_term += 15.0;
                }
            }

            let lock_active =
                t >= self.gear_lock_t0 && t <= self.gear_lock_t1 && self.gear_lock_peak > 0.0;
            if lock_active {
                let p = ((t - self.gear_lock_t0) / (self.gear_lock_t1 - self.gear_lock_t0))
                    .clamp(0.0, 1.0);
                gear_term += self.gear_lock_peak * (PI * p).sin();
            }

            effects.gear_transit_active = gear_is_moving || lock_active;

            if dt_.gear_transit.enable_joystick {
                joystick += gear_term;
            }
            if dt_.gear_transit.enable_throttle {
                throttle_left += gear_term;
                throttle_right += gear_term;
            }
        } else {
            effects.gear_transit_active = false;
        }

        self.prev_gear_pct = vars.gear_pct;

        RumbleOutput {
            joystick_intensity: joystick.clamp(0.0, 255.0).round() as u8,
            throttle_left_intensity: throttle_left.clamp(0.0, 255.0).round() as u8,
            throttle_right_intensity: throttle_right.clamp(0.0, 255.0).round() as u8,
            effects,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WtConfig {
        WtConfig::default()
    }

    fn vars() -> WtVars {
        WtVars {
            t: 0.0,
            in_mission: true,
            weapon1_firing: false,
            weapon2_firing: false,
            flaps_pct: 0.0,
            gear_pct: 100.0,
            weapon1_ammo: None,
            weapon2_ammo: None,
            vehicle_type: String::new(),
            speed_kt: 0.0,
            altitude_ft: 0.0,
            ias_kmh: 0.0,
            aoa_deg: 0.0,
            wx_deg_s: 0.0,
            rpm_1: 0.0,
            power_1_hp: 0.0,
        }
    }

    #[test]
    fn silent_when_not_in_mission() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        v.in_mission = false;
        v.weapon1_firing = true;
        let out = engine.step(&v, &cfg(), false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn silent_on_hold() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        v.weapon1_firing = true;
        let out = engine.step(&v, &cfg(), true);
        assert_eq!(out.joystick_intensity, 0);
    }

    #[test]
    fn weapon1_firing_routes_to_joystick_only() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        v.weapon1_firing = true;
        // Несколько тиков (20 Гц), чтобы пройти attack-рампу (default attack_ms=41).
        let mut out = engine.step(&v, &cfg(), false);
        for i in 1..10 {
            v.t = i as f64 * 0.02;
            out = engine.step(&v, &cfg(), false);
        }
        assert!(out.joystick_intensity > 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn weapon2_firing_routes_to_both_throttle_motors_only() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        v.weapon2_firing = true;
        let mut out = engine.step(&v, &cfg(), false);
        for i in 1..10 {
            v.t = i as f64 * 0.02;
            out = engine.step(&v, &cfg(), false);
        }
        assert_eq!(out.joystick_intensity, 0);
        assert!(out.throttle_left_intensity > 0);
        assert!(out.throttle_right_intensity > 0);
    }

    #[test]
    fn weapon_disabled_stays_silent_even_while_firing() {
        let mut engine = WtRumbleState::new();
        let mut c = cfg();
        c.weapon1_enabled = false;
        c.weapon2_enabled = false;
        let mut v = vars();
        v.weapon1_firing = true;
        v.weapon2_firing = true;
        let out = engine.step(&v, &c, false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn flaps_change_triggers_bump_then_settles() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        engine.step(&v, &cfg(), false); // baseline at 0%, t=0.0
        v.flaps_pct = 30.0;
        v.t = 0.05;
        let out = engine.step(&v, &cfg(), false);
        assert!(out.joystick_intensity > 0 || out.throttle_left_intensity > 0);

        // После окончания короткого окна (FLAPS_BUMP_DURATION_S=0.15с) без
        // нового изменения — эффект должен смолкнуть.
        v.t = 0.05 + FLAPS_BUMP_DURATION_S + 0.05;
        let out2 = engine.step(&v, &cfg(), false);
        assert_eq!(out2.joystick_intensity, 0);
        assert_eq!(out2.throttle_left_intensity, 0);
        assert_eq!(out2.throttle_right_intensity, 0);
    }

    #[test]
    fn gear_does_not_lock_bump_on_first_frame() {
        // Первый кадр уже приходит с gear_pct=100 (выпущено с начала полёта) —
        // не должно считаться переходом "на замок".
        let mut engine = WtRumbleState::new();
        let v = vars(); // gear_pct = 100.0
        let out = engine.step(&v, &cfg(), false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn gear_retraction_then_lock_produces_bump() {
        let mut engine = WtRumbleState::new();
        let mut v = vars();
        v.gear_pct = 100.0;
        engine.step(&v, &cfg(), false); // init at 100 (down & locked), t=0.0
        v.gear_pct = 50.0;
        v.t = 0.1;
        engine.step(&v, &cfg(), false); // in transit
        v.gear_pct = 0.0;
        v.t = 0.2;
        engine.step(&v, &cfg(), false); // edge tick: attack of the lock-bump sine starts at 0
        // Пик attack/decay-огибающей приходится не на сам фронт, а чуть позже —
        // проверяем следующий тик внутри окна удара (GEAR_LOCK_BUMP_DURATION_S=0.8с).
        v.t = 0.3;
        let locked_out = engine.step(&v, &cfg(), false);
        assert!(
            locked_out.joystick_intensity > 0
                || locked_out.throttle_left_intensity > 0
                || locked_out.throttle_right_intensity > 0
        );
    }

    #[test]
    fn gear_transit_disabled_never_bumps() {
        let mut engine = WtRumbleState::new();
        let mut c = cfg();
        c.gear_transit_enabled = false;
        let mut v = vars();
        v.gear_pct = 100.0;
        engine.step(&v, &c, false);
        v.gear_pct = 0.0;
        let out = engine.step(&v, &c, false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Stall/буффет срыва потока (Bf 109 F-4, см. StallState/aero_profiles).
    // ═══════════════════════════════════════════════════════════════════

    fn bf109f4_vars() -> WtVars {
        let mut v = vars();
        v.vehicle_type = "bf-109f-4".to_string();
        v.ias_kmh = 300.0; // выше ias_safety_cutoff_kmh (40.0)
        v
    }

    #[test]
    fn stall_silent_below_ias_safety_cutoff() {
        let mut engine = WtRumbleState::new();
        let mut v = bf109f4_vars();
        v.ias_kmh = 20.0; // ниже cutoff 40.0
        v.aoa_deg = 25.0; // заведомо выше clean_stall_aoa_deg (19.9)
        let out = engine.step(&v, &cfg(), false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn stall_silent_below_pre_stall_ramp_start() {
        let mut engine = WtRumbleState::new();
        let mut v = bf109f4_vars();
        v.flaps_pct = 0.0; // чистое крыло: stall_aoa=19.9, ramp start=19.9-4.9=15.0
        v.aoa_deg = 10.0;
        let out = engine.step(&v, &cfg(), false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn stall_pre_threshold_intensity_grows_linearly_to_255() {
        // Ниже порога (19.9°) — РОВНЫЙ линейный рост 0..255 напрямую от AoA,
        // без джиттера/ceiling (по запросу пользователя) — детерминированный,
        // значения можно проверить точно. Не путать с пилой в срыве (см.
        // следующий тест) — там тот же диапазон, но другой характер сигнала.
        let mut engine = WtRumbleState::new();
        let mut v = bf109f4_vars();
        v.flaps_pct = 0.0; // stall_aoa=19.9, ramp start=15.0, width=4.9
        let c = cfg();

        // (aoa, ожидаемая intensity = round((aoa-15.0)/4.9 * 255))
        for (aoa, expected) in [(15.5, 26u8), (17.0, 104), (18.5, 182), (19.8, 250)] {
            v.aoa_deg = aoa;
            let out = engine.step(&v, &c, false);
            assert_eq!(
                out.joystick_intensity, expected,
                "linear ramp mismatch at aoa={aoa}"
            );
        }
    }

    #[test]
    fn stall_pulse_is_square_wave_half_duty_while_stalled() {
        // По живому тесту (2026-07-29): вместо плавной пилы — чёткий
        // прямоугольный импульс: пауза (0) полпериода, затем резкий скачок
        // на ПОЛНУЮ мощность (255) на вторую половину — не ограничена
        // ceiling, отдельный контрастный режим от плавного роста на подходе
        // (см. предыдущий тест). Период 1/STALL_SAWTOOTH_HZ=0.2с, пауза и
        // работа мотора — ровно по 0.1с (STALL_PULSE_DUTY=0.5).
        let mut engine = WtRumbleState::new();
        let mut v = bf109f4_vars();
        v.flaps_pct = 0.0;
        v.aoa_deg = 25.0; // заведомо за порогом (19.9)
        let c = cfg();

        // Первый тик взводит и запускает break-импульс (t0=0.0, длится
        // BREAK_IMPULSE_DURATION_S=0.25с) — он суммируется поверх
        // continuous-члена, поэтому для чистой проверки square-wave берём
        // точки заведомо после его затухания.
        v.t = 0.0;
        engine.step(&v, &c, false);

        // phase = frac(t*STALL_SAWTOOTH_HZ) < 0.5 -> пауза (0), иначе -> 255.
        for (t, expected) in [(1.05, 0u8), (1.15, 255), (1.25, 0), (1.35, 255)] {
            v.t = t;
            let out = engine.step(&v, &c, false);
            assert_eq!(
                out.joystick_intensity, expected,
                "square-wave pulse mismatch at t={t}"
            );
        }
    }

    #[test]
    fn break_impulse_rearms_only_after_hysteresis_recovery() {
        // Регрессия на живой баг: в штопоре AoA дрожит прямо у порога
        // сваливания, и без гистерезиса break-импульс ретриггерился на
        // каждом мелком колебании вместо одного чёткого щелчка на отрыве.
        let mut stall = StallState::new();
        let profile = &aero_profiles::BF_109_F4;
        let mut v = bf109f4_vars();
        v.flaps_pct = 0.0; // stall_aoa = 19.9, rearm-порог = 19.9-3.0 = 16.9
        let ceiling = 30.0;

        v.aoa_deg = 25.0;
        v.t = 0.0;
        stall.step(v.t, &v, profile, ceiling);
        assert_eq!(
            stall.break_impulse_t0, 0.0,
            "первое пересечение должно взвести импульс"
        );

        // Колебание у самого порога (внутри окна гистерезиса) — НЕ должно
        // взводить новый импульс.
        v.aoa_deg = 18.9; // ниже порога (19.9), но выше rearm-порога (16.9)
        v.t = 0.05;
        stall.step(v.t, &v, profile, ceiling);
        v.aoa_deg = 25.0;
        v.t = 0.1;
        stall.step(v.t, &v, profile, ceiling);
        assert_eq!(
            stall.break_impulse_t0, 0.0,
            "повторное пересечение внутри гистерезиса не должно взводить импульс заново"
        );

        // Явное восстановление ниже rearm-порога — импульс взводится заново.
        v.aoa_deg = 10.0;
        v.t = 0.5;
        stall.step(v.t, &v, profile, ceiling);
        v.aoa_deg = 25.0;
        v.t = 0.55;
        stall.step(v.t, &v, profile, ceiling);
        assert_eq!(
            stall.break_impulse_t0, 0.55,
            "восстановление за гистерезисом должно взводить импульс заново"
        );
    }

    #[test]
    fn stall_threshold_interpolates_with_flaps() {
        // Чистое крыло: stall_aoa=19.9, ramp start=19.9-4.9=15.0 -> при 13.0
        // ещё тихо. С закрылками 100%: stall_aoa=(15+17)/2=16.0,
        // ramp start=16.0-4.9=11.1 -> тот же AoA=13.0 уже внутри рампы.
        let mut engine_clean = WtRumbleState::new();
        let mut v_clean = bf109f4_vars();
        v_clean.flaps_pct = 0.0;
        v_clean.aoa_deg = 13.0;
        let out_clean = engine_clean.step(&v_clean, &cfg(), false);

        let mut engine_landing = WtRumbleState::new();
        let mut v_landing = bf109f4_vars();
        v_landing.flaps_pct = 100.0;
        v_landing.aoa_deg = 13.0;
        let out_landing = engine_landing.step(&v_landing, &cfg(), false);

        assert_eq!(out_clean.joystick_intensity, 0);
        assert!(out_landing.joystick_intensity > 0);
    }

    #[test]
    fn stall_disabled_stays_silent_even_past_critical_aoa() {
        let mut engine = WtRumbleState::new();
        let mut c = cfg();
        c.stall_enabled = false;
        let mut v = bf109f4_vars();
        v.aoa_deg = 25.0;
        let out = engine.step(&v, &c, false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn stall_stays_silent_on_unknown_aircraft() {
        let mut engine = WtRumbleState::new();
        let mut v = vars(); // vehicle_type = "" (не в таблице профилей)
        v.ias_kmh = 300.0;
        v.aoa_deg = 30.0;
        let out = engine.step(&v, &cfg(), false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    // --- Engine Start/Stop ---

    fn engine_vars(rpm: f64, power: f64, t: f64) -> WtVars {
        let mut v = vars();
        v.t = t;
        v.rpm_1 = rpm;
        v.power_1_hp = power;
        v
    }

    #[test]
    fn engine_silent_when_disabled() {
        let mut engine = WtRumbleState::new();
        let mut c = cfg();
        c.engine_start_enabled = false;
        let v = engine_vars(0.0, 0.0, 0.0);
        let out = engine.step(&v, &c, false);
        assert_eq!(out.joystick_intensity, 0);
        // Даже если бы отслеживалось состояние — с выключенным cfg молчит и
        // на резком скачке оборотов.
        let v2 = engine_vars(400.0, 0.0, 0.5);
        let out2 = engine.step(&v2, &c, false);
        assert_eq!(out2.joystick_intensity, 0);
        assert_eq!(out2.throttle_left_intensity, 0);
        assert_eq!(out2.throttle_right_intensity, 0);
    }

    #[test]
    fn engine_silent_on_steady_idle() {
        let mut engine = WtRumbleState::new();
        let c = cfg();
        // Первый тик — уже в холостых (борт заспавнился с работающим
        // двигателем), не пуск.
        let out = engine.step(&engine_vars(500.0, 20.0, 0.0), &c, false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
        // Несколько тиков спустя обороты и мощность стабильны — тишина.
        for i in 1..20 {
            let out = engine.step(&engine_vars(500.0, 20.0, i as f64 * 0.05), &c, false);
            assert_eq!(out.joystick_intensity, 0, "tick {i}");
            assert_eq!(out.throttle_left_intensity, 0, "tick {i}");
        }
    }

    #[test]
    fn engine_no_false_start_on_first_tick_with_high_rpm() {
        // Регрессия: борт может заспавниться в бою с уже раскрученным
        // двигателем (см. session_20260728_204134.jsonl — первый же тик
        // RPM=2301). Наивный детектор "RPM ушёл с нуля" не должен принять
        // это за только что запущенный двигатель.
        let mut engine = WtRumbleState::new();
        let out = engine.step(&engine_vars(2301.0, 974.2, 0.0), &cfg(), false);
        assert_eq!(out.joystick_intensity, 0);
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn engine_no_false_start_on_respawn_after_prior_stop_in_same_session() {
        // Регрессия: один и тот же WtRumbleState переживает несколько
        // вылетов подряд (одна сессия воркера). Прошлый вылет закончился
        // полной остановкой двигателя (phase=Off) — если следующий борт
        // спавнится в НОВОМ бою уже с работающим двигателем, это не должно
        // расцениваться как пуск (раньше фаза Off просто продолжалась и
        // ловила рост RPM как Off → Start).
        let mut engine = WtRumbleState::new();
        let c = cfg();
        let mut t = 0.0;

        // Прошлый вылет: несколько тиков с заглушенным двигателем — сессия
        // заканчивается в Off, не в Run/Coast.
        for _ in 0..5 {
            engine.step(&engine_vars(0.0, 0.0, t), &c, false);
            t += 0.05;
        }

        // Вышли из боя (ангар/меню) на несколько тиков.
        for _ in 0..5 {
            let mut v = engine_vars(0.0, 0.0, t);
            v.in_mission = false;
            engine.step(&v, &c, false);
            t += 0.05;
        }

        // Новый бой: другой борт уже с работающим двигателем на споне.
        let out = engine.step(&engine_vars(595.0, 20.0, t), &c, false);
        assert_eq!(
            out.joystick_intensity, 0,
            "must not treat an already-running engine at new mission spawn as a fresh start"
        );
        assert_eq!(out.throttle_left_intensity, 0);
        assert_eq!(out.throttle_right_intensity, 0);
    }

    #[test]
    fn engine_no_false_start_when_gear_already_retracted_even_if_rpm_reads_zero() {
        // Страховка от гонки телеметрии: если самый первый ответ игры
        // пришёл с ещё не заполненным RPM (0), но шасси уже убрано — борт
        // точно уже в воздухе с работающим двигателем, а не только что
        // заведён на земле (убранное шасси на земле у поршневого
        // истребителя не бывает).
        let mut engine = WtRumbleState::new();
        let mut v = engine_vars(0.0, 0.0, 0.0);
        v.gear_pct = 0.0; // убрано
        let out = engine.step(&v, &cfg(), false);
        assert_eq!(out.joystick_intensity, 0);

        // Через несколько тиков телеметрия "доезжает" и обороты появляются —
        // это по-прежнему не должно расцениваться как пуск.
        for i in 1..10 {
            let mut v2 = engine_vars(595.0, 20.0, i as f64 * 0.05);
            v2.gear_pct = 0.0;
            let out2 = engine.step(&v2, &cfg(), false);
            assert_eq!(out2.joystick_intensity, 0, "tick {i}");
        }
    }

    #[test]
    fn engine_no_false_start_when_gear_and_rpm_settle_a_few_ticks_after_spawn() {
        // Основная гонка, из-за которой пуск ловился ложно на живом тесте:
        // на самом первом тике после захода в бой RPM и шасси иногда ЕЩЁ НЕ
        // устаканились (техника только грузится) — оба читаются как "ещё не
        // летит" (rpm=0, gear=100), решение "Off" фиксировалось бы сразу и
        // навсегда, а через пару тиков, когда приходят настоящие RPM=595 и
        // gear=0, Off → Start уже не перепроверяет шасси и ловит пуск.
        // Окно устаканивания (ENGINE_SPAWN_SETTLE_WINDOW_S) должно поймать
        // это позже пришедшее "уже летит" не хуже, чем самый первый тик.
        let mut engine = WtRumbleState::new();
        let c = cfg();

        // Первые два тика — телеметрия ещё не устаканилась.
        for i in 0..2 {
            let out = engine.step(&engine_vars(0.0, 0.0, i as f64 * 0.05), &c, false);
            assert_eq!(out.joystick_intensity, 0, "settling tick {i}");
        }
        // С третьего тика приходят реальные значения: борт уже летит.
        for i in 2..15 {
            let mut v = engine_vars(595.0, 20.0, i as f64 * 0.05);
            v.gear_pct = 0.0;
            let out = engine.step(&v, &c, false);
            assert_eq!(
                out.joystick_intensity, 0,
                "tick {i}: telemetry settling late must not be read as a fresh start"
            );
        }
    }

    #[test]
    fn engine_genuine_ground_start_still_works_after_spawn_settle_window() {
        // Окно устаканивания не должно скрывать честный пуск с нуля: если
        // ни RPM, ни шасси ни разу не показали "уже летит" за всё окно
        // (двигатель правда выключен на земле, шасси выпущено), по истечении
        // окна детектор обязан признать Off и по-прежнему ловить реальную
        // прокрутку стартером.
        let mut engine = WtRumbleState::new();
        let c = cfg();
        let mut t = 0.0;

        // Всё окно устаканивания — двигатель выключен, шасси выпущено.
        while t < ENGINE_SPAWN_SETTLE_WINDOW_S + 0.2 {
            let out = engine.step(&engine_vars(0.0, 0.0, t), &c, false);
            assert_eq!(out.joystick_intensity, 0, "t={t}");
            t += 0.05;
        }

        // Теперь честная прокрутка стартером — должна ощущаться как пуск.
        let mut felt_anything = false;
        for _ in 0..30 {
            t += 0.05;
            let out = engine.step(&engine_vars(t * 15.0, 0.0, t), &c, false);
            if out.joystick_intensity > 0 {
                felt_anything = true;
            }
        }
        assert!(
            felt_anything,
            "a genuine ground start after the settle window must still produce the start effect"
        );
    }

    #[test]
    fn engine_catch_impulse_fires_once_per_start_cycle() {
        let mut state = EngineState::new();
        // Инициализация "с нуля" (борт заглушен).
        state.step(0.0, &engine_vars(0.0, 0.0, 0.0), 255.0);

        // Прокрутка стартером — медленный рост, как в живом логе (t=27.9..30.25с).
        let mut t = 0.05;
        let mut rpm = 0.0;
        while rpm < 35.0 {
            rpm += 1.0;
            state.step(t, &engine_vars(rpm, 0.0, t), 255.0);
            t += 0.05;
        }
        assert!(
            !state.catch_fired,
            "не должен сработать на медленной прокрутке"
        );

        // Резкий скачок (воспламенение) — имитация t=30.30..30.45 из лога.
        for rpm in [46.0, 68.0, 90.0, 120.0] {
            state.step(t, &engine_vars(rpm, 0.0, t), 255.0);
            t += 0.05;
        }
        assert!(
            state.catch_fired,
            "должен взвестись на резком скачке оборотов"
        );
        let fired_at = state.catch_pulse_t0;
        assert!(fired_at >= 0.0);

        // Дальнейшие скачки в той же сессии не переустанавливают импульс —
        // "схватывание" происходит один раз за цикл пуска.
        for rpm in [150.0, 200.0, 300.0] {
            state.step(t, &engine_vars(rpm, 0.0, t), 255.0);
            t += 0.05;
        }
        assert_eq!(state.catch_pulse_t0, fired_at);
    }

    #[test]
    fn engine_coast_uses_captured_rpm_ref_not_hardcoded_idle() {
        // Разные борты имеют разные холостые (485 в одной сессии, 595 в
        // другой) — эффект не должен зависеть от захардкоженного значения.
        let mut state = EngineState::new();
        state.step(0.0, &engine_vars(595.0, 20.0, 0.0), 255.0);
        // Питание пропадает — переход в выбег, rpm_ref захватывается ЖИВЫМ
        // значением оборотов, а не константой.
        let out = state.step(0.05, &engine_vars(595.0, 0.0, 0.05), 255.0);
        assert_eq!(state.phase, EnginePhase::Coast);
        assert_eq!(state.rpm_ref, 595.0);
        // На первом тике выбега обороты ещё равны rpm_ref — коэффициент 1.0,
        // сигнал не молчит (частота/амплитуда на максимуме выбега).
        assert!(out > 0.0);
    }
}
