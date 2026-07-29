pub mod aircraft_profiles;
pub mod hid;
pub mod i18n;
pub mod log;
pub mod profiles;
pub mod rumble;
pub mod settings;
pub mod sim;
pub mod types;

#[cfg(all(windows, feature = "app"))]
pub mod tray;
#[cfg(all(windows, feature = "app"))]
pub mod ui;
#[cfg(all(windows, feature = "app"))]
pub mod updater;

// Общий код разведки телеметрии War Thunder — используется двумя бинарниками
// (src/bin/wt_probe/ — текстовый TUI, src/bin/wt_probe_gui/ — окно eframe),
// поэтому живёт в библиотеке, а не внутри одного из них.
#[cfg(feature = "wt-probe")]
pub mod wt_probe;

// Продовый конвейер телеметрии+эффектов War Thunder (этап 1, см.
// wt_link/mod.rs). http.rs общий с recon-инструментом (wt_probe::http его
// реэкспортирует), поэтому доступен под ЛЮБОЙ из двух фич; остальные части
// модуля (vars/rumble/worker) — только под "app", recon-бинарникам они не нужны.
#[cfg(any(feature = "app", feature = "wt-probe"))]
pub mod wt_link;

pub use log::LogBuffer;
pub use rumble::RumbleEngine;
pub use types::*; // Делаем структуру доступной для worker.rs через crate::RumbleEngine
