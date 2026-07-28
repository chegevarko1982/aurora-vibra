//! wt_probe_gui — та же разведка телеметрии War Thunder, что и wt_probe, но
//! вывод в собственное окно (eframe/egui) вместо консоли. Вся логика опроса/
//! записи/схемы/детекта оружия — из библиотеки (`aurora_vibra::wt_probe`,
//! см. src/wt_probe/), здесь только сборка UI и управление жизненным циклом
//! потоков опроса/записи из главного потока окна. Панель со значениями —
//! ровно тот же текст (с расшифровкой параметров), что строит
//! `Dashboard::format_text`, использованный и в текстовом wt_probe — чтобы
//! два бинарника не могли разойтись в том, что и как показывают.
//!
//! cargo run --bin wt_probe_gui --features wt-probe

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use aurora_vibra::wt_probe::dashboard::{self, Dashboard};
use aurora_vibra::wt_probe::http::WtClient;
use aurora_vibra::wt_probe::model::{Endpoint, PollOutcome};
use aurora_vibra::wt_probe::shared::{Shared, SharedHandle};
use aurora_vibra::wt_probe::writer::{self, WriterResult};
use aurora_vibra::wt_probe::{poller, schema, weapons};
use chrono::Local;
use crossbeam_channel::unbounded;
use eframe::egui;

struct Config {
    host: String,
    port: u16,
    state_hz: f64,
    indicators_hz: f64,
    hudmsg_hz: f64,
    context_hz: f64,
    timeout_ms: u64,
    dashboard_hz: f64,
    out_dir: String,
    dashboard_only: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8111,
            state_hz: 20.0,
            indicators_hz: 20.0,
            hudmsg_hz: 5.0,
            context_hz: 1.0,
            timeout_ms: 150,
            dashboard_hz: 4.0,
            out_dir: "wt_probe_sessions".to_string(),
            dashboard_only: false,
        }
    }
}

/// Всё, что живёт, пока сессия опроса запущена — потоки, разделяемое
/// состояние и накопленный за кадр текст панели (перестраивается не чаще
/// `dashboard_hz`, иначе format_text() гонялся бы на каждый egui-кадр,
/// а он может рисоваться заметно чаще).
struct RunningSession {
    shutdown: Arc<AtomicBool>,
    shared: SharedHandle,
    poller_handles: Vec<JoinHandle<()>>,
    writer_handle: JoinHandle<WriterResult>,
    dashboard: Dashboard,
    dashboard_hz: f64,
    cached_text: String,
    last_render: Instant,
    /// Состояние полей weapon1/weapon2 — читается каждый кадр отдельно от
    /// cached_text (который перестраивается не чаще dashboard_hz), чтобы
    /// цветной индикатор стрельбы реагировал без задержки панели.
    weapon_states: Vec<(String, f64)>,
    session_path: Option<PathBuf>,
    events_path: Option<PathBuf>,
    schema_path: PathBuf,
    weapons_path: PathBuf,
}

struct App {
    config: Config,
    session: Option<RunningSession>,
    status: String,
    last_summary: Option<String>,
}

impl App {
    fn new() -> Self {
        Self {
            config: Config::default(),
            session: None,
            status: "Настрой параметры и нажми «Старт», чтобы начать запись.".to_string(),
            last_summary: None,
        }
    }

