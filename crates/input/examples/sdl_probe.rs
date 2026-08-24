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
type SdlSensorData = unsafe extern "C" fn(*mut c_void, c_int, *mut f32, c_int) -> bool;
type SdlPump = unsafe extern "C" fn();
type SdlUpdateSensors = unsafe extern "C" fn();
type SdlGetType = unsafe extern "C" fn(*mut c_void) -> c_int;
type SdlGetError = unsafe extern "C" fn() -> *const c_char;
#[repr(C)]
struct SdlVersion {
    major: i32,
    minor: i32,
    patch: i32,
}
type SdlGetVersion = unsafe extern "C" fn(*mut SdlVersion);
type SdlGetJsAxis = unsafe extern "C" fn(*mut c_void, c_int) -> i16;
type SdlGetJsName = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type SdlGetNumJsAxes = unsafe extern "C" fn(*mut c_void) -> c_int;

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
    get_type: SdlGetType,
    get_path: SdlGetName,
    has_sensor: SdlHasSensor,
    set_sensor_enabled: SdlSetSensorEnabled,
    sensor_data_with_time: Option<SdlSensorDataWithTime>,
    sensor_data: SdlSensorData,
    pump_events: SdlPump,
    update_sensors: SdlUpdateSensors,
    get_error: SdlGetError,
    get_version: SdlGetVersion,
    get_js_axis: SdlGetJsAxis,
    get_js_name: SdlGetJsName,
    get_num_js_axes: SdlGetNumJsAxes,
    get_joysticks: SdlGetGamepads,
    open_joystick: SdlOpenGamepad,
    close_joystick: SdlCloseGamepad,
}

