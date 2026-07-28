// GUI-only in both debug and release: logging goes to the log file
// (see LogBuffer::try_init_file_prefer_exe_dir below) instead of a console,
// so no console window pops up alongside the app window during development.
#![windows_subsystem = "windows"]

use aurora_vibra::{
    ConfigShared, EffectsShared, EffectsState, FlightVars, HidCmd, UiCmd,
    aircraft_profiles::AircraftProfiles,
    hid::hid_worker,
    log::LogBuffer,
    profiles::ProfileState,
    sim::sim_worker,
    ui::{Tab, UiState},
};

use anyhow::Result;
use crossbeam_channel::unbounded;
use parking_lot::Mutex;
use std::sync::{Arc, atomic::AtomicBool};
use std::{thread, time::Duration};

/// Returns `true` if another Aurora Vibra instance is already running. In
/// that case the existing window is restored + focused (reusing the same
/// "find window by title" logic the tray icon already relies on) and the
/// caller should exit immediately without touching HID/SimConnect.
fn acquire_single_instance_lock() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::PCWSTR;

    let name: Vec<u16> = "Global\\AuroraVibraSingleInstance\0"
        .encode_utf16()
        .collect();
    // SAFETY: straightforward FFI call with a valid null-terminated wide
    // string. `HANDLE` has no `Drop` impl (it's a plain wrapper, not RAII),
    // so we never close it — the mutex stays held for the whole process
    // lifetime and Windows tears it down automatically on process exit.
    let already_running = unsafe {
        let _handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()));
        GetLastError() == ERROR_ALREADY_EXISTS
    };

    if already_running {
        aurora_vibra::tray::bring_main_to_front();
    }

    already_running
}

