// Локализация UI (egui-окно, трей-меню, диалоги автообновления). Активный
// язык хранится и в UiState (для egui-потока), и в этом глобальном атомике
// (для tray.rs/updater.rs — они выполняются на отдельных OS-потоках и не
// имеют прямого доступа к UiState). set() обновляет оба места разом — см.
// вызов в ui.rs при клике по переключателю EN/RU.
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

impl Lang {
    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::En => &EN,
            Lang::Ru => &RU,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "EN",
            Lang::Ru => "RU",
        }
    }
}

static CURRENT_LANG: AtomicU8 = AtomicU8::new(0); // 0 = En, 1 = Ru

pub fn get() -> Lang {
    match CURRENT_LANG.load(Ordering::Relaxed) {
        1 => Lang::Ru,
        _ => Lang::En,
    }
}

pub fn set(lang: Lang) {
    let v = match lang {
        Lang::En => 0,
        Lang::Ru => 1,
    };
    CURRENT_LANG.store(v, Ordering::Relaxed);
}

pub struct Strings {
    // --- Верхняя панель ---
    pub connected: &'static str,
    pub disconnected: &'static str,
    pub in_flight: &'static str,
    pub simconnect_missing: &'static str,
    pub hover_simconnect_missing: &'static str,
    pub sidestick: &'static str,
    pub throttle: &'static str,
    pub unknown_aircraft: &'static str,
    pub btn_load: &'static str,
    pub btn_save: &'static str,
    pub hover_also_default: &'static str,
    pub tab_main: &'static str,
    pub tab_debug: &'static str,
    pub btn_stop: &'static str,
    pub btn_resume: &'static str,
    pub btn_options: &'static str,
    pub hover_help: &'static str,
    pub help_text: &'static str,
    pub hover_help_us: &'static str,
    pub help_us_text: &'static str,
    pub chk_close_to_tray: &'static str,
    pub hover_close_to_tray: &'static str,
    pub chk_record_wt_session: &'static str,
    pub hover_record_wt_session: &'static str,
    pub lbl_game_override: &'static str,
    pub opt_game_auto: &'static str,
    pub opt_game_force_msfs: &'static str,
    pub opt_game_force_wt: &'static str,
    pub opt_game_force_xp: &'static str,
    pub hover_game_msfs: &'static str,
    pub hover_game_wt: &'static str,
    pub hover_game_xp: &'static str,
    pub heading_game_not_detected: &'static str,
    pub msg_game_not_detected: &'static str,

    // --- Aircraft Profiles ---
    pub heading_aircraft_profiles: &'static str,
    pub empty_profiles_hint: &'static str,
    pub hover_delete_profile: &'static str,
    pub hover_active_profile: &'static str,
    pub hover_apply_profile: &'static str,

    // --- Rumble Effects ---
    pub heading_rumble_effects: &'static str,

    pub overspeed_effect_name: &'static str,
    pub chk_override: &'static str,
    pub hover_override: &'static str,
    pub lbl_limit: &'static str,
    pub limit_na: &'static str,
    pub val_na: &'static str,
    pub status_active: &'static str,
    pub status_idle: &'static str,
    pub status_off: &'static str,

    // Легенда над списком эффектов. Раньше смысл иконок устройства и
    // кружка-статуса жил только во всплывающей подсказке — тестировщик
    // сообщил, что «непонятно, что означают отдельные квадратики», т.е.
    // подсказку надо ещё догадаться вызвать. Легенда показывает то же самое
    // постоянно и видимым текстом.
    pub legend_devices: &'static str,
    pub legend_stick: &'static str,
    pub legend_throttle: &'static str,
    pub legend_hint: &'static str,
    pub legend_states: &'static str,
    pub legend_state_off: &'static str,
    pub legend_state_idle: &'static str,
    pub legend_state_active: &'static str,

    pub device_label: &'static str,
    pub hover_joystick_hw: &'static str,
    pub hover_throttle_hw: &'static str,

    pub lbl_intensity: &'static str,
    pub hover_overspeed_intensity: &'static str,
    pub hover_overspeed_limit_barberpole: &'static str,

    pub name_ground_roll: &'static str,
    pub hover_ground_roll: &'static str,

    pub heading_taxi_thump: &'static str,
    pub name_taxi_start: &'static str,
    pub hover_taxi_start: &'static str,
    pub name_taxi_end: &'static str,
    pub hover_taxi_end: &'static str,
    pub lbl_period_curve: &'static str,
    pub hover_period_curve_full: &'static str,
    pub hover_period_curve_short: &'static str,

    pub name_flaps: &'static str,
    pub hover_flaps: &'static str,

    pub name_stall: &'static str,
    pub hover_stall: &'static str,

    pub name_spoilers: &'static str,
    pub hover_spoilers: &'static str,
    pub lbl_threshold_pct: &'static str,
    pub hover_spoilers_threshold: &'static str,

    pub hover_engine_start: &'static str,
    pub lbl_n2_idle: &'static str,
    pub hover_n2_idle: &'static str,
    pub name_engine_start: &'static str,
    pub chk_four_eng_mode: &'static str,
    pub hover_four_eng_mode: &'static str,
    pub chk_swap_hands: &'static str,
    pub hover_swap_hands: &'static str,

    pub heading_gear_comp: &'static str,
    pub chk_enabled: &'static str,
    pub hover_headroom: &'static str,
    pub name_nose_peak: &'static str,
    pub name_left_peak: &'static str,
    pub name_right_peak: &'static str,

    pub lbl_gear_transit: &'static str,
    // Скрытый встроенный эффект "касание шасси" (RumbleConfig::gear_enabled,
    // временно отключён и скрыт из UI — см. типы.rs) — единственное имя,
    // под которым он вообще где-либо показывается: шаг 1 редактора
    // пользовательских эффектов, когда источник FlightGearHandle вытесняет
    // именно его (см. custom_fx::overrides::primary_builtin_for).
    pub name_gear_bump: &'static str,

    pub lbl_bank_turb: &'static str,
    pub hover_bank_intensity: &'static str,
    pub lbl_threshold_deg: &'static str,
    pub hover_bank_threshold: &'static str,

    pub btn_reset_defaults: &'static str,
    pub btn_hide_telemetry: &'static str,
    pub btn_show_telemetry: &'static str,

    // --- Live Aircraft Data ---
    pub heading_live_aircraft_data: &'static str,
    pub lbl_airspeed: &'static str,
    pub lbl_barber_pole: &'static str,
    pub lbl_overspeed_warning: &'static str,
    pub lbl_lear_horn: &'static str,
    pub lbl_gs: &'static str,
    pub lbl_on_ground: &'static str,
    pub lbl_bank_deg: &'static str,
    pub lbl_flaps_pct: &'static str,
    pub lbl_slats_pct: &'static str,
    pub lbl_gear: &'static str,
    pub val_down: &'static str,
    pub val_up: &'static str,
    pub lbl_spoilers_pct: &'static str,
    pub lbl_spoiler_l: &'static str,
    pub lbl_spoiler_r: &'static str,
    pub lbl_nose_gear: &'static str,
    pub lbl_left_main: &'static str,
    pub lbl_right_main: &'static str,
    pub lbl_stall: &'static str,
    pub lbl_paused: &'static str,
    pub lbl_no_data: &'static str,

    // --- Engine Telemetry ---
    pub heading_engine_telemetry: &'static str,
    pub eng1_header: &'static str,
    pub eng2_header: &'static str,
    pub eng3_header: &'static str,
    pub eng4_header: &'static str,
    pub lbl_n2: &'static str,
    pub lbl_combustion: &'static str,
    pub lbl_starter_active: &'static str,
    pub lbl_pmdg_starter_lvar: &'static str,
    pub lbl_pct_max_rpm: &'static str,
    pub lbl_engine_rpm: &'static str,
    pub lbl_prop_rpm: &'static str,
    pub val_on: &'static str,
    pub val_off: &'static str,

    // --- Debug tab ---
    pub heading_logs: &'static str,
    pub chk_autoscroll: &'static str,

    // --- Tray ---
    pub tray_stop: &'static str,
    pub tray_resume: &'static str,
    pub tray_check_updates: &'static str,
    pub tray_quit: &'static str,
    pub tray_status_active: &'static str,
    pub tray_status_stopped: &'static str,

    // --- Updater dialogs ---
    pub upd_title_up_to_date: &'static str,
    pub upd_title_update_available: &'static str,
    pub upd_title_updating: &'static str,
    pub upd_body_updating: &'static str,
    pub upd_title_update_failed: &'static str,
    pub upd_title_update_check_failed: &'static str,
    pub upd_title_launch_failed: &'static str,
    pub upd_title_admin_required: &'static str,
    pub upd_body_admin_required: &'static str,

    // --- Navigation / Layout (v4 redesign) ---
    pub nav_rumble: &'static str,
    pub nav_taxi: &'static str,
    pub nav_engines: &'static str,
    pub nav_gear: &'static str,
    pub nav_telemetry: &'static str,
    pub heading_live_monitor: &'static str,
    pub lbl_no_active_effects: &'static str,
    pub hover_monitor_collapse: &'static str,
    pub hover_monitor_expand: &'static str,
    pub btn_set_n2_idle: &'static str,
    pub hover_set_n2_idle: &'static str,
    pub log_n2_idle_set: &'static str,