/// Guided pose capture: averages accel/gyro over three timed windows so
/// the resulting vectors reveal the device's sensor frame without guessing.
fn capture_poses(api: &Api, gamepad: *mut c_void) {
    const WINDOW_MS: u64 = 4000;
    // SDL returns zeros for disabled sensors; the streaming path below
    // enables them, so the pose windows must do the same.
    for sensor in [SDL_SENSOR_GYRO, SDL_SENSOR_ACCEL] {
        if unsafe { (api.has_sensor)(gamepad, sensor) }
            && !unsafe { (api.set_sensor_enabled)(gamepad, sensor, true) }
        {
            println!("probe: FAIL - could not enable sensor {sensor}");
            return;
        }
    }
    let phases = [
        ("hold flat, buttons up, grips level", 0),
        ("tilt forward 90deg (triggers toward the desk)", 1),
        ("roll left 90deg (left grip down, flat edge up)", 2),
    ];
    for (title, index) in phases {
        println!("\nNEXT POSE ({index}/2): {title}");
        for count in (1..=3).rev() {
            println!("  starting in {count}...");
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        println!("  HOLD IT...");
        let mut gyro = [0.0f32; 3];
        let mut accel = [0.0f32; 3];
        let mut samples = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(WINDOW_MS);
        while std::time::Instant::now() < deadline {
            unsafe {
                (api.pump_events)();
                (api.update_sensors)();
                let mut g = [0.0f32; 3];
                let mut a = [0.0f32; 3];
                let _ = (api.sensor_data)(gamepad, SDL_SENSOR_GYRO, g.as_mut_ptr(), 3);
                let _ = (api.sensor_data)(gamepad, SDL_SENSOR_ACCEL, a.as_mut_ptr(), 3);
                for i in 0..3 {
                    gyro[i] += g[i];
                    accel[i] += a[i];
                }
            }
            samples += 1;
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        println!(
            "  accel avg {:?}  gyro avg {:?}  ({} samples)",
            [
                accel[0] / samples as f32,
                accel[1] / samples as f32,
                accel[2] / samples as f32
            ],
            [
                gyro[0] / samples as f32,
                gyro[1] / samples as f32,
                gyro[2] / samples as f32
            ],
            samples
        );
    }
    println!("\ncapture done - paste all three lines back");
    for sensor in [SDL_SENSOR_GYRO, SDL_SENSOR_ACCEL] {
        unsafe { (api.set_sensor_enabled)(gamepad, sensor, false) };
    }
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
        get_type: sym!("SDL_GetGamepadType", SdlGetType),
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
        get_js_axis: sym!("SDL_GetJoystickAxis", SdlGetJsAxis),
        get_js_name: sym!("SDL_GetJoystickName", SdlGetJsName),
        get_num_js_axes: sym!("SDL_GetNumJoystickAxes", SdlGetNumJsAxes),
        get_joysticks: sym!("SDL_GetJoysticks", SdlGetGamepads),
        open_joystick: sym!("SDL_OpenJoystick", SdlOpenGamepad),
        close_joystick: sym!("SDL_CloseJoystick", SdlCloseGamepad),
    };
    let mut version = SdlVersion {
        major: 0,
        minor: 0,
        patch: 0,
    };
    unsafe { (api.get_version)(&mut version) };
    println!(
        "probe: SDL {}.{}.{}",
        version.major, version.minor, version.patch
    );
    if !unsafe { (api.init)(SDL_INIT_EVENTS | SDL_INIT_GAMEPAD | SDL_INIT_SENSOR) } {
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
    let mut gyro_fallback: Option<u32> = None;
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
        let gamepad_type = unsafe { (api.get_type)(gamepad) };
        println!("  [{id}] {name} ({path}) type={gamepad_type} gyro={has_gyro} accel={has_accel}");
        let wanted = std::env::var("PROBE_TARGET").unwrap_or_default();
        let matches = if wanted.is_empty() {
            name.contains("Ira Virtual") || name.contains("Nintendo Switch Pro")
        } else {
            name.contains(&wanted)
        };
        if matches && target.is_none() {
            target = Some(id);
        }
        if has_gyro && gyro_fallback.is_none() {
            gyro_fallback = Some(id);
        }
        unsafe { (api.close_gamepad)(gamepad) };
    }
    if target.is_none() {
        target = gyro_fallback;
    }

    if std::env::var("AXES").is_ok() {
        // Enumerate raw joysticks: the driver's IMU twin node can appear as
        // its own joystick or bleed into the pad's handle; streaming every
        // axis of every joystick shows where each value really comes from.
        let mut count = 0;
        let ids = unsafe { (api.get_joysticks)(&mut count) };
        if ids.is_null() {
            println!("probe: no joysticks");
            return;
        }
        let ids = unsafe { std::slice::from_raw_parts(ids, count as usize) }.to_vec();
        let mut joysticks: Vec<(*mut c_void, usize)> = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            let js = unsafe { (api.open_joystick)(*id) };
            if js.is_null() {
                continue;
            }
            let name = unsafe { CStr::from_ptr((api.get_js_name)(js)) }
                .to_string_lossy()
                .into_owned();
            let axes = unsafe { (api.get_num_js_axes)(js) };
            println!("js{index} \"{name}\" axes={axes}");
            joysticks.push((js, index));
        }
        let mut last = vec![Vec::new(); joysticks.len()];
        let start = std::time::Instant::now();
        while start.elapsed().as_secs_f32() < 30.0 {
            unsafe { (api.pump_events)() };
            for (slot, (js, index)) in joysticks.iter().enumerate() {
                for axis in 0..unsafe { (api.get_num_js_axes)(*js) } {
                    let axis = axis as usize;
                    let value = unsafe { (api.get_js_axis)(*js, axis as c_int) };
                    if last[slot].get(axis) != Some(&value) {
                        while last[slot].len() <= axis {
                            last[slot].push(0);
                        }
                        last[slot][axis] = value;
                        println!(
                            "{:6.2}s js{index}[{axis}] = {:+.3}",
                            start.elapsed().as_secs_f32(),
                            f32::from(value) / 32767.0
                        );
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        for (js, _) in joysticks {
            unsafe { (api.close_joystick)(js) };
        }
        return;
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
    if std::env::var("POSES").is_ok() {
        capture_poses(&api, gamepad);
        unsafe { (api.close_gamepad)(gamepad) };
        return;
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
                if ok && values.iter().any(|v| v.abs() > 0.001) {
                    nonzero += 1;
                    if printed < 3 {
                        println!("probe: sensor {sensor} sample {values:?} @ {timestamp}us");
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
        println!("probe: sensor {sensor} -> {nonzero} nonzero reads over 3s, final {values:?}");
    }
    unsafe { (api.close_gamepad)(gamepad) };
    if moving == 2 {
        println!("probe: PASS - both sensors stream from the virtual pad");
    } else {
        println!("probe: FAIL - sensors missing or silent (moving={moving})");
        std::process::exit(1);
    }
}
