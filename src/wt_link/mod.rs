//! Продовый конвейер телеметрии+эффектов War Thunder (этап 1). Не путать с
//! `wt_probe` — тот разведывательный инструмент (два отдельных бинарника),
//! этот модуль — часть основного приложения, работает при включённом
//! `settings::wt_enabled()` и шлёт вибрацию через тот же `hid_worker`, что и
//! MSFS (см. `crate::sim::sim_worker`).
//!
//! `http` — общий с `wt_probe` (см. реэкспорт в `wt_probe::mod`), доступен
//! под любой из двух фич. Остальное — только под "app": recon-бинарникам
//! (`wt_probe`/`wt_probe_gui`, feature "wt-probe") эффекты/воркер не нужны.

pub mod http;

#[cfg(feature = "app")]
pub mod aero_profiles;
#[cfg(feature = "app")]
pub mod ammo;
#[cfg(feature = "app")]
pub mod rumble;
#[cfg(feature = "app")]
pub mod vars;

#[cfg(all(windows, feature = "app"))]
mod worker;
#[cfg(all(windows, feature = "app"))]
pub use worker::wt_worker;

#[cfg(all(not(windows), feature = "app"))]
mod stub;
#[cfg(all(not(windows), feature = "app"))]
pub use stub::wt_worker;