    // --- War Thunder (этап 1) ---
    pub nav_wt: &'static str,
    pub wt_status_disconnected: &'static str,
    pub wt_status_menu: &'static str,
    pub wt_status_in_battle: &'static str,
    pub name_wt_weapon1: &'static str,
    pub name_wt_weapon2: &'static str,
    pub hover_wt_weapon1: &'static str,
    pub hover_wt_weapon2: &'static str,
    pub hover_wt_flaps: &'static str,
    pub hover_wt_gear_transit: &'static str,
    pub name_wt_stall: &'static str,
    pub hover_wt_stall: &'static str,
    pub name_wt_engine_start: &'static str,
    pub hover_wt_engine_start: &'static str,
    pub name_wt_overspeed: &'static str,
    pub hover_wt_overspeed: &'static str,
    pub name_wt_gear_overspeed: &'static str,
    pub hover_wt_gear_overspeed: &'static str,
    pub heading_wt_telemetry: &'static str,
    pub lbl_wt_flaps_pct: &'static str,
    pub lbl_wt_gear_pct: &'static str,
    pub lbl_wt_aoa_deg: &'static str,
    pub lbl_wt_wx_deg_s: &'static str,
    pub lbl_wt_rpm1: &'static str,
    pub lbl_wt_weapon1_ammo: &'static str,
    pub lbl_wt_weapon2_ammo: &'static str,
    pub lbl_wt_ammo_unknown: &'static str,
    pub lbl_wt_vehicle_type: &'static str,
    pub lbl_wt_vehicle_type_unknown: &'static str,
    pub lbl_wt_speed_kt: &'static str,
    pub lbl_wt_altitude_ft: &'static str,

    // --- Конструктор эффектов («Мои эффекты») ---
    pub nav_effects: &'static str,
    // Оба движка (встроенный набор и пользовательские эффекты) считаются
    // ОДНОВРЕМЕННО, глобального переключателя режима больше нет (см.
    // custom_fx::overrides) — короткое пояснение новой логики в шапке
    // раздела «Мои эффекты» вместо удалённых radio-кнопок режима.
    pub msg_fx_effects_coexist: &'static str,
    // Шаг 1 редактора («Источник»): поясняет, какой встроенный эффект
    // вытесняет ВЫБРАННЫЙ источник (см. `custom_fx::overrides::
    // primary_builtin_for`), и что происходит, если он не вытесняет ничего.
    pub lbl_fx_overrides_builtin: &'static str,
    pub lbl_fx_overrides_builtin_none: &'static str,
    // Пометка на карточке ВСТРОЕННОГО эффекта (разделы Аэродинамика/Стыки
    // ВПП/Двигатели/Стойки шасси/War Thunder в ui.rs), когда его вытеснил
    // пользовательский эффект на том же источнике телеметрии — карточка при
    // этом приглушается визуально (см. UiState::effect_card).
    pub lbl_builtin_overridden_by: &'static str,
    // Короткая метка в Live Monitor для встроенного эффекта, вытесненного
    // пользовательским — вместо обычной пустой "—"/выключенного вида.
    pub lbl_replaced_by_custom: &'static str,
    pub btn_fx_new: &'static str,
    // Задача 1: заготовки нового эффекта — кнопка "+ Новый эффект" открывает
    // меню из 4 пунктов (см. `EffectPreset` в effects_editor.rs), каждый —
    // название заготовки + короткая подсказка, что это за эффект и для чего.
    pub preset_impact: &'static str,
    pub hover_preset_impact: &'static str,
    pub preset_hum: &'static str,
    pub hover_preset_hum: &'static str,
    pub preset_pulsation: &'static str,
    pub hover_preset_pulsation: &'static str,
    pub preset_growing: &'static str,
    pub hover_preset_growing: &'static str,
    pub btn_fx_duplicate: &'static str,
    pub btn_fx_delete: &'static str,
    pub btn_fx_import: &'static str,
    pub btn_fx_export: &'static str,
    pub empty_fx_hint: &'static str,
    pub lbl_fx_name: &'static str,
    pub lbl_fx_games: &'static str,
    pub lbl_fx_aircraft: &'static str,
    pub hover_fx_aircraft: &'static str,
    // Задача 3 (custom LVAR source) — дополнительные поля шага 1,
    // показываются только когда effect.source == SourceId::Lvar (см.
    // ui/effects_editor.rs::show_lvar_source_fields).
    pub lbl_fx_lvar_name: &'static str,
    pub lbl_fx_lvar_hint: &'static str,
    pub lbl_fx_lvar_prefix_hint: &'static str,
    pub lbl_fx_lvar_unit: &'static str,
    pub opt_fx_lvar_unit_custom: &'static str,
    pub lbl_fx_lvar_unit_custom: &'static str,
    pub msg_fx_lvar_msfs_only: &'static str,
    // Заголовки групп в выпадающем списке "1. Источник" (fx_source_select) —
    // разделяют источники MSFS/X-Plane, War Thunder и MSFS-only LVAR, чтобы
    // было видно, к какой игре относится строка. См. effects_editor.rs.
    pub hdr_fx_source_group_flight: &'static str,
    pub hdr_fx_source_group_wt: &'static str,
    pub hdr_fx_source_group_lvar: &'static str,
    pub step_fx_source: &'static str,
    pub step_fx_when: &'static str,
    pub step_fx_curve: &'static str,
    pub step_fx_shape: &'static str,
    pub lbl_fx_live_value: &'static str,
    pub lbl_fx_no_signal: &'static str,
    pub trigger_always: &'static str,
    pub trigger_above: &'static str,
    pub trigger_below: &'static str,
    pub trigger_between: &'static str,
    pub trigger_is_true: &'static str,
    pub trigger_changed: &'static str,
    pub hover_trigger_changed: &'static str,
    pub lbl_fx_threshold: &'static str,
    pub lbl_fx_hysteresis: &'static str,
    pub hover_fx_hysteresis: &'static str,
    pub lbl_fx_range_lo: &'static str,
    pub lbl_fx_range_hi: &'static str,
    pub lbl_fx_eps: &'static str,
    pub lbl_fx_hold: &'static str,
    pub lbl_fx_curve_hint: &'static str,
    pub shape_constant: &'static str,
    pub shape_pulse: &'static str,
    pub shape_oneshot: &'static str,
    pub shape_sine: &'static str,
    pub shape_sawtooth: &'static str,
    pub lbl_fx_freq: &'static str,
    pub hover_fx_freq: &'static str,
    pub lbl_fx_duty: &'static str,
    pub lbl_fx_jitter: &'static str,
    pub lbl_fx_floor: &'static str,
    pub lbl_fx_attack: &'static str,
    pub lbl_fx_decay: &'static str,
    pub lbl_fx_depth: &'static str,
    pub lbl_fx_strength: &'static str,
    pub lbl_fx_smoothing: &'static str,
    pub lbl_fx_mix: &'static str,
    pub mix_max: &'static str,
    pub mix_add: &'static str,
    pub lbl_fx_output: &'static str,
    pub lbl_fx_out_joystick: &'static str,
    pub lbl_fx_out_throttle_left: &'static str,
    pub lbl_fx_out_throttle_right: &'static str,
    pub heading_fx_preview: &'static str,
    pub btn_fx_play: &'static str,
    pub btn_fx_stop: &'static str,
    /// Подпись рядом с кнопкой/индикаторами предпросмотра — показывается
    /// ТОЛЬКО для событийных эффектов (`Shape::OneShot`/`Trigger::Changed`,
    /// см. `ui/effects_editor.rs`): в реальной игре такой эффект срабатывает
    /// один раз по событию, а предпросмотр зацикливает его, чтобы удар можно
    /// было настроить на ощупь, не долбя Стоп/Играть каждые полсекунды.
    pub lbl_fx_preview_loops_events: &'static str,
    pub lbl_fx_test_value: &'static str,
    pub btn_fx_open_session: &'static str,
    pub btn_fx_play_session: &'static str,
    pub lbl_fx_session_none: &'static str,
    /// Задача 1: строка на месте `lbl_fx_session_none` после успешной
    /// загрузки записи — имя файла (без пути, он длинный и ломает вёрстку;
    /// путь целиком уходит в `.on_hover_text`), длительность и число кадров.
    /// Плейсхолдеры `{name}`/`{frames}`/`{dur:.1}` подставляются через
    /// `.replace(...)`, тот же приём, что `log_n2_idle_set` в ui.rs.
    pub lbl_fx_session_loaded: &'static str,
    /// Задача 2: запись загружена, но источник эффекта в ней ни разу не дал
    /// сигнала — потому что запись сделана в другой игре (не пустая и не
    /// битая запись, `sources::read` штатно возвращает `None` для чужой
    /// игры). Показывается вместо `lbl_fx_no_signal` на графике прогона.
    /// Плейсхолдеры `{rec}` (игра записи) и `{src}` (игра источника).
    pub lbl_fx_session_game_mismatch: &'static str,
    pub warn_fx_no_output: &'static str,
    pub warn_fx_always_on: &'static str,
    pub warn_fx_wrong_game: &'static str,
    // Заголовки/фильтры системных диалогов «Открыть/Сохранить файл» (см.
    // file_dialog::open_file/save_file в ui/effects_editor.rs) — отдельные
    // строки, а не переиспользование не по смыслу подходящих существующих
    // (nav_telemetry и т.п.).
    pub dlg_fx_export_title: &'static str,
    pub dlg_fx_import_title: &'static str,
    pub dlg_fx_session_title: &'static str,
    pub filter_fx_file: &'static str,
    pub filter_session_file: &'static str,
}

