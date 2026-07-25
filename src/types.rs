use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FlightVars {
    pub sim_time_s: f64,
    pub airspeed_indicated: f64,
    pub on_ground: bool,
    pub bank_deg: f64,
    pub flaps_pct: f64,
    pub flaps_index: i32,
    pub gear_handle: f64,
    pub stalled: bool,
    pub ground_speed_kt: f64,
    pub paused: bool,
    pub spoilers_pct: f64, // Положение спойлеров в % (0.0 - 100.0)
    pub gear_comp_nose: f64,
    pub gear_comp_left: f64,
    pub gear_comp_right: f64,
    pub trailing_edge_flaps_left_percent: f64,
    // Телеметрия запуска двигателей (Engine Spool-up & Ignition)
    pub eng1_n2_percent: f64,
    pub eng1_combustion: f64,
    pub eng2_n2_percent: f64,
    pub eng2_combustion: f64,
    // Двигатели 3/4 — для 4-моторных самолётов (см. RumbleConfig::four_engine_mode).
    // На 2-моторных самолётах SimConnect обычно возвращает 0.0 для этих индексов,
    // что корректно интерпретируется как "двигатель выключен" (deadzone).
    pub eng3_n2_percent: f64,
    pub eng3_combustion: f64,
    pub eng4_n2_percent: f64,
    pub eng4_combustion: f64,
    // GENERAL ENG STARTER:1..4 — универсальная модель запуска (Starter + Combustion),
    // работает одинаково на поршневых и турбинных двигателях (в отличие от N2,
    // которое на поршневых обычно недоступно/равно 0). true = стартер крутит
    // двигатель (турбина ещё не воспламенилась / поршневой ещё не завёлся).
    pub eng1_starter: bool,
    pub eng2_starter: bool,
    pub eng3_starter: bool,
    pub eng4_starter: bool,
    // GENERAL ENG PCT MAX RPM — универсальная поддержка поршневых двигателей.
    // На турбинах примерно повторяет N2; на поршневых даёт реальный % оборотов
    // (в отличие от TURB ENG N2, которое на поршневых обычно равно 0). См.
    // rumble.rs: effective spool value = max(n2_percent, pct_max_rpm).
    pub eng1_pct_max_rpm: f64,
    pub eng2_pct_max_rpm: f64,
    pub eng3_pct_max_rpm: f64,
    pub eng4_pct_max_rpm: f64,
    // Поршневые двигатели (Piston Engine Telemetry) — сырые обороты для
    // телеметрической панели в UI. GENERAL ENG RPM — обороты коленвала,
    // PROP RPM — обороты воздушного винта (могут отличаться от RPM
    // двигателя из-за редуктора). Порядок полей ниже соответствует порядку
    // add_data_definition() в src/sim/worker.rs (см. рядом идущий комментарий
    // там же с индексами elem[]).
    pub eng1_rpm: f64,
    pub eng2_rpm: f64,
    pub eng3_rpm: f64,
    pub eng4_rpm: f64,
    pub prop1_rpm: f64,
    pub prop2_rpm: f64,
    pub prop3_rpm: f64,
    pub prop4_rpm: f64,
    // DESIGN SPEED VC — динамический порог Overspeed, приходящий из SimConnect
    // для текущего загруженного самолёта (вместо ручного слайдера в UI).
    // 0.0 означает "ещё не получено от SimConnect" (см. UI: "Limit: N/A").
    pub design_speed_vc_kn: f64,
    // Предкрылки (Slats): среднее LEADING EDGE FLAPS LEFT/RIGHT PERCENT.
    // Пока только для отображения в UI ("Live Aircraft Data"), в логику
    // эффектов не задействовано.
    pub slats_pct: f64,
}

/// Привязка одного эффекта вибрации к устройствам вывода.
/// Позволяет независимо включать/выключать отправку конкретного эффекта
/// на Combat Joystick R и/или на WINCTRL URSA MINOR Throttle (РУД).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectDeviceTarget {
    pub enable_joystick: bool, // Отправлять на Combat Joystick R
    pub enable_throttle: bool, // Отправлять на URSA MINOR Throttle
}

