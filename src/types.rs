use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
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
    pub spoilers_pct: f64, // min(L, R) — эффективное положение спойлеров для эффекта, см. sim/parse.rs
    pub spoilers_left_pct: f64, // сырое положение левой плоскости, для телеметрии/отладки
    pub spoilers_right_pct: f64, // сырое положение правой плоскости, для телеметрии/отладки
    // TFDI MD-11: среднее по 5 секциям L:MD11_EXT_L/R_SPOILER_1..5 — доп.
    // проверка симметрии для этого борта (см. rumble.rs). На других самолётах
    // эти L-vars не определены и остаются 0.0 (самонейтрализуется).
    pub spoilers_md11_left_avg: f64,
    pub spoilers_md11_right_avg: f64,
    pub gear_comp_nose: f64,
    pub gear_comp_left: f64,
    pub gear_comp_right: f64,
    // Fenix A320: сырое значение L:A320_Gear_Nose (0 = убрано, 1000 = выпущено).
    // GEAR ANIMATION POSITION:0/1/2 на этом борте не отражает реальное движение
    // стоек, поэтому эффект Gear Transit в rumble.rs берёт позицию отсюда
    // (см. is_fenix ниже), когда этот флаг взведён. Также остаётся в
    // телеметрии как "F_Gear" для отладки.
    pub fenix_gear_nose_raw: f64,
    // true, если aircraft title содержит "Fenix" (см.
    // profiles::is_fenix_aircraft) — резолвится один раз в sim/parse.rs,
    // чтобы rumble.rs не тянул за собой строку title на каждый тик, только
    // готовый флаг. Используется, чтобы эффект Gear Transit переключался на
    // fenix_gear_nose_raw вместо gear_comp_nose/left/right, которые на этом
    // борте не отражают движение стоек. Эффект Gear Strut Compression
    // (Touchdown) НЕ тронут — он по-прежнему читает gear_comp_* напрямую.
    pub is_fenix: bool,
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
    // Порог Overspeed для текущего загруженного самолёта (вместо ручного
    // слайдера в UI): на большинстве бортов — AIRSPEED BARBER POLE
    // (динамическая "красная черта" Vmo/Mmo, сим сам двигает её вниз при
    // наборе высоты), на Fenix A320 (title содержит "Fenix") — его
    // собственный L:I_PFD_VMAX, т.к. этот аддон не держит AIRSPEED BARBER
    // POLE синхронной с реальным PFD (см. sim/parse.rs). Если выбранный
    // источник ещё не пришёл от SimConnect (0.0/невалиден), sim/parse.rs
    // подставляет дефолт 350.0 узлов.
    pub overspeed_barber_pole_kn: f64,
    // Предкрылки (Slats): среднее LEADING EDGE FLAPS LEFT/RIGHT PERCENT.
    // Пока только для отображения в UI ("Live Aircraft Data"), в логику
    // эффектов не задействовано.
    pub slats_pct: f64,
    // OVERSPEED WARNING — булев флаг "клацера" сима (сработавшего предупреждения
    // о превышении скорости). Пока только для отображения в UI, чтобы сравнить
    // с нашим порогом overspeed_barber_pole_kn — на некоторых аддонах (см. Override
    // в UI) эти значения могут не совпадать.
    pub overspeed_warning: bool,
    // Learjet 35A (Flysimware, FSW_L35A): L:XMLSND75 — L-var, к которому в
    // sound.xml аддона привязан звук "overspeed / mach trim" (аварийный
    // клаксон превышения Vmo/Mmo). На прочих самолётах этот L-var не
    // определён, SimConnect отдаёт 0.0 — поле остаётся false (см.
    // src/profiles.rs про включение самого эффекта только на этом борту).
    pub overspeed_lear_horn: bool,
    // PMDG 737 (NG3, MSFS): L:EngineStart1b/2b_Ext — true, пока стартер крутит
    // двигатель до воспламенения (взято из sound.xml PMDG). Подтверждено
    // тестом в симе как надёжный сигнал — используется в rumble.rs для
    // маркера воспламенения (момент, когда real TURB ENG N2 впервые > 0) и
    // для чуть более раннего распознавания борта как джета; сама N2-кривая
    // раскрутки читает реальный TURB ENG N2 напрямую (см. rumble.rs — real-world
    // capture показал, что N2 у PMDG растёт плавно с момента включения
    // стартера, никакой синтетической рампы не нужно). Отдельный
    // L:EngineStart1c/2c_Ext ("маркер воспламенения") был проверен в реальном
    // тесте и ОТКЛОНЁН — не сработал корректно, поэтому здесь не читается. На
    // прочих самолётах L:EngineStart1b/2b_Ext не определены, читается false
    // (самонейтрализуется).
    pub eng1_pmdg_starter_ext: bool,
    pub eng2_pmdg_starter_ext: bool,
    // GENERAL ENG STARTER ACTIVE:1/2 — пока только для телеметрии (сравнение
    // с L:EngineStart1b/2b_Ext в UI).
    pub eng1_starter_active: bool,
    pub eng2_starter_active: bool,
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
        // По умолчанию все эффекты идут и на джойстик, и на РУД — пользователь
        // может выключить любое из направлений вручную в UI.
        Self {
            enable_joystick: true,
            enable_throttle: true,
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
    pub gear_comp_nose: EffectDeviceTarget, // Обжатие носовой стойки (Touchdown)
    pub gear_comp_left: EffectDeviceTarget, // Обжатие левой стойки (Touchdown)
    pub gear_comp_right: EffectDeviceTarget, // Обжатие правой стойки (Touchdown)
    pub gear_transit: EffectDeviceTarget,   // Движение стоек + удар дверей на замке
}