pub const EN: Strings = Strings {
    connected: "Connected",
    disconnected: "Disconnected",
    in_flight: "In Flight",
    simconnect_missing: "SimConnect.dll not found",
    hover_simconnect_missing: "The SimConnect client library could not be loaded, so no telemetry can be read. This is not a simulator problem — restarting MSFS will not help. The app carries its own copy and extracts it to the temp folder, so this usually means antivirus quarantined it or the temp folder is not writable. Workaround: place a SimConnect.dll next to AuroraVibra.exe and restart. See AuroraVibra.log for the paths that were tried.",
    sidestick: "Sidestick",
    throttle: "Throttle",
    unknown_aircraft: "Unknown Aircraft",
    btn_load: "⬆ Load",
    btn_save: "⬇ Save",
    hover_also_default: "The current config is also written as default — applies to any aircraft without its own named profile",
    tab_main: "Main",
    tab_debug: "Debug",
    btn_stop: "⛔ Stop",
    btn_resume: "▶ Resume",
    btn_options: "Options",
    hover_help: "Help",
    help_text: "Aurora Vibra drives tactile feedback (rumble) on the joystick and throttle (WinWing and compatible): runway-joint thumps on taxi, gear touchdown kick, flap/spoiler vibration, stall and overspeed effects, engine start spool-up. Everything is computed from the aircraft's real telemetry via SimConnect, not from the cockpit switch positions.

## 1. Launch & connection
The top bar shows the SimConnect link status (Connected/Disconnected/In Flight) and two dots — Sidestick and Throttle — that light up once each physical device is detected. If it says \"SimConnect.dll not found\" next to the status, there's no telemetry to read from; that's not an MSFS problem, restarting the sim won't help (hover it for where to look in AuroraVibra.log).

The current aircraft name next to the dots changes color:
- white — no saved profile for this aircraft yet, the default set is in use;
- green — this aircraft has its own saved profile, and it's loaded;
- blue — the aircraft has built-in custom telemetry handling (PMDG, Fenix A320, MADDOG, Learjet) — effects are tuned to its non-standard SimConnect variables regardless of whether a profile is saved.

## 2. Aircraft profiles
The app detects the aircraft on every flight load and applies either its named profile (if one was saved before) or the shared default. The \"Aircraft Profiles\" list on the left shows the saved aircraft — click a row to load it, the delete icon removes it.

Workflow: adjust the effects, then click Save to write them as the current aircraft's named profile (if it doesn't have one yet, it's created right there — no separate button needed). The 📌 toggle next to Save also writes the same values as the default, applied to any aircraft without its own profile. Load re-reads the settings file from disk, useful to discard unsaved edits. While there are unsaved changes, the Save button is highlighted in orange.

## 3. Tuning effects
The middle of the window has 5 sections (buttons on the left): Aerodynamics, Taxi Thump, Engines Start/Stop, Gears Touchdown, Telemetry. Every effect is a card with the same layout:
- the checkbox on the left enables/disables the effect;
- the card's border lights up while the effect is actually triggering right now — handy for dialing in the strength in flight without looking away;
- the intensity slider sets the vibration strength as a percentage;
- the two icons on the right (joystick/throttle) route the effect to a device — one, both, or neither;
- some effects have extra fields, e.g. a speed/angle/deployment threshold above which the effect switches on.

Section summary:
- Aerodynamics — overspeed (threshold read from the SimConnect barber pole, or set manually via Override), background airflow rumble, flap/slat motor buzz while moving, minimum stall shake, spoiler airflow vibration, bank/turbulence shake.
- Taxi Thump — runway-joint rhythm on taxi: start speed, speed at full frequency, the curve shape of the ramp-up.
- Engines Start/Stop — starter spool-up plus a fixed ignition kick; N2 idle threshold; 4-engine grouping mode and a hand-swap option for mirrored cockpit layouts.
- Gears Touchdown — separate impact strength for the nose and each main gear, and how much landing hardness stretches the impact's duration.
- Telemetry — live aircraft data (airspeed, bank, flap/gear/spoiler position) and engine data (N2, RPM, starter status) for monitoring and profile tuning.

The Reset button at the bottom restores the effects to factory defaults — but only live: nothing is written to disk until Save is pressed, and Load would still bring back the previously saved profile.

## 4. Pause & background
Stop/Resume in the left column mutes all effects temporarily without touching the settings — handy to kill the rumble for a minute without losing the configuration. The \"Close to tray\" option (\"...\" menu → Options) makes closing the window with the X button hide it to the system tray instead of quitting — the app keeps running in the background. The tray icon offers the same Stop/Resume plus a full Exit.

## 5. Updates
\"Check for updates…\" in the \"...\" menu → Options runs a one-off version check; if a newer release exists, it offers to install it and restart the app automatically.

## 6. Building your own effect
The Effect Editor (\"My effects\" section) builds a custom effect in 4 steps: 1) pick a telemetry source; 2) pick when it fires; 3) draw the response curve; 4) pick a shape and where it plays. \"+ New effect\" opens a menu of 4 ready-made starting points (Impact, Hum, Pulsation, Growing) instead of a blank effect. Important: built-in and custom effects are mutually exclusive — only the engine selected at the top of that screen (Built-in effects / My effects) actually drives the motors.",
    hover_help_us: "Donate",
    help_us_text: "Your support helps keep Aurora Vibra up to date — adding new hardware devices, simulators, and custom aircraft support.

USDT (TRC-20):
TSP24RnqTRzA215LNDzWrNQBawWpR9z5YD

BTC:
bc1p5txluxsen8uqhy0k3j0v9s6afemt5zkyftzjv4asc5uh3lw44u7snkplr5

YooMoney:
https://yoomoney.ru/to/410011348629282",
    chk_close_to_tray: "Close to tray",
    hover_close_to_tray: "When enabled, closing the window with the X button hides it to the system tray instead of quitting — the app keeps running in the background. Right-click the tray icon to Stop or Exit.",
    chk_record_wt_session: "Record session (debug)",
    hover_record_wt_session: "Writes raw telemetry from whichever game is currently active (War Thunder /state + /indicators, or MSFS/X-Plane flight vars) to wt_probe_sessions/session_<timestamp>.jsonl next to the app — for diagnosing missing/incorrect effects on specific aircraft. Off by default, not saved between restarts.",
    lbl_game_override: "Active game:",
    opt_game_auto: "Auto-detect",
    opt_game_force_msfs: "Force MSFS",
    opt_game_force_wt: "Force War Thunder",
    opt_game_force_xp: "Force X-Plane",
    hover_game_msfs: "Microsoft Flight Simulator",
    hover_game_wt: "War Thunder",
    hover_game_xp: "X-Plane",
    heading_game_not_detected: "Game not detected",
    msg_game_not_detected: "Launch Microsoft Flight Simulator or War Thunder to start receiving tactile feedback. War Thunder needs \"Local host telemetry\" turned on in its game settings (localhost:8111).",

    heading_aircraft_profiles: "Aircraft Profiles",
    empty_profiles_hint: "No named profiles yet — use the button next to the aircraft name above to create the first one.",
    hover_delete_profile: "Delete profile",
    hover_active_profile: "Currently loaded profile",
    hover_apply_profile: "Copy this profile's settings into the current config (does not switch which aircraft it's saved under — press Save afterwards to store them for the current aircraft)",

    heading_rumble_effects: "Rumble Effects",

    overspeed_effect_name: "Overspeed Effect",
    chk_override: "Override",
    hover_override: "Set the Overspeed threshold manually instead of AIRSPEED BARBER POLE from SimConnect — useful if the addon doesn't sync that variable to the real cockpit gauge",
    lbl_limit: "Limit:",
    limit_na: "Limit: N/A",
    val_na: "N/A",
    status_active: "ACTIVE",
    status_idle: "Idle",
    status_off: "Off",

    legend_devices: "Sends vibration to:",
    legend_stick: "Stick",
    legend_throttle: "Throttle",
    legend_hint: "(click these icons inside a card to change where that effect goes)",
    legend_states: "Effect state:",
    legend_state_off: "— switched off",
    legend_state_idle: "— on, not firing",
    legend_state_active: "— vibrating right now",

    device_label: "Device:",
    hover_joystick_hw: "Combat Joystick R",
    hover_throttle_hw: "WINCTRL URSA MINOR Throttle",

    lbl_intensity: "Intensity:",
    hover_overspeed_intensity: "Vibration strength once past the red line (Vmo/Mmo). 10% minimum at 1 knot over, growing with the overspeed amount — 100% at +120 knots",
    hover_overspeed_limit_barberpole: "Current red line (AIRSPEED BARBER POLE / Vmo·Mmo) for this aircraft at this altitude — the sim moves it on its own",

    name_ground_roll: "Ground Roll",
    hover_ground_roll: "Soft background effect (below the gear touchdown compression impact)",

    heading_taxi_thump: "Taxi Thump Settings",
    name_taxi_start: "Start (kt)",
    hover_taxi_start: "Speed at which the runway-joint thump effect (Ground Roll thump) starts working",
    name_taxi_end: "End (kt)",
    hover_taxi_end: "Speed at which the effect reaches full strength: thumps get faster from Start to End, then frequency stops increasing",
    lbl_period_curve: "Period curve:",
    hover_period_curve_full: "How fast the gap between thumps shrinks as speed increases.\n1.0 = pure physics (t = slab length / speed).\nAbove 1.0 = smoother at the start of the taxi roll.\nBelow 1.0 = sharper than pure physics.",
    hover_period_curve_short: "1.0 = physics. Higher = the thump rhythm accelerates more slowly at the start.",

    name_flaps: "Flaps (bump)",
    hover_flaps: "Flaps/slats motor vibration while they're moving (tracks flaps/slats position, not the handle)",

    name_stall: "Stall ceiling",
    hover_stall: "Minimum vibration strength during a stall — the output never drops below this value (but can go higher if other effects stack on top)",

    name_spoilers: "Spoilers Airflow",
    hover_spoilers: "Vibration strength with spoilers/spoilerons symmetrically deployed — grows with deployment depth and indicated airspeed",
    lbl_threshold_pct: "Threshold (%):",
    hover_spoilers_threshold: "Minimum symmetric spoiler deployment (%) above which the effect turns on",

    hover_engine_start: "Starter spool-up + fixed-strength ignition kick (1s) on engine start. The slider sets the curve's ceiling AFTER ignition — from 1% (all the way left) to 80% of max strength (all the way right); it doesn't affect the ignition kick",
    lbl_n2_idle: "N2 Idle%:",
    hover_n2_idle: "N2 RPM (%) at which the engine is considered at idle — shapes the spool-up curve (not the strength). Some aircraft override this automatically via their profile",
    name_engine_start: "Engine Start / Ignition",
    chk_four_eng_mode: "4-Eng Mode (1&2->Left, 3&4->Right)",
    hover_four_eng_mode: "4-engine aircraft: Eng1/Eng2 (left wing) are grouped on the Throttle (left hand), Eng3/Eng4 (right wing) on the Joystick (right hand). The max N2 in the pair is used, and the ignition kick triggers from either engine in its group.",
    chk_swap_hands: "Swap hands (Joystick=Left, Throttle=Right)",
    hover_swap_hands: "By default, side-bound effects (pre-ignition engine start, split touchdown) assume the Throttle is under the left hand and the Joystick under the right. If your Joystick physically sits on the left and the Throttle on the right, enable this to mirror the side. It doesn't affect which motor (left/right) on the Throttle drives Eng1/Eng2 — that's fixed by the quadrant hardware.",

    heading_gear_comp: "Gear Strut Compression (Touchdown)",
    chk_enabled: "Enabled",
    hover_headroom: "Sets how much landing hardness affects the impact's duration, not its strength: every touchdown always hits at full 255. 0% keeps the duration fixed and minimal (~230ms) no matter how hard you land. 100% lets a hard landing stretch the hit up to ~550ms, while a soft one still stays around 230ms",
    name_nose_peak: "Nose Peak",
    name_left_peak: "Left Peak",
    name_right_peak: "Right Peak",

    lbl_gear_transit: "Gear Transit & Doors",
    name_gear_bump: "Landing Gear (bump)",

    lbl_bank_turb: "Bank / Turb",
    hover_bank_intensity: "Vibration strength once past the bank threshold (below). Pulses get faster the more the angle exceeds the threshold",
    lbl_threshold_deg: "Threshold (°):",
    hover_bank_threshold: "Bank angle (°) above which the Bank/Turb effect turns on",

    btn_reset_defaults: "Reset",
    btn_hide_telemetry: "Hide Telemetry",
    btn_show_telemetry: "Show Telemetry",

    heading_live_aircraft_data: "Live Aircraft Data",
    lbl_airspeed: "Airspeed (kt):",
    lbl_barber_pole: "Barber Pole (kt):",
    lbl_overspeed_warning: "Overspeed Warning:",
    lbl_lear_horn: "Lear Horn (XMLSND75):",
    lbl_gs: "GS (kt):",
    lbl_on_ground: "On Ground:",
    lbl_bank_deg: "Bank (°):",
    lbl_flaps_pct: "Flaps (%):",
    lbl_slats_pct: "Slats (%):",
    lbl_gear: "Gear:",
    val_down: "Down",
    val_up: "Up",
    lbl_spoilers_pct: "Spoilers (%):",
    lbl_spoiler_l: "Spoiler L (%):",
    lbl_spoiler_r: "Spoiler R (%):",
    lbl_nose_gear: "Nose Gear (%):",
    lbl_left_main: "Left Main (%):",
    lbl_right_main: "Right Main (%):",
    lbl_stall: "Stall:",
    lbl_paused: "Paused:",
    lbl_no_data: "No data",

    heading_engine_telemetry: "Engine Telemetry",
    eng1_header: "Engine 1 (Left / Throttle)",
    eng2_header: "Engine 2 (Right / Joystick)",
    eng3_header: "Engine 3 (4-Eng: contributes to Left)",
    eng4_header: "Engine 4 (4-Eng: contributes to Right)",
    lbl_n2: "N2:",
    lbl_combustion: "Combustion:",
    lbl_starter_active: "Starter Active:",
    lbl_pmdg_starter_lvar: "PMDG Starter L-Var:",
    lbl_pct_max_rpm: "% Max RPM:",
    lbl_engine_rpm: "Engine RPM:",
    lbl_prop_rpm: "Prop RPM:",
    val_on: "ON",
    val_off: "OFF",

    heading_logs: "Logs",
    chk_autoscroll: "Auto-scroll",

    tray_stop: "Stop",
    tray_resume: "Resume",
    tray_check_updates: "Check for updates…",
    tray_quit: "Exit",
    tray_status_active: "● Active",
    tray_status_stopped: "○ Stopped",

    upd_title_up_to_date: "Up to date",
    upd_title_update_available: "Update available",
    upd_title_updating: "Updating",
    upd_body_updating: "The application will now close to apply the update. It will relaunch automatically.",
    upd_title_update_failed: "Update failed",
    upd_title_update_check_failed: "Update check failed",
    upd_title_launch_failed: "Launch failed",
    upd_title_admin_required: "Administrator permission required",
    upd_body_admin_required: "The app is installed in a protected folder (e.g., Program Files).\nTo update, click Yes on the elevation prompt, or move the app to a writable folder (e.g., Documents) and try again.",

    nav_rumble: "Aerodynamics",
    nav_taxi: "Taxi Thump",
    nav_engines: "Engines Start/Stop",
    nav_gear: "Gears Touchdown",
    nav_telemetry: "Telemetry",
    heading_live_monitor: "Live Monitor",
    lbl_no_active_effects: "No effects active",
    hover_monitor_collapse: "Hide Live Monitor",
    hover_monitor_expand: "Show Live Monitor",
    btn_set_n2_idle: "(SET)",
    hover_set_n2_idle: "Set this value minus 1.5% as Engine Idle N2",
    log_n2_idle_set: "N2 Idle set to {val:.1}% (from Eng1 telemetry - 1.5%)",

    nav_wt: "War Thunder",
    wt_status_disconnected: "Disconnected",
    wt_status_menu: "Hangar / Menu",
    wt_status_in_battle: "In Battle",
    name_wt_weapon1: "Weapon 1",
    name_wt_weapon2: "Weapon 2",
    hover_wt_weapon1: "First weapon group firing (machine guns) — routed to the joystick only, tuned as a fast-firing texture.",
    hover_wt_weapon2: "Second weapon group firing (cannons) — routed to both throttle motors only, tuned as a slower, heavier texture. Routing is fixed for both groups (not user-configurable) so they stay distinguishable in different hands.",
    hover_wt_flaps: "Flaps motor vibration while flaps position is changing.",
    hover_wt_gear_transit: "Gear motor hum while retracting/extending, plus a lock bump when gear reaches fully up or fully down. War Thunder has no separate landing-gear-door telemetry, so this covers transit + lock only.",
    name_wt_stall: "Stall buffet",
    hover_wt_stall: "Airflow-separation buffet as angle of attack approaches the critical stall angle. v1 uses a single hardcoded profile (Bf 109 F-4 only) — on any other aircraft this effect stays silent, since War Thunder's game data has no critical-AoA figure to derive it from automatically. Thresholds are a starting hypothesis pending live-test calibration on the real aircraft.",
    name_wt_engine_start: "Engine start/stop",
    hover_wt_engine_start: "Engine start and shutdown, driven by RPM 1 (mono — no per-engine/per-side split, multi-engine aircraft start together). Cranking and spool-up as pulses that speed up and get sharper, a single hard hit on ignition catch, silence on a steady running engine, then a coast-down as the prop windmills to a stop — pulses slow down AND fade out linearly to silence, ending in two distinct jerks when it fully stops. Self-calibrating — no per-aircraft idle-RPM table needed. Thresholds are a starting hypothesis pending live-test calibration.",
    name_wt_overspeed: "Overspeed (Vne)",
    hover_wt_overspeed: "Vibration on both throttle motors as indicated airspeed approaches the aircraft's never-exceed speed — builds linearly over the last 10 km/h below the threshold, then holds at full power once reached or exceeded. If flaps are extended, the threshold tightens to their own break speed (where that data exists for the aircraft); otherwise the general Vne applies. Gear has its own separate effect below. Data comes from ~1300-aircraft tables sourced from the War Thunder wiki; aircraft missing from those tables stay silent rather than guessing.",
    name_wt_gear_overspeed: "Gear Overspeed",
    hover_wt_gear_overspeed: "Vibration on both throttle motors when landing gear is extended and indicated airspeed approaches the gear's own break speed — builds linearly over the last 20 km/h below that threshold, then holds at full power once reached or exceeded. Silent whenever the gear is retracted, or on aircraft with no gear break-speed data on the War Thunder wiki (e.g. fixed gear).",
    heading_wt_telemetry: "War Thunder Telemetry",
    lbl_wt_flaps_pct: "Flaps",
    lbl_wt_gear_pct: "Gear",
    lbl_wt_aoa_deg: "AoA (deg)",
    lbl_wt_wx_deg_s: "Roll rate Wx (deg/s)",
    lbl_wt_rpm1: "RPM",
    lbl_wt_weapon1_ammo: "Weapon1 ammo",
    lbl_wt_weapon2_ammo: "Weapon2 ammo",
    lbl_wt_ammo_unknown: "— (no counter on this aircraft)",
    lbl_wt_vehicle_type: "Aircraft",
    lbl_wt_vehicle_type_unknown: "— (unknown)",
    lbl_wt_speed_kt: "Speed (kt)",
    lbl_wt_altitude_ft: "Altitude (ft)",

    nav_effects: "Effect Editor",
    msg_fx_effects_coexist: "Built-in and custom effects run at the same time — your effect only replaces the built-in one on the same source.",
    lbl_fx_overrides_builtin: "Overrides the built-in effect:",
    lbl_fx_overrides_builtin_none: "This source doesn't override any built-in effect — both will run together.",
    lbl_builtin_overridden_by: "Overridden by your effect:",
    lbl_replaced_by_custom: "replaced",
    btn_fx_new: "+ New effect",
    preset_impact: "Impact",
    hover_preset_impact: "A short kick on an event — touchdown, gear extended. Fires once and fades.",
    preset_hum: "Hum",
    hover_preset_hum: "A textured buzz while a value stays above a threshold — overspeed, stall. Same texture as the built-in gunfire effect.",
    preset_pulsation: "Pulsation",
    hover_preset_pulsation: "A soft, smooth pulse while a value stays above a threshold — a gentle background warning.",
    preset_growing: "Growing",
    hover_preset_growing: "Strength follows the source value directly, no threshold — the plain default behavior.",
    btn_fx_duplicate: "Duplicate",
    btn_fx_delete: "Delete",
    btn_fx_import: "Import…",
    btn_fx_export: "Export…",
    empty_fx_hint: "No custom effects yet — press \"+ New effect\" to build one.",
    lbl_fx_name: "Name",
    lbl_fx_games: "Games",
    lbl_fx_aircraft: "Only for aircraft",
    hover_fx_aircraft: "Substring of the aircraft name; empty = any aircraft.",
    lbl_fx_lvar_name: "Variable name",
    lbl_fx_lvar_hint: "Find the exact name in the sim itself: Developer Mode > Behaviors (or the local variables list).",
    lbl_fx_lvar_prefix_hint: "Local variables usually start with \"L:\".",
    lbl_fx_lvar_unit: "Unit",
    opt_fx_lvar_unit_custom: "Custom…",
    lbl_fx_lvar_unit_custom: "Custom unit name",
    msg_fx_lvar_msfs_only: "Custom variables only exist in MSFS — games are locked to MSFS.",
    hdr_fx_source_group_flight: "MSFS / X-Plane — flight parameters",
    hdr_fx_source_group_wt: "War Thunder",
    hdr_fx_source_group_lvar: "Microsoft Flight Simulator — custom variable",
    step_fx_source: "1. Source",
    step_fx_when: "2. When it fires",
    step_fx_curve: "3. Response curve",
    step_fx_shape: "4. Shape and output",
    lbl_fx_live_value: "Live value",
    lbl_fx_no_signal: "no signal",
    trigger_always: "Always",
    trigger_above: "Above threshold",
    trigger_below: "Below threshold",
    trigger_between: "In range",
    trigger_is_true: "When on",
    trigger_changed: "While changing",
    hover_trigger_changed: "Fires while the value keeps moving — for flaps, gear and other travelling surfaces.",
    lbl_fx_threshold: "Threshold",
    lbl_fx_hysteresis: "Hysteresis",
    hover_fx_hysteresis: "Dead band that keeps the effect from chattering right at the threshold.",
    lbl_fx_range_lo: "From",
    lbl_fx_range_hi: "To",
    lbl_fx_eps: "Minimum step",
    lbl_fx_hold: "Hold",
    lbl_fx_curve_hint: "Drag a point to move it, click the line to add one, right-click a point to remove it.",
    shape_constant: "Steady",
    shape_pulse: "Pulsing",
    shape_oneshot: "Single hit",
    shape_sine: "Wave",
    shape_sawtooth: "Sawtooth",
    lbl_fx_freq: "Rate",
    hover_fx_freq: "Capped at 6.5 Hz: the device is fed 20 times a second, faster pulses are lost between frames.",
    lbl_fx_duty: "Pulse width",
    lbl_fx_jitter: "Roughness",
    lbl_fx_floor: "Between pulses",
    lbl_fx_attack: "Attack",
    lbl_fx_decay: "Decay",
    lbl_fx_depth: "Depth",
    lbl_fx_strength: "Strength",
    lbl_fx_smoothing: "Smoothing",
    lbl_fx_mix: "If effects overlap",
    mix_max: "Take the strongest",
    mix_add: "Add up",
    lbl_fx_output: "Send to",
    lbl_fx_out_joystick: "Joystick",
    lbl_fx_out_throttle_left: "Throttle, left motor",
    lbl_fx_out_throttle_right: "Throttle, right motor",
    heading_fx_preview: "Preview",
    btn_fx_play: "Play on device",
    btn_fx_stop: "Stop",
    lbl_fx_preview_loops_events: "In-game this effect fires once per event; the preview loops it so you can dial it in by feel.",
    lbl_fx_test_value: "Test value",
    btn_fx_open_session: "Open recording…",
    btn_fx_play_session: "Play recording",
    lbl_fx_session_none: "No recording loaded",
    lbl_fx_session_loaded: "Loaded: {name} — {frames} frame(s), {dur:.1}s",
    lbl_fx_session_game_mismatch: "This recording is from {rec} — the effect's source belongs to {src} and isn't recorded here. Pick a source for {rec}, or open a recording from {src}.",
    warn_fx_no_output: "This effect is not routed to any motor.",
    warn_fx_always_on: "This effect will vibrate non-stop: it has no threshold and a steady shape.",
    warn_fx_wrong_game: "This source is not available in the active game.",
    dlg_fx_export_title: "Export effects",
    dlg_fx_import_title: "Import effects",
    dlg_fx_session_title: "Open recorded session",
    filter_fx_file: "Aurora Vibra effects",
    filter_session_file: "Recorded session",
};

pub const RU: Strings = Strings {
    connected: "Подключено",
    disconnected: "Не подключено",
    in_flight: "В полёте",
    simconnect_missing: "SimConnect.dll не найдена",
    hover_simconnect_missing: "Не удалось загрузить клиентскую библиотеку SimConnect — телеметрию читать нечем. Это не проблема симулятора: перезапуск MSFS не поможет. Приложение несёт свою копию и распаковывает её во временную папку, так что обычно причина — карантин антивируса либо временная папка недоступна для записи. Обходной путь: положить SimConnect.dll рядом с AuroraVibra.exe и перезапустить. Перебранные пути — в AuroraVibra.log.",
    sidestick: "Сайдстик",
    throttle: "РУД",
    unknown_aircraft: "Неизвестный самолёт",
    btn_load: "⬆ Загрузить",
    btn_save: "⬇ Сохранить",
    hover_also_default: "Текущий конфиг дополнительно запишется как default — применится ко всем самолётам без именного профиля",
    tab_main: "Основное",
    tab_debug: "Отладка",
    btn_stop: "⛔ Стоп",
    btn_resume: "▶ Продолжить",
    btn_options: "Опции",
    hover_help: "Справка",
    help_text: "Aurora Vibra отдаёт на джойстик и РУД (WinWing и совместимые) тактильную отдачу: удары стоек ВПП на рулении, отдачу шасси при касании, вибрацию закрылков/спойлеров, эффекты сваливания и превышения скорости, раскрутку двигателя при запуске. Всё считается из реальной телеметрии борта через SimConnect, а не из положения переключателей в кабине.

## 1. Запуск и подключение
Верхняя панель показывает статус связи с симулятором (Connected/Disconnected/In Flight) и два индикатора — Sidestick и Throttle: горят, когда соответствующее физическое устройство опознано. Если рядом со статусом связи написано «SimConnect.dll не найдена» — телеметрию читать нечем; это не проблема MSFS, перезапуск симулятора не поможет (наведите — там же путь, куда смотреть в AuroraVibra.log).

Название текущего борта рядом с индикаторами меняет цвет:
- белый — для этого борта ещё нет сохранённого профиля, используется набор по умолчанию;
- зелёный — для этого борта уже есть свой сохранённый профиль, он и загружен;
- синий — у борта есть встроенная логика чтения телеметрии (PMDG, Fenix A320, MADDOG, Learjet) — эффекты подстроены под его нестандартные SimConnect-переменные независимо от того, сохранён профиль или нет.

## 2. Профили самолётов
Приложение само определяет борт при каждой загрузке полёта и подставляет либо его именной профиль (если сохранён раньше), либо общий default. Список «Профили самолётов» слева — это уже сохранённые борта; клик по строке загружает профиль, значок удаления рядом — убирает его.

Рабочий цикл: настроили эффекты → кнопка «Сохранить» записывает их как именной профиль текущего борта (если для него ещё не было профиля — он создаётся тут же, отдельной кнопки для этого не нужно). Флажок 📌 рядом с «Сохранить» дополнительно фиксирует те же значения как default — то, что подставится любому борту без своего профиля. «Загрузить» перечитывает файл настроек с диска, если нужно откатить несохранённые правки. Пока есть несохранённые изменения, кнопка «Сохранить» подсвечивается оранжевым.

## 3. Настройка эффектов
Средняя часть окна разбита на 5 разделов (кнопки слева): Аэродинамика, Стыки ВПП, Двигатели: Запуск/Останов, Стойки шасси: касание, Телеметрия. Каждый эффект — карточка с одинаковым устройством:
- чекбокс слева включает/выключает эффект;
- рамка карточки подсвечивается, когда эффект срабатывает прямо сейчас — удобно подбирать силу вживую, не отрываясь от полёта;
- слайдер интенсивности задаёт силу вибрации в процентах;
- две иконки справа (джойстик/РУД) переключают, на какое устройство эффект отправляется — на одно, на оба или ни на одно;
- у части эффектов есть дополнительные поля — например, порог по скорости/углу/проценту выпуска, после которого эффект включается.

Коротко по разделам:
- Аэродинамика — превышение скорости (порог берётся из барбер-пола SimConnect либо задаётся вручную флажком Override), фоновая вибрация от воздушного потока, реакция закрылков/предкрылков на движение, минимальный уровень тряски при сваливании, вибрация от выпущенных спойлеров, тряска от крена/турбулентности.
- Стыки ВПП — ритм ударов от стыков плит на рулении: скорость начала эффекта, скорость выхода на полную частоту, кривизна нарастания ритма.
- Двигатели: Запуск/Останов — раскрутка стартером плюс фиксированный удар воспламенения; порог холостых оборотов N2; режим для 4-моторных бортов и смена сторон джойстик/РУД под свою посадку в кабине.
- Стойки шасси: касание — отдельная сила удара для носовой и обеих основных стоек, и насколько жёсткость посадки растягивает удар во времени.
- Телеметрия — живые параметры борта (скорость, крен, положение закрылков/шасси/спойлеров) и параметры двигателей (N2, обороты, статус стартера) — для контроля и отладки профиля.

Кнопка «Сброс» внизу возвращает эффекты к заводским значениям — но только на лету: пока не нажата «Сохранить», на диск это не пишется, и «Загрузить» вернёт прежний сохранённый профиль.

## 4. Пауза и фон
«Стоп/Продолжить» в левой колонке временно глушит все эффекты, не трогая настройки — удобно, если нужно на минуту отвлечься от вибрации, не теряя конфигурацию. Опция «Сворачивать в трей» (меню «...» → Опции) делает так, что закрытие окна крестиком прячет его в системный трей вместо выхода — программа продолжает работать в фоне. Через иконку в трее доступны те же Стоп/Продолжить и полный выход.

## 5. Обновления
Пункт «Проверить обновления…» в меню «...» → Опции запускает разовую проверку версии; при наличии новой — предложит установить и перезапустить приложение автоматически.

## 6. Как собрать свой эффект
Раздел «Редактор эффектов» («Мои эффекты») собирает пользовательский эффект в 4 шага: 1) выбери источник телеметрии; 2) выбери, когда он срабатывает; 3) нарисуй кривую отклика; 4) выбери форму сигнала и куда её отправить. Кнопка «+ Новый эффект» открывает меню из 4 готовых заготовок (Удар, Гул, Пульсация, Нарастание) вместо пустого эффекта. Важно: встроенные и пользовательские эффекты взаимоисключающие — моторы реально ведёт только тот движок, что выбран вверху этого экрана (Встроенные эффекты / Мои эффекты).",
    hover_help_us: "Поддержать",
    help_us_text: "Ваша поддержка помогает актуализировать Aurora Vibra — добавлять поддержку новых устройств, симуляторов и кастомных самолётов.

