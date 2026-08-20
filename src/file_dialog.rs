//! Тонкая обёртка над классическими Win32-диалогами `GetOpenFileNameW` /
//! `GetSaveFileNameW` (comdlg32.dll) — крейта `rfd` в проекте нет и добавлять
//! его нельзя, а нужный Win32-функционал уже покрывается подключённым
//! `windows = "0.56"` (см. `Win32_UI_Controls_Dialogs` в Cargo.toml).
//!
//! ВАЖНО: обе функции модальные — вызов блокирует поток вызывающего кода
//! (в нашем случае GUI-поток egui/eframe) до тех пор, пока пользователь не
//! закроет диалог кнопкой "Открыть"/"Сохранить" или "Отмена".

use std::ffi::{OsStr, OsString};
use std::iter::once;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
    OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::core::{PCWSTR, PWSTR};

/// MAX_PATH. Диалог всегда возвращает обычный (не \\?\-длинный) путь, так
/// что этого запаса достаточно и под запись, и под чтение.
const PATH_BUF_LEN: usize = 260;

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(once(0)).collect()
}

/// Буфер `lpstrFile` пишется как одна NUL-терминированная строка (мы не
/// ставим `OFN_ALLOWMULTISELECT`), поэтому обрезаем по первому нулю, а не
/// по концу буфера — остаток буфера после имени файла не инициализирован
/// смыслом.
fn utf16_to_path(buf: &[u16]) -> PathBuf {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    PathBuf::from(OsString::from_wide(&buf[..len]))
}

/// Строит `lpstrFilter` для comdlg32: список пар "метка\0маска\0",
/// заканчивающийся ДВОЙНЫМ нулём (пустая строка — маркер конца списка).
/// У нас всегда ровно два варианта — пользовательский тип файла и "Все
/// файлы", так что формат жёстко зашит, а не собирается из произвольного
/// списка пар.
fn build_filter(label: &str, ext: &str) -> Vec<u16> {
    let mut v: Vec<u16> = Vec::new();
    v.extend(OsStr::new(label).encode_wide());
    v.push(0);
    v.extend(OsStr::new(&format!("*.{ext}")).encode_wide());
    v.push(0);
    v.extend(OsStr::new("Все файлы").encode_wide());
    v.push(0);
    v.extend(OsStr::new("*.*").encode_wide());
    v.push(0);
    v.push(0); // терминатор списка: доп. NUL после последней маски
    v
}

/// Системный диалог «Открыть файл». `None`, если пользователь нажал
/// «Отмена» (или закрыл окно) — GetOpenFileNameW в этом случае возвращает
/// FALSE без установки кода ошибки, отличного от "отменено", так что здесь
/// осознанно не различаем отмену и реальную ошибку API.
pub fn open_file(
    title: &str,
    filter_label: &str,
    filter_ext: &str,
    start_dir: Option<&Path>,
) -> Option<PathBuf> {
    // ВРЕМЯ ЖИЗНИ: OPENFILENAMEW хранит только указатели (PCWSTR/PWSTR), а не
    // владеет данными. Все буферы ниже — именованные переменные, живущие до
    // конца unsafe-вызова; если бы они были временными выражениями прямо в
    // инициализаторе структуры, компилятор мог бы уничтожить их до того, как
    // GetOpenFileNameW успеет их разыменовать (висячий указатель).
    let filter = build_filter(filter_label, filter_ext);
    let title_w = wide(title);
    let start_dir_w = start_dir.map(|p| wide(&p.to_string_lossy()));
    let mut file_buf = vec![0u16; PATH_BUF_LEN];

    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: HWND::default(), // без владельца: у диалога нет привязки к конкретному eframe-HWND
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        lpstrTitle: PCWSTR(title_w.as_ptr()),
        lpstrInitialDir: start_dir_w
            .as_ref()
            .map_or(PCWSTR::null(), |w| PCWSTR(w.as_ptr())),
        // FILEMUSTEXIST/PATHMUSTEXIST — по требованию задачи; EXPLORER — новый
        // (не Win3.1) стиль диалога; NOCHANGEDIR — без него GetOpenFileNameW
        // молча меняет текущую директорию процесса на выбранную пользователем,
        // что ломает все последующие относительные пути в приложении.
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_EXPLORER | OFN_NOCHANGEDIR,
        ..Default::default()
    };

    // SAFETY: все указатели в ofn ссылаются на буферы выше, которые живы до
    // конца этого выражения; lStructSize соответствует фактическому размеру
    // структуры на текущей архитектуре. Вызов модальный и блокирует поток.
    let ok = unsafe { GetOpenFileNameW(&mut ofn as *mut OPENFILENAMEW) }.as_bool();
    if !ok {
        return None;
    }
    Some(utf16_to_path(&file_buf))
}

/// Системный диалог «Сохранить как». Если пользователь ввёл имя без
/// расширения, comdlg32 сам допишет `filter_ext` (через `lpstrDefExt`) —
/// это единственный документированный способ сделать так без ручного
/// постпарсинга результата диалога.
pub fn save_file(
    title: &str,
    filter_label: &str,
    filter_ext: &str,
    default_name: &str,
    start_dir: Option<&Path>,
) -> Option<PathBuf> {
    // См. open_file — те же требования по времени жизни буферов.
    let filter = build_filter(filter_label, filter_ext);
    let title_w = wide(title);
    let start_dir_w = start_dir.map(|p| wide(&p.to_string_lossy()));
    let def_ext_w = wide(filter_ext);

    // lpstrFile должен уже содержать имя по умолчанию: comdlg32 его не
    // подставляет сам, только редактирует то, что лежит в буфере.
    let mut file_buf = vec![0u16; PATH_BUF_LEN];
    let name_w = wide(default_name);
    file_buf[..name_w.len()].copy_from_slice(&name_w);

    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: HWND::default(),
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        lpstrTitle: PCWSTR(title_w.as_ptr()),
        lpstrInitialDir: start_dir_w
            .as_ref()
            .map_or(PCWSTR::null(), |w| PCWSTR(w.as_ptr())),
        lpstrDefExt: PCWSTR(def_ext_w.as_ptr()),
        // OVERWRITEPROMPT — по требованию задачи; EXPLORER/NOCHANGEDIR — см.
        // комментарий в open_file, тот же довод применим и к сохранению.
        Flags: OFN_OVERWRITEPROMPT | OFN_EXPLORER | OFN_NOCHANGEDIR,
        ..Default::default()
    };

    // SAFETY: см. open_file.
    let ok = unsafe { GetSaveFileNameW(&mut ofn as *mut OPENFILENAMEW) }.as_bool();
    if !ok {
        return None;
    }
    Some(utf16_to_path(&file_buf))
}

#[cfg(test)]
mod tests {
    use super::build_filter;

    #[test]
    fn filter_is_two_null_separated_pairs_ending_in_double_null() {
        let f = build_filter("Профили эффектов", "vibx");

        // Должно оканчиваться двойным нулём — терминатор списка для comdlg32.
        assert_eq!(&f[f.len() - 2..], &[0, 0]);

        // Разбиваем по одиночным NUL и проверяем, что метка и маска — это
        // раздельные записи ("метка", "*.ext", "Все файлы", "*.*"), а не одна
        // слипшаяся строка.
        let parts: Vec<String> = f
            .split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf16(s).unwrap())
            .collect();
        assert_eq!(
            parts,
            vec!["Профили эффектов", "*.vibx", "Все файлы", "*.*"]
        );
    }
}
