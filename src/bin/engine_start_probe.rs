// Standalone SimConnect probe: dumps raw engine-start telemetry (PMDG 737 NG3
// focus) to CSV at sim-frame rate, so real spool-up/ignition curves can be
// used to tune the engine-start haptic effect. Not wired into the main app —
// run it directly (`cargo run --bin engine_start_probe`) while MSFS is open
// and perform a cold-and-dark engine start.
//
// Columns: t_s,eng{1,2}_n1,eng{1,2}_n2,eng{1,2}_combustion,eng{1,2}_starter,
// eng{1,2}_starter_active,eng{1,2}_pmdg_start_ext

use std::ffi::{c_char, c_void, CString};
use std::fs::File;
use std::io::Write as _;
use std::thread;
use std::time::{Duration, Instant};

type DWord = u32;
type HResult = i32;
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
struct SimRecvException {
    base: SimRecv,
    dw_exception: DWord,
    dw_send_id: DWord,
    dw_index: DWord,
}

const SIMCONNECT_RECV_ID_OPEN: DWord = 2;
const SIMCONNECT_RECV_ID_QUIT: DWord = 3;
const SIMCONNECT_RECV_ID_EXCEPTION: DWord = 5;
const SIMCONNECT_RECV_ID_SIMOBJECT_DATA: DWord = 8;

const SIMCONNECT_PERIOD_SIM_FRAME: DWord = 3;
const SIMCONNECT_DATATYPE_FLOAT64: DWord = 4;
const USER_OBJECT_ID: DWord = 0;

const DEF_ENGINES: DWord = 9001;
const REQ_ENGINES: DWord = 9101;

type PfnOpen =
    unsafe extern "system" fn(*mut Handle, *const c_char, HWnd, DWord, Handle, DWord) -> HResult;
type PfnClose = unsafe extern "system" fn(Handle) -> HResult;
type PfnAddToDataDefinition = unsafe extern "system" fn(
    Handle,
    DWord,
    *const c_char,
    *const c_char,
    DWord,
    f32,
    DWord,
) -> HResult;
type PfnRequestDataOnSimObject = unsafe extern "system" fn(
    Handle,
    DWord,
    DWord,
    DWord,
    DWord,
    DWord,
    DWord,
    DWord,
    DWord,
) -> HResult;
type PfnGetNextDispatch =
    unsafe extern "system" fn(Handle, *mut *mut SimRecv, *mut DWord) -> HResult;

const EMBED_SIMCONNECT_BYTES: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/lib/SimConnect.dll"));

struct Fns {
    _lib: libloading::Library,
    open: PfnOpen,
    close: PfnClose,
    add_to_def: PfnAddToDataDefinition,
    req_data: PfnRequestDataOnSimObject,
    next_dispatch: PfnGetNextDispatch,
}

fn load_simconnect() -> Fns {
    let lib = match unsafe { libloading::Library::new("SimConnect.dll") } {
        Ok(l) => {
            println!("SimConnect.dll loaded via normal search (EXE dir / PATH)");
            l
        }
        Err(e) => {
            println!("normal search failed ({e}), falling back to embedded DLL");
            let mut dst = std::env::temp_dir();
            dst.push("aurora-engine-probe-simconnect-64.dll");
            std::fs::write(&dst, EMBED_SIMCONNECT_BYTES).expect("write embedded SimConnect.dll");
            unsafe { libloading::Library::new(&dst) }.expect("load embedded SimConnect.dll")
        }
    };
    unsafe {
        let open: PfnOpen = *lib.get(b"SimConnect_Open\0").expect("SimConnect_Open");
        let close: PfnClose = *lib.get(b"SimConnect_Close\0").expect("SimConnect_Close");
        let add_to_def: PfnAddToDataDefinition = *lib
            .get(b"SimConnect_AddToDataDefinition\0")
            .expect("SimConnect_AddToDataDefinition");
        let req_data: PfnRequestDataOnSimObject = *lib
            .get(b"SimConnect_RequestDataOnSimObject\0")
            .expect("SimConnect_RequestDataOnSimObject");
        let next_dispatch: PfnGetNextDispatch = *lib
            .get(b"SimConnect_GetNextDispatch\0")
            .expect("SimConnect_GetNextDispatch");
        Fns {
            _lib: lib,
            open,
            close,
            add_to_def,
            req_data,
            next_dispatch,
        }
    }
}

