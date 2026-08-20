//! Хранение списка пользовательских эффектов: разделяемое состояние для UI/
//! движка (`CustomFxShared`, тот же приём Mutex+AtomicU64 rev, что и
//! `ConfigShared` в `types.rs`) плюс чтение/запись на диск.
//!
//! Отдельный файл `AuroraVibra.effects.json`, а НЕ поле `SettingsFile`
//! (settings.rs): `RumbleConfig` там копируется в КАЖДЫЙ именной профиль
//! борта (см. `aircraft_profiles::AircraftProfile`), и список кастомных
//! эффектов дублировался бы в каждом профиле — эффекты же задуманы как один
//! общий список, отфильтрованный по борту через `CustomEffect::aircraft_match`,
//! а не как часть профиля.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

use crate::custom_fx::model::{CustomEffect, new_effect};
use crate::custom_fx::sources::SourceId;

const FILE_NAME: &str = "AuroraVibra.effects.json";

/// Разделяемый список эффектов + rev — ровно тот же приём, что `ConfigShared`
/// (types.rs): `with_mut` поднимает rev, только если список реально
/// изменился (сравнение до/после), чтобы движок/Live Monitor не пересобирали
/// состояния на каждый холостой апдейт из UI.
pub struct CustomFxShared {
    inner: Mutex<Vec<CustomEffect>>,
    rev: AtomicU64,
}

