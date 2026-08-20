//! Модель одного пользовательского эффекта вибрации: чистые serde-структуры
//! без побочных эффектов и без ввода-вывода. Движок (другой агент, будущий
//! `engine.rs`) на каждый тик берёт готовый `TelemetryFrame` (см.
//! `super::sources`), прогоняет его через `ResponseCurve`/`Shape` и получает
//! итоговую интенсивность; UI (тоже другой агент) читает и правит эти
//! структуры напрямую и сохраняет их тем же каналом, что и `RumbleConfig`
//! (см. `settings.rs`).

use crate::custom_fx::sources::SourceId;
use crate::types::ActiveGame;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Потолок частоты для `Shape::Pulse`/`Sine`/`Sawtooth`. Тот же Nyquist-предел,
/// что уже подобран для `WtConfig::weapon1_gun` (types.rs) и объяснён там же:
/// HID-канал шлёт интенсивность не чаще 20 Гц (см. `hid/worker.rs`
/// SEND_INTERVAL), частоты выше половины этого — алиасятся и "съедаются"
/// между отправками. Единый потолок применяется здесь для ЛЮБОГО
/// пользовательского эффекта, а не только для встроенной стрельбы WT.
pub const MAX_SHAPE_FREQ_HZ: f32 = 6.5;

/// Один пользовательский эффект вибрации целиком: откуда брать значение,
/// когда считать его "активным", как превратить сырое значение в
/// интенсивность и куда её отправить.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomEffect {
    /// Стабильный uid — см. `new_effect`. Не путать с `name`: `name`
    /// пользователь может переименовать, `id` — нет (на него ссылаются
    /// сохранённые пресеты/порядок в списке UI).
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub games: GameMask,
    /// Фильтр по подстроке в названии борта (aircraft title в MSFS/X-Plane,
    /// `vehicle_type` в WT). `None` = применяется к любому борту.
    pub aircraft_match: Option<String>,
    pub source: SourceId,
    /// Имя и единица пользовательской MSFS LVAR — заполнено только когда
    /// `source == SourceId::Lvar`, для остальных источников всегда `None`.
    /// Хранится ВНУТРИ эффекта, а не отдельной таблицей со ссылкой по
    /// индексу/id: эффекты экспортируются файлом и передаются другим людям, и
    /// при хранении отдельной таблицей экспортированный эффект приезжал бы с
    /// висящей ссылкой на переменную, которой у получателя нет (или которая
    /// там означает что-то другое). Так эффект остаётся самодостаточным —
    /// весь его смысл читается из одного JSON-объекта.
    #[serde(default)]
    pub lvar: Option<LvarSpec>,
    pub trigger: Trigger,
    /// Сырое значение источника (в его единицах, см. `SourceDef::unit`) ->
    /// нормализованная интенсивность 0..1, ДО применения `shape`/`strength_pct`.
    pub curve: ResponseCurve,
    /// EMA-сглаживание итоговой интенсивности: 0.0 = без сглаживания
    /// (мгновенный отклик на каждый тик), ближе к 1.0 — всё более вялый.
    pub smoothing_alpha: f32,
    pub shape: Shape,
    /// Итоговая сила эффекта, 0..100 — множитель поверх кривой/формы, а не
    /// абсолютная величина ШИМ (пересчёт в 0..255 — забота движка).
    pub strength_pct: f32,
    pub out: OutputRouting,
    /// Как сводиться с ДРУГИМИ кастомными эффектами, если оба целятся в один
    /// и тот же мотор одновременно.
    pub mix: MixMode,
}

impl Default for CustomEffect {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: true,
            games: GameMask::default(),
            aircraft_match: None,
            source: SourceId::FlightAirspeedKn,
            lvar: None,
            trigger: Trigger::default(),
            curve: ResponseCurve::default(),
            smoothing_alpha: 0.0,
            shape: Shape::default(),
            strength_pct: 50.0,
            out: OutputRouting::default(),
            mix: MixMode::default(),
        }
    }
}

static EFFECT_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Создаёт новый эффект с разумными дефолтами и стабильным уникальным id.
/// uuid сознательно не используется (новые крейты добавлять запрещено) —
/// unix-время в миллисекундах + монотонный счётчик в пределах процесса дают
/// уникальность даже при создании нескольких эффектов за один кадр UI (когда
/// миллисекунды могут совпасть).
pub fn new_effect(name: String, source: SourceId) -> CustomEffect {
    let unix_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let counter = EFFECT_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    CustomEffect {
        id: format!("fx-{unix_millis}-{counter}"),
        name,
        source,
        ..CustomEffect::default()
    }
}

