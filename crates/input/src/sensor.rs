use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use crate::{DeviceInfo, GyroAxis, GyroCalibration, InputEvent, InputSource};

const SDL_INIT_GAMEPAD: u32 = 0x0000_2000;
const SDL_INIT_EVENTS: u32 = 0x0000_4000;
const SDL_INIT_SENSOR: u32 = 0x0000_8000;
const SDL_SENSOR_GYRO: i32 = 2;

type SdlInit = unsafe extern "C" fn(u32) -> bool;
type SdlGetGamepads = unsafe extern "C" fn(*mut i32) -> *mut i32;
type SdlOpenGamepad = unsafe extern "C" fn(i32) -> *mut c_void;
type SdlCloseGamepad = unsafe extern "C" fn(*mut c_void);
type SdlGetGamepadName = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type SdlGetGamepadPath = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type SdlGetGamepadVendor = unsafe extern "C" fn(*mut c_void) -> u16;
type SdlGetGamepadProduct = unsafe extern "C" fn(*mut c_void) -> u16;
type SdlGamepadHasSensor = unsafe extern "C" fn(*mut c_void, i32) -> bool;
type SdlSetGamepadSensorEnabled = unsafe extern "C" fn(*mut c_void, i32, bool) -> bool;
type SdlGetGamepadSensorData = unsafe extern "C" fn(*mut c_void, i32, *mut f32, i32) -> bool;
type SdlGetSensors = unsafe extern "C" fn(*mut i32) -> *mut i32;
type SdlGetSensorNameForId = unsafe extern "C" fn(i32) -> *const c_char;
type SdlGetSensorTypeForId = unsafe extern "C" fn(i32) -> i32;
type SdlOpenSensor = unsafe extern "C" fn(i32) -> *mut c_void;
type SdlCloseSensor = unsafe extern "C" fn(*mut c_void);
type SdlGetSensorData = unsafe extern "C" fn(*mut c_void, *mut f32, i32) -> bool;
type SdlPumpEvents = unsafe extern "C" fn();
type SdlUpdateSensors = unsafe extern "C" fn();
type SdlQuit = unsafe extern "C" fn();
type SdlFree = unsafe extern "C" fn(*mut c_void);

#[derive(Clone, Copy)]
struct Sdl3Api {
    init: SdlInit,
    get_gamepads: SdlGetGamepads,
    open_gamepad: SdlOpenGamepad,
    close_gamepad: SdlCloseGamepad,
    get_name: SdlGetGamepadName,
    get_path: SdlGetGamepadPath,
    get_vendor: SdlGetGamepadVendor,
    get_product: SdlGetGamepadProduct,
    has_sensor: SdlGamepadHasSensor,
    set_sensor_enabled: SdlSetGamepadSensorEnabled,
    get_gamepad_sensor_data: SdlGetGamepadSensorData,
    get_sensors: SdlGetSensors,
    get_sensor_name_for_id: SdlGetSensorNameForId,
    get_sensor_type_for_id: SdlGetSensorTypeForId,
    open_sensor: SdlOpenSensor,
    close_sensor: SdlCloseSensor,
    get_sensor_data: SdlGetSensorData,
    pump_events: SdlPumpEvents,
    update_sensors: SdlUpdateSensors,
    quit: SdlQuit,
    free: SdlFree,
}

