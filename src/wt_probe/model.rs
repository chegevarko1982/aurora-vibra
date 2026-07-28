//! Общие типы, разделяемые между poller/writer/schema/weapons/dashboard.

use serde_json::Value;

/// Эндпоинты War Thunder HTTP API, которые опрашивает утилита.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Endpoint {
    State,
    Indicators,
    HudMsg,
    MapObj,
    Mission,
    MapInfo,
}

impl Endpoint {
    pub fn as_str(self) -> &'static str {
        match self {
            Endpoint::State => "state",
            Endpoint::Indicators => "indicators",
            Endpoint::HudMsg => "hudmsg",
            Endpoint::MapObj => "map_obj",
            Endpoint::Mission => "mission",
            Endpoint::MapInfo => "map_info",
        }
    }

    pub fn all() -> [Endpoint; 6] {
        [
            Endpoint::State,
            Endpoint::Indicators,
            Endpoint::HudMsg,
            Endpoint::MapObj,
            Endpoint::Mission,
            Endpoint::MapInfo,
        ]
    }

    pub fn parse_name(s: &str) -> Option<Endpoint> {
        match s {
            "state" => Some(Endpoint::State),
            "indicators" => Some(Endpoint::Indicators),
            "hudmsg" => Some(Endpoint::HudMsg),
            "map_obj" => Some(Endpoint::MapObj),
            "mission" => Some(Endpoint::Mission),
            "map_info" => Some(Endpoint::MapInfo),
            _ => None,
        }
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Успешно полученный и разобранный ответ одного эндпоинта.
#[derive(Debug, Clone)]
pub struct RawRecord {
    /// Секунды с начала записи сессии.
    pub t: f64,
    pub endpoint: Endpoint,
    /// Разобранное тело ответа. Для записи в session_*.jsonl оно compact-
    /// пересериализуется в writer.rs (см. комментарий у `WtClient::handle`
    /// в http.rs — почему не хранится исходный текст ответа как есть).
    pub parsed: Value,
}

/// Результат одного тика опроса: либо успех, либо сетевая/протокольная ошибка.
/// Ошибки тоже идут в канал — так writer-поток может обновить статус
/// соединения и показать причину на дашборде, не роняя поток опроса.
#[derive(Debug, Clone)]
pub enum PollOutcome {
    Ok(RawRecord),
    Err { endpoint: Endpoint, message: String },
}

/// Состояние соединения с игрой, определяемое по коду ответа и по полю
/// "valid" в /state (см. документацию эндпоинта в задаче).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnStatus {
    /// /state не отвечает вовсе (игра не запущена или порт закрыт).
    Disconnected,
    /// /state отвечает, но "valid": false — игра в меню/ангаре, не в бою.
    Menu,
    /// /state отвечает и "valid": true — идёт бой.
    InBattle,
}

impl std::fmt::Display for ConnStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ConnStatus::Disconnected => "нет игры (нет соединения)",
            ConnStatus::Menu => "меню/ангар",
            ConnStatus::InBattle => "в бою",
        };
        f.write_str(s)
    }
}

/// Одна плоская пара "путь.к.ключу" -> значение, полученная после
/// разворачивания вложенного JSON (см. schema::flatten).
pub type FlatPair = (String, Value);