/// Привязка к устройствам для Flaps/Gear Transit & Doors (War Thunder, этап
/// 1). Weapon1/Weapon2 НЕ входят сюда — их маршрутизация зафиксирована в коде
/// (`wt_link::rumble`), не настраивается пользователем: weapon1 → только
/// джойстик, weapon2 → только РУД (оба мотора сразу). Это подтверждено живым
/// подбором на железе (см. scratchpad-заметки сессии подбора пресетов
/// стрельбы) — разведение по рукам это и есть смысл эффекта (два разных
/// орудия должны ощущаться в разных руках), а не предпочтение пользователя,
/// поэтому чекбоксы тут были бы просто источником путаницы.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WtDeviceTargets {
    pub flaps: EffectDeviceTarget,
    pub gear_transit: EffectDeviceTarget,
    pub stall: EffectDeviceTarget,
    pub engine_start: EffectDeviceTarget,
    pub overspeed: EffectDeviceTarget,
    pub gear_overspeed: EffectDeviceTarget,
}

/// Параметры генератора "гул с текстурой" для одной группы оружия — перенесены
/// 1:1 из `src/bin/test_gun1.rs` (программный ШИМ на несущей частоте + джиттер
/// амплитуды за цикл + attack-рампа на передний фронт очереди). Значения по
/// умолчанию для weapon1/weapon2 — РАЗНЫЕ (см. `WtConfig::default`), подобраны
/// и подтверждены на живом железе.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GunPreset {
    pub carrier_freq_hz: f32,
    pub duty_pct: f32,
    pub jitter_pct: f32,
    pub peak: f32,
    pub floor: f32,
    pub attack_ms: f32,
}

impl Default for GunPreset {
    fn default() -> Self {
        // Дефолт структуры (используется, только если поле когда-либо
        // возникнет без явной инициализации) — берём консервативный
        // weapon1-пресет с пониженной несущей, см. WtConfig::default.
        Self {
            carrier_freq_hz: 6.5,
            duty_pct: 33.0,
            jitter_pct: 12.0,
            peak: 255.0,
            floor: 35.0,
            attack_ms: 41.0,
        }
    }
}

/// Настройки этапа 1 поддержки War Thunder (см. план): Weapon1/Weapon2
/// (стрельба), Flaps, Gear Transit & Doors. `stall_*` — эффект срыва
/// потока/сваливания (см. `wt_link::aero_profiles`/`wt_link::rumble::StallState`),
/// захардкоженный профиль порогов только для Bf 109 F-4 в v1 — на любом
/// другом борту эффект молчит независимо от `stall_enabled`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WtConfig {
    pub weapon1_enabled: bool,
    pub weapon2_enabled: bool,
    pub weapon1_gun: GunPreset,
    pub weapon2_gun: GunPreset,
    pub flaps_enabled: bool,
    pub flaps_peak: f32,
    pub gear_transit_enabled: bool,
    pub gear_peak: f32,
    pub stall_enabled: bool,
    pub stall_ceiling: f32,
    pub engine_start_enabled: bool,
    pub engine_start_peak: f32,
    pub overspeed_enabled: bool,
    pub overspeed_ceiling: f32,
    pub gear_overspeed_enabled: bool,
    pub gear_overspeed_ceiling: f32,
    pub device_targets: WtDeviceTargets,
}