impl Sdl3Api {
    unsafe fn load(handle: *mut c_void) -> Result<Self, String> {
        macro_rules! symbol {
            ($name:literal, $type:ty) => {{
                let pointer = libc::dlsym(handle, concat!($name, "\0").as_ptr().cast());
                if pointer.is_null() {
                    return Err(format!("SDL3 symbol {} is unavailable", $name));
                }
                std::mem::transmute::<*mut c_void, $type>(pointer)
            }};
        }

        Ok(Self {
            init: symbol!("SDL_Init", SdlInit),
            get_gamepads: symbol!("SDL_GetGamepads", SdlGetGamepads),
            open_gamepad: symbol!("SDL_OpenGamepad", SdlOpenGamepad),
            close_gamepad: symbol!("SDL_CloseGamepad", SdlCloseGamepad),
            get_name: symbol!("SDL_GetGamepadName", SdlGetGamepadName),
            get_path: symbol!("SDL_GetGamepadPath", SdlGetGamepadPath),
            get_vendor: symbol!("SDL_GetGamepadVendor", SdlGetGamepadVendor),
            get_product: symbol!("SDL_GetGamepadProduct", SdlGetGamepadProduct),
            has_sensor: symbol!("SDL_GamepadHasSensor", SdlGamepadHasSensor),
            set_sensor_enabled: symbol!("SDL_SetGamepadSensorEnabled", SdlSetGamepadSensorEnabled),
            get_gamepad_sensor_data: symbol!("SDL_GetGamepadSensorData", SdlGetGamepadSensorData),
            get_sensors: symbol!("SDL_GetSensors", SdlGetSensors),
            get_sensor_name_for_id: symbol!("SDL_GetSensorNameForID", SdlGetSensorNameForId),
            get_sensor_type_for_id: symbol!("SDL_GetSensorTypeForID", SdlGetSensorTypeForId),
            open_sensor: symbol!("SDL_OpenSensor", SdlOpenSensor),
            close_sensor: symbol!("SDL_CloseSensor", SdlCloseSensor),
            get_sensor_data: symbol!("SDL_GetSensorData", SdlGetSensorData),
            pump_events: symbol!("SDL_PumpEvents", SdlPumpEvents),
            update_sensors: symbol!("SDL_UpdateSensors", SdlUpdateSensors),
            quit: symbol!("SDL_Quit", SdlQuit),
            free: symbol!("SDL_free", SdlFree),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroSample {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub timestamp_us: u64,
}

impl GyroSample {
    pub fn input_events(self) -> [InputEvent; 3] {
        [
            InputEvent {
                source: InputSource::Gyro(GyroAxis::X),
                value: self.x,
                timestamp_us: self.timestamp_us,
            },
            InputEvent {
                source: InputSource::Gyro(GyroAxis::Y),
                value: self.y,
                timestamp_us: self.timestamp_us,
            },
            InputEvent {
                source: InputSource::Gyro(GyroAxis::Z),
                value: self.z,
                timestamp_us: self.timestamp_us,
            },
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdlGamepadInfo {
    pub id: i32,
    pub name: String,
    pub path: Option<String>,
    pub vendor: u16,
    pub product: u16,
    pub has_gyro: bool,
    pub has_accelerometer: bool,
}

#[derive(Clone, Copy)]
enum SensorSource {
    Gamepad(*mut c_void),
    Global(*mut c_void),
}

impl GyroCalibration {
    pub fn from_samples(samples: &[GyroSample]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let count = samples.len() as f32;
        let sums = samples.iter().fold([0.0; 3], |mut sums, sample| {
            sums[0] += sample.x;
            sums[1] += sample.y;
            sums[2] += sample.z;
            sums
        });
        Some(Self {
            x: sums[0] / count,
            y: sums[1] / count,
            z: sums[2] / count,
        })
    }

    pub fn apply(self, sample: GyroSample) -> GyroSample {
        GyroSample {
            x: sample.x - self.x,
            y: sample.y - self.y,
            z: sample.z - self.z,
            timestamp_us: sample.timestamp_us,
        }
    }
}

pub struct Sdl3SensorBackend {
    handle: *mut c_void,
    api: Sdl3Api,
    source: SensorSource,
}

impl Sdl3SensorBackend {
    pub fn open(device: &DeviceInfo) -> Result<Option<Self>, String> {
        let Some((handle, api)) = load_sdl()? else {
            return Ok(None);
        };

        let ids = gamepad_ids(&api);

        for id in ids {
            let gamepad = unsafe { (api.open_gamepad)(id) };
            if gamepad.is_null() {
                continue;
            }
            if matches_device(&api, gamepad, device)
                && unsafe { (api.has_sensor)(gamepad, SDL_SENSOR_GYRO) }
                && unsafe { (api.set_sensor_enabled)(gamepad, SDL_SENSOR_GYRO, true) }
            {
                return Ok(Some(Self {
                    handle,
                    api,
                    source: SensorSource::Gamepad(gamepad),
                }));
            }
            unsafe { (api.close_gamepad)(gamepad) };
        }

        let gyro_ids = sensor_ids(&api)
            .into_iter()
            .filter(|id| unsafe { (api.get_sensor_type_for_id)(*id) } == SDL_SENSOR_GYRO)
            .collect::<Vec<_>>();
        let sensor_id = gyro_ids
            .iter()
            .find(|id| sensor_matches_device(&api, **id, device))
            .copied()
            .or_else(|| (gyro_ids.len() == 1).then_some(gyro_ids[0]));
        if let Some(sensor_id) = sensor_id {
            let sensor = unsafe { (api.open_sensor)(sensor_id) };
            if !sensor.is_null() {
                return Ok(Some(Self {
                    handle,
                    api,
                    source: SensorSource::Global(sensor),
                }));
            }
        }
        unsafe {
            (api.quit)();
            libc::dlclose(handle);
        }
        Ok(None)
    }

    pub fn read(&mut self, timestamp_us: u64) -> Result<Option<GyroSample>, String> {
        self.read_raw(timestamp_us)
    }

    pub fn calibrate(&mut self, duration: Duration) -> Result<GyroCalibration, String> {
        let deadline = Instant::now() + duration;
        let mut samples = Vec::new();
        while Instant::now() < deadline {
            if let Some(sample) = self.read_raw(0)? {
                samples.push(sample);
            }
            thread::sleep(Duration::from_millis(4));
        }
        GyroCalibration::from_samples(&samples)
            .ok_or_else(|| "SDL3 returned no gyro samples during calibration".to_string())
    }

    fn read_raw(&mut self, timestamp_us: u64) -> Result<Option<GyroSample>, String> {
        let mut data = [0.0; 3];
        unsafe {
            (self.api.pump_events)();
            (self.api.update_sensors)();
        }
        let available = unsafe {
            match self.source {
                SensorSource::Gamepad(gamepad) => (self.api.get_gamepad_sensor_data)(
                    gamepad,
                    SDL_SENSOR_GYRO,
                    data.as_mut_ptr(),
                    3,
                ),
                SensorSource::Global(sensor) => {
                    (self.api.get_sensor_data)(sensor, data.as_mut_ptr(), 3)
                }
            }
        };
        if !available {
            return Ok(None);
        }
        if data.iter().any(|value| !value.is_finite()) {
            return Err("SDL3 returned a non-finite gyro sample".to_string());
        }
        Ok(Some(GyroSample {
            x: data[0],
            y: data[1],
            z: data[2],
            timestamp_us,
        }))
    }
}

pub fn discover_sdl_gamepads() -> Result<Vec<SdlGamepadInfo>, String> {
    let Some((handle, api)) = load_sdl()? else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for id in gamepad_ids(&api) {
        let gamepad = unsafe { (api.open_gamepad)(id) };
        if gamepad.is_null() {
            continue;
        }
        result.push(sdl_gamepad_info(&api, gamepad, id));
        unsafe { (api.close_gamepad)(gamepad) };
    }
    unsafe {
        (api.quit)();
        libc::dlclose(handle);
    }
    Ok(result)
}

impl Drop for Sdl3SensorBackend {
    fn drop(&mut self) {
        unsafe {
            match self.source {
                SensorSource::Gamepad(gamepad) => (self.api.close_gamepad)(gamepad),
                SensorSource::Global(sensor) => (self.api.close_sensor)(sensor),
            }
            (self.api.quit)();
            libc::dlclose(self.handle);
        }
    }
}

fn matches_device(api: &Sdl3Api, gamepad: *mut c_void, device: &DeviceInfo) -> bool {
    let path = unsafe { (api.get_path)(gamepad) };
    let path_matches = (!path.is_null())
        .then(|| unsafe { CStr::from_ptr(path) })
        .and_then(|path| path.to_str().ok())
        .is_some_and(|path| {
            path == device.path.to_string_lossy()
                || Path::new(path).file_name() == device.path.file_name()
        });
    let name = unsafe { (api.get_name)(gamepad) };
    let name_matches = (!name.is_null())
        .then(|| unsafe { CStr::from_ptr(name) })
        .and_then(|name| name.to_str().ok())
        .is_some_and(|name| {
            name == device.name || name.ends_with(&device.name) || device.name.ends_with(name)
        });
    let identity_matches = unsafe { (api.get_vendor)(gamepad) } == device.vendor
        && unsafe { (api.get_product)(gamepad) } == device.product;
    path_matches || (name_matches && identity_matches)
}

fn sensor_matches_device(api: &Sdl3Api, sensor_id: i32, device: &DeviceInfo) -> bool {
    let name = unsafe { (api.get_sensor_name_for_id)(sensor_id) };
    c_string(name).is_some_and(|name| names_match(&name, &device.name))
}

fn names_match(left: &str, right: &str) -> bool {
    let left = left.to_ascii_lowercase();
    let right = right.to_ascii_lowercase();
    left == right || left.contains(&right) || right.contains(&left)
}

fn load_sdl() -> Result<Option<(*mut c_void, Sdl3Api)>, String> {
    let library_name = CString::new("libSDL3.so.0").expect("static SDL3 library name");
    let handle = unsafe { libc::dlopen(library_name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return Ok(None);
    }
    let api = match unsafe { Sdl3Api::load(handle) } {
        Ok(api) => api,
        Err(error) => {
            unsafe { libc::dlclose(handle) };
            return Err(error);
        }
    };
    if !unsafe { (api.init)(SDL_INIT_EVENTS | SDL_INIT_GAMEPAD | SDL_INIT_SENSOR) } {
        unsafe { libc::dlclose(handle) };
        return Err("SDL3 failed to initialize gamepad and sensor support".to_string());
    }
    Ok(Some((handle, api)))
}

fn gamepad_ids(api: &Sdl3Api) -> Vec<i32> {
    let mut count = 0;
    let ids_ptr = unsafe { (api.get_gamepads)(&mut count) };
    if ids_ptr.is_null() || count <= 0 {
        return Vec::new();
    }
    let ids = unsafe { std::slice::from_raw_parts(ids_ptr, count as usize) }.to_vec();
    unsafe { (api.free)(ids_ptr.cast()) };
    ids
}

fn sensor_ids(api: &Sdl3Api) -> Vec<i32> {
    let mut count = 0;
    let ids_ptr = unsafe { (api.get_sensors)(&mut count) };
    if ids_ptr.is_null() || count <= 0 {
        return Vec::new();
    }
    let ids = unsafe { std::slice::from_raw_parts(ids_ptr, count as usize) }.to_vec();
    unsafe { (api.free)(ids_ptr.cast()) };
    ids
}

fn sdl_gamepad_info(api: &Sdl3Api, gamepad: *mut c_void, id: i32) -> SdlGamepadInfo {
    SdlGamepadInfo {
        id,
        name: c_string(unsafe { (api.get_name)(gamepad) })
            .unwrap_or_else(|| "Unknown gamepad".to_string()),
        path: c_string(unsafe { (api.get_path)(gamepad) }),
        vendor: unsafe { (api.get_vendor)(gamepad) },
        product: unsafe { (api.get_product)(gamepad) },
        has_gyro: unsafe { (api.has_sensor)(gamepad, SDL_SENSOR_GYRO) },
        has_accelerometer: unsafe { (api.has_sensor)(gamepad, 1) },
    }
}

fn c_string(pointer: *const c_char) -> Option<String> {
    (!pointer.is_null())
        .then(|| unsafe { CStr::from_ptr(pointer) })
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::{names_match, GyroCalibration, GyroSample};

    #[test]
    fn test_sensor_name_matches_dinput_device_name() {
        assert!(names_match(
            "8BitDo Ultimate 2 Wireless Controller for PC",
            "8BitDo 8BitDo Ultimate 2 Wireless Controller for PC",
        ));
        assert!(!names_match("Laptop motion sensor", "8BitDo controller"));
    }

    #[test]
    fn test_gyro_calibration_removes_bias() {
        let calibration = GyroCalibration {
            x: 0.1,
            y: -0.2,
            z: 0.3,
        };
        let calibrated = calibration.apply(GyroSample {
            x: 0.4,
            y: 0.1,
            z: 0.8,
            timestamp_us: 42,
        });
        assert!((calibrated.x - 0.3).abs() < 0.001);
        assert!((calibrated.y - 0.3).abs() < 0.001);
        assert!((calibrated.z - 0.5).abs() < 0.001);
        assert_eq!(calibrated.timestamp_us, 42);
    }

    #[test]
    fn test_gyro_calibration_averages_samples() {
        let calibration = GyroCalibration::from_samples(&[
            GyroSample {
                x: 0.1,
                y: 0.2,
                z: 0.3,
                timestamp_us: 1,
            },
            GyroSample {
                x: 0.3,
                y: 0.4,
                z: 0.5,
                timestamp_us: 2,
            },
        ])
        .unwrap();
        assert!((calibration.x - 0.2).abs() < 0.001);
        assert!((calibration.y - 0.3).abs() < 0.001);
        assert!((calibration.z - 0.4).abs() < 0.001);
        assert!(GyroCalibration::from_samples(&[]).is_none());
    }

    #[test]
    fn test_gyro_sample_becomes_three_mapping_events() {
        let events = (GyroSample {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            timestamp_us: 42,
        })
        .input_events();
        assert_eq!(events[0].value, 1.0);
        assert_eq!(events[1].value, 2.0);
        assert_eq!(events[2].value, 3.0);
        assert!(events.iter().all(|event| event.timestamp_us == 42));
    }
}
