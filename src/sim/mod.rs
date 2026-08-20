pub mod elem_idx;
pub mod parse;

// worker.rs тянет движок пользовательских эффектов (custom_fx) и рекордер
// сессий (recorder) — оба объявлены в lib.rs под feature "app", поэтому и
// сам воркер должен собираться только вместе с ней (по тому же образцу, что
// уже применён в wt_link/mod.rs и xp_link/mod.rs).
#[cfg(all(windows, feature = "app"))]
mod worker;

#[cfg(all(windows, feature = "app"))]
pub use worker::sim_worker;

#[cfg(all(not(windows), feature = "app"))]
mod stub;

#[cfg(all(not(windows), feature = "app"))]
pub use stub::sim_worker;
