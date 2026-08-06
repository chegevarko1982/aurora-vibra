use std::ffi::{c_char, c_void};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use libloading::Library;
use parking_lot::Mutex;

use crate::RumbleEngine;
use crate::game_state::GameSlot;
use crate::sim::elem_idx::ElemIdx;
use crate::sim::parse::{flight_status, parse_main_elems};
use crate::{ActiveGame, ConfigShared, EffectsShared, FlightVars, HidCmd, LogBuffer, SimStatus};

type DWord = u32;
// Имя намеренно совпадает с типом из Win32 SDK — так подписи FFI ниже читаются
// один в один с документацией SimConnect. Переименование в Hresult ради
// clippy::upper_case_acronyms сделало бы их только труднее сверять.
#[allow(clippy::upper_case_acronyms)]
type HRESULT = i32;
type Handle = *mut c_void;
type HWnd = *mut c_void;

#[repr(C)]
struct SimRecv {
    dw_size: DWord,
    dw_version: DWord,
    dw_id: DWord,
}

#[repr(C)]
struct SimRecvSimObjectData {
    base: SimRecv,
    dw_request_id: DWord,
    dw_object_id: DWord,
    dw_define_id: DWord,
    dw_flags: DWord,
    dw_entrynumber: DWord,
    dw_outof: DWord,
    dw_define_count: DWord,
    dw_data: DWord,
}

#[repr(C)]
struct SimRecvEvent {
    base: SimRecv,
    u_group_id: DWord,
    u_event_id: DWord,
    dw_data: DWord,
}

// Отражает SIMCONNECT_RECV_EXCEPTION из SimConnect SDK. dw_send_id — это ID
// того самого вызова (Add_to_data_definition/RequestDataOnSimObject/...),
// который вызвал исключение, что позволяет сопоставить его с конкретным
// запросом (например, с REQ_TITLE) по логам.
#[repr(C)]
struct SimRecvException {
    base: SimRecv,
    dw_exception: DWord,
    dw_send_id: DWord,
    dw_index: DWord,
}

// SIMCONNECT_RECV_SYSTEM_STATE — returned by RequestSystemState.
// The layout mirrors the SDK struct: base header + request id +
// the actual state value (a STRING256 in our case for "AircraftLoaded").
#[repr(C)]
struct SimRecvSystemState {
    base: SimRecv,
    dw_request_id: DWord,
    dw_data: DWord,
}

#[repr(C)]
struct SimAircraftTitle {
    pub title: [u8; 256],
}

fn parse_aircraft_title(buf: &[u8; 256]) -> String {
    let nul_pos = buf.iter().position(|&b| b == 0).unwrap_or(256);
    String::from_utf8_lossy(&buf[..nul_pos]).trim().to_string()
}

const SIMCONNECT_RECV_ID_OPEN: DWord = 2;
const SIMCONNECT_RECV_ID_QUIT: DWord = 3;
const SIMCONNECT_RECV_ID_EVENT: DWord = 4;
const SIMCONNECT_RECV_ID_EXCEPTION: DWord = 5;
const SIMCONNECT_RECV_ID_SIMOBJECT_DATA: DWord = 8;
const SIMCONNECT_RECV_ID_SYSTEM_STATE: DWord = 11;

const SIMCONNECT_PERIOD_ONCE: DWord = 1;
const SIMCONNECT_PERIOD_SIM_FRAME: DWord = 3;
#[allow(dead_code)]
const SIMCONNECT_PERIOD_SECOND: DWord = 4;

// SIMCONNECT_DATA_REQUEST_FLAG_CHANGED: сервер шлёт SIMOBJECT_DATA только
// когда значение реально ИЗМЕНИЛОСЬ с прошлого тика периода — то есть при
// SIMCONNECT_PERIOD_SECOND с этим флагом мы не заваливаем канал одинаковыми
// пакетами каждую секунду, а получаем новый TITLE ровно в тот момент, когда
// он появился/сменился (включая случай, когда самолёт уже стоял на перроне
// ДО подключения приложения — первая же секунда после Open() пришлёт
// актуальное значение, а не только при событии SimStart).
#[allow(dead_code)]
const SIMCONNECT_DATA_REQUEST_FLAG_CHANGED: DWord = 1;

const SIMCONNECT_DATATYPE_FLOAT64: DWord = 4;
const SIMCONNECT_DATATYPE_STRING256: DWord = 12;

const USER_OBJECT_ID: DWord = 0;

const EVT_SIM_START: DWord = 1001;
const EVT_SIM_STOP: DWord = 1002;
const EVT_FRAME: DWord = 1003;
const EVT_AIRCRAFT_LOADED: DWord = 1004;