impl Default for EffectDeviceTarget {
    fn default() -> Self {
        // Сохраняем прежнее поведение по умолчанию: все эффекты идут на
        // джойстик, РУД молчит, пока пользователь явно не включит его в UI.
        Self {
            enable_joystick: true,
            enable_throttle: false,
        }
    }
}

/// Привязка к устройствам для каждого эффекта вибрации по отдельности.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EffectDeviceTargets {
    pub overspeed: EffectDeviceTarget,
    pub ground_roll: EffectDeviceTarget, // Удар о стыки плит ВПП (Taxi/Takeoff Thump)
    pub flaps: EffectDeviceTarget,
    pub gear_bump: EffectDeviceTarget, // Импульс от ручки шасси (Down/Up)
    pub stall: EffectDeviceTarget,
    pub spoilers: EffectDeviceTarget,
    pub bank: EffectDeviceTarget,
    pub gear_comp_nose: EffectDeviceTarget,  // Обжатие носовой стойки (Touchdown)
    pub gear_comp_left: EffectDeviceTarget,  // Обжатие левой стойки (Touchdown)
    pub gear_comp_right: EffectDeviceTarget, // Обжатие правой стойки (Touchdown)
    pub gear_transit: EffectDeviceTarget,    // Движение стоек + удар дверей на замке
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RumbleConfig {
    // Overspeed settings
    // Порог скорости (overspeed_threshold_kn) больше не хранится в конфиге —
    // он приходит динамически из SimConnect (DESIGN SPEED VC) для текущего
    // самолёта, см. FlightVars::design_speed_vc_kn.
    pub overspeed_enabled: bool,
    pub overspeed_intensity: f32,
     pub overspeed_max_kn: f32,

    // Gear Strut Compression settings
    // РЕМАРКА: слайдер *_peak в UI имеет диапазон 0..55 — это НЕ итоговая сила,
    // а "запас сверху" над обязательным полом 200 (см. rumble.rs). 0 → всегда
    // строго 200 (на любой посадке). 55 → диапазон 200 (мягкая посадка) ..
    // 255 (жёсткая). Итоговая сила эффекта физически не выходит за [200..255].
    pub gear_comp_enabled: bool,
    pub gear_comp_nose_enabled: bool,
    pub gear_comp_nose_peak: f32,
    pub gear_comp_left_enabled: bool,
    pub gear_comp_left_peak: f32,
    pub gear_comp_right_enabled: bool,
    pub gear_comp_right_peak: f32,
    pub gear_transit_enabled: bool,
    // SPLIT-режим для удара обжатия основных стоек (crosswind landing awareness):
    // левая стойка → эксклюзивно РУД, правая стойка → эксклюзивно джойстик.
    // Носовая стойка не участвует в SPLIT и продолжает маршрутизироваться
    // обычными чекбоксами device_targets.gear_comp_nose.
    pub split_touchdown: bool,

    // Bank/Turb settings
    pub bank_enabled: bool,
    pub bank_intensity: f32,     // Максимальная интенсивность (0-200)
    pub bank_threshold_deg: f32, // Порог срабатывания (0-90°)

    // РЕМАРКА: слайдер ground_roll в UI имеет диапазон 0..50 — это и есть
    // итоговый потолок силы эффекта стыков плит (amplitude_curve лишь
    // масштабирует от 0 до этого значения). Мягкий фоновый эффект, НЕ должен
    // соперничать по ощущению с ударом сжатия стоек (200-255).
    pub ground_roll: f32,
    pub flaps_peak: f32,
    pub gear_peak: f32,
    pub stall_ceiling: f32,
    pub max_output: u8,
    pub smoothing_alpha: f32,
    pub ias_deadband_kn: f64,
    pub taxi_start_enabled: bool,
    pub taxi_start_kn: f64,
    pub taxi_end_enabled: bool,
    pub taxi_end_kn: f64,
    pub thump_min_period_s: f64,
    pub thump_max_period_s: f64,
    pub thump_duty: f64,
    // Физическая модель удара о стыки бетонных плит (заменяет старую duty-based модель)
    pub runway_slab_length_m: f32, // Длина одной плиты ВПП в метрах
    pub thump_duration_ms: f32,    // Длительность одного импульса удара в мс
    // Коэффициент кривизны нарастания частоты ударов (см. ремарку в rumble.rs):
    // 1.0 = чистая физика (t=S/V), >1.0 = плавнее на старте, <1.0 = резче на старте
    pub thump_period_curve: f32,
    // Минимальное время, на которое motor-hum закрылков остаётся включённым
    // после КАЖДОГО зафиксированного изменения flaps_pct/slats_pct (см.
    // rumble.rs) — нужно только чтобы мгновенный скачок за один тик
    // (например MADDOG: 0 -> 27) успел дать ощутимый щелчок, а не потерялся
    // за один PWM-кадр. Не выведено в UI намеренно: включение/выключение
    // эффекта должно ощущаться практически мгновенным, а не "мотором,
    // который разгоняется/тормозит" — поэтому значение маленькое.
    pub flaps_bump_duration_s: f64,
    pub flaps_bump_eps_pct: f64,
    pub flaps_duty: f64,
    pub gear_bump_duration_s: f64,

    pub ground_enabled: bool,
    pub flaps_enabled: bool,
    pub gear_enabled: bool,
    pub stall_enabled: bool,
    // На некоторых бортах (см. src/profiles.rs, MADDOG) предкрылки убираются
    // отдельным, более поздним движением ручки, чем закрылки — flaps_pct уже
    // 0, когда slats_pct только падает в 0. Этот флаг включается автоматически
    // встроенным профилем самолёта (не предназначен для ручного переключения
    // в UI) и заставляет motor-hum дополнительно ловить движение по slats_pct.
    pub flaps_track_slats: bool,

    // Spoilers settings
    pub spoilers_enabled: bool,
    pub spoilers_threshold_pct: f64,
    pub spoilers_intensity: f32,

    pub is_combat_edition: bool,

    // Engine Spool-up & Ignition settings
    pub enable_engine_start: bool,
    pub engine_start_strength: f32,
    // Порог N2 (%), при котором двигатель считается вышедшим на Idle.
    // Разные самолёты выходят на Idle при разных значениях N2, поэтому
    // это значение настраивается пользователем в UI (по умолчанию 60.0).
    pub engine_idle_n2: f32,
    // 4-моторный режим: двигатели группируются по крылу — Eng1/Eng2 → Throttle
    // (левая рука), Eng3/Eng4 → Joystick (правая рука). Используется максимум N2
    // в паре и OR по combustion для срабатывания удара воспламенения.
    pub four_engine_mode: bool,

    // Зеркалирование посадки рук: по умолчанию все side-bound эффекты
    // (engine-start pre-combustion, split touchdown) считают, что РУД — под
    // левой рукой, джойстик — под правой (Eng1/left strut → РУД, Eng2/right
    // strut → джойстик). Если физически джойстик стоит слева, а РУД справа —
    // этот флаг меняет местами, какая сторона считается "рукой РУД", а какая
    // "рукой джойстика", не трогая при этом маршрутизацию по мотору РУД
    // (throttle_left/throttle_right остаются жёстко привязаны к Eng1/Eng2 —
    // это железо квадранта, а не поза за столом).
    pub swap_hand_layout: bool,

    // Привязка каждого эффекта к устройствам (Джойстик / РУД / оба).
    // #[serde(default)] на уровне структуры уже гарантирует, что старые
    // settings.json без этого поля подхватят EffectDeviceTargets::default()
    // (все эффекты → джойстик, РУД выключен — прежнее поведение).
    pub device_targets: EffectDeviceTargets,
}