    fn start(&mut self) {
        if self.session.is_some() {
            return;
        }
        let client = match WtClient::new(
            &self.config.host,
            self.config.port,
            Duration::from_millis(self.config.timeout_ms),
        ) {
            Ok(c) => c,
            Err(e) => {
                self.status = format!("Ошибка: {e}");
                return;
            }
        };

        let out_dir = PathBuf::from(&self.config.out_dir);
        if let Err(e) = fs::create_dir_all(&out_dir) {
            self.status = format!("Не удалось создать каталог {}: {e}", out_dir.display());
            return;
        }

        let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
        let recording = !self.config.dashboard_only;

        let mut outputs = writer::Outputs {
            session: None,
            events: None,
        };
        let mut session_path = None;
        let mut events_path = None;
        if recording {
            let sp = out_dir.join(format!("session_{timestamp}.jsonl"));
            let ep = out_dir.join(format!("events_{timestamp}.jsonl"));
            let session_file = match File::create(&sp) {
                Ok(f) => f,
                Err(e) => {
                    self.status = format!("Не удалось создать {}: {e}", sp.display());
                    return;
                }
            };
            let events_file = match File::create(&ep) {
                Ok(f) => f,
                Err(e) => {
                    self.status = format!("Не удалось создать {}: {e}", ep.display());
                    return;
                }
            };
            outputs.session = Some(BufWriter::new(session_file));
            outputs.events = Some(BufWriter::new(events_file));
            session_path = Some(sp);
            events_path = Some(ep);
        }

        let schema_path = out_dir.join(format!("schema_{timestamp}.md"));
        let weapons_path = out_dir.join(format!("weapons_{timestamp}.md"));

        let shutdown = Arc::new(AtomicBool::new(false));
        let (tx, rx) = unbounded::<PollOutcome>();
        let session_start = Instant::now();
        let shared: SharedHandle = Arc::new(Mutex::new(Shared::new()));

        let poller_handles = vec![
            poller::spawn_fixed(
                client.clone(),
                Endpoint::State,
                self.config.state_hz,
                session_start,
                tx.clone(),
                shutdown.clone(),
            ),
            poller::spawn_fixed(
                client.clone(),
                Endpoint::Indicators,
                self.config.indicators_hz,
                session_start,
                tx.clone(),
                shutdown.clone(),
            ),
            poller::spawn_hudmsg(
                client.clone(),
                self.config.hudmsg_hz,
                session_start,
                tx.clone(),
                shutdown.clone(),
            ),
            poller::spawn_fixed(
                client.clone(),
                Endpoint::MapObj,
                self.config.context_hz,
                session_start,
                tx.clone(),
                shutdown.clone(),
            ),
            poller::spawn_fixed(
                client.clone(),
                Endpoint::Mission,
                self.config.context_hz,
                session_start,
                tx.clone(),
                shutdown.clone(),
            ),
            poller::spawn_fixed(
                client,
                Endpoint::MapInfo,
                self.config.context_hz,
                session_start,
                tx.clone(),
                shutdown.clone(),
            ),
        ];
        // Свой клон отправителя роняем — как и в текстовом wt_probe, канал
        // закроется сам, когда завершится последний поток опроса.
        drop(tx);

        let writer_shared = shared.clone();
        let writer_shutdown = shutdown.clone();
        let writer_handle =
            thread::spawn(move || writer::run(rx, outputs, writer_shared, writer_shutdown));

        let targets = BTreeMap::from([
            (Endpoint::State, self.config.state_hz),
            (Endpoint::Indicators, self.config.indicators_hz),
            (Endpoint::HudMsg, self.config.hudmsg_hz),
            (Endpoint::MapObj, self.config.context_hz),
            (Endpoint::Mission, self.config.context_hz),
            (Endpoint::MapInfo, self.config.context_hz),
        ]);

        self.status = "Идёт запись...".to_string();
        self.last_summary = None;
        self.session = Some(RunningSession {
            shutdown,
            shared,
            poller_handles,
            writer_handle,
            dashboard: Dashboard::new(targets, recording),
            dashboard_hz: self.config.dashboard_hz.max(0.1),
            cached_text: String::new(),
            last_render: Instant::now() - Duration::from_secs(3600),
            weapon_states: Vec::new(),
            session_path,
            events_path,
            schema_path,
            weapons_path,
        });
    }

    /// Останавливает текущую сессию (если есть): взводит флаг завершения,
    /// дожидается потоков опроса и записи, пишет отчёты. Безопасно вызывать
    /// и когда сессии нет (например, из on_exit при закрытом окне без
    /// активной записи) — тогда просто ничего не делает.
    fn stop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        session.shutdown.store(true, Ordering::Relaxed);
        for h in session.poller_handles {
            let _ = h.join();
        }
        let result = match session.writer_handle.join() {
            Ok(r) => r,
            Err(_) => {
                self.status =
                    "Поток записи завершился с паникой, отчёты могут быть неполными".to_string();
                WriterResult {
                    schema: schema::SchemaTracker::new(),
                    weapons: weapons::WeaponDetector::new(),
                }
            }
        };

        if let Err(e) = fs::write(&session.schema_path, result.schema.to_markdown()) {
            self.status = format!("Не удалось записать {}: {e}", session.schema_path.display());
        }
        if let Err(e) = fs::write(&session.weapons_path, result.weapons.to_markdown()) {
            self.status = format!(
                "Не удалось записать {}: {e}",
                session.weapons_path.display()
            );
        }

        let (lines_written, bytes_written) = {
            let s = session.shared.lock().unwrap_or_else(|e| e.into_inner());
            (s.lines_written, s.bytes_written)
        };

        let mut summary = String::new();
        summary.push_str("Сессия завершена.\n");
        if let Some(p) = &session.session_path {
            summary.push_str(&format!("  Сырой лог:      {}\n", p.display()));
        }
        if let Some(p) = &session.events_path {
            summary.push_str(&format!("  События hudmsg: {}\n", p.display()));
        }
        summary.push_str(&format!(
            "  Схема:          {}\n",
            session.schema_path.display()
        ));
        summary.push_str(&format!(
            "  Оружие:         {}\n",
            session.weapons_path.display()
        ));
        summary.push_str(&format!(
            "  Строк записано: {lines_written}, байт: {bytes_written}\n"
        ));
        self.last_summary = Some(summary);
        self.status = "Остановлено.".to_string();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("wt_probe — разведка телеметрии War Thunder");
            ui.label(
                "Подключается к HTTP-телеметрии игры (localhost:8111), пишет сырой JSONL и \
                 строит отчёты по схеме полей и по оружию. Не управляет никаким железом.",
            );
            ui.separator();