fn main() -> Result<()> {
    if aurora_vibra::updater::early_self_update_hook() {
        return Ok(());
    }

    if acquire_single_instance_lock() {
        return Ok(());
    }

    let (tx_hid, rx_hid) = unbounded::<HidCmd>();
    let (tx_ui, rx_ui) = unbounded::<UiCmd>();

    let controller_connected = Arc::new(AtomicBool::new(false));
    let throttle_connected = Arc::new(AtomicBool::new(false));
    let last_vars = Arc::new(Mutex::new(None::<FlightVars>));
    let effects: EffectsShared = Arc::new(EffectsState::default());
    let hold = Arc::new(AtomicBool::new(false));
    let force_quit = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(aurora_vibra::SimStatus::Disconnected));
    let aircraft_title = Arc::new(Mutex::new(String::new()));
    let logs = LogBuffer::default();

    match logs.try_init_file_prefer_exe_dir() {
        Ok(p) => logs.push(format!("File logging enabled → {}", p.display())),
        Err(e) => logs.push(format!("File logging disabled: {}", e)),
    }

    let settings_file = match aurora_vibra::settings::load() {
        Some(sf) => {
            logs.push("Settings loaded from disk".to_string());
            sf
        }
        None => {
            logs.push("No saved settings found, using defaults".to_string());
            aurora_vibra::settings::SettingsFile::default()
        }
    };

    let lang = settings_file.lang;
    aurora_vibra::i18n::set(lang);
    let close_to_tray = settings_file.close_to_tray;
    aurora_vibra::settings::set_close_to_tray(close_to_tray);
    let monitor_collapsed = settings_file.monitor_collapsed;
    aurora_vibra::settings::set_monitor_collapsed(monitor_collapsed);
    aurora_vibra::settings::set_simconnect_dll_path(settings_file.simconnect_dll_path.clone());

    let config = Arc::new(ConfigShared::new_with(settings_file.default.clone()));
    let aircraft_profiles = Arc::new(Mutex::new(AircraftProfiles {
        default: settings_file.default,
        profiles: settings_file.profiles,
        active_match: None,
        loaded_rev: config.current_rev(),
    }));
    let profile_state = Arc::new(Mutex::new(ProfileState::new()));

    {
        let controller_flag = controller_connected.clone();
        let throttle_flag = throttle_connected.clone();
        let rx = rx_hid.clone();
        let logs = logs.clone();
        thread::spawn(move || hid_worker(controller_flag, throttle_flag, rx, logs));
    }

    {
        let last_vars_c = last_vars.clone();
        let tx_hid_c = tx_hid.clone();
        let logs = logs.clone();
        let cfg = config.clone();
        let effects_c = effects.clone();
        let hold_c = hold.clone();
        let status_c = status.clone();
        let ac_title = aircraft_title.clone();
        let aircraft_profiles_c = aircraft_profiles.clone();
        let profile_state_c = profile_state.clone();
        thread::spawn(move || {
            sim_worker(
                last_vars_c,
                tx_hid_c,
                logs,
                cfg,
                effects_c,
                hold_c,
                status_c,
                ac_title,
                aircraft_profiles_c,
                profile_state_c,
            )
        });
    }

    // Без явной иконки eframe подставляет свою иконку-заглушку ("e" на чёрном
    // фоне, см. eframe::native::epi_integration::load_default_egui_icon) поверх
    // иконки, зашитой в ресурсы .exe — поэтому грузим свою явно.
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
        .expect("failed to load embedded assets/icon.png");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // Ширина — чтобы верхняя панель (статусы, Load/Save, Options, ?,
            // EN/RU) помещалась в одну строку без переноса. Высота — чтобы
            // список эффектов был виден целиком до кнопки "Show Telemetry"
            // включительно (сама телеметрия свёрнута по умолчанию, см.
            // RumbleConfig::telemetry_expanded). Точные значения не проверены
            // визуально в этой среде — подгони при первом запуске, если не
            // совпадёт с реальным рендером/экраном.
            .with_inner_size([1000.0, 660.0])
            .with_min_inner_size([700.0, 600.0])
            .with_resizable(true) // Разрешили изменение размера
            .with_maximize_button(true)
            .with_minimize_button(true)
            .with_icon(icon),
        // eframe по умолчанию запоминает размер/позицию окна между запусками
        // (persist_window: true) и подставляет их вместо with_inner_size выше —
        // из-за этого окно каждый раз открывалось в старом (узком) размере.
        // Отключаем, чтобы дефолтный размер всегда применялся при запуске.
        persist_window: false,
        ..Default::default()
    };

    let app = UiState {
        controller_connected,
        throttle_connected,

        status,
        aircraft_title,
        aircraft_profiles,
        profile_state,
        save_as_default_too: false,

        config,
        effects,

        #[cfg(debug_assertions)]
        test_level: 0x80,
        #[cfg(debug_assertions)]
        raw_hex: "02 07 BF 00 00 03 49 00 19 00 00 00 00 00".to_string(),

        tx_hid: tx_hid.clone(),
        logs: logs.clone(),
        last_vars,

        autoscroll: true,
        last_log_count: 0,

        #[cfg(debug_assertions)]
        show_hid_out: true,
        #[cfg(debug_assertions)]
        show_hid_opened: true,

        active_tab: Tab::Main,
        active_section: aurora_vibra::ui::Section::Rumble,
        monitor_show_disabled: false,
        monitor_collapsed,
        hold,
        close_to_tray,
        force_quit: force_quit.clone(),
        lang,

        rx_ui,
        tx_ui: tx_ui.clone(),
    };

    let tx_ui_for_tray = tx_ui.clone();

    let window_title = format!("Aurora Vibra v{}", env!("CARGO_PKG_VERSION"));

    let run = eframe::run_native(
        &window_title,
        native_options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            aurora_vibra::ui::apply_theme(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            aurora_vibra::tray::spawn_tray_with_ctx(
                tx_ui_for_tray.clone(),
                ctx.clone(),
                env!("CARGO_PKG_VERSION"),
                force_quit.clone(),
            );
            Ok(Box::new(app))
        }),
    );

    let _ = tx_hid.send(HidCmd::SendIntensity {
        joystick: 0,
        throttle_left: 0,
        throttle_right: 0,
    });
    thread::sleep(Duration::from_millis(60));

    run.map_err(|e| anyhow::anyhow!("eframe failed: {e}"))
}
