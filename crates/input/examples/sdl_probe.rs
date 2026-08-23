//! Diagnostic: list SDL gamepads and check whether Ira's virtual pad
//! exposes motion sensors through the evdev pairing path, printing live
//! samples. Run it while `ira-input` is wrapping a game:
//!
//! ```sh
//! cargo run -p ira-input --example sdl_probe
//! ```
//!
//! It dlopens `libSDL3.so.0` (override with `SDL3_LIB`), so the same binary
//! can run inside a flatpak sandbox to test against the app's own SDL.

use std::ffi::{c_char, c_int, c_void, CStr, CString};

type SdlInit = unsafe extern "C" fn(u32) -> bool;
type SdlGetGamepads = unsafe extern "C" fn(*mut c_int) -> *mut u32;
type SdlOpenGamepad = unsafe extern "C" fn(u32) -> *mut c_void;
type SdlCloseGamepad = unsafe extern "C" fn(*mut c_void);
type SdlGetName = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type SdlHasSensor = unsafe extern "C" fn(*mut c_void, c_int) -> bool;
type SdlSetSensorEnabled = unsafe extern "C" fn(*mut c_void, c_int, bool) -> bool;
type SdlSensorDataWithTime =
    unsafe extern "C" fn(*mut c_void, c_int, *mut f32, *mut u64, c_int) -> bool;
type SdlSensorData =
    unsafe extern "C" fn(*mut c_void, c_int, *mut f32, c_int) -> bool;
type SdlPump = unsafe extern "C" fn();
type SdlUpdateSensors = unsafe extern "C" fn();
type SdlGetError = unsafe extern "C" fn() -> *const c_char;
#[repr(C)]
struct SdlVersion { major: i32, minor: i32, patch: i32 }
type SdlGetVersion = unsafe extern "C" fn(*mut SdlVersion);

const SDL_INIT_GAMEPAD: u32 = 0x0000_2000;
const SDL_INIT_EVENTS: u32 = 0x0000_4000;
const SDL_INIT_SENSOR: u32 = 0x0000_8000;
const SDL_SENSOR_ACCEL: c_int = 1;
const SDL_SENSOR_GYRO: c_int = 2;

struct Api {
    init: SdlInit,
    get_gamepads: SdlGetGamepads,
    open_gamepad: SdlOpenGamepad,
    close_gamepad: SdlCloseGamepad,
    get_name: SdlGetName,
    get_path: SdlGetName,
    has_sensor: SdlHasSensor,
    set_sensor_enabled: SdlSetSensorEnabled,
    sensor_data_with_time: Option<SdlSensorDataWithTime>,
    sensor_data: SdlSensorData,
    pump_events: SdlPump,
    update_sensors: SdlUpdateSensors,
    get_error: SdlGetError,
    get_version: SdlGetVersion,
}

