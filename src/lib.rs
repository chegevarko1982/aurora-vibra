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

pub use log::LogBuffer;
pub use rumble::RumbleEngine;
pub use types::*; // Делаем структуру доступной для worker.rs через crate::RumbleEngine
