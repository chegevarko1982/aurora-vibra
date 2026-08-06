//! Точка входа wt_probe_gui — вся начинка в `gui.rs`, здесь только гейт по
//! платформе. eframe/egui объявлены в `[target.'cfg(windows)'.dependencies]`
//! (см. Cargo.toml), поэтому на Linux модуль `gui` не компилируется вовсе.
//!
//! Без этого гейта бинарник ломал Linux-CI: шаг `cargo clippy --bin wt_probe
//! --features wt-probe --tests` из-за `--tests` собирает тест-таргеты ВСЕХ
//! бинарников, подходящих по required-features, включая этот, и падал на
//! `unresolved import eframe` — при том что сам wt_probe кроссплатформенный.
//!
//! cargo run --bin wt_probe_gui --features wt-probe

#[cfg(windows)]
mod gui;

#[cfg(windows)]
fn main() -> eframe::Result<()> {
    gui::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "wt_probe_gui is Windows-only (eframe/egui are declared under cfg(windows)); \
         use the cross-platform `wt_probe` binary instead"
    );
}
