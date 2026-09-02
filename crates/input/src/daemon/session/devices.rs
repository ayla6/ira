use std::path::{Path, PathBuf};

use super::sensor::SensorPipeline;

use crate::{
    discover_gamepads, GyroProcessor, GyroProcessingOptions, InputProfile, MappingEngine,
    PhysicalGamepad, Sdl3SensorBackend, VirtualKeyboard, VirtualMouse,
};

/// Open the initially detected controller. Returns `None` (no error) when no
/// controller is plugged in yet — the session keeps running and picks one up
/// the moment it appears.
pub(crate) fn open_initial_gamepad(device: Option<&Path>) -> Option<PhysicalGamepad> {
    if let Some(path) = device {
        return match PhysicalGamepad::open(path, false) {
            Ok(gamepad) => Some(gamepad),
            Err(error) => {
                eprintln!("ira-input: {error}");
                None
            }
        };
    }
    let info = discover_gamepads().into_iter().next()?;
    match PhysicalGamepad::open(&info.path, false) {
        Ok(gamepad) => Some(gamepad),
        Err(error) => {
            eprintln!("ira-input: failed to open {}: {error}", info.path.display());
            None
        }
    }
}

/// Reconnect the previously-used controller, or detect a brand-new one when
/// none was present at launch. Returns true when a controller is now open.
pub(crate) fn reconnect_gamepad(gamepad: &mut Option<PhysicalGamepad>) -> Result<bool, String> {
    if let Some(existing) = gamepad.as_mut() {
        return existing.try_reconnect();
    }
    let Some(info) = discover_gamepads().into_iter().next() else {
        return Ok(false);
    };
    match PhysicalGamepad::open(&info.path, false) {
        Ok(opened) => {
            *gamepad = Some(opened);
            Ok(true)
        }
        Err(error) => {
            eprintln!("ira-input: failed to open {}: {error}", info.path.display());
            Ok(false)
        }
    }
}

pub(crate) fn load_profile(path: Option<&Path>) -> Result<InputProfile, String> {
    let profile = match path {
        None => InputProfile::default_gamepad(),
        Some(path) if path == Path::new("builtin:default_gamepad") => {
            InputProfile::default_gamepad()
        }
        Some(path) => {
            let contents = std::fs::read_to_string(path)
                .map_err(|error| format!("failed to read profile {}: {error}", path.display()))?;
            InputProfile::from_json(&contents)
                .map_err(|error| format!("failed to parse profile {}: {error}", path.display()))?
        }
    };
    profile.validate().map_err(|error| match path {
        Some(path) => format!("invalid profile {}: {error}", path.display()),
        None => format!("invalid builtin profile: {error}"),
    })?;
    Ok(profile)
}

/// Like [`load_profile`], but an unreadable or invalid profile file falls
/// back to the builtin default layout instead of aborting: the wrapper
/// sits between the launcher and the game command, so exiting here would
/// stop the game from starting at all.
pub(crate) fn load_profile_or_default(path: Option<&Path>) -> InputProfile {
    match load_profile(path) {
        Ok(profile) => profile,
        Err(error) => {
            eprintln!(
                "ira-input: {error}; falling back to the builtin default layout, \
                 the game still launches"
            );
            InputProfile::default_gamepad()
        }
    }
}

pub(crate) fn is_real_profile(path: &Path) -> bool {
    !path.to_string_lossy().starts_with("builtin:")
}

pub(crate) fn create_keyboard(keycodes: Vec<u16>) -> Result<Option<VirtualKeyboard>, String> {
    if keycodes.is_empty() {
        return Ok(None);
    }
    VirtualKeyboard::create(keycodes)
        .map(Some)
        .map_err(|error| format!("failed to create virtual keyboard: {error}"))
}

pub(crate) fn create_mouse(needed: bool) -> Result<Option<VirtualMouse>, String> {
    if !needed {
        return Ok(None);
    }
    VirtualMouse::create()
        .map(Some)
        .map_err(|error| format!("failed to create virtual mouse: {error}"))
}