const EVT_PAUSE_SYS: DWord = 4101;
const EVT_PAUSE_EX1_SYS: DWord = 4102;

const DEF_MAIN: DWord = 2001;
const REQ_MAIN: DWord = 3001;
const DEF_PING: DWord = 2101;
const REQ_PING: DWord = 3101;
const DEF_TITLE: DWord = 2201;
const REQ_TITLE: DWord = 3201;
const REQ_SYS_STATE: DWord = 3301;

type PfnSimConnectOpen =
    unsafe extern "system" fn(*mut Handle, *const c_char, HWnd, DWord, Handle, DWord) -> HRESULT;
type PfnSimConnectClose = unsafe extern "system" fn(Handle) -> HRESULT;
type PfnSimConnectAddToDataDefinition = unsafe extern "system" fn(
    Handle,
    DWord,
    *const c_char,
    *const c_char,
    DWord,
    f32,
    DWord,
) -> HRESULT;
type PfnSimConnectRequestDataOnSimObject = unsafe extern "system" fn(
    Handle,
    DWord,
    DWord,
    DWord,
    DWord,
    DWord,
    DWord,
    DWord,
    DWord,
) -> HRESULT;
type PfnSimConnectGetNextDispatch =
    unsafe extern "system" fn(Handle, *mut *mut SimRecv, *mut DWord) -> HRESULT;
type PfnSimConnectSubscribeToSystemEvent =
    unsafe extern "system" fn(Handle, DWord, *const c_char) -> HRESULT;
type PfnSimConnectRequestSystemState =
    unsafe extern "system" fn(Handle, DWord, *const c_char, DWord, DWord, DWord) -> HRESULT;

#[inline]
fn hr_hex(hr: HRESULT) -> String {
    format!("0x{:08X}", hr as u32)
}

#[derive(Clone)]
struct SimConnectFns {
    _lib: Arc<Library>,
    open: PfnSimConnectOpen,
    close: PfnSimConnectClose,
    add_to_def: PfnSimConnectAddToDataDefinition,
    req_data: PfnSimConnectRequestDataOnSimObject,
    next_dispatch: PfnSimConnectGetNextDispatch,
    subscribe_event: Option<PfnSimConnectSubscribeToSystemEvent>,
    request_system_state: Option<PfnSimConnectRequestSystemState>,
}

// Проприетарный компонент Microsoft, не покрытый MIT этого проекта —
// см. THIRD-PARTY-NOTICES.md.
const EMBED_SIMCONNECT_BYTES: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/lib/SimConnect.dll"));

/// Пишет вшитую DLL во временный файл и грузит её оттуда.
///
/// Запись идёт во временное имя с последующим rename: имя результата
/// фиксировано, и две одновременно запущенные копии приложения иначе писали бы
/// в один и тот же файл, пока другая его уже грузит. Если файл уже на месте и
/// совпадает по содержимому — не трогаем его вовсе: перезапись DLL, которую
/// держит открытой соседний процесс, всё равно не удалась бы.
fn try_load_embedded_simconnect(logs: &LogBuffer) -> Result<Library> {
    let dst = std::env::temp_dir().join("aurora-simconnect-embedded-64.dll");

    let up_to_date = std::fs::read(&dst)
        .map(|existing| existing == EMBED_SIMCONNECT_BYTES)
        .unwrap_or(false);

    if up_to_date {
        logs.push(format!(
            "SimConnect: embedded DLL already extracted at {}",
            dst.display()
        ));
    } else {
        logs.push(format!(
            "SimConnect: writing embedded DLL to {}",
            dst.display()
        ));
        let tmp = std::env::temp_dir().join(format!(
            "aurora-simconnect-embedded-64.{}.tmp",
            std::process::id()
        ));
        std::fs::write(&tmp, EMBED_SIMCONNECT_BYTES)
            .with_context(|| format!("write {}", tmp.display()))?;
        if let Err(e) = std::fs::rename(&tmp, &dst) {
            // Занят другим экземпляром — грузим из своего временного файла.
            logs.push(format!(
                "SimConnect: rename to {} failed ({e}), using {}",
                dst.display(),
                tmp.display()
            ));
            let lib = unsafe { Library::new(&tmp) }
                .with_context(|| format!("Library::new({})", tmp.display()))?;
            logs.push("SimConnect: embedded DLL loaded successfully");
            return Ok(lib);
        }
    }

    logs.push(format!(
        "SimConnect: loading embedded DLL from {}",
        dst.display()
    ));
    let lib = unsafe { Library::new(&dst) }
        .with_context(|| format!("Library::new({})", dst.display()))?;

    logs.push("SimConnect: embedded DLL loaded successfully");
    Ok(lib)
}