impl CustomFxShared {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
            rev: AtomicU64::new(1),
        }
    }

    /// Создаёт с уже готовым списком — например, тем, что вернул `load()` при
    /// старте программы (см. main.rs).
    pub fn new_with(v: Vec<CustomEffect>) -> Self {
        Self {
            inner: Mutex::new(v),
            rev: AtomicU64::new(1),
        }
    }

    pub fn get(&self) -> Vec<CustomEffect> {
        self.inner.lock().clone()
    }

    pub fn set(&self, v: Vec<CustomEffect>) {
        *self.inner.lock() = v;
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    pub fn with_mut<F: FnOnce(&mut Vec<CustomEffect>)>(&self, f: F) {
        let mut g = self.inner.lock();
        let before = g.clone();
        f(&mut g);
        if *g != before {
            self.rev.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn current_rev(&self) -> u64 {
        self.rev.load(Ordering::Relaxed)
    }
}

impl Default for CustomFxShared {
    fn default() -> Self {
        Self::new()
    }
}

/// Пути-кандидаты для чтения, в порядке приоритета — тот же приём и порядок,
/// что `settings::candidate_paths`: (1) рядом с exe (портативная установка),
/// (2) `%LOCALAPPDATA%\AuroraVibra\`.
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

/// Путь, который будет использован при сохранении в первую очередь — рядом с
/// exe, как и `settings::primary_save_path`.
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

/// Загружает список эффектов с диска. `None`, если файл не найден ни в одном
/// известном месте либо повреждён — вызывающий код (main.rs) в этом случае
/// просто стартует с пустым списком, как и `settings::load` для настроек.
pub fn load() -> Option<Vec<CustomEffect>> {
    for path in candidate_paths() {
        if let Ok(data) = std::fs::read_to_string(&path)
            && let Ok(mut effects) = serde_json::from_str::<Vec<CustomEffect>>(&data)
        {
            migrate_lvar_ranges(&mut effects);
            return Some(effects);
        }
    }
    None
}

/// Миграция для файлов, сохранённых ДО того, как у LVAR появился явный
/// диапазон (`LvarSpec::range_lo/range_hi`, см. `custom_fx/model.rs`). Такой
/// файл читается с диапазоном по умолчанию `0.0..1.0` (`LvarSpec::default`),
/// а реальная кривая эффекта при этом вполне может быть построена совсем на
/// других числах — раньше ось X сама растягивалась под точки кривой
/// (`curve_x_bounds` в `ui/effects_editor.rs`, теперь убрана), так что
/// пользователь мог годами накапливать кривую в единицах то одного, то
/// другого источника, которым когда-то пробовал LVAR.
///
/// Расширяет диапазон эффекта до фактического размаха точек его кривой —
/// НЕ трогает диапазон, если он уже покрывает все точки. Так пользователь
/// увидит в полях "от/до" (шаг 1 конструктора) те числа, которые фактически
/// есть в его кривой, и сможет поправить их руками на настоящие единицы
/// переменной — вместо того чтобы молча потерять настроенную кривую при
/// первом же зажиме (`ResponseCurve::clamp_x_into`, шаг 3).
fn migrate_lvar_ranges(effects: &mut [CustomEffect]) {
    for effect in effects.iter_mut() {
        if effect.source != SourceId::Lvar {
            continue;
        }
        let points = &effect.curve.points;
        if points.is_empty() {
            continue;
        }
        let mut lo = points[0].x;
        let mut hi = points[0].x;
        for p in points {
            lo = lo.min(p.x);
            hi = hi.max(p.x);
        }
        if let Some(lvar) = effect.lvar.as_mut() {
            if lo < lvar.range_lo {
                lvar.range_lo = lo;
            }
            if hi > lvar.range_hi {
                lvar.range_hi = hi;
            }
        }
    }
}

/// Сохраняет список эффектов на диск. Порядок фолбэков — тот же, что у
/// `settings::save`: рядом с exe -> `%LOCALAPPDATA%\AuroraVibra` -> temp
/// (последний резерв, чтобы сохранение не падало вовсе, даже если оба
/// обычных места недоступны для записи).
pub fn save(effects: &[CustomEffect]) -> std::io::Result<PathBuf> {
    let json = serde_json::to_string_pretty(effects).map_err(std::io::Error::other)?;

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

    let tmp = std::env::temp_dir().join(FILE_NAME);
    std::fs::write(&tmp, &json)?;
    Ok(tmp)
}

/// Экспорт по произвольному пути, выбранному пользователем в UI (диалог
/// "Сохранить как") — в отличие от `save()`, без фолбэков: если путь
/// недоступен для записи, вызывающий код должен показать ошибку, а не молча
/// уйти в другое место на диске.
pub fn export_to(path: &Path, effects: &[CustomEffect]) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(effects).map_err(std::io::Error::other)?;
    std::fs::write(path, json)
}

/// Импорт списка эффектов из произвольного файла (диалог "Загрузить").
///
/// Перегенерирует `id` у импортируемых эффектов, если такой `id` уже
/// встречается — либо среди уже сохранённого на диске списка (`load()`),
/// либо среди самих импортируемых эффектов (дубликат внутри одного файла).
/// Без этого чужой экспортированный файл с тем же `id`, что и у уже
/// настроенного своего эффекта, затёр бы состояние движка (EMA/гистерезис/
/// фазу Pulse) — движок ключует состояние именно по `id` (см. engine.rs).
///
/// `existing` — ЖИВОЙ список эффектов вызывающего (`CustomFxShared::get()`), а не
/// содержимое файла на диске: пользователь мог создать эффект и ещё не сохранить
/// его, и импорт обязан считать такой id занятым — иначе новорождённый эффект и
/// импортированный поделили бы одну ячейку состояния движка.
pub fn import_from(path: &Path, existing: &[CustomEffect]) -> std::io::Result<Vec<CustomEffect>> {
    let data = std::fs::read_to_string(path)?;
    let mut imported: Vec<CustomEffect> =
        serde_json::from_str(&data).map_err(std::io::Error::other)?;

    let mut seen: std::collections::HashSet<String> =
        existing.iter().map(|e| e.id.clone()).collect();

    for effect in &mut imported {
        if effect.id.trim().is_empty() || seen.contains(&effect.id) {
            effect.id = new_effect(effect.name.clone(), effect.source).id;
        }
        seen.insert(effect.id.clone());
    }

    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_fx::sources::SourceId;

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "aurora_vibra_custom_fx_store_test_{}_{}",
            std::process::id(),
            name
        ));
        p
    }

    #[test]
    fn shared_get_set_roundtrip_and_rev_bumps() {
        let shared = CustomFxShared::new();
        let rev0 = shared.current_rev();
        assert!(shared.get().is_empty());

        let e = new_effect("A".into(), SourceId::FlightAirspeedKn);
        shared.set(vec![e.clone()]);
        assert_eq!(shared.get(), vec![e]);
        assert!(shared.current_rev() > rev0);
    }

    #[test]
    fn with_mut_bumps_rev_only_on_real_change() {
        let shared =
            CustomFxShared::new_with(vec![new_effect("A".into(), SourceId::FlightAirspeedKn)]);
        let rev0 = shared.current_rev();

        // Пустая правка — список после `f` идентичен списку до неё.
        shared.with_mut(|_v| {});
        assert_eq!(
            shared.current_rev(),
            rev0,
            "no-op правка не должна поднимать rev"
        );

        shared.with_mut(|v| v[0].name = "B".into());
        assert!(
            shared.current_rev() > rev0,
            "реальная правка обязана поднять rev"
        );
    }

    #[test]
    fn save_and_load_roundtrip_via_temp_file() {
        // load()/save() ходят по candidate_paths()/primary_save_path(), на
        // которые тест не может повлиять напрямую (нет DI пути) — здесь
        // проверяется именно контракт сериализации/десериализации через
        // export_to/import_from по временному файлу, который тест полностью
        // контролирует.
        let path = tmp_path("roundtrip.json");
        let effects = vec![
            new_effect("A".into(), SourceId::FlightAirspeedKn),
            new_effect("B".into(), SourceId::WtAoaDeg),
        ];

        export_to(&path, &effects).expect("export_to должен успешно писать во временный файл");
        let data = std::fs::read_to_string(&path).unwrap();
        let back: Vec<CustomEffect> = serde_json::from_str(&data).unwrap();
        assert_eq!(back, effects);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn import_from_regenerates_conflicting_ids() {
        let mut a = new_effect("A".into(), SourceId::FlightAirspeedKn);
        a.id = "fx-fixed-1".into();
        let mut b = new_effect("B".into(), SourceId::WtAoaDeg);
        b.id = "fx-fixed-1".into(); // дубликат внутри одного импортируемого файла

        let path = tmp_path("import_conflict.json");
        std::fs::write(&path, serde_json::to_string(&vec![a, b]).unwrap()).unwrap();

        let imported = import_from(&path, &[]).expect("import_from должен разобрать файл");
        assert_eq!(imported.len(), 2);
        assert_ne!(
            imported[0].id, imported[1].id,
            "два одинаковых id внутри одного файла обязаны разъехаться"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn import_from_regenerates_id_colliding_with_live_list() {
        // Эффект пользователя мог быть только что создан в UI и ещё не сохранён
        // на диск — импорт обязан считать его id занятым, иначе оба эффекта
        // поделили бы одну ячейку состояния движка (он ключует по id).
        let mut mine = new_effect("Мой".into(), SourceId::FlightBankDeg);
        mine.id = "fx-collides".into();
        let mut theirs = new_effect("Чужой".into(), SourceId::WtAoaDeg);
        theirs.id = "fx-collides".into();

        let path = tmp_path("import_live_conflict.json");
        std::fs::write(&path, serde_json::to_string(&vec![theirs]).unwrap()).unwrap();

        let imported = import_from(&path, std::slice::from_ref(&mine)).unwrap();
        assert_ne!(
            imported[0].id, mine.id,
            "id, занятый несохранённым эффектом, обязан быть перевыдан"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn import_from_keeps_unique_ids_untouched() {
        let a = new_effect("A".into(), SourceId::FlightAirspeedKn);
        let original_id = a.id.clone();

        let path = tmp_path("import_unique.json");
        std::fs::write(&path, serde_json::to_string(&vec![a]).unwrap()).unwrap();

        let imported = import_from(&path, &[]).unwrap();
        assert_eq!(
            imported[0].id, original_id,
            "уникальный id не должен меняться без причины"
        );

        let _ = std::fs::remove_file(&path);
    }

    // -------------------------------------------------------------
    // Задача C: миграция диапазона LVAR
    // -------------------------------------------------------------

    #[test]
    fn migrate_lvar_ranges_expands_default_range_to_cover_curve_points() {
        use crate::custom_fx::model::{CurvePoint, LvarSpec, ResponseCurve};

        let mut e = new_effect("T".into(), SourceId::Lvar);
        // Диапазон по умолчанию, как у файла, сохранённого ДО появления
        // range_lo/range_hi (LvarSpec::default даёт ровно 0.0..1.0).
        e.lvar = Some(LvarSpec {
            name: "L:A320_Gear_Nose".into(),
            unit: "Number".into(),
            ..LvarSpec::default()
        });
        e.curve = ResponseCurve {
            points: vec![
                CurvePoint { x: -90.0, y: 0.0 },
                CurvePoint { x: 170.6, y: 0.5 },
                CurvePoint { x: 400.0, y: 1.0 },
            ],
        };
        let mut effects = vec![e];

        migrate_lvar_ranges(&mut effects);

        let lvar = effects[0].lvar.as_ref().unwrap();
        assert_eq!(lvar.range_lo, -90.0);
        assert_eq!(lvar.range_hi, 400.0);
    }

    #[test]
    fn migrate_lvar_ranges_does_not_shrink_an_already_wide_range() {
        use crate::custom_fx::model::{LvarSpec, ResponseCurve};

        let mut e = new_effect("T".into(), SourceId::Lvar);
        e.lvar = Some(LvarSpec {
            name: "L:Test".into(),
            unit: "Number".into(),
            range_lo: -1000.0,
            range_hi: 1000.0,
        });
        e.curve = ResponseCurve::linear(0.0, 10.0);
        let mut effects = vec![e];

        migrate_lvar_ranges(&mut effects);

        let lvar = effects[0].lvar.as_ref().unwrap();
        assert_eq!(lvar.range_lo, -1000.0);
        assert_eq!(lvar.range_hi, 1000.0);
    }

    #[test]
    fn migrate_lvar_ranges_ignores_non_lvar_effects() {
        let mut effects = vec![new_effect("T".into(), SourceId::FlightAirspeedKn)];
        // Не должно паниковать и не должно трогать effect.lvar (остаётся None).
        migrate_lvar_ranges(&mut effects);
        assert!(effects[0].lvar.is_none());
    }
}