/// Имя и единица одной пользовательской MSFS LVAR — то, что пользователь
/// вписывает в UI при выборе источника `SourceId::Lvar`. `unit` нужен
/// отдельно от имени, потому что `SimConnect_AddToDataDefinition` требует
/// единицу измерения явно (`"Number"`, `"Percent"`, `"Bool"`, `"Knots"`,
/// ...) — регистрация переменной (этап 2, `sim/worker.rs`) без неё
/// невозможна.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LvarSpec {
    /// Имя переменной ровно как ввёл пользователь, например
    /// `"L:A320_Gear_Nose"`.
    pub name: String,
    /// Единица SimConnect для `AddToDataDefinition`, например `"Number"`.
    pub unit: String,
}

/// Для каких конвейеров телеметрии активен эффект. Отдельно от
/// `aircraft_match`: эта маска фильтрует по игре целиком, `aircraft_match` —
/// по конкретному борту уже внутри выбранной игры.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameMask {
    pub msfs: bool,
    pub xplane: bool,
    pub wt: bool,
}

impl Default for GameMask {
    fn default() -> Self {
        // По умолчанию эффект работает во всех играх — источник, которого
        // нет в текущем конвейере, и так самонейтрализуется через
        // sources::read -> None (движок просто не получит значение). Эта
        // маска — дополнительный, более грубый ручной рубильник для UI.
        Self {
            msfs: true,
            xplane: true,
            wt: true,
        }
    }
}

impl GameMask {
    pub fn allows(&self, game: ActiveGame) -> bool {
        match game {
            ActiveGame::None => false,
            ActiveGame::Msfs => self.msfs,
            ActiveGame::Xplane => self.xplane,
            ActiveGame::Wt => self.wt,
        }
    }
}

/// Условие, при котором эффект переходит из "молчит" в "активен".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum Trigger {
    /// Активен всегда, пока разрешён `GameMask`/`aircraft_match` — амплитуду
    /// целиком определяет `ResponseCurve`.
    #[default]
    Always,
    /// Активен, пока значение источника выше `value`. `hysteresis` — на
    /// сколько нужно опуститься НИЖЕ `value`, чтобы выключиться обратно
    /// (гасит дребезг на границе, тот же приём, что overspeed/taxi в
    /// rumble.rs).
    Above {
        value: f64,
        hysteresis: f64,
    },
    Below {
        value: f64,
        hysteresis: f64,
    },
    Between {
        lo: f64,
        hi: f64,
    },
    /// Для булевых источников (0.0/1.0) — активен, когда значение "истинно".
    IsTrue,
    /// Импульс на каждое изменение значения источника больше чем на `eps`,
    /// удерживаемый минимум `hold_s` секунд (аналог flaps_bump/gear_bump в
    /// rumble.rs — короткий скачок за один тик не должен теряться за один
    /// PWM-кадр).
    Changed {
        eps: f64,
        hold_s: f64,
    },
}

/// Кусочно-линейная функция отклика: вход — сырое значение источника (в его
/// единицах, см. `SourceDef::unit`), выход — нормализованная интенсивность
/// 0..1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResponseCurve {
    /// Инвариант: отсортированы по `x` по возрастанию. Поддерживает его UI
    /// вызовом `sorted()` после правки — сам `eval` не сортирует на каждый
    /// вызов, это был бы лишний O(n log n) на каждый тик движка ради
    /// инварианта, который меняется только руками пользователя.
    pub points: Vec<CurvePoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CurvePoint {
    pub x: f64,
    pub y: f64,
}

impl Default for ResponseCurve {
    fn default() -> Self {
        Self::linear(0.0, 1.0)
    }
}

impl ResponseCurve {
    /// Две точки: (x0, 0) -> (x1, 1) — самый частый случай ("чем больше
    /// значение, тем сильнее эффект").
    pub fn linear(x0: f64, x1: f64) -> Self {
        Self {
            points: vec![CurvePoint { x: x0, y: 0.0 }, CurvePoint { x: x1, y: 1.0 }],
        }
    }

