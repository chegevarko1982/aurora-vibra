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
    pub chk_wt_enabled: &'static str,
    pub hover_wt_enabled: &'static str,

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
    pub status_off: &'static str,

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
    pub heading_wt_telemetry: &'static str,
    pub lbl_wt_flaps_pct: &'static str,
    pub lbl_wt_gear_pct: &'static str,
    pub lbl_wt_aoa_deg: &'static str,
    pub lbl_wt_wx_deg_s: &'static str,
    pub lbl_wt_weapon1_ammo: &'static str,
    pub lbl_wt_weapon2_ammo: &'static str,
    pub lbl_wt_ammo_unknown: &'static str,
    pub lbl_wt_vehicle_type: &'static str,
    pub lbl_wt_vehicle_type_unknown: &'static str,
    pub lbl_wt_speed_kt: &'static str,
    pub lbl_wt_altitude_ft: &'static str,
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
\"Check for updates…\" in the \"...\" menu → Options runs a one-off version check; if a newer release exists, it offers to install it and restart the app automatically.",
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
    chk_wt_enabled: "War Thunder",
    hover_wt_enabled: "Enables War Thunder support: gunfire, taking hits, gear and flaps, engine start. Telemetry comes from the game's official HTTP API on localhost:8111 — turn on \"Local host telemetry\" in the game settings. Work in progress: the switch is saved but nothing is wired up yet.",

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
    status_off: "Off",

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
    heading_wt_telemetry: "War Thunder Telemetry",
    lbl_wt_flaps_pct: "Flaps",
    lbl_wt_gear_pct: "Gear",
    lbl_wt_aoa_deg: "AoA (deg)",
    lbl_wt_wx_deg_s: "Roll rate Wx (deg/s)",
    lbl_wt_weapon1_ammo: "Weapon1 ammo",
    lbl_wt_weapon2_ammo: "Weapon2 ammo",
    lbl_wt_ammo_unknown: "— (no counter on this aircraft)",
    lbl_wt_vehicle_type: "Aircraft",
    lbl_wt_vehicle_type_unknown: "— (unknown)",
    lbl_wt_speed_kt: "Speed (kt)",
    lbl_wt_altitude_ft: "Altitude (ft)",
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
Пункт «Проверить обновления…» в меню «...» → Опции запускает разовую проверку версии; при наличии новой — предложит установить и перезапустить приложение автоматически.",
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
    chk_wt_enabled: "War Thunder",
    hover_wt_enabled: "Включает поддержку War Thunder: стрельба, попадания по своему борту, шасси и закрылки, запуск двигателя. Телеметрия берётся из официального HTTP API игры на localhost:8111 — не забудь включить «Локальная телеметрия» в настройках игры. В разработке: переключатель сохраняется, но функционал ещё не подключён.",

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
    status_off: "Выкл",

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
    heading_wt_telemetry: "Телеметрия War Thunder",
    lbl_wt_flaps_pct: "Закрылки",
    lbl_wt_gear_pct: "Шасси",
    lbl_wt_aoa_deg: "Угол атаки (град)",
    lbl_wt_wx_deg_s: "Угловая скорость крена Wx (град/с)",
    lbl_wt_weapon1_ammo: "Патроны weapon1",
    lbl_wt_weapon2_ammo: "Патроны weapon2",
    lbl_wt_ammo_unknown: "— (нет счётчика на этом борту)",
    lbl_wt_vehicle_type: "Самолёт",
    lbl_wt_vehicle_type_unknown: "— (не определён)",
    lbl_wt_speed_kt: "Скорость (узлы)",
    lbl_wt_altitude_ft: "Высота (футы)",
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
