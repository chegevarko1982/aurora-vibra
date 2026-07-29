use crate::{RumbleConfig, aircraft_profiles::AircraftProfile, i18n::Lang, types::GameOverride};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const FILE_NAME: &str = "AuroraVibra.settings.json";

// Мirrors `i18n::{get, set}`'s global-atomic pattern: `close_to_tray` needs to
// be readable from aircraft_profiles::save_active (the Save button's code
// path), which doesn't otherwise have access to UiState's copy of the flag —
// without this, every profile Save would silently reset it back to false.
static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(false);

pub fn close_to_tray() -> bool {
    CLOSE_TO_TRAY.load(Ordering::Relaxed)
}

pub fn set_close_to_tray(v: bool) {
    CLOSE_TO_TRAY.store(v, Ordering::Relaxed);
}

// Same trick as CLOSE_TO_TRAY: aircraft_profiles::save_active builds its own
// SettingsFile without access to UiState's monitor_collapsed field — without
// this, every profile Save would silently reset the Live Monitor column back
// to expanded.
static MONITOR_COLLAPSED: AtomicBool = AtomicBool::new(false);

pub fn monitor_collapsed() -> bool {
    MONITOR_COLLAPSED.load(Ordering::Relaxed)
}

pub fn set_monitor_collapsed(v: bool) {
    MONITOR_COLLAPSED.store(v, Ordering::Relaxed);
}

// Тот же приём, что и у CURRENT_LANG (i18n.rs) — AtomicU8, а не AtomicBool,
// потому что GameOverride трёхвариантный. Читается воркерами (sim/worker.rs,
// wt_link/worker.rs) на каждом тике арбитража — дешёвый enum-матч, не
// требующий доступа к UiState. Заменяет собой старый WT_ENABLED/wt_enabled()
// (ручной чекбокс "War Thunder") — единственный источник истины теперь этот
// 3-вариантный оверрайд, старый bool остался только как поле SettingsFile
// для миграции (см. migrate_wt_enabled ниже).
static GAME_OVERRIDE: AtomicU8 = AtomicU8::new(0); // 0=Auto,1=ForceMsfs,2=ForceWt

pub fn game_override() -> GameOverride {
    match GAME_OVERRIDE.load(Ordering::Relaxed) {
        1 => GameOverride::ForceMsfs,
        2 => GameOverride::ForceWt,
        _ => GameOverride::Auto,
    }
}

pub fn set_game_override(v: GameOverride) {
    let raw = match v {
        GameOverride::Auto => 0,
        GameOverride::ForceMsfs => 1,
        GameOverride::ForceWt => 2,
    };
    GAME_OVERRIDE.store(raw, Ordering::Relaxed);
}

// Тот же приём, что и у CLOSE_TO_TRAY: путь к SimConnect.dll, выбранный
// пользователем вручную, нужен sim::worker::load_simconnect, у которого нет
// доступа ни к SettingsFile, ни к UiState. Путь, а не флаг, поэтому Mutex.
static SIMCONNECT_DLL_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Путь к SimConnect.dll, указанный пользователем вручную (если указывался).
pub fn simconnect_dll_path() -> Option<PathBuf> {
    SIMCONNECT_DLL_PATH.lock().clone()
}

pub fn set_simconnect_dll_path(v: Option<PathBuf>) {
    *SIMCONNECT_DLL_PATH.lock() = v;
}

/// Формат файла на диске: единый default-конфиг (для любого борта без
/// именного профиля) плюс список именных профилей по самолётам (см.
/// src/aircraft_profiles.rs). Старые файлы (плоский RumbleConfig, без
/// профилей) распознаются и мигрируются на лету в load() — см. ниже.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SettingsFile {
    pub default: RumbleConfig,
    pub profiles: Vec<AircraftProfile>,
    // Глобальная настройка (не привязана к самолёту/профилю) — старые файлы
    // без этого поля десериализуются с Lang::default() (En) благодаря
    // #[serde(default)] на всей структуре.
    pub lang: Lang,
    // "Close to tray": при закрытии окна крестиком приложение не завершается,
    // а прячется в трей (см. UiState::close_to_tray / tray.rs). По умолчанию
    // выключено — старые файлы без этого поля десериализуются в false.
    pub close_to_tray: bool,
    // Путь к SimConnect.dll, выбранный пользователем вручную. Проверяется
    // раньше вшитой копии — запасной выход, если та не подошла (устарела либо
    // её забрал в карантин антивирус). None = искать обычным поиском.
    pub simconnect_dll_path: Option<PathBuf>,
    // Свёрнут ли столбец Live Monitor (правая панель). По умолчанию развёрнут —
    // старые файлы без этого поля десериализуются в false.
    pub monitor_collapsed: bool,
    // Legacy-поле дореализации автоопределения игры: раньше единственный
    // источник истины для чекбокса "War Thunder" в Опциях. Теперь ПРОИЗВОДНОЕ
    // от game_override (см. UiState::save_global_settings) — сохраняется как
    // true тогда и только тогда, когда game_override == ForceWt, — чтобы
    // миграция ниже в load() срабатывала ровно один раз, для файлов,
    // сохранённых сборкой ДО этой фичи (где game_override физически
    // отсутствует в JSON).
    pub wt_enabled: bool,
    // Ручной оверрайд автоопределения активной игры (меню Опции): Auto —
    // определять автоматически по живости MSFS/WT, ForceMsfs/ForceWt —
    // прижать конкретную игру независимо от реальной живости другой.
    // Старые файлы без этого поля десериализуются в GameOverride::Auto
    // благодаря #[serde(default)] на всей структуре, миграция ниже поднимает
    // его в ForceWt, если на диске сохранился старый wt_enabled: true.
    pub game_override: GameOverride,
}