pub(crate) fn open_sensor(device: &crate::DeviceInfo) -> Option<GyroSource> {
    // The kernel IMU companion node is a hid-nintendo (or hid-sony)
    // construct: only Nintendo-family pads can legitimately have one, and
    // scanning other pads risks latching onto a same-identity six-axis
    // node that is not a sensor companion — the DInput dongle's motion
    // interface reads through SDL3, never this path.
    if device.family() == crate::ControllerFamily::Nintendo {
        // udev can publish the companion node a beat after the pad node; a
        // short retry keeps a freshly connected pad from losing its gyro
        // to that gap. Capped well under a second: this runs on the input
        // loop, including right after a reconnect.
        for attempt in 0..10 {
            match crate::EvdevImu::open(device) {
                Some(imu) => {
                    eprintln!("ira-input: motion source: kernel IMU node");
                    return Some(GyroSource::Kernel(imu));
                }
                None => {
                    if attempt == 0 {
                        eprintln!(
                            "ira-input: no kernel IMU companion node for '{}' ({:04x}:{:04x}) yet",
                            device.name, device.vendor, device.product
                        );
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let candidates = crate::sensor_node_names(Path::new("/dev/input"));
        if candidates.is_empty() {
            eprintln!("ira-input: no sensor-like evdev node exists on this system");
        } else {
            eprintln!(
                "ira-input: sensor-like evdev nodes seen: {}",
                candidates.join("; ")
            );
        }
    }
    match Sdl3SensorBackend::open(device) {
        Ok(Some(sensor)) => {
            eprintln!("ira-input: motion source: SDL3");
            Some(GyroSource::Sdl(sensor))
        }
        Ok(None) => {
            eprintln!("ira-input: motion source: none (SDL3 found no sensor)");
            None
        }
        Err(error) => {
            eprintln!("ira-input: motion source: none (SDL3 failed: {error})");
            None
        }
    }
}

/// Where motion comes from: the kernel's sensor companion node when the pad's
/// driver exposes one (hid-nintendo, hid-sony), else in-process SDL3 for
/// pads whose protocol only SDL translates (8BitDo DInput).
pub(crate) enum GyroSource {
    Kernel(crate::EvdevImu),
    Sdl(Sdl3SensorBackend),
}

impl GyroSource {
    pub(crate) fn read(&mut self, fallback_timestamp_us: u64) -> Result<Option<crate::SensorSample>, String> {
        match self {
            Self::Kernel(imu) => imu.read(fallback_timestamp_us),
            Self::Sdl(sdl) => sdl.read(fallback_timestamp_us),
        }
    }
}

/// Seed the mapper with the connected controller's calibrated stick
/// deadzone; Joystick modes whose deadzone source is "Controller Preference"
/// read it from the engine.
pub(crate) fn apply_controller_deadzone(
    mapper: &mut MappingEngine,
    calibration_store: Option<&Path>,
    vendor: u16,
    product: u16,
) {
    let (left, right) = calibration_store
        .and_then(|path| crate::load_calibration(path, vendor, product))
        .map(|calibration| {
            (
                calibration.stick_deadzone_left,
                calibration.stick_deadzone_right,
            )
        })
        .unwrap_or((0.0, 0.0));
    mapper.set_controller_deadzones(left, right);
}

/// Routes one rumble command to whichever path owns the physical pad: the
/// Switch-protocol driver when active, else the evdev/vendor backend.
pub(crate) fn play_physical_rumble(
    pipeline: &mut SensorPipeline,
    rumble_output: &mut Option<crate::PhysicalRumble>,
    command: crate::RumbleCommand,
) {
    if let Some(switch) = pipeline.switch_hidraw.as_mut() {
        switch.play_rumble(command);
    } else if let Some(rumble) = rumble_output.as_mut() {
        rumble.play(command);
    }
}

/// Applies the controller-level Nintendo button layout from the
/// per-controller settings, the way Steam's per-controller toggle does:
/// face buttons swap (A↔B, X↔Y) as they leave the physical pad. A stored
/// choice wins; with no entry, Nintendo-family pads default to swapped.
pub(crate) fn apply_controller_layout(
    gamepad: &mut Option<PhysicalGamepad>,
    calibration_store: Option<&Path>,
) {
    let Some(pad) = gamepad.as_mut() else {
        return;
    };
    pad.set_nintendo_layout(resolved_layout_for(pad.info(), calibration_store));
}

/// The Nintendo layout in effect for a device: the stored per-controller
/// choice wins, else the family default.
pub(crate) fn resolved_layout_for(device: &crate::DeviceInfo, calibration_store: Option<&Path>) -> bool {
    match calibration_store {
        Some(path) => crate::resolved_nintendo_layout(path, device),
        None => device.prefers_nintendo_layout(),
    }
}

/// Opens the physical side of rumble passthrough. Failure reasons are logged
/// exactly once here; a missing handle afterwards simply means "no rumble"
/// and every forwarded command is skipped.
pub(crate) fn open_rumble(
    gamepad: Option<&PhysicalGamepad>,
    enabled: bool,
) -> Option<crate::PhysicalRumble> {
    if !enabled {
        return None;
    }
    let info = gamepad?.info();
    let path = info.path.clone();
    match crate::PhysicalRumble::open(&path) {
        Ok(rumble) => Some(rumble),
        Err(primary_error) => match ff_sibling_node(info, &path) {
            Some(sibling) => {
                eprintln!(
                    "ira-input: {primary_error}; using rumble on {} instead",
                    sibling.display()
                );
                match crate::PhysicalRumble::open(&sibling) {
                    Ok(rumble) => Some(rumble),
                    Err(error) => {
                        eprintln!("ira-input: {error}");
                        None
                    }
                }
            }
            None => match crate::PhysicalRumble::open_vendor_hidraw(
                &path,
                info.vendor,
                info.product,
            ) {
                Ok(rumble) => {
                    eprintln!(
                        "ira-input: {primary_error}; replaying rumble through the 8BitDo \
                         hidraw protocol instead"
                    );
                    Some(rumble)
                }
                Err(error) => {
                    eprintln!("{primary_error}");
                    eprintln!("ira-input: {error}");
                    None
                }
            },
        },
    }
}

/// Finds another evdev node of the same physical controller that does
/// declare FF_RUMBLE. Pads with the classic Linux dual-node split often
/// keep force feedback off the node SDL picks as the gamepad.
fn ff_sibling_node(info: &crate::DeviceInfo, skip: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir("/dev/input").ok()?.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("event") || entry.path() == skip {
            continue;
        }
        let Ok(device) = evdev::Device::open(entry.path()) else {
            continue;
        };
        let id = device.input_id();
        if id.vendor() != info.vendor || id.product() != info.product {
            continue;
        }
        let has_ff = device
            .supported_ff()
            .is_some_and(|effects| effects.contains(evdev::FFEffectCode::FF_RUMBLE));
        if has_ff {
            return Some(entry.path());
        }
    }
    None
}

pub(crate) fn make_gyro_processor(
    profile: &InputProfile,
    vendor: u16,
    product: u16,
    calibration_store: Option<&Path>,
) -> GyroProcessor {
    // Per-controller calibration wins; the profile's stored bias is only a
    // legacy fallback.
    let bias = calibration_store
        .and_then(|path| crate::load_calibration(path, vendor, product))
        .unwrap_or(profile.controller_calibration);
    GyroProcessor::new(
        bias,
        GyroProcessingOptions {
            smoothing: profile.gyro.smoothing,
            auto_calibrate: true,
            orientation: profile.gyro.orientation,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use crate::VirtualGamepadBackend;

    #[test]
    fn test_is_real_profile_excludes_builtin() {
        assert!(!is_real_profile(Path::new("builtin:default_gamepad")));
        assert!(!is_real_profile(Path::new("builtin:anything")));
        assert!(is_real_profile(Path::new("/tmp/profile.json")));
    }

    #[test]
    fn test_load_profile_preserves_backend() {
        let dir = temp_profile_dir("backend");
        let path = dir.join("profile.json");
        let profile = InputProfile::default_gamepad_for_backend(VirtualGamepadBackend::DirectInput);
        std::fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();

        let loaded = load_profile(Some(&path)).unwrap();
        assert_eq!(loaded.backend, VirtualGamepadBackend::DirectInput);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn temp_profile_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ira-input-test-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}