USDT (TRC-20):
TSP24RnqTRzA215LNDzWrNQBawWpR9z5YD

BTC:
bc1p5txluxsen8uqhy0k3j0v9s6afemt5zkyftzjv4asc5uh3lw44u7snkplr5

YooMoney:
https://yoomoney.ru/to/410011348629282",
    chk_close_to_tray: "Сворачивать в трей",
    hover_close_to_tray: "Если включено, закрытие окна крестиком прячет его в системный трей вместо выхода — приложение продолжает работать в фоне. Правый клик по иконке в трее — Stop или Exit.",
    chk_record_wt_session: "Записывать сессию (отладка)",
    hover_record_wt_session: "Пишет сырую телеметрию активной в данный момент игры (War Thunder /state + /indicators, либо параметры полёта MSFS/X-Plane) в wt_probe_sessions/session_<timestamp>.jsonl рядом с программой — для диагностики отсутствующих/неверных эффектов на конкретных самолётах. По умолчанию выключено, между перезапусками не сохраняется.",
    lbl_game_override: "Активная игра:",
    opt_game_auto: "Автоопределение",
    opt_game_force_msfs: "Только MSFS",
    opt_game_force_wt: "Только War Thunder",
    opt_game_force_xp: "Только X-Plane",
    hover_game_msfs: "Microsoft Flight Simulator",
    hover_game_wt: "War Thunder",
    hover_game_xp: "X-Plane",
    heading_game_not_detected: "Игра не обнаружена",
    msg_game_not_detected: "Запустите Microsoft Flight Simulator или War Thunder, чтобы получать тактильную отдачу. Для War Thunder нужно включить «Локальная телеметрия» в настройках игры (localhost:8111).",

    heading_aircraft_profiles: "Профили самолётов",
    empty_profiles_hint: "Именных профилей ещё нет — используйте кнопку рядом с названием самолёта наверху, чтобы создать первый.",
    hover_delete_profile: "Удалить профиль",
    hover_active_profile: "Загруженный сейчас профиль",
    hover_apply_profile: "Скопировать настройки этого профиля в текущий конфиг (борт, за которым профиль сохранён, не меняется — после этого нажмите Сохранить, чтобы записать их под текущим самолётом)",

    heading_rumble_effects: "Эффекты вибрации",

    overspeed_effect_name: "Превышение скорости",
    chk_override: "Override",
    hover_override: "Задать порог превышения скорости вручную вместо AIRSPEED BARBER POLE из SimConnect — полезно, если аддон не синхронизирует эту переменную с реальным прибором на панели",
    lbl_limit: "Порог:",
    limit_na: "Порог: Н/Д",
    val_na: "Н/Д",
    status_active: "АКТИВНО",
    status_idle: "Ждёт",
    status_off: "Выкл",

    legend_devices: "Отдаёт вибрацию на:",
    legend_stick: "Ручку",
    legend_throttle: "РУД",
    legend_hint: "(эти иконки в карточке кликабельны — задают, куда пойдёт эффект)",
    legend_states: "Состояние эффекта:",
    legend_state_off: "— выключен",
    legend_state_idle: "— включён, не срабатывает",
    legend_state_active: "— вибрирует сейчас",

    device_label: "Устройство:",
    hover_joystick_hw: "Combat Joystick R",
    hover_throttle_hw: "WINCTRL URSA MINOR Throttle (РУД)",

    lbl_intensity: "Интенсивность:",
    hover_overspeed_intensity: "Сила вибрации при превышении красной черты (Vmo/Mmo). Минимум 10% сразу на 1 узле превышения, дальше растёт вместе с превышением — 100% при +120 узлов",
    hover_overspeed_limit_barberpole: "Текущая красная черта (AIRSPEED BARBER POLE / Vmo·Mmo) для этого борта на этой высоте — сим двигает её сам",

    name_ground_roll: "Руление по поверхности",
    hover_ground_roll: "Мягкий фоновый эффект (ниже удара сжатия стоек при касании)",

    heading_taxi_thump: "Настройки стыков ВПП",
    name_taxi_start: "Начало (уз)",
    hover_taxi_start: "Скорость, с которой начинает работать эффект стыков плит ВПП (Ground Roll thump)",
    name_taxi_end: "Конец (уз)",
    hover_taxi_end: "Скорость полного эффекта: удары учащаются от Начала до Конца, дальше частота не растёт",
    lbl_period_curve: "Кривизна периода:",
    hover_period_curve_full: "Как быстро сокращается пауза между ударами с ростом скорости.\n1.0 = чистая физика (t = длина плиты / скорость).\nБольше 1.0 = плавнее на старте рулёжки.\nМеньше 1.0 = резче, чем чистая физика.",
    hover_period_curve_short: "1.0 = физика. Больше — медленнее ускоряется ритм ударов на старте.",

    name_flaps: "Закрылки",
    hover_flaps: "Вибрация моторчика закрылков/предкрылков во время их движения (следит за положением flaps/slats, не за ручкой)",

    name_stall: "Порог сваливания",
    hover_stall: "Минимальная сила вибрации во время сваливания — итог не опускается ниже этого значения (но может быть выше, если накладываются другие эффекты)",

    name_spoilers: "Спойлеры",
    hover_spoilers: "Сила вибрации при симметрично выпущенных спойлерах/интерцепторах — растёт с глубиной выпуска и приборной скоростью",
    lbl_threshold_pct: "Порог (%):",
    hover_spoilers_threshold: "Минимальный симметричный выпуск спойлеров (%), после которого включается эффект",

    hover_engine_start: "Раскрутка стартером + удар воспламенения (фиксированная сила, 1с) при запуске двигателя. Слайдер задаёт потолок кривой ПОСЛЕ воспламенения — от 1% (в крайнем левом положении) до 80% максимальной силы (в крайнем правом), удара воспламенения не касается",
    lbl_n2_idle: "N2 Холостой%:",
    hover_n2_idle: "Обороты N2 (%), при которых двигатель считается вышедшим на холостые — задаёт форму кривой разгона (не силу). У некоторых бортов подменяется автоматически профилем самолёта",
    name_engine_start: "Запуск двигателя / Воспламенение",
    chk_four_eng_mode: "4-моторный режим (1&2->Лево, 3&4->Право)",
    hover_four_eng_mode: "4-моторные самолёты: Eng1/Eng2 (левое крыло) группируются на РУД (левая рука), Eng3/Eng4 (правое крыло) — на джойстик (правая рука). Используется максимум N2 в паре, удар воспламенения срабатывает от любого двигателя своей группы.",
    chk_swap_hands: "Поменять стороны (Джойстик=Лево, РУД=Право)",
    hover_swap_hands: "По умолчанию side-bound эффекты (запуск двигателя до воспламенения, split touchdown) считают, что РУД — под левой рукой, джойстик — под правой. Если у вас физически джойстик стоит слева, а РУД справа — включите, чтобы зеркалить сторону. На то, какой мотор РУД (левый/правый) отвечает за Eng1/Eng2, это не влияет — это жёстко задано железом квадранта.",

    heading_gear_comp: "Сжатие стоек шасси (касание)",
    chk_enabled: "Включено",
    hover_headroom: "Задаёт, насколько жёсткость посадки влияет на ДЛИТЕЛЬНОСТЬ удара, а не на его силу: любая посадка всегда бьёт на полных 255. 0% держит длительность фиксированной и минимальной (~230мс) независимо от жёсткости. 100% позволяет жёсткой посадке растянуть удар до ~550мс, а мягкая всё равно останется около 230мс",
    name_nose_peak: "Носовая стойка",
    name_left_peak: "Левая стойка",
    name_right_peak: "Правая стойка",

    lbl_gear_transit: "Уборка/выпуск шасси",
    name_gear_bump: "Касание шасси (удар)",

    lbl_bank_turb: "Крен / Турбулентность",
    hover_bank_intensity: "Сила вибрации при превышении порога крена (Порог ниже). Импульсы учащаются, чем больше угол превышает порог",
    lbl_threshold_deg: "Порог (°):",
    hover_bank_threshold: "Угол крена (°), после которого включается эффект Крен/Турбулентность",

    btn_reset_defaults: "Сброс",
    btn_hide_telemetry: "Скрыть телеметрию",
    btn_show_telemetry: "Показать телеметрию",

    heading_live_aircraft_data: "Телеметрия борта",
    lbl_airspeed: "Приб. скорость (уз):",
    lbl_barber_pole: "Красная черта (уз):",
    lbl_overspeed_warning: "Превышение скорости:",
    lbl_lear_horn: "Lear Horn (XMLSND75):",
    lbl_gs: "Путевая (уз):",
    lbl_on_ground: "На земле:",
    lbl_bank_deg: "Крен (°):",
    lbl_flaps_pct: "Закрылки (%):",
    lbl_slats_pct: "Предкрылки (%):",
    lbl_gear: "Шасси:",
    val_down: "Выпущено",
    val_up: "Убрано",
    lbl_spoilers_pct: "Спойлеры (%):",
    lbl_spoiler_l: "Спойлер Л (%):",
    lbl_spoiler_r: "Спойлер П (%):",
    lbl_nose_gear: "Носовая стойка (%):",
    lbl_left_main: "Левая стойка (%):",
    lbl_right_main: "Правая стойка (%):",
    lbl_stall: "Сваливание:",
    lbl_paused: "Пауза:",
    lbl_no_data: "Нет данных",

    heading_engine_telemetry: "Телеметрия двигателей",
    eng1_header: "Двигатель 1 (Лево / РУД)",
    eng2_header: "Двигатель 2 (Право / Джойстик)",
    eng3_header: "Двигатель 3 (4-мотор: влияет на Лево)",
    eng4_header: "Двигатель 4 (4-мотор: влияет на Право)",
    lbl_n2: "N2:",
    lbl_combustion: "Горение:",
    lbl_starter_active: "Стартер активен:",
    lbl_pmdg_starter_lvar: "PMDG Starter L-Var:",
    lbl_pct_max_rpm: "% Max RPM:",
    lbl_engine_rpm: "Обороты двигателя:",
    lbl_prop_rpm: "Обороты винта:",
    val_on: "ВКЛ",
    val_off: "ВЫКЛ",

    heading_logs: "Логи",
    chk_autoscroll: "Автопрокрутка",

    tray_stop: "Стоп",
    tray_resume: "Продолжить",
    tray_check_updates: "Проверить обновления…",
    tray_quit: "Выход",
    tray_status_active: "● Активно",
    tray_status_stopped: "○ Остановлено",

    upd_title_up_to_date: "Обновлений нет",
    upd_title_update_available: "Доступно обновление",
    upd_title_updating: "Обновление",
    upd_body_updating: "Приложение сейчас закроется, чтобы применить обновление. Оно перезапустится автоматически.",
    upd_title_update_failed: "Ошибка обновления",
    upd_title_update_check_failed: "Не удалось проверить обновления",
    upd_title_launch_failed: "Не удалось запустить",
    upd_title_admin_required: "Требуются права администратора",
    upd_body_admin_required: "Программа установлена в защищённую папку (например, Program Files).\nЧтобы обновить, нажмите «Да» в запросе повышения прав, либо перенесите программу в папку с доступом на запись (например, Документы) и попробуйте снова.",

    nav_rumble: "Аэродинамика",
    nav_taxi: "Стыки ВПП",
    nav_engines: "Двигатели: Запуск/Останов",
    nav_gear: "Стойки шасси: касание",
    nav_telemetry: "Телеметрия",
    heading_live_monitor: "Монитор активности",
    lbl_no_active_effects: "Нет активных эффектов",
    hover_monitor_collapse: "Скрыть монитор активности",
    hover_monitor_expand: "Показать монитор активности",
    btn_set_n2_idle: "(SET)",
    hover_set_n2_idle: "Записать текущее значение минус 1.5% в N2 Холостой%",
    log_n2_idle_set: "N2 Холостой задан: {val:.1}% (из телеметрии Eng1 - 1.5%)",

    nav_wt: "War Thunder",
    wt_status_disconnected: "Нет соединения",
    wt_status_menu: "Ангар / Меню",
    wt_status_in_battle: "В бою",
    name_wt_weapon1: "Оружие 1",
    name_wt_weapon2: "Оружие 2",
    hover_wt_weapon1: "Стрельба первой группы оружия (пулемёты) — маршрутизация только на джойстик, настроена как быстрая текстура огня.",
    hover_wt_weapon2: "Стрельба второй группы оружия (пушки) — маршрутизация только на РУД (оба мотора), настроена как более медленная и тяжёлая текстура. Маршрутизация обеих групп зафиксирована (не настраивается пользователем), чтобы группы оставались различимы в разных руках.",
    hover_wt_flaps: "Вибрация моторчика закрылков во время изменения их положения.",
    hover_wt_gear_transit: "Гул мотора шасси во время уборки/выпуска + удар фиксации, когда шасси приходит в крайнее убранное или выпущенное положение. В API War Thunder нет отдельной телеметрии по створкам шасси, поэтому эффект покрывает только движение + фиксацию.",
    name_wt_stall: "Срыв потока",
    hover_wt_stall: "Баффет от срыва потока при приближении угла атаки к критическому. В v1 — один захардкоженный профиль (только Bf 109 F-4), на любом другом самолёте эффект молчит: в игровых данных War Thunder нет критического угла атаки в градусах, чтобы вывести порог автоматически. Пороги — стартовая гипотеза, требует калибровки по живому тесту на реальном самолёте.",
    name_wt_engine_start: "Пуск/останов двигателя",
    hover_wt_engine_start: "Запуск и останов двигателя по оборотам RPM 1 (моно — без раскладки по двигателям/сторонам, многомоторные борта запускают моторы синхронно). Прокрутка и раскрутка — учащающиеся и всё более резкие толчки, схватывание — один чёткий удар, на устойчивых оборотах — тишина, затем выбег винта — толчки замедляются И линейно затухают до тишины, а в момент полной остановки — два отдельных рывка подряд. Самокалибруется — таблица холостых оборотов по каждому борту не нужна. Пороги — стартовая гипотеза, требует калибровки по живому тесту.",
    name_wt_overspeed: "Overspeed (Vne)",
    hover_wt_overspeed: "Вибрация на оба мотора РУД по мере приближения приборной скорости к предельно допустимой — нарастает линейно на последних 10 км/ч перед порогом, затем держится на полной мощности на/за порогом. Если выпущены закрылки, порог ужесточается до их собственной скорости разрушения (если для борта есть такие данные) — иначе используется общий Vne. У шасси свой отдельный эффект ниже. Данные — из таблиц на ~1300 бортов с wiki War Thunder; борта вне таблиц молчат, а не гадают.",
    name_wt_gear_overspeed: "Overspeed шасси",
    hover_wt_gear_overspeed: "Вибрация на оба мотора РУД, когда шасси выпущено и приборная скорость приближается к скорости разрушения самого шасси — нарастает линейно на последних 20 км/ч перед порогом, затем держится на полной мощности на/за порогом. Молчит, если шасси убрано, или если для борта нет данных по скорости разрушения шасси на wiki War Thunder (например, неубираемое шасси).",
    heading_wt_telemetry: "Телеметрия War Thunder",
    lbl_wt_flaps_pct: "Закрылки",
    lbl_wt_gear_pct: "Шасси",
    lbl_wt_aoa_deg: "Угол атаки (град)",
    lbl_wt_wx_deg_s: "Угловая скорость крена Wx (град/с)",
    lbl_wt_rpm1: "Обороты",
    lbl_wt_weapon1_ammo: "Патроны weapon1",
    lbl_wt_weapon2_ammo: "Патроны weapon2",
    lbl_wt_ammo_unknown: "— (нет счётчика на этом борту)",
    lbl_wt_vehicle_type: "Самолёт",
    lbl_wt_vehicle_type_unknown: "— (не определён)",
    lbl_wt_speed_kt: "Скорость (узлы)",
    lbl_wt_altitude_ft: "Высота (футы)",

    nav_effects: "Редактор эффектов",
    msg_fx_effects_coexist: "Встроенные и пользовательские эффекты работают одновременно — ваш эффект лишь заменяет встроенный на том же источнике.",
    lbl_fx_overrides_builtin: "Заменяет встроенный эффект:",
    lbl_fx_overrides_builtin_none: "Этот источник не заменяет ничего встроенного — оба эффекта будут работать вместе.",
    lbl_builtin_overridden_by: "Заменён вашим эффектом:",
    lbl_replaced_by_custom: "заменён",
    btn_fx_new: "+ Новый эффект",
    preset_impact: "Удар",
    hover_preset_impact: "Короткий импульс на событие — касание ВПП, выпуск шасси. Срабатывает один раз и затухает.",
    preset_hum: "Гул",
    hover_preset_hum: "Текстурный гул, пока значение выше порога — превышение скорости, срыв потока. Та же текстура, что у встроенной стрельбы.",
    preset_pulsation: "Пульсация",
    hover_preset_pulsation: "Мягкая плавная пульсация, пока значение выше порога — деликатное фоновое предупреждение.",
    preset_growing: "Нарастание",
    hover_preset_growing: "Сила напрямую следует за значением источника, без порога — обычное поведение по умолчанию.",
    btn_fx_duplicate: "Дублировать",
    btn_fx_delete: "Удалить",
    btn_fx_import: "Импорт…",
    btn_fx_export: "Экспорт…",
    empty_fx_hint: "Пользовательских эффектов пока нет — нажми «+ Новый эффект», чтобы собрать свой.",
    lbl_fx_name: "Название",
    lbl_fx_games: "Игры",
    lbl_fx_aircraft: "Только для борта",
    hover_fx_aircraft: "Подстрока названия борта; пусто = любой борт.",
    lbl_fx_lvar_name: "Имя переменной",
    lbl_fx_lvar_hint: "Точное имя ищите в самом симуляторе: меню разработчика (Dev Mode) > Behaviors (или список локальных переменных).",
    lbl_fx_lvar_prefix_hint: "Имена локальных переменных обычно начинаются с «L:».",
    lbl_fx_lvar_unit: "Единица измерения",
    opt_fx_lvar_unit_custom: "Своя…",
    lbl_fx_lvar_unit_custom: "Название своей единицы",
    msg_fx_lvar_msfs_only: "Пользовательские переменные существуют только в MSFS — маска игр зафиксирована на MSFS.",
    hdr_fx_source_group_flight: "MSFS / X-Plane — параметры полёта",
    hdr_fx_source_group_wt: "War Thunder",
    hdr_fx_source_group_lvar: "Microsoft Flight Simulator — своя переменная",
    step_fx_source: "1. Источник",
    step_fx_when: "2. Когда срабатывает",
    step_fx_curve: "3. Кривая отклика",
    step_fx_shape: "4. Форма и выход",
    lbl_fx_live_value: "Живое значение",
    lbl_fx_no_signal: "нет сигнала",
    trigger_always: "Всегда",
    trigger_above: "Выше порога",
    trigger_below: "Ниже порога",
    trigger_between: "В диапазоне",
    trigger_is_true: "Когда включено",
    trigger_changed: "Пока меняется",
    hover_trigger_changed: "Срабатывает, пока значение движется — для закрылков, шасси и прочих подвижных плоскостей.",
    lbl_fx_threshold: "Порог",
    lbl_fx_hysteresis: "Гистерезис",
    hover_fx_hysteresis: "Мёртвая полоса, чтобы эффект не дребезжал прямо на пороге.",
    lbl_fx_range_lo: "От",
    lbl_fx_range_hi: "До",
    lbl_fx_eps: "Минимальный шаг",
    lbl_fx_hold: "Удержание",
    lbl_fx_curve_hint: "Тяни точку мышью, клик по линии добавляет новую, правый клик по точке удаляет.",
    shape_constant: "Ровный",
    shape_pulse: "Пульсация",
    shape_oneshot: "Одиночный удар",
    shape_sine: "Волна",
    shape_sawtooth: "Пила",
    lbl_fx_freq: "Частота",
    hover_fx_freq: "Ограничена 6.5 Гц: на устройство мы шлём 20 раз в секунду, более частые импульсы теряются между кадрами.",
    lbl_fx_duty: "Ширина импульса",
    lbl_fx_jitter: "Неровность",
    lbl_fx_floor: "Между импульсами",
    lbl_fx_attack: "Нарастание",
    lbl_fx_decay: "Спад",
    lbl_fx_depth: "Глубина",
    lbl_fx_strength: "Сила",
    lbl_fx_smoothing: "Сглаживание",
    lbl_fx_mix: "При наложении эффектов",
    mix_max: "Брать сильнейший",
    mix_add: "Складывать",
    lbl_fx_output: "Отправлять на",
    lbl_fx_out_joystick: "Джойстик",
    lbl_fx_out_throttle_left: "РУД, левый мотор",
    lbl_fx_out_throttle_right: "РУД, правый мотор",
    heading_fx_preview: "Предпросмотр",
    btn_fx_play: "Играть на устройстве",
    btn_fx_stop: "Стоп",
    lbl_fx_preview_loops_events: "В игре этот эффект срабатывает один раз по событию, а в предпросмотре повторяется по кругу, чтобы его можно было настроить на ощупь.",
    lbl_fx_test_value: "Пробное значение",
    btn_fx_open_session: "Открыть запись…",
    btn_fx_play_session: "Проиграть запись",
    lbl_fx_session_none: "Запись не загружена",
    lbl_fx_session_loaded: "Загружено: {name} — {frames} кадр(ов), {dur:.1} с",
    lbl_fx_session_game_mismatch: "Эта запись сделана в {rec} — источник эффекта относится к {src} и в этой записи не пишется. Выберите источник для {rec}, либо откройте запись из {src}.",
    warn_fx_no_output: "Эффект не направлен ни на один мотор.",
    warn_fx_always_on: "Эффект будет вибрировать непрерывно: у него нет порога и ровная форма.",
    warn_fx_wrong_game: "Этот источник недоступен в активной игре.",
    dlg_fx_export_title: "Экспорт эффектов",
    dlg_fx_import_title: "Импорт эффектов",
    dlg_fx_session_title: "Открыть запись сессии",
    filter_fx_file: "Эффекты Aurora Vibra",
    filter_session_file: "Запись сессии",
};

pub fn upd_body_up_to_date(lang: Lang, version: &str) -> String {
    match lang {
        Lang::En => format!("You are running the latest version ({}).", version),
        Lang::Ru => format!("Вы используете последнюю версию ({}).", version),
    }
}

pub fn upd_body_update_available(lang: Lang, current: &str, latest: &str, name: &str) -> String {
    match lang {
        Lang::En => format!(
            "A new version is available.\n\nCurrent: {}\nLatest:  {}\n\nRelease: {}\n\nInstall now? The app will restart.",
            current, latest, name
        ),
        Lang::Ru => format!(
            "Доступна новая версия.\n\nТекущая: {}\nПоследняя: {}\n\nРелиз: {}\n\nУстановить сейчас? Приложение перезапустится.",
            current, latest, name
        ),
    }
}

pub fn lbl_disabled_count(lang: Lang, n: usize) -> String {
    match lang {
        Lang::En => format!("+{n} disabled"),
        Lang::Ru => format!("+{n} выключено"),
    }
}

pub fn upd_body_launch_failed(lang: Lang, path: &str) -> String {
    match lang {
        Lang::En => format!("Could not start:\n{}", path),
        Lang::Ru => format!("Не удалось запустить:\n{}", path),
    }
}