impl Default for WtConfig {
    fn default() -> Self {
        Self {
            weapon1_enabled: true,
            weapon2_enabled: true,
            // weapon1 — быстрый ствол (текстура пулемёта). Несущая ПОНИЖЕНА
            // с подобранных на железе 12.5 Гц до 6.5 Гц: 12.5 Гц даёт пик
            // короче интервала отправки HID (20 Гц/50мс в hid_worker) —
            // алиасинг через штатный канал. Остальные параметры — как
            // подобрано вживую (duty/jitter/floor/attack).
            weapon1_gun: GunPreset {
                carrier_freq_hz: 6.5,
                duty_pct: 33.0,
                jitter_pct: 12.0,
                peak: 255.0,
                floor: 35.0,
                attack_ms: 41.0,
            },
            // weapon2 — медленный ствол (текстура авиапушки). 4.3 Гц уже
            // безопасно проходит через штатный 20 Гц HID-канал без изменений
            // (период 232.6мс, пик 93мс — с запасом выше порога).
            weapon2_gun: GunPreset {
                carrier_freq_hz: 4.3,
                duty_pct: 40.0,
                jitter_pct: 3.0,
                peak: 255.0,
                floor: 100.0,
                attack_ms: 22.0,
            },
            flaps_enabled: true,
            flaps_peak: 153.0, // тот же дефолт, что и MSFS flaps_peak
            gear_transit_enabled: true,
            gear_peak: 110.0,
            stall_enabled: true,
            stall_ceiling: 80.0, // см. WT_STALL_CEILING_HARD_CAP в wt_link/rumble.rs
            engine_start_enabled: true,
            engine_start_peak: 200.0,
            overspeed_enabled: true,
            overspeed_ceiling: 80.0, // тот же потолок по умолчанию, что у stall_ceiling
            gear_overspeed_enabled: true,
            gear_overspeed_ceiling: 80.0,
            device_targets: WtDeviceTargets::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RumbleConfig {
    // Overspeed settings
    // Порог скорости (overspeed_threshold_kn) больше не хранится в конфиге —
    // он приходит динамически из SimConnect (AIRSPEED BARBER POLE) для
    // текущего самолёта, см. FlightVars::overspeed_barber_pole_kn. Исключение —
    // overspeed_override_enabled: некоторые сложные аддоны (например TFDI
    // MADDOG) не синхронизируют эту переменную с реальным прибором в кабине,
    // тогда порог можно задать вручную через overspeed_manual_kn.
    pub overspeed_enabled: bool,
    pub overspeed_intensity: f32,
    pub overspeed_max_kn: f32,
    pub overspeed_override_enabled: bool,
    pub overspeed_manual_kn: f64,
    // Learjet 35A (Flysimware): дополнительный триггер эффекта Overspeed от
    // L:XMLSND75 — того же L-var, к которому в sound.xml аддона привязан
    // звук "overspeed / mach trim" (аварийный клаксон превышения Vmo/Mmo).
    // Экспериментально: AIRSPEED BARBER POLE может быть ненадёжен на этом
    // борту, поэтому вместо/вместе с порогом IAS используем сам сигнал
    // клаксона. Включается автоматически встроенным профилем самолёта
    // (src/profiles.rs, подстрока "LEARJET" в title) — не предназначено
    // для ручного переключения в UI.
    pub overspeed_lear_horn_enabled: bool,

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
    // SPLIT-режим для удара обжатия стоек (three-point landing awareness).
    // ВАЖНО: оба мотора РУД (throttle_left/throttle_right) физически находятся
    // в ОДНОЙ руке (один блок), джойстик — в другой. Поэтому ЛЕВАЯ и ПРАВАЯ
    // основные стойки (которые почти всегда касаются практически ОДНОВРЕМЕННО
    // на посадке на два колеса) разводятся по РАЗНЫМ рукам — РУД vs джойстик
    // — иначе они сливались бы в одно ощущение в одной ладони. Какая сторона
    // достаётся какой руке — определяет swap_hand_layout (по умолчанию: левая
    // стойка → РУД, правая → джойстик; при swap_hand_layout — наоборот; та же
    // семантика, что уже используется для двигателей). Носовая стойка касается
    // заметно ПОЗЖЕ основных, поэтому безопасно делит руку РУД со "своей"
    // основной стойкой на свободный второй мотор — плюс у неё намеренно другая,
    // более резкая и короткая форма импульса (см. rumble.rs), чтобы не сливаться
    // с основной стойкой даже при редком наложении по времени. Все три маршрута
    // игнорируют обычные чекбоксы device_targets.gear_comp_*. Когда SPLIT
    // выключен — все три стойки маршрутизируются обычными чекбоксами
    // device_targets.gear_comp_nose/left/right (throttle означает ОБА мотора
    // РУД одновременно).
    pub split_touchdown: bool,

    // Живой снимок "физически подключено" для джойстика/РУД — пишется
    // автоматически из UI каждый кадр (см. self.controller_connected /
    // self.throttle_connected), не пользовательский чекбокс. Используется
    // rumble-движком, чтобы при подключённом ТОЛЬКО ОДНОМ устройстве
    // сливать в него весь эффект целиком (см. split_touchdown выше — та же
    // идея, но для Engine Start), а не терять "чужую" половину эффекта в
    // канале несуществующего устройства. Default true/true — если по
    // какой-то причине UI ещё не успел выставить актуальное значение
    // (первый кадр), считаем оба устройства подключёнными — прежнее
    // поведение "как есть", без потери вибрации.
    pub joystick_hw_connected: bool,
    pub throttle_hw_connected: bool,

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
    pub gear_bump_duration_s: f64,

    pub ground_enabled: bool,
    pub flaps_enabled: bool,
    pub gear_enabled: bool,
    pub stall_enabled: bool,

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

    // Зеркалирование посадки рук: по умолчанию side-bound эффекты
    // (engine-start pre-combustion, split_touchdown — см. поле ниже) считают,
    // что РУД — под левой рукой, джойстик — под правой (Eng1/left strut → РУД,
    // Eng2/right strut → джойстик). Если физически джойстик стоит слева, а РУД
    // справа — этот флаг меняет местами, какая сторона считается "рукой РУД",
    // а какая "рукой джойстика". Для двигателей это не трогает маршрутизацию
    // по мотору РУД (throttle_left/throttle_right остаются жёстко привязаны к
    // Eng1/Eng2 — это железо квадранта, а не поза за столом), но для
    // split_touchdown ЭТОТ флаг как раз и определяет, какая основная стойка
    // (лево/право) достаётся РУДу, а какая — джойстику (см. split_touchdown).
    pub swap_hand_layout: bool,

    // Привязка каждого эффекта к устройствам (Джойстик / РУД / оба).
    // #[serde(default)] на уровне структуры уже гарантирует, что старые
    // settings.json без этого поля подхватят EffectDeviceTargets::default()
    // (все эффекты → джойстик, РУД выключен — прежнее поведение).
    pub device_targets: EffectDeviceTargets,

    // Состояние UI (не параметр эффекта): развёрнута ли секция телеметрии
    // (Live Aircraft Data / Engine Telemetry) под кнопкой "Telemetry".
    // Хранится здесь только потому, что это уже готовый персистентный
    // канал (settings.json) — по умолчанию (и для старых settings.json без
    // этого поля, см. #[serde(default)] на структуре) секция развёрнута.
    pub telemetry_expanded: bool,

    // Настройки этапа 1 поддержки War Thunder (см. settings::game_override —
    // то, какая игра сейчас активна, лежит там, не здесь, т.к. это глобальный
    // переключатель, а не параметр эффекта, привязанный к профилю борта).
    pub wt: WtConfig,
}

impl Default for RumbleConfig {
    fn default() -> Self {
        Self {
            overspeed_enabled: true,
            overspeed_intensity: 100.0,
            overspeed_max_kn: 350.0,
            overspeed_override_enabled: false,
            overspeed_manual_kn: 350.0,
            overspeed_lear_horn_enabled: false,

            bank_enabled: true,
            bank_intensity: 70.0,
            bank_threshold_deg: 45.0,

            ground_roll: 7.5,  // 15% от техн. предела 50
            flaps_peak: 153.0, // ~0.6 duty cycle — прежняя фиксированная сила эффекта
            gear_peak: 110.0,
            stall_ceiling: 10.0, // жёсткий потолок STALL_CEILING_HARD_CAP в rumble.rs — см. там
            max_output: 255,
            smoothing_alpha: 0.18,
            ias_deadband_kn: 1.0,
            taxi_start_enabled: true,
            taxi_start_kn: 1.0,
            taxi_end_enabled: true,
            taxi_end_kn: 120.0,
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
            gear_bump_duration_s: 0.8,

            ground_enabled: true,
            flaps_enabled: true,
            gear_enabled: false, // "Landing Gear (bump)" — временно отключён и скрыт из UI, см. ui.rs
            stall_enabled: true,

            spoilers_enabled: true,
            spoilers_threshold_pct: 10.0,
            spoilers_intensity: 90.0,

            gear_comp_enabled: true,
            gear_comp_nose_enabled: true,
            gear_comp_nose_peak: 55.0, // 100% в UI: полный диапазон длительности 230..550мс по жёсткости посадки (см. rumble.rs)
            gear_comp_left_enabled: true,
            gear_comp_left_peak: 55.0,
            gear_comp_right_enabled: true,
            gear_transit_enabled: true,
            gear_comp_right_peak: 55.0,
            split_touchdown: false,
            joystick_hw_connected: true,
            throttle_hw_connected: true,

            is_combat_edition: false,

            enable_engine_start: true,
            engine_start_strength: 100.0,
            engine_idle_n2: 60.0,
            four_engine_mode: false,
            swap_hand_layout: false,

            device_targets: EffectDeviceTargets::default(),

            telemetry_expanded: false,

            wt: WtConfig::default(),
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

    // War Thunder (этап 1): flaps_bump_active/gear_transit_active выше уже
    // generic (не завязаны в имени на MSFS) и переиспользуются как есть для
    // Flaps/Gear Transit & Doors в режиме WT. Weapon1/Weapon2 — новые, своего
    // аналога в MSFS-наборе effects нет.
    pub wt_weapon1_active: bool,
    pub wt_weapon2_active: bool,
    pub wt_overspeed_active: bool,
    pub wt_gear_overspeed_active: bool,
}

#[derive(Debug)]
pub enum HidCmd {
    /// Раздельная интенсивность для Combat Joystick R и для WINCTRL URSA
    /// MINOR Throttle (РУД) — какое значение реально уйдёт на каждое
    /// устройство, решает EffectDeviceTarget конкретного эффекта в rumble.rs.
    /// РУД имеет два независимых вибромотора (левый/правый), поэтому
    /// throttle тоже разбит на два канала.
    SendIntensity {
        joystick: u8,
        throttle_left: u8,
        throttle_right: u8,
    },
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

    /// Создаёт ConfigShared с уже готовым конфигом (например, полем `default`
    /// загруженного с диска SettingsFile — см. main.rs).
    pub fn new_with(cfg: RumbleConfig) -> Self {
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

    pub wt_weapon1_active: AtomicBool,
    pub wt_weapon2_active: AtomicBool,
    pub wt_overspeed_active: AtomicBool,
    pub wt_gear_overspeed_active: AtomicBool,
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

        self.wt_weapon1_active
            .store(snap.wt_weapon1_active, Ordering::Relaxed);
        self.wt_weapon2_active
            .store(snap.wt_weapon2_active, Ordering::Relaxed);
        self.wt_overspeed_active
            .store(snap.wt_overspeed_active, Ordering::Relaxed);
        self.wt_gear_overspeed_active
            .store(snap.wt_gear_overspeed_active, Ordering::Relaxed);
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SimStatus {
    #[default]
    Disconnected,
    Connected,
    InFlight,
    /// Клиентскую библиотеку SimConnect загрузить не удалось — это не то же
    /// самое, что Disconnected («симулятор не запущен»): здесь перезапуск сима
    /// не поможет, нужно действие пользователя. Без отдельного статуса воркер
    /// молча завершался, а бейдж оставался Disconnected — причина сбоя видна
    /// была только в файле лога, панель которого в релизной сборке скрыта.
    SimConnectMissing,
}

/// Какая игра сейчас владеет HID-каналом/GUI. Не дублирует `SimStatus` —
/// `SimStatus` про "глубину" соединения конкретного конвейера, `ActiveGame`
/// про то, какая игра активна прямо сейчас (см. `crate::game_state::GameSlot`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ActiveGame {
    #[default]
    None,
    Msfs,
    Wt,
}

/// Ручной оверрайд автоопределения (меню Опции). Персистится в SettingsFile,
/// как `Lang` (см. `i18n.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GameOverride {
    #[default]
    Auto,
    ForceMsfs,
    ForceWt,
}