fn bind_simconnect(lib: Library) -> Result<SimConnectFns> {
    unsafe {
        let open: PfnSimConnectOpen = *lib.get(b"SimConnect_Open\0")?;
        let close: PfnSimConnectClose = *lib.get(b"SimConnect_Close\0")?;
        let add_to_def: PfnSimConnectAddToDataDefinition =
            *lib.get(b"SimConnect_AddToDataDefinition\0")?;
        let req_data: PfnSimConnectRequestDataOnSimObject =
            *lib.get(b"SimConnect_RequestDataOnSimObject\0")?;
        let next_dispatch: PfnSimConnectGetNextDispatch =
            *lib.get(b"SimConnect_GetNextDispatch\0")?;
        let subscribe_event: Option<PfnSimConnectSubscribeToSystemEvent> = lib
            .get::<PfnSimConnectSubscribeToSystemEvent>(b"SimConnect_SubscribeToSystemEvent\0")
            .ok()
            .map(|s| *s);
        let request_system_state: Option<PfnSimConnectRequestSystemState> = lib
            .get::<PfnSimConnectRequestSystemState>(b"SimConnect_RequestSystemState\0")
            .ok()
            .map(|s| *s);

        Ok(SimConnectFns {
            _lib: std::sync::Arc::new(lib),
            open,
            close,
            add_to_def,
            req_data,
            next_dispatch,
            subscribe_event,
            request_system_state,
        })
    }
}

/// Ищет клиентскую библиотеку SimConnect в явном порядке кандидатов.
///
/// Полагаться на голый `Library::new("SimConnect.dll")` недостаточно: на машине,
/// где стоит только MSFS 2024, файла с таким именем нет нигде. Симулятор держит
/// свою копию под именем SimConnect_internal.dll, а каталог WindowsApps закрыт
/// ACL — поэтому «просто найдётся сам» не работает, и каждый шаг логируется,
/// чтобы по логу было видно, что именно перепробовано.
fn load_simconnect(logs: &LogBuffer) -> Result<SimConnectFns> {
    // 1. Путь, указанный пользователем вручную, — имеет приоритет над всем.
    if let Some(path) = crate::settings::simconnect_dll_path() {
        logs.push(format!(
            "SimConnect: trying user-configured path {}...",
            path.display()
        ));
        match unsafe { Library::new(&path) } {
            Ok(lib) => {
                logs.push("SimConnect: loaded from user-configured path");
                return bind_simconnect(lib);
            }
            Err(e) => logs.push(format!("SimConnect: user-configured path failed: {e}")),
        }
    }

    // 2. Каталог рядом с exe — явно, не полагаясь на порядок поиска Win32.
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    {
        let path = dir.join("SimConnect.dll");
        if path.exists() {
            logs.push(format!("SimConnect: trying {}...", path.display()));
            match unsafe { Library::new(&path) } {
                Ok(lib) => {
                    logs.push("SimConnect: loaded from EXE directory");
                    return bind_simconnect(lib);
                }
                Err(e) => logs.push(format!("SimConnect: EXE directory failed: {e}")),
            }
        }
    }

    // 3. Обычный поиск Win32 — PATH и системные каталоги.
    logs.push("SimConnect: trying normal load (PATH / system dirs)...");
    match unsafe { Library::new("SimConnect.dll") } {
        Ok(lib) => {
            logs.push("SimConnect: loaded via normal search");
            return bind_simconnect(lib);
        }
        Err(e) => {
            logs.push(format!("SimConnect: normal search failed: {e}"));
        }
    }

    // 4. Последний резерв — вшитая копия.
    let lib = try_load_embedded_simconnect(logs)
        .context("embedded SimConnect fallback was unavailable or failed to load")?;
    bind_simconnect(lib)
}

