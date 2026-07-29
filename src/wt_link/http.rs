//! Тонкая обёртка над блокирующим reqwest-клиентом для War Thunder HTTP API.
//!
//! Общий для recon-инструмента (`wt_probe`/`wt_probe_gui`, feature
//! "wt-probe") и продового конвейера (`wt_link::worker`, feature "app") —
//! отсюда и расположение в `wt_link`, а не внутри `wt_probe` (тот собирается
//! под отдельной фичей, без "app", и наоборот). `wt_probe::http`
//! реэкспортирует этот модуль как есть, чтобы не дублировать код.
//!
//! Выбран `reqwest::blocking` (не tokio): опрос устроен как несколько
//! независимых потоков с собственным ритмом (20/5/1 Гц) — это ровно то, для
//! чего годится модель "поток на источник" без разделяемого рантайма.
//! Асинхронный рантайм не даёт здесь ничего, кроме лишней зависимости и
//! сложности, а blocking-клиент уже используется в updater.rs этого же
//! проекта.

use std::time::Duration;

use reqwest::blocking::Client;
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum FetchError {
    /// TCP-соединение не установлено — сервер не поднят (игра не запущена).
    Connect(String),
    /// Запрос не уложился в таймаут.
    Timeout,
    /// Сервер ответил, но не 2xx.
    Status(u16),
    /// Тело ответа не удалось прочитать или разобрать как JSON.
    Decode(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Connect(m) => write!(f, "нет соединения: {m}"),
            FetchError::Timeout => write!(f, "таймаут запроса"),
            FetchError::Status(code) => write!(f, "HTTP {code}"),
            FetchError::Decode(m) => write!(f, "не удалось разобрать JSON: {m}"),
        }
    }
}

/// `reqwest::blocking::Client` внутри — это `Arc` на пул соединений, клон
/// дешёвый и безопасно расшаривается между потоками опроса.
#[derive(Clone)]
pub struct WtClient {
    client: Client,
    base: String,
}

impl WtClient {
    pub fn new(host: &str, port: u16, timeout: Duration) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .build()
            .map_err(|e| format!("не удалось создать HTTP-клиент: {e}"))?;
        Ok(Self {
            client,
            base: format!("http://{host}:{port}"),
        })
    }

    pub fn state(&self) -> Result<Value, FetchError> {
        self.get("/state")
    }

    pub fn indicators(&self) -> Result<Value, FetchError> {
        self.get("/indicators")
    }

    pub fn hudmsg(&self, last_evt: i64, last_dmg: i64) -> Result<Value, FetchError> {
        let url = format!("{}/hudmsg", self.base);
        let resp = self
            .client
            .get(&url)
            .query(&[("lastEvt", last_evt), ("lastDmg", last_dmg)])
            .send();
        Self::handle(resp)
    }

    pub fn map_obj(&self) -> Result<Value, FetchError> {
        self.get("/map_obj.json")
    }

    pub fn mission(&self) -> Result<Value, FetchError> {
        self.get("/mission.json")
    }

    pub fn map_info(&self) -> Result<Value, FetchError> {
        self.get("/map_info.json")
    }

    fn get(&self, path: &str) -> Result<Value, FetchError> {
        let url = format!("{}{}", self.base, path);
        let resp = self.client.get(&url).send();
        Self::handle(resp)
    }

    /// Тело парсится в `Value` сразу здесь и дальше везде передаётся уже как
    /// `Value` (а не как исходный текст ответа): War Thunder отдаёт /state и
    /// /hudmsg pretty-printed, с настоящими переводами строк внутри тела —
    /// хранить такой текст как есть было бы нельзя, это сломало бы формат
    /// JSON Lines (одна запись — одна строка) в session_*.jsonl. Компактная
    /// пересериализация `Value` в writer.rs — единственный способ гарантировать
    /// валидный JSONL, не теряя при этом ни одного значения.
    fn handle(
        resp: Result<reqwest::blocking::Response, reqwest::Error>,
    ) -> Result<Value, FetchError> {
        let resp = resp.map_err(|e| {
            if e.is_timeout() {
                FetchError::Timeout
            } else {
                FetchError::Connect(e.to_string())
            }
        })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(FetchError::Status(status.as_u16()));
        }
        let text = resp.text().map_err(|e| FetchError::Decode(e.to_string()))?;
        serde_json::from_str(text.trim()).map_err(|e| FetchError::Decode(e.to_string()))
    }
}

/// Ищет максимальный "id" среди элементов JSON-массива по ключу `array_key`
/// в объекте `body` — используется для продвижения курсоров lastEvt/lastDmg
/// у /hudmsg. Это часть документированного протокола самого эндпоинта
/// (курсорная модель по id), а не угадывание внутренних имён полей игры.
pub fn max_id_in_array(body: &Value, array_key: &str) -> Option<i64> {
    body.get(array_key)?
        .as_array()?
        .iter()
        .filter_map(|e| e.get("id")?.as_i64())
        .max()
}