    /// Кусочно-линейная интерполяция с зажимом за пределами диапазона.
    /// Пустая кривая -> 0.0, кривая из одной точки -> её `y` (тоже зажатый в
    /// 0..1) — оба случая не паникуют, чтобы недособранный пресет из UI не
    /// ронял движок.
    pub fn eval(&self, x: f64) -> f64 {
        match self.points.as_slice() {
            [] => 0.0,
            [only] => only.y.clamp(0.0, 1.0),
            points => {
                let first = points[0];
                let last = points[points.len() - 1];
                if x <= first.x {
                    return first.y.clamp(0.0, 1.0);
                }
                if x >= last.x {
                    return last.y.clamp(0.0, 1.0);
                }
                for w in points.windows(2) {
                    let (p0, p1) = (w[0], w[1]);
                    if x >= p0.x && x <= p1.x {
                        let dx = p1.x - p0.x;
                        let y = if dx.abs() < f64::EPSILON {
                            p1.y
                        } else {
                            p0.y + (x - p0.x) / dx * (p1.y - p0.y)
                        };
                        return y.clamp(0.0, 1.0);
                    }
                }
                // Недостижимо при отсортированных точках и x внутри
                // [first.x, last.x], но на временно несортированном вводе
                // (UI ещё не вызвал sorted() после перетаскивания точки)
                // безопасный 0.0 лучше паники.
                0.0
            }
        }
    }

    /// Нормализует порядок точек по x. Вызывается UI после ручной правки —
    /// перетаскивание точки на графике может нарушить порядок.
    pub fn sorted(&mut self) {
        self.points
            .sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    }
}

/// Форма сигнала интенсивности ПОВЕРХ значения `ResponseCurve` — например,
/// `Pulse` превращает постоянный уровень 0.7 в мигающий ШИМ с той же
/// средней амплитудой, `Constant` ничего не меняет.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum Shape {
    #[default]
    Constant,
    /// Периодические импульсы — те же параметры, что у `GunPreset` (types.rs,
    /// текстура пулемёта/пушки WT), но настраиваемые пользователем для
    /// произвольного источника.
    Pulse {
        freq_hz: f32,
        duty_pct: f32,
        jitter_pct: f32,
        floor_pct: f32,
        attack_ms: f32,
    },
    /// Один импульс на срабатывание триггера: атака-полка-спад (см.
    /// gear_comp_* touchdown в rumble.rs). `decay_exp` — показатель степени
    /// кривой спада (1 = линейный, >1 = более резкий сброс в начале).
    OneShot {
        attack_ms: f32,
        hold_ms: f32,
        decay_ms: f32,
        decay_exp: i32,
    },
    Sine {
        freq_hz: f32,
        depth_pct: f32,
    },
    Sawtooth {
        freq_hz: f32,
        depth_pct: f32,
    },
}

impl Shape {
    pub fn freq_hz(&self) -> Option<f32> {
        match self {
            Shape::Pulse { freq_hz, .. }
            | Shape::Sine { freq_hz, .. }
            | Shape::Sawtooth { freq_hz, .. } => Some(*freq_hz),
            Shape::Constant | Shape::OneShot { .. } => None,
        }
    }

    /// Режет частоту до `MAX_SHAPE_FREQ_HZ` — вызывается UI после ввода
    /// пользователем произвольного числа (см. doc-комментарий константы про
    /// Nyquist-предел HID-канала). У форм без частоты — no-op.
    pub fn clamp_freq(&mut self) {
        if let Shape::Pulse { freq_hz, .. }
        | Shape::Sine { freq_hz, .. }
        | Shape::Sawtooth { freq_hz, .. } = self
        {
            *freq_hz = freq_hz.min(MAX_SHAPE_FREQ_HZ);
        }
    }
}

/// Маршрутизация одного эффекта на моторы. По смыслу как
/// `EffectDeviceTarget` (types.rs), но с раздельными левым/правым мотором
/// РУД — кастомному эффекту может понадобиться только один из двух.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputRouting {
    pub joystick: bool,
    pub throttle_left: bool,
    pub throttle_right: bool,
}

impl Default for OutputRouting {
    fn default() -> Self {
        Self {
            joystick: true,
            throttle_left: false,
            throttle_right: false,
        }
    }
}

