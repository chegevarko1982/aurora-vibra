use std::path::PathBuf;

use clap::Parser;

/// wt_probe — разведывательная утилита War Thunder HTTP-телеметрии.
///
/// Опрашивает localhost:8111 (/state, /indicators, /hudmsg, /map_obj.json,
/// /mission.json, /map_info.json), пишет сырой JSONL-лог и по итогам сессии
/// строит отчёты об обнаруженной схеме полей и о поведении счётчиков
/// боеприпасов. Не управляет никаким железом и не реализует эффекты —
/// только сбор данных для последующего офлайн-проектирования.
#[derive(Parser, Debug, Clone)]
#[command(name = "wt_probe", version)]
pub struct Args {
    /// Хост War Thunder HTTP API.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Порт War Thunder HTTP API.
    #[arg(long, default_value_t = 8111)]
    pub port: u16,

    /// Частота опроса /state, Гц.
    #[arg(long = "state-hz", default_value_t = 20.0)]
    pub state_hz: f64,

    /// Частота опроса /indicators, Гц.
    #[arg(long = "indicators-hz", default_value_t = 20.0)]
    pub indicators_hz: f64,

    /// Частота опроса /hudmsg, Гц.
    #[arg(long = "hudmsg-hz", default_value_t = 5.0)]
    pub hudmsg_hz: f64,

    /// Частота опроса /map_obj.json, /mission.json, /map_info.json, Гц.
    #[arg(long = "context-hz", default_value_t = 1.0)]
    pub context_hz: f64,

    /// Таймаут одного HTTP-запроса, миллисекунды.
    #[arg(long = "timeout-ms", default_value_t = 150)]
    pub timeout_ms: u64,

    /// Каталог для выходных файлов сессии.
    #[arg(long = "out-dir", default_value = "wt_probe_sessions")]
    pub out_dir: PathBuf,

    /// Частота перерисовки TUI-дашборда, Гц.
    #[arg(long = "dashboard-hz", default_value_t = 4.0)]
    pub dashboard_hz: f64,

    /// Только дашборд: опрашивать и показывать вживую, но ничего не писать
    /// на диск (ни session_*.jsonl, ни events_*.jsonl). Отчёты по схеме и
    /// оружию по Ctrl+C всё равно печатаются в терминал.
    #[arg(long = "dashboard-only")]
    pub dashboard_only: bool,

    /// Не запускать живой опрос игры, а проиграть ранее записанный
    /// session_*.jsonl с исходным таймингом. Позволяет отлаживать логику без
    /// запущенной игры и гонять всё в CI на Linux.
    #[arg(long)]
    pub replay: Option<PathBuf>,

    /// Множитель скорости воспроизведения для --replay (2.0 = вдвое быстрее).
    #[arg(long = "replay-speed", default_value_t = 1.0)]
    pub replay_speed: f64,

    /// Свой позывной в War Thunder — включает детектор "попадания по мне" в
    /// /hudmsg.damage[] (см. src/wt_probe/hits.rs). Пусто = детектор
    /// выключен: игра не отдаёт позывной телеметрией, взять его неоткуда,
    /// кроме как от оператора.
    #[arg(long, default_value = "")]
    pub callsign: String,
}

impl Args {
    pub fn parse_args() -> Self {
        Args::parse()
    }
}
