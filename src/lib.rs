pub mod aircraft_profiles;
pub mod game_state;
pub mod hid;
pub mod i18n;
pub mod log;
pub mod profiles;
pub mod rumble;
pub mod settings;
pub mod sim;
pub mod timing;
pub mod types;

#[cfg(all(windows, feature = "app"))]
pub mod tray;
#[cfg(all(windows, feature = "app"))]
pub mod ui;
#[cfg(all(windows, feature = "app"))]
pub mod updater;
// Обёртка над системными диалогами «Открыть/Сохранить файл»
// (GetOpenFileNameW/GetSaveFileNameW) для редактора пользовательских
// эффектов — крейта rfd в проекте нет, поэтому тонкий Win32-слой поверх уже
// подключённого windows-crate, как tray/ui/updater выше.
#[cfg(all(windows, feature = "app"))]
pub mod file_dialog;

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

// Продовый конвейер телеметрии+эффектов X-Plane 12 (этап 1, см.
// xp_link/mod.rs). В отличие от wt_link, разведывательного бинарника для
// X-Plane пока нет — весь модуль нужен только основному приложению.
#[cfg(feature = "app")]
pub mod xp_link;

// Данные (не движок и не UI) конструктора пользовательских эффектов
// вибрации: модель одного эффекта + таблица источников телеметрии (см.
// custom_fx/mod.rs). Как и xp_link, нужен только основному приложению —
// recon-бинарникам (wt_probe/wt_probe_gui) это не сдалось.
#[cfg(feature = "app")]
pub mod custom_fx;

// Общий JSONL-рекордер сессий телеметрии — раньше жил только в wt_link
// (умел писать только War Thunder), теперь общий модуль: его вызывают все
// три воркера (wt_link::worker, sim::worker, xp_link::worker), не только
// WT-конвейер. См. recorder.rs — формат/имя WT-файлов не менялись при
// переносе, только добавлен второй метод записи для MSFS/X-Plane.
#[cfg(feature = "app")]
pub mod recorder;

pub use log::LogBuffer;
pub use rumble::RumbleEngine;
pub use types::*; // Делаем структуру доступной для worker.rs через crate::RumbleEngine