/// Как сводить несколько кастомных эффектов, которые в один и тот же момент
/// целятся в один и тот же мотор.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum MixMode {
    /// Берётся максимум — безопасный дефолт: несколько эффектов не
    /// "перегружают" мотор суммой, заметен только самый сильный из активных.
    #[default]
    Max,
    /// Складываются (итоговый зажим на 255 — забота движка) — для
    /// намеренного усиления комбинацией нескольких эффектов.
    Add,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_fx::sources::SourceId;

    #[test]
    fn eval_hits_nodes_exactly() {
        let c = ResponseCurve {
            points: vec![
                CurvePoint { x: 0.0, y: 0.0 },
                CurvePoint { x: 10.0, y: 1.0 },
            ],
        };
        assert_eq!(c.eval(0.0), 0.0);
        assert_eq!(c.eval(10.0), 1.0);
    }

    #[test]
    fn eval_interpolates_between_nodes() {
        let c = ResponseCurve::linear(0.0, 10.0);
        assert!((c.eval(5.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn eval_clamps_outside_range() {
        let c = ResponseCurve::linear(0.0, 10.0);
        assert_eq!(c.eval(-5.0), 0.0);
        assert_eq!(c.eval(50.0), 1.0);
    }

    #[test]
    fn eval_single_point_and_empty_do_not_panic() {
        let single = ResponseCurve {
            points: vec![CurvePoint { x: 3.0, y: 0.4 }],
        };
        assert_eq!(single.eval(100.0), 0.4);

        let empty = ResponseCurve { points: vec![] };
        assert_eq!(empty.eval(1.0), 0.0);
    }

    #[test]
    fn sorted_normalizes_out_of_order_points() {
        let mut c = ResponseCurve {
            points: vec![
                CurvePoint { x: 10.0, y: 1.0 },
                CurvePoint { x: 0.0, y: 0.0 },
            ],
        };
        c.sorted();
        assert_eq!(c.points[0].x, 0.0);
        assert_eq!(c.points[1].x, 10.0);
        assert!((c.eval(5.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn shape_clamp_freq_caps_at_max() {
        let mut s = Shape::Sine {
            freq_hz: 50.0,
            depth_pct: 100.0,
        };
        s.clamp_freq();
        assert_eq!(s.freq_hz(), Some(MAX_SHAPE_FREQ_HZ));
    }

    #[test]
    fn shape_clamp_freq_is_noop_without_frequency() {
        let mut s = Shape::Constant;
        s.clamp_freq();
        assert_eq!(s.freq_hz(), None);
    }

    #[test]
    fn custom_effect_roundtrips_through_json() {
        let mut fx = new_effect("Test".into(), SourceId::WtAoaDeg);
        fx.trigger = Trigger::Above {
            value: 15.0,
            hysteresis: 1.0,
        };
        fx.shape = Shape::Pulse {
            freq_hz: 4.0,
            duty_pct: 30.0,
            jitter_pct: 5.0,
            floor_pct: 10.0,
            attack_ms: 20.0,
        };
        let json = serde_json::to_string(&fx).unwrap();
        let back: CustomEffect = serde_json::from_str(&json).unwrap();
        assert_eq!(fx, back);
    }

    #[test]
    fn new_effect_ids_are_unique() {
        let a = new_effect("A".into(), SourceId::FlightAirspeedKn);
        let b = new_effect("B".into(), SourceId::FlightAirspeedKn);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn custom_effect_with_lvar_roundtrips_through_json() {
        let mut fx = new_effect("Test".into(), SourceId::Lvar);
        fx.lvar = Some(LvarSpec {
            name: "L:A320_Gear_Nose".into(),
            unit: "Number".into(),
        });
        let json = serde_json::to_string(&fx).unwrap();
        let back: CustomEffect = serde_json::from_str(&json).unwrap();
        assert_eq!(fx, back);
    }

    #[test]
    fn custom_effect_without_lvar_field_reads_from_old_json() {
        // Обратная совместимость: пресет, сохранённый ДО появления поля
        // `lvar`, не содержит его в JSON вообще — должен читаться как `None`
        // благодаря `#[serde(default)]` на поле.
        let fx = new_effect("Test".into(), SourceId::FlightAirspeedKn);
        let mut value = serde_json::to_value(&fx).unwrap();
        value.as_object_mut().unwrap().remove("lvar");
        let json = serde_json::to_string(&value).unwrap();
        assert!(!json.contains("\"lvar\""));

        let restored: CustomEffect = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.lvar, None);
    }

    #[test]
    fn game_mask_allows() {
        let mask = GameMask {
            msfs: true,
            xplane: false,
            wt: true,
        };
        assert!(mask.allows(ActiveGame::Msfs));
        assert!(!mask.allows(ActiveGame::Xplane));
        assert!(mask.allows(ActiveGame::Wt));
        assert!(!mask.allows(ActiveGame::None));
    }
}
