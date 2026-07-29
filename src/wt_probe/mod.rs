//! Разведывательная библиотека телеметрии War Thunder (localhost:8111) —
//! общий код для двух бинарников: `wt_probe` (текстовый TUI, ANSI) и
//! `wt_probe_gui` (нативное окно, eframe/egui). Сама логика опроса/записи/
//! разбора схемы/детекта оружия не зависит от того, как её потом показывают —
//! см. `dashboard::Dashboard::format_text`, которую вызывают оба бинарника.

pub mod cli;
pub mod dashboard;
pub mod hits;
// http.rs перенесён в crate::wt_link::http — общий и для recon-инструмента
// (эта фича), и для продового wt_link::worker (feature "app"). Реэкспорт
// сохраняет прежний путь `wt_probe::http::...`, которым пользуются
// poller.rs/replay.rs/wt_probe_gui, не дублируя код.
pub use crate::wt_link::http;
pub mod interesting;
pub mod model;
pub mod poller;
pub mod replay;
pub mod schema;
pub mod shared;
pub mod weapons;
pub mod writer;