impl Default for RumbleConfig {
    fn default() -> Self {
        Self {
            overspeed_enabled: true,
            overspeed_intensity: 100.0,
            overspeed_max_kn: 350.0,

            bank_enabled: true,
            bank_intensity: 70.0,
            bank_threshold_deg: 45.0,

            ground_roll: 38.0,
            flaps_peak: 60.0,
            gear_peak: 110.0,
            stall_ceiling: 160.0,
            max_output: 255,
            smoothing_alpha: 0.18,
            ias_deadband_kn: 1.0,
            taxi_start_enabled: true,
            taxi_start_kn: 3.0,
            taxi_end_enabled: true,
            taxi_end_kn: 10.0,
            // ВНИМАНИЕ: thump_min_period_s ограничен полосой передачи на устройство —
            // hid/worker.rs шлёт интенсивность не чаще раза в SEND_INTERVAL (сейчас 50мс = 20 Гц).
            // Частота Найквиста этого канала = 10 Гц, поэтому min_period_s не должен
            // быть меньше ~0.12-0.15с (6.7-8.3 Гц), иначе рост частоты с ростом GS
            // перестанет ощущаться корректно (часть импульсов "съедается" между отправками).
            thump_min_period_s: 0.15,
            thump_max_period_s: 1.3,
            thump_duty: 0.18,
            runway_slab_length_m: 6.0,
            thump_duration_ms: 300.0,
            thump_period_curve: 2.5, // плавнее на старте, чем чистая физика
            flaps_bump_duration_s: 0.15,
            flaps_bump_eps_pct: 2.0,
            flaps_duty: 0.6,
            gear_bump_duration_s: 0.8,

            ground_enabled: true,
            flaps_enabled: true,
            gear_enabled: true,
            stall_enabled: true,
            flaps_track_slats: false,

            spoilers_enabled: true,
            spoilers_threshold_pct: 10.0,
            spoilers_intensity: 90.0,

            gear_comp_enabled: true,
            gear_comp_nose_enabled: true,
            gear_comp_nose_peak: 25.0, // запас 0..55 над полом 200 → факт. 200..225..255
            gear_comp_left_enabled: true,
            gear_comp_left_peak: 30.0,
            gear_comp_right_enabled: true,
            gear_transit_enabled: true,
            gear_comp_right_peak: 30.0,
            split_touchdown: false,

            is_combat_edition: false,

            enable_engine_start: true,
            engine_start_strength: 100.0,
            engine_idle_n2: 60.0,
            four_engine_mode: false,
            swap_hand_layout: false,

            device_targets: EffectDeviceTargets::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
 pub struct EffectsSnapshot {
     pub flaps_bump_active: bool,
     pub gear_bump_active: bool,
     pub ground_active: bool,
     pub ground_thump_active: bool,
     pub taxi_start_crossed: bool,
     pub taxi_end_crossed: bool,
     pub bank_active: bool,
     pub stall_active: bool,
     pub spoilers_active: bool,
     pub overspeed_active: bool,
     
     // Gear Strut Compression (Touchdown) status
     pub gear_comp_nose_active: bool,
     pub gear_comp_left_active: bool,
     pub gear_comp_right_active: bool,

     // Gear Transit & Doors (движение стоек + удар фиксации на замке)
     pub gear_transit_active: bool,

     // Engine Spool-up & Ignition status
     pub engine_start_active: bool,
 }

#[derive(Debug)]
pub enum HidCmd {
    /// Раздельная интенсивность для Combat Joystick R и для WINCTRL URSA
    /// MINOR Throttle (РУД) — какое значение реально уйдёт на каждое
    /// устройство, решает EffectDeviceTarget конкретного эффекта в rumble.rs.
    /// РУД имеет два независимых вибромотора (левый/правый), поэтому
    /// throttle тоже разбит на два канала.
    SendIntensity { joystick: u8, throttle_left: u8, throttle_right: u8 },
    SendRaw(Vec<u8>),
    StopAll,
    ReopenDevices,
    SetHold(bool),
}

pub struct ConfigShared {
    inner: Mutex<RumbleConfig>,
    rev: AtomicU64,
}

impl ConfigShared {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(RumbleConfig::default()),
            rev: AtomicU64::new(1),
        }
    }

    /// Создаёт ConfigShared, пытаясь подгрузить сохранённый конфиг с диска
    /// (см. settings::load()). Если файла нет или он повреждён — используются
    /// значения по умолчанию.
    pub fn new_loaded() -> Self {
        let cfg = crate::settings::load().unwrap_or_default();
        Self {
            inner: Mutex::new(cfg),
            rev: AtomicU64::new(1),
        }
    }

    pub fn get(&self) -> RumbleConfig {
        self.inner.lock().clone()
    }

    pub fn set(&self, v: RumbleConfig) {
        *self.inner.lock() = v;
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    pub fn with_mut<F: FnOnce(&mut RumbleConfig)>(&self, f: F) {
        let mut g = self.inner.lock();
        let before = g.clone();
        f(&mut g);
        if *g != before {
            self.rev.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn current_rev(&self) -> u64 {
        self.rev.load(Ordering::Relaxed)
    }
}

impl Default for ConfigShared {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
pub struct EffectsState {
    pub flaps_bump_active: AtomicBool,
    pub gear_bump_active: AtomicBool,
    pub ground_active: AtomicBool,
    pub ground_thump_active: AtomicBool,
    pub taxi_start_crossed: AtomicBool,
    pub taxi_end_crossed: AtomicBool,
    pub bank_active: AtomicBool,
    pub stall_active: AtomicBool,
    pub spoilers_active: AtomicBool,
    pub overspeed_active: AtomicBool,

    pub gear_comp_nose_active: AtomicBool,
    pub gear_comp_left_active: AtomicBool,
    pub gear_comp_right_active: AtomicBool,

    pub gear_transit_active: AtomicBool,

    pub engine_start_active: AtomicBool,
}

pub type EffectsShared = Arc<EffectsState>;

impl EffectsState {
    pub fn apply_snapshot(&self, snap: &EffectsSnapshot) {
        self.flaps_bump_active
            .store(snap.flaps_bump_active, Ordering::Relaxed);
        self.gear_bump_active
            .store(snap.gear_bump_active, Ordering::Relaxed);
        self.ground_active
            .store(snap.ground_active, Ordering::Relaxed);
        self.ground_thump_active
            .store(snap.ground_thump_active, Ordering::Relaxed);
        self.taxi_start_crossed
            .store(snap.taxi_start_crossed, Ordering::Relaxed);
        self.taxi_end_crossed
            .store(snap.taxi_end_crossed, Ordering::Relaxed);
        self.bank_active.store(snap.bank_active, Ordering::Relaxed);
        self.stall_active
            .store(snap.stall_active, Ordering::Relaxed);
        self.spoilers_active
            .store(snap.spoilers_active, Ordering::Relaxed);
        self.overspeed_active
            .store(snap.overspeed_active, Ordering::Relaxed);

        self.gear_comp_nose_active
            .store(snap.gear_comp_nose_active, Ordering::Relaxed);
        self.gear_comp_left_active
            .store(snap.gear_comp_left_active, Ordering::Relaxed);
        self.gear_comp_right_active
            .store(snap.gear_comp_right_active, Ordering::Relaxed);

        self.gear_transit_active
            .store(snap.gear_transit_active, Ordering::Relaxed);

        self.engine_start_active
            .store(snap.engine_start_active, Ordering::Relaxed);
    }

    pub fn clear_all(&self) {
        self.apply_snapshot(&EffectsSnapshot::default());
    }
}

#[derive(Debug, Clone, Copy)]
pub enum UiCmd {
    Show,
    Hide,
    Toggle,
    Stop,
    Resume,
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SimStatus {
    Disconnected,
    Connected,
    InFlight,
}

impl Default for SimStatus {
    fn default() -> Self {
        Self::Disconnected
    }
}
