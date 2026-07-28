//! wt_probe — автономная разведывательная утилита War Thunder HTTP-телеметрии.
//!
//! Подключается к http://host:port (по умолчанию 127.0.0.1:8111), которое
//! сама игра поднимает во время боя, опрашивает /state, /indicators,
//! /hudmsg, /map_obj.json, /mission.json, /map_info.json каждый в своём
//! потоке со своим ритмом, пишет сырой JSON построчно (JSON Lines) и по
//! завершении сессии (Ctrl+C) строит два Markdown-отчёта: по обнаруженной
//! схеме полей и по обнаруженным счётчикам боеприпасов/темпу стрельбы.
//!
//! Утилита не управляет никаким железом и не реализует тактильные эффекты —
//! только сбор и предварительный анализ данных для их последующего
//! офлайн-проектирования.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Local;
use crossbeam_channel::unbounded;

use aurora_vibra::wt_probe::cli::Args;
use aurora_vibra::wt_probe::dashboard::Dashboard;
use aurora_vibra::wt_probe::http::WtClient;
use aurora_vibra::wt_probe::model::{Endpoint, PollOutcome};
use aurora_vibra::wt_probe::shared::{Shared, SharedHandle};
use aurora_vibra::wt_probe::writer::WriterResult;
use aurora_vibra::wt_probe::{poller, replay, schema, weapons, writer};

fn main() {
    let args = Args::parse_args();
    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let shutdown = shutdown.clone();
        // Обработчик можно установить только один раз за процесс; если это
        // почему-то не удалось — работаем дальше без перехвата: Ctrl+C
        // тогда просто завершит процесс штатно (без наших отчётов), но без
        // паники внутри самой утилиты.
        if let Err(e) = ctrlc::set_handler(move || {
            shutdown.store(true, Ordering::Relaxed);
        }) {
            eprintln!("предупреждение: не удалось установить обработчик Ctrl+C: {e}");
        }
    }

    let code = match &args.replay {
        Some(path) => run_replay(&args, path, shutdown),
        None => run_live(&args, shutdown),
    };
    std::process::exit(code);
}

fn run_live(args: &Args, shutdown: Arc<AtomicBool>) -> i32 {
    let client = match WtClient::new(
        &args.host,
        args.port,
        Duration::from_millis(args.timeout_ms),
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return 1;
        }
    };

    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let recording = !args.dashboard_only;

    if let Err(e) = fs::create_dir_all(&args.out_dir) {
        eprintln!(
            "ошибка: не удалось создать каталог {}: {e}",
            args.out_dir.display()
        );
        return 1;
    }

    let mut outputs = writer::Outputs {
        session: None,
        events: None,
    };
    let mut session_path = None;
    let mut events_path = None;
    if recording {
        let sp = args.out_dir.join(format!("session_{timestamp}.jsonl"));
        let ep = args.out_dir.join(format!("events_{timestamp}.jsonl"));
        let session_file = match File::create(&sp) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("ошибка: не удалось создать {}: {e}", sp.display());
                return 1;
            }
        };
        let events_file = match File::create(&ep) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("ошибка: не удалось создать {}: {e}", ep.display());
                return 1;
            }
        };
        outputs.session = Some(BufWriter::new(session_file));
        outputs.events = Some(BufWriter::new(events_file));
        session_path = Some(sp);
        events_path = Some(ep);
    }

    let schema_path = args.out_dir.join(format!("schema_{timestamp}.md"));
    let weapons_path = args.out_dir.join(format!("weapons_{timestamp}.md"));

    let (tx, rx) = unbounded::<PollOutcome>();
    let session_start = Instant::now();
    let shared: SharedHandle = Arc::new(Mutex::new(Shared::new()));

    let handles = vec![
        poller::spawn_fixed(
            client.clone(),
            Endpoint::State,
            args.state_hz,
            session_start,
            tx.clone(),
            shutdown.clone(),
        ),
        poller::spawn_fixed(
            client.clone(),
            Endpoint::Indicators,
            args.indicators_hz,
            session_start,
            tx.clone(),
            shutdown.clone(),
        ),
        poller::spawn_hudmsg(
            client.clone(),
            args.hudmsg_hz,
            session_start,
            tx.clone(),
            shutdown.clone(),
        ),
        poller::spawn_fixed(
            client.clone(),
            Endpoint::MapObj,
            args.context_hz,
            session_start,
            tx.clone(),
            shutdown.clone(),
        ),
        poller::spawn_fixed(
            client.clone(),
            Endpoint::Mission,
            args.context_hz,
            session_start,
            tx.clone(),
            shutdown.clone(),
        ),
        poller::spawn_fixed(
            client,
            Endpoint::MapInfo,
            args.context_hz,
            session_start,
            tx.clone(),
            shutdown.clone(),
        ),
    ];
    // Наш собственный клон отправителя роняем: канал закроется сам, как
    // только последний poller-поток завершится и отбросит свой клон — тогда
    // writer-поток естественно выходит из цикла по Disconnected.
    drop(tx);

    let writer_shared = shared.clone();
    let writer_shutdown = shutdown.clone();
    let writer_handle =
        thread::spawn(move || writer::run(rx, outputs, writer_shared, writer_shutdown));

    let targets = BTreeMap::from([
        (Endpoint::State, args.state_hz),
        (Endpoint::Indicators, args.indicators_hz),
        (Endpoint::HudMsg, args.hudmsg_hz),
        (Endpoint::MapObj, args.context_hz),
        (Endpoint::Mission, args.context_hz),
        (Endpoint::MapInfo, args.context_hz),
    ]);
    let mut dashboard = Dashboard::new(targets, recording);
    let frame = Duration::from_secs_f64(1.0 / args.dashboard_hz.max(0.1));
    while !shutdown.load(Ordering::Relaxed) {
        dashboard.render(&shared);
        thread::sleep(frame);
    }
    dashboard.render(&shared);

    for h in handles {
        let _ = h.join();
    }
    let result = match writer_handle.join() {
        Ok(r) => r,
        Err(_) => {
            eprintln!(
                "предупреждение: поток записи завершился с паникой, отчёты могут быть неполными"
            );
            WriterResult {
                schema: schema::SchemaTracker::new(),
                weapons: weapons::WeaponDetector::new(),
            }
        }
    };

    write_reports(&schema_path, &weapons_path, &result);

    let (lines_written, bytes_written) = {
        let s = shared.lock().unwrap_or_else(|e| e.into_inner());
        (s.lines_written, s.bytes_written)
    };
    aurora_vibra::wt_probe::dashboard::print_final_summary(
        session_path.as_deref(),
        &schema_path,
        &weapons_path,
        events_path.as_deref(),
        lines_written,
        bytes_written,
    );
    0
}