// Точка входа рабочего потока: каждый аргумент — отдельный разделяемый с UI
// примитив (см. вызов в main.rs). Схлопывать их в один "контекст"-структуру
// смысла нет — она была бы ровно этим же списком полей, только с лишним слоем.
#[allow(clippy::too_many_arguments)]
pub fn sim_worker(
    last_vars: Arc<Mutex<Option<FlightVars>>>,
    tx_hid: Sender<HidCmd>,
    logs: LogBuffer,
    config: Arc<ConfigShared>,
    effects: EffectsShared,
    hold: Arc<AtomicBool>,
    status: Arc<Mutex<SimStatus>>,
    aircraft_title: Arc<Mutex<String>>,
    aircraft_profiles: Arc<Mutex<crate::aircraft_profiles::AircraftProfiles>>,
    profile_state: Arc<Mutex<crate::profiles::ProfileState>>,
    game: GameSlot,
) {
    logs.push("SimConnect: worker started");

    let fns = match load_simconnect(&logs) {
        Ok(f) => {
            logs.push("SimConnect: loaded (normal search or embedded fallback)");
            f
        }
        Err(e) => {
            logs.push(format!("SimConnect: {}", e));
            // Без этого бейдж остался бы на Disconnected — неотличимо от
            // «симулятор не запущен», хотя перезапуск сима тут не поможет.
            *status.lock() = SimStatus::SimConnectMissing;
            return;
        }
    };

    unsafe {
        loop {
            let mut h_sc: Handle = std::ptr::null_mut();
            let name = std::ffi::CString::new("AuroraVibra").unwrap();
            let hr = (fns.open)(
                &mut h_sc,
                name.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0xFFFFFFFF,
            );
            if hr < 0 || h_sc.is_null() {
                logs.push(format!("SimConnect: Open failed {}", hr_hex(hr)));
                thread::sleep(Duration::from_millis(1000));
                continue;
            }

            // MSFS считается живой с самого момента успешного Open() (не по
            // отдельному вотчдогу) — заявляем слот сразу же. Если WT/X-Plane
            // уже владеет им (липкое владение) или форс-оверрайд другой игры
            // запрещает MSFS — claim не проходит, и ниже мы просто не пишем
            // status/aircraft_title/телеметрию/HID, пока слот не освободится
            // (повторные попытки — на каждой итерации внутреннего цикла).
            let mut owns_slot = if crate::settings::game_override().vetoes(ActiveGame::Msfs) {
                game.release_if_owned(ActiveGame::Msfs);
                false
            } else {
                game.try_claim(ActiveGame::Msfs)
            };
            if owns_slot {
                *status.lock() = SimStatus::Connected;
                *aircraft_title.lock() = String::new();
            }

            let mut in_flight: bool = true;
            let mut rumble_engine = RumbleEngine::new();

            if let Some(sub) = fns.subscribe_event {
                for (id, ev) in &[
                    (EVT_SIM_START, "SimStart"),
                    (EVT_SIM_STOP, "SimStop"),
                    (EVT_FRAME, "Frame"),
                    (EVT_AIRCRAFT_LOADED, "AircraftLoaded"),
                ] {
                    let ev_c = std::ffi::CString::new(*ev).unwrap();
                    let hr = sub(h_sc, *id, ev_c.as_ptr());
                    if hr < 0 {
                        logs.push(format!(
                            "SimConnect: subscribe {} FAILED {}",
                            ev,
                            hr_hex(hr)
                        ));
                    }
                }

                if let Ok(c) = std::ffi::CString::new("Pause") {
                    let hr = sub(h_sc, EVT_PAUSE_SYS, c.as_ptr());
                    if hr < 0 {
                        logs.push(format!("SimConnect: subscribe Pause FAILED {}", hr_hex(hr)));
                    }
                }
                if let Ok(c) = std::ffi::CString::new("Pause_EX1") {
                    let hr = sub(h_sc, EVT_PAUSE_EX1_SYS, c.as_ptr());
                    if hr < 0 {
                        logs.push(format!(
                            "SimConnect: subscribe Pause_EX1 FAILED {}",
                            hr_hex(hr)
                        ));
                    }
                }
                logs.push("SimConnect: Pause subscriptions active.".to_string());
            }

            let add = |def_id: DWord, name_s: &str, unit_s: &str| -> HRESULT {
                let n = std::ffi::CString::new(name_s).unwrap();
                let u = std::ffi::CString::new(unit_s).unwrap();
                (fns.add_to_def)(
                    h_sc,
                    def_id,
                    n.as_ptr(),
                    u.as_ptr(),
                    SIMCONNECT_DATATYPE_FLOAT64,
                    0.0,
                    0xFFFF_FFFF,
                )
            };

            // Registration order (and the corresponding elem[] read order in
            // sim/parse.rs) lives in ElemIdx::DEFS — see sim/elem_idx.rs for
            // the full per-variable rationale (which addon needs it, what was
            // tried and rejected).
            for &(name, unit) in ElemIdx::DEFS {
                let hr = add(DEF_MAIN, name, unit);
                if hr < 0 {
                    logs.push(format!(
                        "SimConnect: AddToDef {:?} [{}] FAILED {}",
                        name,
                        unit,
                        hr_hex(hr)
                    ));
                }
            }

            {
                let n = std::ffi::CString::new("TITLE").unwrap();
                // ВАЖНО: для строковых типов (SIMCONNECT_DATATYPE_STRINGxx) SDK
                // требует передавать NULL в качестве UnitsName — у строк нет
                // единиц измерения. Ранее здесь передавалась строка "string",
                // что не соответствует контракту AddToDataDefinition и на
                // некоторых сборках SimConnect может привести к отказу
                // регистрации определения (см. SIMCONNECT_RECV_ID_EXCEPTION
                // в логах ниже, если это всё же произойдёт).
                let hr = (fns.add_to_def)(
                    h_sc,
                    DEF_TITLE,
                    n.as_ptr(),
                    std::ptr::null(),
                    SIMCONNECT_DATATYPE_STRING256,
                    0.0,
                    0xFFFF_FFFF,
                );
                if hr < 0 {
                    logs.push(format!("SimConnect: AddToDef TITLE FAILED {}", hr_hex(hr)));
                } else {
                    logs.push("SimConnect: AddToDef TITLE ok");
                }
            }

            {
                let n = std::ffi::CString::new("SIM ON GROUND").unwrap();
                let u = std::ffi::CString::new("Bool").unwrap();
                let hr = (fns.add_to_def)(
                    h_sc,
                    DEF_PING,
                    n.as_ptr(),
                    u.as_ptr(),
                    SIMCONNECT_DATATYPE_FLOAT64,
                    0.0,
                    0xFFFF_FFFF,
                );
                if hr < 0 {
                    logs.push(format!("SimConnect: AddToDef PING FAILED {}", hr_hex(hr)));
                }
            }

            // --- MobiFlight late-connect strategy: RequestSystemState fallback ---
            // Right after connection (and after the TITLE data definition is
            // registered), we call RequestSystemState with the "AircraftLoaded"
            // state name. This forces MSFS to IMMEDIATELY return the current
            // aircraft's file path via SIMCONNECT_RECV_SYSTEM_STATE, even when
            // the application was launched AFTER the flight had already loaded.
            // This is the key difference from the naive approach: instead of
            // relying solely on SimStart events (which are never sent "retroactively"
            // to late-connecting clients), we ask the simulator for the current
            // state directly.
            if let Some(req_sys) = fns.request_system_state {
                let state_name = std::ffi::CString::new("AircraftLoaded").unwrap();
                let hr_sys = req_sys(
                    h_sc,
                    REQ_SYS_STATE,
                    state_name.as_ptr(),
                    0, // dw_data: 0 = request current value
                    0, // dw_flags: reserved, must be 0
                    0, // dw_event_id: 0 = no event
                );
                if hr_sys < 0 {
                    logs.push(format!(
                        "SimConnect: RequestSystemState AircraftLoaded FAILED {}",
                        hr_hex(hr_sys)
                    ));
                } else {
                    logs.push("SimConnect: RequestSystemState AircraftLoaded sent (late-connect fallback)");
                }
            }

            // PERIOD_SECOND + FLAG_CHANGED вместо PERIOD_ONCE: одноразовый запрос
            // не покрывает случай, когда самолёт УЖЕ стоял загруженным на
            // перроне ДО запуска приложения (SimStart в этом случае вообще не
            // придёт — SimConnect не шлёт его "задним числом" подключившимся
            // позже клиентам) — а также любой другой транзиентный сбой первого
            // ответа. При PERIOD_SECOND сервер лично перепроверяет TITLE каждую
            // секунду и присылает пакет ТОЛЬКО когда значение действительно
            // изменилось (FLAG_CHANGED) — то есть уже в первую секунду после
            // подписки мы получим текущее (уже ненулевое) значение.
            let hr_title_req = (fns.req_data)(
                h_sc,
                REQ_TITLE,
                DEF_TITLE,
                USER_OBJECT_ID,
                SIMCONNECT_PERIOD_ONCE,
                0,
                0,
                0,
                0,
            );
            if hr_title_req < 0 {
                logs.push(format!(
                    "SimConnect: RequestDataOnSimObject TITLE FAILED {}",
                    hr_hex(hr_title_req)
                ));
            } else {
                logs.push("SimConnect: TITLE requested (initial ONCE)");
            }
            let _ = (fns.req_data)(
                h_sc,
                REQ_MAIN,
                DEF_MAIN,
                USER_OBJECT_ID,
                SIMCONNECT_PERIOD_SIM_FRAME,
                0,
                0,
                0,
                0,
            );
            thread::sleep(Duration::from_millis(60));
            let _ = (fns.req_data)(
                h_sc,
                REQ_MAIN,
                DEF_MAIN,
                USER_OBJECT_ID,
                SIMCONNECT_PERIOD_SIM_FRAME,
                0,
                0,
                0,
                0,
            );
            let _ = (fns.req_data)(
                h_sc,
                REQ_PING,
                DEF_PING,
                USER_OBJECT_ID,
                SIMCONNECT_PERIOD_ONCE,
                0,
                0,
                0,
                0,
            );

            let mut main_seen = false;
            let mut last_main_rx = Instant::now();

            let mut paused_event_flag: bool = false;
            let mut paused_ex1_bits: u32 = 0;

            let mut title_resolved = false;
            let mut last_title_request_time = Instant::now() - Duration::from_secs(10); // force immediate request first tick

            loop {
                let mut p_recv: *mut SimRecv = std::ptr::null_mut();
                let mut cb: DWord = 0;
                let hr = (fns.next_dispatch)(h_sc, &mut p_recv, &mut cb);

                if hr < 0 {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }

                if !p_recv.is_null() && cb >= std::mem::size_of::<SimRecv>() as u32 {
                    match (*p_recv).dw_id {
                        SIMCONNECT_RECV_ID_OPEN => {}
                        SIMCONNECT_RECV_ID_QUIT => {
                            break;
                        }
                        SIMCONNECT_RECV_ID_EVENT => {
                            let ev = &*(p_recv as *const SimRecvEvent);

                            if ev.u_event_id == EVT_SIM_START {
                                in_flight = true;
                                *last_vars.lock() = None;
                                rumble_engine.reset();
                                effects.clear_all();

                                // When simulation starts, we trigger title retrieval.
                                // Instead of making a continuous subscription right here, we reset the resolution flag
                                // and let our smart polling mechanism fetch it via ONCE requests.
                                title_resolved = false;
                                last_title_request_time = Instant::now() - Duration::from_secs(10);
                                logs.push(
                                    "SimConnect: triggered TITLE retrieval on SimStart".to_string(),
                                );
                            } else if ev.u_event_id == EVT_AIRCRAFT_LOADED {
                                logs.push(
                                    "SimConnect: AircraftLoaded system event received!".to_string(),
                                );
                                title_resolved = false;
                                last_title_request_time = Instant::now() - Duration::from_secs(10); // force immediate request next tick
                            } else if ev.u_event_id == EVT_SIM_STOP {
                                in_flight = false;
                                let _ = tx_hid.send(HidCmd::SendIntensity {
                                    joystick: 0,
                                    throttle_left: 0,
                                    throttle_right: 0,
                                });
                                *last_vars.lock() = None;
                                effects.clear_all();
                            } else if ev.u_event_id == EVT_PAUSE_SYS {
                                paused_event_flag = ev.dw_data != 0;
                            } else if ev.u_event_id == EVT_PAUSE_EX1_SYS {
                                paused_ex1_bits = ev.dw_data;
                            }
                        }
                        SIMCONNECT_RECV_ID_SIMOBJECT_DATA => {
                            let sod = &*(p_recv as *const SimRecvSimObjectData);
                            let base_ptr = p_recv as *const u8;
                            let data_ptr = (&sod.dw_data as *const DWord) as *const u8;
                            let header_bytes =
                                (data_ptr as usize).saturating_sub(base_ptr as usize);
                            let payload_len = (cb as usize).saturating_sub(header_bytes);

                            if sod.dw_request_id == REQ_TITLE {
                                // Explicit debug logging of raw bytes
                                let raw_bytes =
                                    std::slice::from_raw_parts(data_ptr, payload_len.min(256));
                                logs.push(format!(
                                    "SimConnect: TITLE packet received in dispatch. payload_len={}, raw bytes (first 32): {:?}",
                                    payload_len,
                                    &raw_bytes[..raw_bytes.len().min(32)]
                                ));

                                if payload_len >= std::mem::size_of::<SimAircraftTitle>() {
                                    let title_struct = &*(data_ptr as *const SimAircraftTitle);
                                    let title = parse_aircraft_title(&title_struct.title);
                                    logs.push(format!(
                                        "SimConnect: TITLE parsed (req_id={}) -> {:?}",
                                        sod.dw_request_id, title
                                    ));
                                    if !title.is_empty() {
                                        let prev = std::mem::replace(
                                            &mut *aircraft_title.lock(),
                                            title.clone(),
                                        );
                                        title_resolved = true;
                                        if prev != title {
                                            crate::aircraft_profiles::apply_for_aircraft(
                                                &mut aircraft_profiles.lock(),
                                                &config,
                                                &mut profile_state.lock(),
                                                &title,
                                                &logs,
                                            );
                                        }
                                    } else {
                                        logs.push("SimConnect: received empty/null TITLE, will retry polling...".to_string());
                                        title_resolved = false;
                                    }
                                } else {
                                    logs.push(format!(
                                        "SimConnect: TITLE payload too short ({} bytes, expected >=256), ignoring",
                                        payload_len
                                    ));
                                }
                                continue;
                            }

                            if sod.dw_request_id == REQ_MAIN {
                                // Слот мог быть занят WT в момент Open() —
                                // пробуем повторно на каждом полученном пакете
                                // телеметрии, чтобы подхватить его, как только
                                // WT сам его освободит, без переоткрытия
                                // SimConnect-соединения.
                                if !owns_slot {
                                    owns_slot = if crate::settings::game_override()
                                        .vetoes(ActiveGame::Msfs)
                                    {
                                        false
                                    } else {
                                        game.try_claim(ActiveGame::Msfs)
                                    };
                                    if owns_slot {
                                        logs.push("SimConnect: claimed game slot".to_string());
                                        *status.lock() = SimStatus::Connected;
                                        *aircraft_title.lock() = String::new();
                                    }
                                }

                                main_seen = true;
                                last_main_rx = Instant::now();

                                if !in_flight {
                                    if owns_slot {
                                        *status.lock() = SimStatus::Connected;
                                        *last_vars.lock() = None;
                                        let _ = tx_hid.send(HidCmd::SendIntensity {
                                            joystick: 0,
                                            throttle_left: 0,
                                            throttle_right: 0,
                                        });
                                        effects.clear_all();
                                    }
                                    continue;
                                }

                                let count = sod.dw_define_count as usize;
                                if count == 0 {
                                    continue;
                                }

                                let want_f64 = payload_len >= count * 8;
                                let want_f32 = !want_f64 && payload_len >= count * 4;
                                if !want_f64 && !want_f32 {
                                    continue;
                                }

                                // Field layout: see ElemIdx in sim/elem_idx.rs — the
                                // buffer length and clamp below are derived from the
                                // same enum that drives DEFS registration above, so
                                // they can't drift out of sync with it again (this
                                // used to be a hardcoded count.min(53), which meant
                                // OVERSPEED WARNING and everything after it never got
                                // copied from live SimConnect data).
                                let mut elem = [0f64; ElemIdx::COUNT];
                                if want_f64 {
                                    let v = std::slice::from_raw_parts(
                                        data_ptr as *const f64,
                                        count.min(ElemIdx::COUNT),
                                    );
                                    for (i, &x) in v.iter().enumerate() {
                                        elem[i] = x;
                                    }
                                } else {
                                    let v = std::slice::from_raw_parts(
                                        data_ptr as *const f32,
                                        count.min(ElemIdx::COUNT),
                                    );
                                    for (i, &x) in v.iter().enumerate() {
                                        elem[i] = x as f64;
                                    }
                                }

                                let paused_from_events =
                                    paused_event_flag || (paused_ex1_bits != 0);
                                let cfg_now = config.get();
                                let title_snapshot = aircraft_title.lock().clone();
                                let fv = parse_main_elems(
                                    &elem,
                                    paused_from_events,
                                    cfg_now.ias_deadband_kn,
                                    &title_snapshot,
                                );

                                if owns_slot {
                                    *last_vars.lock() = Some(fv);
                                    *status.lock() = flight_status(&fv);
                                }

                                // War Thunder может владеть слотом одновременно
                                // с тем, что MSFS открыт (см. game_state::GameSlot) —
                                // оба конвейера взаимоисключающие по HID-каналу, так
                                // что MSFS rumble-движок пропускается, пока слот не
                                // наш, а моторы держатся на нуле вместо зависания на
                                // последнем значении.
                                if !owns_slot {
                                    effects.clear_all();
                                    let _ = tx_hid.send(HidCmd::SendIntensity {
                                        joystick: 0,
                                        throttle_left: 0,
                                        throttle_right: 0,
                                    });
                                } else {
                                    let out = rumble_engine.step(
                                        &fv,
                                        &cfg_now,
                                        config.current_rev(),
                                        hold.load(Ordering::Relaxed),
                                    );
                                    effects.apply_snapshot(&out.effects);
                                    let _ = tx_hid.send(HidCmd::SendIntensity {
                                        joystick: out.joystick_intensity,
                                        throttle_left: out.throttle_left_intensity,
                                        throttle_right: out.throttle_right_intensity,
                                    });
                                }
                            }
                        }
                        SIMCONNECT_RECV_ID_SYSTEM_STATE => {
                            // MobiFlight late-connect: the system state response
                            // delivers the aircraft's file path (or title) even
                            // when we connected AFTER the flight was already
                            // loaded. We log which channel delivers the data
                            // first for debugging purposes.
                            let ss = &*(p_recv as *const SimRecvSystemState);
                            let base_ptr = p_recv as *const u8;
                            let data_ptr = (&ss.dw_data as *const DWord) as *const u8;
                            let header_bytes =
                                (data_ptr as usize).saturating_sub(base_ptr as usize);
                            let payload_len = (cb as usize).saturating_sub(header_bytes);

                            logs.push(format!(
                                "SimConnect: SYSTEM_STATE received (req_id={}), payload_len={}",
                                ss.dw_request_id, payload_len
                            ));

                            if ss.dw_request_id == REQ_SYS_STATE && payload_len >= 256 {
                                let title_struct = &*(data_ptr as *const SimAircraftTitle);
                                let title = parse_aircraft_title(&title_struct.title);
                                logs.push(format!(
                                    "SimConnect: SYSTEM_STATE AircraftLoaded -> title={:?} (delivered BEFORE SIMOBJECT_DATA)",
                                    title
                                ));
                                if !title.is_empty() {
                                    let prev = std::mem::replace(
                                        &mut *aircraft_title.lock(),
                                        title.clone(),
                                    );
                                    title_resolved = true;
                                    if prev != title {
                                        crate::aircraft_profiles::apply_for_aircraft(
                                            &mut aircraft_profiles.lock(),
                                            &config,
                                            &mut profile_state.lock(),
                                            &title,
                                            &logs,
                                        );
                                    }
                                }
                            }
                        }
                        SIMCONNECT_RECV_ID_EXCEPTION => {
                            let ex = &*(p_recv as *const SimRecvException);
                            logs.push(format!(
                                "SimConnect: EXCEPTION code={} send_id={} index={}",
                                ex.dw_exception, ex.dw_send_id, ex.dw_index
                            ));
                        }
                        _ => {}
                    }
                } else {
                    thread::sleep(Duration::from_millis(10));
                }

                let timeout = if main_seen {
                    Duration::from_millis(2500)
                } else {
                    Duration::from_millis(800)
                };
                if last_main_rx.elapsed() >= timeout {
                    let _ = (fns.req_data)(
                        h_sc,
                        REQ_MAIN,
                        DEF_MAIN,
                        USER_OBJECT_ID,
                        SIMCONNECT_PERIOD_SIM_FRAME,
                        0,
                        0,
                        0,
                        0,
                    );
                    last_main_rx = Instant::now();
                }

                if !title_resolved && last_title_request_time.elapsed() >= Duration::from_secs(1) {
                    // MobiFlight late-connect: on retry, do NOT just spam
                    // request_data_on_simobject with the same ID. Instead,
                    // re-register the data definition first — this forces
                    // SimConnect to re-evaluate the TITLE field and actually
                    // deliver a SIMCONNECT_RECV_SIMOBJECT_DATA packet.
                    let title_name = std::ffi::CString::new("TITLE").unwrap();
                    let hr_redef = (fns.add_to_def)(
                        h_sc,
                        DEF_TITLE,
                        title_name.as_ptr(),
                        std::ptr::null(),
                        SIMCONNECT_DATATYPE_STRING256,
                        0.0,
                        0xFFFF_FFFF,
                    );
                    if hr_redef < 0 {
                        logs.push(format!(
                            "SimConnect: AddToDef TITLE (retry re-register) FAILED {}",
                            hr_hex(hr_redef)
                        ));
                    } else {
                        logs.push("SimConnect: AddToDef TITLE re-registered on retry".to_string());
                    }
                    let _ = (fns.req_data)(
                        h_sc,
                        REQ_TITLE,
                        DEF_TITLE,
                        USER_OBJECT_ID,
                        SIMCONNECT_PERIOD_ONCE,
                        0,
                        0,
                        0,
                        0,
                    );
                    last_title_request_time = Instant::now();
                    logs.push(
                        "SimConnect: TITLE retry request sent (ONCE, after re-add)".to_string(),
                    );
                }
            }

            let _ = (fns.close)(h_sc);
            if owns_slot {
                *status.lock() = SimStatus::Disconnected;
                *aircraft_title.lock() = String::new();
                *last_vars.lock() = None;
                let _ = tx_hid.send(HidCmd::SendIntensity {
                    joystick: 0,
                    throttle_left: 0,
                    throttle_right: 0,
                });
                game.release_if_owned(ActiveGame::Msfs);
            }
            thread::sleep(Duration::from_millis(600));
        }
    }
}