fn main() {
    let lib = std::env::var("SDL3_LIB").unwrap_or_else(|_| "libSDL3.so.0".to_string());
    let name = CString::new(lib.clone()).unwrap();
    let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        eprintln!("probe: cannot dlopen {lib}");
        std::process::exit(2);
    }
    macro_rules! sym {
        ($n:literal, $t:ty) => {{
            let p = unsafe { libc::dlsym(handle, concat!($n, "\0").as_ptr().cast()) };
            if p.is_null() {
                eprintln!("probe: missing symbol {}", $n);
                std::process::exit(2);
            }
            unsafe { std::mem::transmute::<*mut c_void, $t>(p) }
        }};
    }
    let api = Api {
        init: sym!("SDL_Init", SdlInit),
        get_gamepads: sym!("SDL_GetGamepads", SdlGetGamepads),
        open_gamepad: sym!("SDL_OpenGamepad", SdlOpenGamepad),
        close_gamepad: sym!("SDL_CloseGamepad", SdlCloseGamepad),
        get_name: sym!("SDL_GetGamepadName", SdlGetName),
        get_path: sym!("SDL_GetGamepadPath", SdlGetName),
        has_sensor: sym!("SDL_GamepadHasSensor", SdlHasSensor),
        set_sensor_enabled: sym!("SDL_SetGamepadSensorEnabled", SdlSetSensorEnabled),
            sensor_data_with_time: {
            let name = CString::new("SDL_GetGamepadSensorDataWithTime").unwrap();
            let p = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
            if p.is_null() {
                None
            } else {
                Some(unsafe { std::mem::transmute::<*mut c_void, SdlSensorDataWithTime>(p) })
            }
        },
        sensor_data: sym!("SDL_GetGamepadSensorData", SdlSensorData),
        pump_events: sym!("SDL_PumpEvents", SdlPump),
        update_sensors: sym!("SDL_UpdateSensors", SdlUpdateSensors),
        get_error: sym!("SDL_GetError", SdlGetError),
        get_version: sym!("SDL_GetVersion", SdlGetVersion),
    };
    let mut version = SdlVersion { major: 0, minor: 0, patch: 0 };
    unsafe { (api.get_version)(&mut version) };
    println!("probe: SDL {}.{}.{}", version.major, version.minor, version.patch);
    if !unsafe {
        (api.init)(SDL_INIT_EVENTS | SDL_INIT_GAMEPAD | SDL_INIT_SENSOR)
    } {
        let err = unsafe { CStr::from_ptr((api.get_error)()) }
            .to_string_lossy()
            .into_owned();
        eprintln!("probe: SDL_Init failed: {err}");
        std::process::exit(2);
    }

    let mut count = 0;
    let ids = unsafe { (api.get_gamepads)(&mut count) };
    if ids.is_null() {
        eprintln!("probe: no gamepads");
        std::process::exit(1);
    }
    let ids = unsafe { std::slice::from_raw_parts(ids, count as usize) }.to_vec();

    let mut target: Option<u32> = None;
    println!("probe: {} gamepad(s)", ids.len());
    for id in ids {
        let gamepad = unsafe { (api.open_gamepad)(id) };
        if gamepad.is_null() {
            continue;
        }
        let name = unsafe { CStr::from_ptr((api.get_name)(gamepad)) }
            .to_string_lossy()
            .into_owned();
        let path = unsafe {
            let p = (api.get_path)(gamepad);
            if p.is_null() {
                "-".to_string()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        let has_gyro = unsafe { (api.has_sensor)(gamepad, SDL_SENSOR_GYRO) };
        let has_accel = unsafe { (api.has_sensor)(gamepad, SDL_SENSOR_ACCEL) };
        println!("  [{id}] {name} ({path}) gyro={has_gyro} accel={has_accel}");
        if name.contains("Ira Virtual") && target.is_none() {
            target = Some(id);
        }
        unsafe { (api.close_gamepad)(gamepad) };
    }

    let Some(id) = target else {
        println!("probe: FAIL - no Ira virtual gamepad found");
        std::process::exit(1);
    };
    let gamepad = unsafe { (api.open_gamepad)(id) };
    if gamepad.is_null() {
        println!("probe: FAIL - could not reopen virtual gamepad");
        std::process::exit(1);
    }
    let mut moving = 0usize;
    for sensor in [SDL_SENSOR_GYRO, SDL_SENSOR_ACCEL] {
        if !unsafe { (api.has_sensor)(gamepad, sensor) } {
            println!("probe: FAIL - virtual pad lacks sensor {sensor}");
            continue;
        }
        if !unsafe { (api.set_sensor_enabled)(gamepad, sensor, true) } {
            let err = unsafe { CStr::from_ptr((api.get_error)()) }
                .to_string_lossy()
                .into_owned();
            println!("probe: FAIL - enabling sensor {sensor}: {err}");
            continue;
        }
        let mut values = [0.0f32; 3];
        let mut timestamp = 0u64;
        let mut nonzero = 0usize;
        let mut printed = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            unsafe {
                (api.pump_events)();
                (api.update_sensors)();
                let ok = match api.sensor_data_with_time {
                    Some(f) => f(gamepad, sensor, values.as_mut_ptr(), &mut timestamp, 3),
                    None => (api.sensor_data)(gamepad, sensor, values.as_mut_ptr(), 3),
                };
                if ok && values.iter().any(|v| v.abs() > 0.001)
                {
                    nonzero += 1;
                    if printed < 3 {
                        println!(
                            "probe: sensor {sensor} sample {values:?} @ {timestamp}us"
                        );
                        printed += 1;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if sensor == SDL_SENSOR_ACCEL {
            // Gravity always reads ~1g on one axis while held.
            moving += nonzero.min(1);
        } else {
            moving += nonzero.min(1);
        }
        println!("probe: sensor {sensor} -> {nonzero} nonzero reads over 3s");
    }
    unsafe { (api.close_gamepad)(gamepad) };
    if moving == 2 {
        println!("probe: PASS - both sensors stream from the virtual pad");
    } else {
        println!("probe: FAIL - sensors missing or silent (moving={moving})");
        std::process::exit(1);
    }
}