fn main() {
    let duration_s: f64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(150.0);

    let mut out_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir);
    out_path.push("engine_start_telemetry.csv");
    let out_path = std::env::args().nth(2).map(Into::into).unwrap_or(out_path);

    println!("=== engine_start_probe ===");
    println!("Duration: {duration_s:.0}s, output: {}", out_path.display());
    println!("Fields: t_s, eng{{1,2}}_n1, eng{{1,2}}_n2, eng{{1,2}}_combustion,");
    println!("        eng{{1,2}}_starter, eng{{1,2}}_starter_active, eng{{1,2}}_pmdg_start_ext");
    println!("Waiting for SimConnect... start MSFS + PMDG 737 flight if not already loaded.");

    let fns = load_simconnect();

    let mut h_sc: Handle = std::ptr::null_mut();
    let name = CString::new("EngineStartProbe").unwrap();
    let hr = unsafe {
        (fns.open)(
            &mut h_sc,
            name.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0xFFFF_FFFF,
        )
    };
    if hr < 0 || h_sc.is_null() {
        eprintln!("SimConnect_Open failed: 0x{:08X}", hr as u32);
        std::process::exit(1);
    }
    println!("Connected to SimConnect.");

    let add = |name_s: &str, unit_s: &str| -> HResult {
        let n = CString::new(name_s).unwrap();
        let u = CString::new(unit_s).unwrap();
        unsafe {
            (fns.add_to_def)(
                h_sc,
                DEF_ENGINES,
                n.as_ptr(),
                u.as_ptr(),
                SIMCONNECT_DATATYPE_FLOAT64,
                0.0,
                0xFFFF_FFFF,
            )
        }
    };

    let vars = [
        ("TURB ENG N1:1", "Percent"),
        ("TURB ENG N2:1", "Percent"),
        ("GENERAL ENG COMBUSTION:1", "Bool"),
        ("GENERAL ENG STARTER:1", "Bool"),
        ("GENERAL ENG STARTER ACTIVE:1", "Bool"),
        ("L:EngineStart1b_Ext", "Bool"),
        ("TURB ENG N1:2", "Percent"),
        ("TURB ENG N2:2", "Percent"),
        ("GENERAL ENG COMBUSTION:2", "Bool"),
        ("GENERAL ENG STARTER:2", "Bool"),
        ("GENERAL ENG STARTER ACTIVE:2", "Bool"),
        ("L:EngineStart2b_Ext", "Bool"),
    ];
    for (n, u) in vars {
        let hr = add(n, u);
        if hr < 0 {
            eprintln!("AddToDataDefinition {n:?} [{u}] FAILED 0x{:08X}", hr as u32);
        }
    }

    let hr = unsafe {
        (fns.req_data)(
            h_sc,
            REQ_ENGINES,
            DEF_ENGINES,
            USER_OBJECT_ID,
            SIMCONNECT_PERIOD_SIM_FRAME,
            0,
            0,
            0,
            0,
        )
    };
    if hr < 0 {
        eprintln!("RequestDataOnSimObject FAILED 0x{:08X}", hr as u32);
    }

    let mut file = File::create(&out_path).expect("create output CSV");
    writeln!(
        file,
        "t_s,eng1_n1,eng1_n2,eng1_combustion,eng1_starter,eng1_starter_active,eng1_pmdg_start_ext,\
eng2_n1,eng2_n2,eng2_combustion,eng2_starter,eng2_starter_active,eng2_pmdg_start_ext"
    )
    .unwrap();

    let start = Instant::now();
    let mut last_print = Instant::now();
    let mut rows = 0u64;

    loop {
        if start.elapsed().as_secs_f64() >= duration_s {
            println!("Duration elapsed, stopping. Rows written: {rows}");
            break;
        }

        let mut p_recv: *mut SimRecv = std::ptr::null_mut();
        let mut cb: DWord = 0;
        let hr = unsafe { (fns.next_dispatch)(h_sc, &mut p_recv, &mut cb) };
        if hr < 0 || p_recv.is_null() || (cb as usize) < std::mem::size_of::<SimRecv>() {
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        unsafe {
            match (*p_recv).dw_id {
                SIMCONNECT_RECV_ID_OPEN => {}
                SIMCONNECT_RECV_ID_QUIT => {
                    println!("Sim quit, stopping.");
                    break;
                }
                SIMCONNECT_RECV_ID_EXCEPTION => {
                    let ex = &*(p_recv as *const SimRecvException);
                    eprintln!(
                        "SimConnect EXCEPTION code={} send_id={} index={}",
                        ex.dw_exception, ex.dw_send_id, ex.dw_index
                    );
                }
                SIMCONNECT_RECV_ID_SIMOBJECT_DATA => {
                    let sod = &*(p_recv as *const SimRecvSimObjectData);
                    if sod.dw_request_id != REQ_ENGINES {
                        continue;
                    }
                    let base_ptr = p_recv as *const u8;
                    let data_ptr = (&sod.dw_data as *const DWord) as *const u8;
                    let header_bytes = (data_ptr as usize).saturating_sub(base_ptr as usize);
                    let payload_len = (cb as usize).saturating_sub(header_bytes);
                    let count = sod.dw_define_count as usize;
                    if count == 0 || payload_len < count * 8 {
                        continue;
                    }
                    let v = std::slice::from_raw_parts(data_ptr as *const f64, count.min(12));
                    let mut e = [0f64; 12];
                    for (i, &x) in v.iter().enumerate() {
                        e[i] = x;
                    }

                    let t = start.elapsed().as_secs_f64();
                    writeln!(
                        file,
                        "{t:.3},{},{},{},{},{},{},{},{},{},{},{},{}",
                        e[0], e[1], e[2], e[3], e[4], e[5], e[6], e[7], e[8], e[9], e[10], e[11]
                    )
                    .unwrap();
                    file.flush().unwrap();
                    rows += 1;

                    if last_print.elapsed() >= Duration::from_millis(500) {
                        println!(
                            "t={t:6.1}s | E1 N1={:5.1} N2={:5.1} comb={:.0} start={:.0}/{:.0} pmdg={:.0} | E2 N1={:5.1} N2={:5.1} comb={:.0} start={:.0}/{:.0} pmdg={:.0}",
                            e[0], e[1], e[2], e[3], e[4], e[5],
                            e[6], e[7], e[8], e[9], e[10], e[11]
                        );
                        last_print = Instant::now();
                    }
                }
                _ => {}
            }
        }
    }

    unsafe {
        (fns.close)(h_sc);
    }
    println!("Done. CSV written to {}", out_path.display());
}