/// Возвращает список путей, где может лежать файл настроек, в порядке приоритета:
/// 1) рядом с exe-файлом (удобно для портативной установки)
/// 2) %LOCALAPPDATA%\AuroraVibra\ (на случай, если папка с exe недоступна для записи)
fn candidate_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();

    if let Ok(p) = std::env::current_exe()
        && let Some(dir) = p.parent()
    {
        v.push(dir.join(FILE_NAME));
    }

    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        let mut p = PathBuf::from(base);
        p.push("AuroraVibra");
        p.push(FILE_NAME);
        v.push(p);
    }

    v
}

/// Путь, который будет использован при сохранении (первый доступный для записи вариант).
fn primary_save_path() -> PathBuf {
    if let Ok(p) = std::env::current_exe()
        && let Some(dir) = p.parent()
    {
        return dir.join(FILE_NAME);
    }
    let mut p = std::env::temp_dir();
    p.push(FILE_NAME);
    p
}

fn local_appdata_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let mut p = PathBuf::from(base);
    p.push("AuroraVibra");
    Some(p.join(FILE_NAME))
}

/// Пытается загрузить сохранённый набор профилей с диска.
/// Возвращает None, если файл не найден ни в одном из известных мест,
/// либо если его не удалось разобрать (например, повреждён).
///
/// Формат определяется по наличию top-level ключа "profiles" в JSON: старые
/// файлы (плоский RumbleConfig, без этого ключа) читаются как единый
/// default-конфиг без именных профилей — лишившись автосохранения, они и не
/// могли получить это поле никаким другим путём, так что распознавание
/// однозначно.
pub fn load() -> Option<SettingsFile> {
    for path in candidate_paths() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) else {
                continue;
            };
            if value.get("profiles").is_some() {
                if let Ok(mut file) = serde_json::from_value::<SettingsFile>(value) {
                    migrate_wt_enabled(&mut file);
                    return Some(file);
                }
            } else if let Ok(cfg) = serde_json::from_value::<RumbleConfig>(value) {
                let mut file = SettingsFile {
                    default: cfg,
                    profiles: Vec::new(),
                    lang: Lang::default(),
                    close_to_tray: false,
                    simconnect_dll_path: None,
                    monitor_collapsed: false,
                    wt_enabled: false,
                    game_override: GameOverride::default(),
                };
                migrate_wt_enabled(&mut file);
                return Some(file);
            }
        }
    }
    None
}

/// Поднимает старый флаг `wt_enabled: true` (от сборок до автоопределения
/// игры) в `game_override: ForceWt`. Срабатывает только когда `game_override`
/// ещё Auto — то есть либо ключа не было в JSON вовсе (старый файл), либо
/// пользователь никогда не трогал новый 3-way контрол. `wt_enabled` при
/// сохранении теперь производный от `game_override` (см. комментарий у поля
/// в SettingsFile), поэтому эта миграция реально применяется только один раз:
/// как только файл пересохранится, `wt_enabled` и `game_override` синхронно
/// отражают одно и то же состояние, и повторный запуск этой ветки — no-op.
fn migrate_wt_enabled(file: &mut SettingsFile) {
    if file.game_override == GameOverride::Auto && file.wt_enabled {
        file.game_override = GameOverride::ForceWt;
    }
}

/// Сохраняет набор профилей на диск.
/// Сначала пробует папку рядом с exe, затем %LOCALAPPDATA%\AuroraVibra.
pub fn save(file: &SettingsFile) -> std::io::Result<PathBuf> {
    let json = serde_json::to_string_pretty(file).map_err(std::io::Error::other)?;

    let primary = primary_save_path();
    if std::fs::write(&primary, &json).is_ok() {
        return Ok(primary);
    }

    if let Some(fallback) = local_appdata_path() {
        if let Some(dir) = fallback.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&fallback, &json)?;
        return Ok(fallback);
    }

    // Последний резерв — временная директория.
    let tmp = std::env::temp_dir().join(FILE_NAME);
    std::fs::write(&tmp, &json)?;
    Ok(tmp)
}