            ui.add_enabled_ui(self.session.is_none(), |ui| {
                egui::Grid::new("config_grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Хост:");
                        ui.text_edit_singleline(&mut self.config.host);
                        ui.end_row();

                        ui.label("Порт:");
                        ui.add(egui::Slider::new(&mut self.config.port, 1..=65535));
                        ui.end_row();

                        ui.label("Частота /state:");
                        ui.add(
                            egui::Slider::new(&mut self.config.state_hz, 0.5..=60.0).suffix(" Гц"),
                        );
                        ui.end_row();

                        ui.label("Частота /indicators:");
                        ui.add(
                            egui::Slider::new(&mut self.config.indicators_hz, 0.5..=60.0)
                                .suffix(" Гц"),
                        );
                        ui.end_row();

                        ui.label("Частота /hudmsg:");
                        ui.add(
                            egui::Slider::new(&mut self.config.hudmsg_hz, 0.5..=20.0).suffix(" Гц"),
                        );
                        ui.end_row();

                        ui.label("Частота карты/миссии:");
                        ui.add(
                            egui::Slider::new(&mut self.config.context_hz, 0.2..=10.0)
                                .suffix(" Гц"),
                        );
                        ui.end_row();

                        ui.label("Таймаут запроса:");
                        ui.add(
                            egui::Slider::new(&mut self.config.timeout_ms, 50..=1000).suffix(" мс"),
                        );
                        ui.end_row();

                        ui.label("Обновление панели:");
                        ui.add(
                            egui::Slider::new(&mut self.config.dashboard_hz, 1.0..=15.0)
                                .suffix(" Гц"),
                        );
                        ui.end_row();

                        ui.label("Каталог вывода:");
                        ui.text_edit_singleline(&mut self.config.out_dir);
                        ui.end_row();

                        ui.label("Только панель:");
                        ui.checkbox(
                            &mut self.config.dashboard_only,
                            "не писать session_*.jsonl / events_*.jsonl на диск",
                        );
                        ui.end_row();
                    });
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.session.is_none(), egui::Button::new("Старт"))
                    .clicked()
                {
                    self.start();
                }
                if ui
                    .add_enabled(self.session.is_some(), egui::Button::new("Стоп"))
                    .clicked()
                {
                    self.stop();
                }
            });
            ui.label(&self.status);
            ui.separator();

            if let Some(session) = self.session.as_mut() {
                // Читается каждый кадр отдельно от cached_text (тот
                // перестраивается не чаще dashboard_hz) — индикатор стрельбы
                // должен реагировать без задержки панели.
                session.weapon_states = {
                    let s = session.shared.lock().unwrap_or_else(|e| e.into_inner());
                    s.last_flat
                        .get(&Endpoint::Indicators)
                        .map(dashboard::weapon_fields)
                        .unwrap_or_default()
                };
                if !session.weapon_states.is_empty() {
                    ui.horizontal(|ui| {
                        for (key, value) in &session.weapon_states {
                            let firing = *value >= 0.5;
                            let color = if firing {
                                egui::Color32::from_rgb(230, 50, 50)
                            } else {
                                egui::Color32::from_rgb(120, 120, 120)
                            };
                            let badge = if firing {
                                "● ОГОНЬ"
                            } else {
                                "○ тишина"
                            };
                            let label = dashboard::weapon_label(key);
                            ui.label(
                                egui::RichText::new(format!("{label}: {badge}"))
                                    .size(20.0)
                                    .strong()
                                    .color(color),
                            );
                        }
                    });
                    ui.separator();
                }

                if session.last_render.elapsed().as_secs_f64() >= 1.0 / session.dashboard_hz {
                    session.cached_text = session.dashboard.format_text(&session.shared);
                    session.last_render = Instant::now();
                }
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.monospace(&session.cached_text);
                    });
                ctx.request_repaint_after(Duration::from_secs_f64(1.0 / session.dashboard_hz));
            } else if let Some(summary) = &self.last_summary {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.monospace(summary);
                    });
            }
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Закрытие окна не должно бросать запись без отчётов — как и Ctrl+C
        // в текстовом wt_probe, здесь тоже даём потокам корректно
        // остановиться и дописываем schema_*.md/weapons_*.md.
        self.stop();
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([480.0, 360.0])
            .with_resizable(true),
        ..Default::default()
    };
    eframe::run_native(
        "Aurora Vibra — wt_probe (телеметрия War Thunder)",
        native_options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