fn run_replay(args: &Args, path: &Path, shutdown: Arc<AtomicBool>) -> i32 {
    if let Err(e) = fs::create_dir_all(&args.out_dir) {
        eprintln!(
            "ошибка: не удалось создать каталог {}: {e}",
            args.out_dir.display()
        );
        return 1;
    }
    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let schema_path = args.out_dir.join(format!("schema_replay_{timestamp}.md"));
    let weapons_path = args.out_dir.join(format!("weapons_replay_{timestamp}.md"));

    let shared: SharedHandle = Arc::new(Mutex::new(Shared::new()));

    let replay_shared = shared.clone();
    let replay_shutdown = shutdown.clone();
    let replay_path = path.to_path_buf();
    let speed = args.replay_speed;
    let replay_handle =
        thread::spawn(move || replay::run(&replay_path, speed, replay_shared, replay_shutdown));

    // Частоты опроса неприменимы при воспроизведении из файла — дашборд
    // покажет фактический темп чтения строк, целей для сравнения нет.
    let mut dashboard = Dashboard::new(BTreeMap::new(), false);
    let frame = Duration::from_secs_f64(1.0 / args.dashboard_hz.max(0.1));
    while !replay_handle.is_finished() && !shutdown.load(Ordering::Relaxed) {
        dashboard.render(&shared);
        thread::sleep(frame);
    }
    if !replay_handle.is_finished() {
        shutdown.store(true, Ordering::Relaxed);
    }
    dashboard.render(&shared);

    let result = match replay_handle.join() {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            eprintln!("ошибка воспроизведения: {e}");
            return 1;
        }
        Err(_) => {
            eprintln!("предупреждение: поток воспроизведения завершился с паникой");
            return 1;
        }
    };

    write_reports(&schema_path, &weapons_path, &result);
    println!();
    println!("Воспроизведение {} завершено.", path.display());
    println!("  Схема:  {}", schema_path.display());
    println!("  Оружие: {}", weapons_path.display());
    0
}

fn write_reports(schema_path: &Path, weapons_path: &Path, result: &WriterResult) {
    if let Err(e) = fs::write(schema_path, result.schema.to_markdown()) {
        eprintln!("ошибка: не удалось записать {}: {e}", schema_path.display());
    }
    if let Err(e) = fs::write(weapons_path, result.weapons.to_markdown()) {
        eprintln!(
            "ошибка: не удалось записать {}: {e}",
            weapons_path.display()
        );
    }
}
