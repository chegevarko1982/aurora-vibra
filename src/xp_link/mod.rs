//! Продовый конвейер телеметрии+эффектов X-Plane 12 (финальный этап):
//! каталог datarefs + UDP-транспорт RREF + конверсии в `FlightVars` +
//! фоновый воркер, шлющий вибрацию через тот же `hid_worker`, что и
//! MSFS/War Thunder.
//!
//! X-Plane 12 не имеет аналога SimConnect/HTTP API War Thunder — вместо
//! этого он с рождения поддерживает штатный бинарный UDP-протокол RREF
//! ("Request DataRef") на порту 49000: клиент подписывается на нужные
//! datarefs с желаемой частотой, сим сам присылает пакеты с их значениями.
//! `rref` реализует этот транспорт, `datarefs` — упорядоченный каталог того,
//! на что подписываемся (см. doc-комментарий `datarefs.rs`).
//!
//! `vars` собирает из значений datarefs готовый `crate::FlightVars` и отдаёт
//! его в уже существующий движок эффектов `crate::rumble::RumbleEngine` (тот
//! же, что использует MSFS-конвейер). `worker`/`stub` — фоновый поток по
//! аналогии с `wt_link` (windows+app — реальный опрос RREF, остальные
//! платформы — заглушка с той же сигнатурой). Владение HID-каналом в
//! моменте, как и у остальных конвейеров, арбитрируется через
//! `crate::game_state::GameSlot`.

pub mod datarefs;
pub mod rref;
pub mod vars;

#[cfg(all(windows, feature = "app"))]
mod worker;
#[cfg(all(windows, feature = "app"))]
pub use worker::xp_worker;

#[cfg(all(not(windows), feature = "app"))]
mod stub;
#[cfg(all(not(windows), feature = "app"))]
pub use stub::xp_worker;
