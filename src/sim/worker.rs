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
use crate::sim::parse::{flight_status, parse_main_elems};
use crate::{ConfigShared, EffectsShared, FlightVars, HidCmd, LogBuffer, SimStatus};

type DWord = u32;
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

const EMBED_SIMCONNECT_BYTES: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/lib/SimConnect.dll"));

fn try_load_embedded_simconnect(logs: &LogBuffer) -> Result<Library> {
    let mut dst = std::env::temp_dir();
    dst.push("aurora-simconnect-embedded-64.dll");

    logs.push(format!(
        "SimConnect: writing embedded DLL to {}",
        dst.display()
    ));
    std::fs::write(&dst, EMBED_SIMCONNECT_BYTES)
        .with_context(|| format!("write {}", dst.display()))?;

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

fn load_simconnect(logs: &LogBuffer) -> Result<SimConnectFns> {
    logs.push("SimConnect: trying normal load (EXE dir / PATH)...");
    match unsafe { Library::new("SimConnect.dll") } {
        Ok(lib) => {
            logs.push("SimConnect: loaded via normal search");
            return bind_simconnect(lib);
        }
        Err(e) => {
            logs.push(format!("SimConnect: normal search failed: {e}"));
        }
    }

    let lib = try_load_embedded_simconnect(logs)
        .context("embedded SimConnect fallback was unavailable or failed to load")?;
    bind_simconnect(lib)
}

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
) {
    logs.push("SimConnect: worker started");

    let fns = match load_simconnect(&logs) {
        Ok(f) => {
            logs.push("SimConnect: loaded (normal search or embedded fallback)");
            f
        }
        Err(e) => {
            logs.push(format!("SimConnect: {}", e));
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
            *status.lock() = SimStatus::Connected;
            *aircraft_title.lock() = String::new();

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

            let defs = [
                ("AIRSPEED INDICATED", "Knots"),
                ("SIM ON GROUND", "Bool"),
                ("PLANE BANK DEGREES", "Degrees"),
                ("TRAILING EDGE FLAPS LEFT PERCENT", "Percent"),
                ("TRAILING EDGE FLAPS RIGHT PERCENT", "Percent"),
                ("FLAPS HANDLE INDEX", "Number"),
                ("GEAR HANDLE POSITION", "Bool"),
                ("STALL WARNING", "Bool"),
                ("ABSOLUTE TIME", "Seconds"),
                ("GROUND VELOCITY", "Knots"), // Строку "PAUSED" удалили. Теперь спойлеры железно сидят на 11-м месте (индекс 10)
                ("SPOILERS HANDLE POSITION", "Percent"),
                ("GEAR ANIMATION POSITION:0", "Percent"),
                ("GEAR ANIMATION POSITION:1", "Percent"),
                ("GEAR ANIMATION POSITION:2", "Percent"),
                // Телеметрия запуска двигателей (Engine Spool-up & Ignition) —
                // индексы 14/15/16/17 в буфере elem[] ниже.
                ("TURB ENG N2:1", "Percent"),
                ("GENERAL ENG COMBUSTION:1", "Bool"),
                ("TURB ENG N2:2", "Percent"),
                ("GENERAL ENG COMBUSTION:2", "Bool"),
                // Двигатели 3/4 для 4-моторных самолётов (RumbleConfig::four_engine_mode) —
                // индексы 18/19/20/21 в буфере elem[] ниже.
                ("TURB ENG N2:3", "Percent"),
                ("GENERAL ENG COMBUSTION:3", "Bool"),
                ("TURB ENG N2:4", "Percent"),
                ("GENERAL ENG COMBUSTION:4", "Bool"),
                // Универсальная поддержка поршневых двигателей: GENERAL ENG PCT MAX RPM
                // работает и на турбинах (примерно повторяет N2), и на поршневых
                // (даёт % от макс. оборотов, в отличие от TURB ENG N2, которое на
                // поршневых обычно равно 0). Индексы 22/23/24/25 в буфере elem[] ниже.
                ("GENERAL ENG PCT MAX RPM:1", "Percent"),
                ("GENERAL ENG PCT MAX RPM:2", "Percent"),
                ("GENERAL ENG PCT MAX RPM:3", "Percent"),
                ("GENERAL ENG PCT MAX RPM:4", "Percent"),
                // Универсальная модель запуска (Starter + Combustion): работает
                // одинаково на поршневых и турбинных двигателях. Индексы
                // 26/27/28/29 в буфере elem[] ниже.
                ("GENERAL ENG STARTER:1", "Bool"),
                ("GENERAL ENG STARTER:2", "Bool"),
                ("GENERAL ENG STARTER:3", "Bool"),
                ("GENERAL ENG STARTER:4", "Bool"),
                // Поршневые двигатели (Piston Engine Telemetry) — сырые обороты
                // коленвала и воздушного винта, для UI-инспектора телеметрии.
                // Индексы 30/31/32/33 в буфере elem[] ниже.
                ("GENERAL ENG RPM:1", "Rpm"),
                ("GENERAL ENG RPM:2", "Rpm"),
                ("GENERAL ENG RPM:3", "Rpm"),
                ("GENERAL ENG RPM:4", "Rpm"),
                // Обороты воздушного винта. Индексы 34/35/36/37 в буфере elem[] ниже.
                ("PROP RPM:1", "Rpm"),
                ("PROP RPM:2", "Rpm"),
                ("PROP RPM:3", "Rpm"),
                ("PROP RPM:4", "Rpm"),
                // AIRSPEED BARBER POLE — динамическая "красная черта" (Vmo/Mmo,
                // сим сам двигает её вниз при наборе высоты), порог срабатывания
                // эффекта Overspeed, заменяет ручной слайдер в UI.
                // Индекс 38 в буфере elem[] ниже.
                ("AIRSPEED BARBER POLE", "Knots"),
                // Предкрылки (Slats) — пока только для отображения в UI
                // ("Live Aircraft Data"), в логику эффектов не задействованы.
                // Индексы 39/40 в буфере elem[] ниже.
                ("LEADING EDGE FLAPS LEFT PERCENT", "Percent"),
                ("LEADING EDGE FLAPS RIGHT PERCENT", "Percent"),
                // Раздельная позиция спойлеров по крыльям — нужна, чтобы отличать
                // честный симметричный выпуск спидбрейков от асимметричного подъёма
                // панелей при крене (roll spoilers/спойлероны, напр. TFDI MD-11,
                // где SPOILERS HANDLE POSITION некорректно следует за креном).
                // Индексы 41/42 в буфере elem[] ниже.
                ("SPOILERS LEFT POSITION", "Percent"),
                ("SPOILERS RIGHT POSITION", "Percent"),
                // TFDI MD-11: собственные L-vars по каждой из 5 секций спойлеров
                // на крыло (SPOILERS LEFT/RIGHT POSITION выше на этом борте тоже
                // не всегда надёжны). Используются ТОЛЬКО как дополнительная
                // проверка симметрии (см. rumble.rs) — на других самолётах эти
                // L-vars не определены, SimConnect отдаёт по ним 0.0, и проверка
                // самонейтрализуется (0 против 0 — симметрично). Индексы 43-52.
                ("L:MD11_EXT_L_SPOILER_1", "Number"),
                ("L:MD11_EXT_L_SPOILER_2", "Number"),
                ("L:MD11_EXT_L_SPOILER_3", "Number"),
                ("L:MD11_EXT_L_SPOILER_4", "Number"),
                ("L:MD11_EXT_L_SPOILER_5", "Number"),
                ("L:MD11_EXT_R_SPOILER_1", "Number"),
                ("L:MD11_EXT_R_SPOILER_2", "Number"),
                ("L:MD11_EXT_R_SPOILER_3", "Number"),
                ("L:MD11_EXT_R_SPOILER_4", "Number"),
                ("L:MD11_EXT_R_SPOILER_5", "Number"),
                // OVERSPEED WARNING — булев флаг сима "клацера" (overspeed
                // clacker), для сравнения с нашим порогом AIRSPEED BARBER POLE
                // в телеметрии (пока только для отображения в UI). Индекс 53.
                ("OVERSPEED WARNING", "Bool"),
                // Learjet 35A (Flysimware, FSW_L35A): L:XMLSND75 — тот же
                // L-var, к которому в sound.xml аддона привязан звук
                // "overspeed / mach trim" (аварийный клаксон превышения
                // Vmo/Mmo). Экспериментальный альтернативный триггер эффекта
                // Overspeed для этого борта (см. profiles.rs/rumble.rs) — на
                // остальных самолётах L-var не определён, SimConnect отдаёт
                // 0.0 (самонейтрализуется, тот же паттерн, что у
                // L:MD11_EXT_*_SPOILER_* выше). Индекс 54.
                ("L:XMLSND75", "Bool"),
                // Fenix A320 (FenixSim): L:I_PFD_VMAX — этот аддон не держит
                // AIRSPEED BARBER POLE синхронной с реальным PFD (см. отчёты
                // пользователей), зато сам выставляет свой предел overspeed в
                // этот L-var. Используется вместо AIRSPEED BARBER POLE только
                // когда aircraft title содержит "Fenix" (см. sim/parse.rs) —
                // на остальных бортах L-var не определён, SimConnect отдаёт
                // 0.0 (самонейтрализуется, тот же паттерн, что у L:XMLSND75
                // выше). Индекс 59.
                ("L:I_PFD_VMAX", "Knots"),
                // PMDG 737 (NG3, MSFS): L:EngineStart1b/2b_Ext — true, пока стартер
                // крутит двигатель до воспламенения (взято из sound.xml самого PMDG).
                // Подтверждено тестом в симе, что это надёжный сигнал — используется в
                // rumble.rs как маркер воспламенения (момент, когда real TURB ENG N2
                // впервые > 0) и для более раннего распознавания борта как джета; сама
                // N2-кривая раскрутки идёт напрямую по real TURB ENG N2 (см. rumble.rs —
                // реальный захват телеметрии показал, что N2 у PMDG растёт плавно с
                // момента включения стартера, а не держится на 0 до воспламенения).
                // L:EngineStart1c/2c_Ext (отдельный "маркер воспламенения") ПРОВЕРЕН и
                // ОТКЛОНЁН — не сработал корректно в реальном тесте, поэтому вместо
                // него как маркер воспламенения используется момент, когда сам
                // TURB ENG N2 становится > 0 (см. rumble.rs). На остальных самолётах
                // L:EngineStart1b/2b_Ext не определены, SimConnect отдаёт false
                // (самонейтрализуется). Индексы 55/56.
                ("L:EngineStart1b_Ext", "Bool"),
                ("L:EngineStart2b_Ext", "Bool"),
                // GENERAL ENG STARTER ACTIVE:1/2 — пока только для телеметрии
                // (сравнение с L:EngineStart1b/2b_Ext). Индексы 57/58.
                ("GENERAL ENG STARTER ACTIVE:1", "Bool"),
                ("GENERAL ENG STARTER ACTIVE:2", "Bool"),
            ];
            for (name, unit) in defs {
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
                                main_seen = true;
                                last_main_rx = Instant::now();

                                if !in_flight {
                                    *status.lock() = SimStatus::Connected;
                                    *last_vars.lock() = None;
                                    let _ = tx_hid.send(HidCmd::SendIntensity {
                                        joystick: 0,
                                        throttle_left: 0,
                                        throttle_right: 0,
                                    });
                                    effects.clear_all();
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

                                // 26 элементов: SPOILERS HANDLE POSITION (10),
                                // GEAR ANIMATION POSITION:0/1/2 (11/12/13) и
                                // телеметрия запуска двигателей (14/15/16/17):
                                // TURB ENG N2:1, GENERAL ENG COMBUSTION:1,
                                // TURB ENG N2:2, GENERAL ENG COMBUSTION:2,
                                // а также двигатели 3/4 для 4-моторных самолётов
                                // (18/19/20/21): TURB ENG N2:3, GENERAL ENG COMBUSTION:3,
                                // TURB ENG N2:4, GENERAL ENG COMBUSTION:4, и наконец
                                // GENERAL ENG PCT MAX RPM:1/2/3/4 (22/23/24/25) для
                                // универсальной поддержки поршневых двигателей,
                                // GENERAL ENG STARTER:1/2/3/4 (26/27/28/29) для
                                // универсальной модели запуска (Starter + Combustion), и
                                // наконец сырая телеметрия поршневых двигателей:
                                // GENERAL ENG RPM:1/2/3/4 (30/31/32/33),
                                // PROP RPM:1/2/3/4 (34/35/36/37) и, наконец,
                                // AIRSPEED BARBER POLE (38) — динамический порог
                                // срабатывания эффекта Overspeed. LEADING EDGE
                                // FLAPS LEFT/RIGHT PERCENT (39/40) — предкрылки
                                // (Slats), пока только для UI. SPOILERS LEFT/RIGHT
                                // POSITION (41/42) — раздельная позиция спойлеров
                                // по крыльям для фильтра симметрии (см. parse.rs).
                                // L:MD11_EXT_L/R_SPOILER_1..5 (43-52) — TFDI MD-11,
                                // дополнительная проверка симметрии по секциям.
                                // OVERSPEED WARNING (53) — булев флаг "клацера" сима,
                                // для сравнения в телеметрии с нашим порогом.
                                // L:XMLSND75 (54) — клаксон "overspeed / mach trim"
                                // на Learjet 35A (Flysimware), см. profiles.rs.
                                // PMDG 737 (NG3), см. defs выше: L:EngineStart1b/2b_Ext
                                // (55/56) — используется в rumble.rs для pre-spool
                                // разгона; GENERAL ENG STARTER ACTIVE:1/2 (57/58) —
                                // пока только для телеметрии. L:I_PFD_VMAX (59) —
                                // Fenix A320, альтернативный источник порога
                                // Overspeed вместо AIRSPEED BARBER POLE (см.
                                // sim/parse.rs).
                                //
                                // ВНИМАНИЕ: длина elem[] и clamp count.min(..) ниже
                                // должны покрывать ВСЕ зарегистрированные индексы
                                // (0..=59, т.е. 60 элементов) — раньше здесь стоял
                                // count.min(53), из-за чего индекс 53 (OVERSPEED
                                // WARNING) никогда не копировался из живых данных
                                // SimConnect и оставался равен 0.0/false.
                                let mut elem = [0f64; 60];
                                if want_f64 {
                                    let v = std::slice::from_raw_parts(
                                        data_ptr as *const f64,
                                        count.min(60),
                                    );
                                    for (i, &x) in v.iter().enumerate() {
                                        elem[i] = x;
                                    }
                                } else {
                                    let v = std::slice::from_raw_parts(
                                        data_ptr as *const f32,
                                        count.min(60),
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

                                *last_vars.lock() = Some(fv);
                                *status.lock() = flight_status(&fv);

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
            *status.lock() = SimStatus::Disconnected;
            *aircraft_title.lock() = String::new();
            *last_vars.lock() = None;
            let _ = tx_hid.send(HidCmd::SendIntensity {
                joystick: 0,
                throttle_left: 0,
                throttle_right: 0,
            });
            thread::sleep(Duration::from_millis(600));
        }
    }
}
