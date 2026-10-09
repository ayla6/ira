//! The session's virtual output devices, built from a profile: the pad the
//! game sees, optional keyboard and mouse, the native uhid twins, and the
//! cemuhook motion stream. Built once at session start and rebuilt on the
//! fly whenever a profile reload changes the controller kind.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    Ds4UhidDevice, DualsenseUhidDevice, ImuUhidDevice, InputProfile, MotionServer, SwitchProUhidDevice,
    VirtualGamepad, VirtualGamepadBackend, VirtualKeyboard, VirtualMotionSensor, VirtualMouse,
    MOTION_PORT,
};

use super::devices::{create_keyboard, create_mouse};
use super::sensor::{open_motion_node, spawn_paired_imu, SensorPipeline};

/// Uniquifier for uhid twins so a mid-session rebuild never collides with
/// the kernel names of the devices it replaces.
static TWIN_SERIAL: AtomicU64 = AtomicU64::new(0);

/// Whether the built stack lacks motion outputs the profile wants while motion
/// is available (built before the controller appeared).
pub(crate) fn motion_outputs_missing(
    pipeline: &SensorPipeline,
    profile: &InputProfile,
) -> bool {
    if profile.native_motion && pipeline.motion_device.is_none() {
        return true;
    }
    // Twins only exist for the native-controller backends, and only when
    // the profile wants native motion at all.
    let twin_missing = match profile.backend {
        VirtualGamepadBackend::DualShock4 => pipeline.ds4_hid.is_none(),
        VirtualGamepadBackend::SwitchPro => pipeline.switch_pro_hid.is_none(),
        VirtualGamepadBackend::DualSense => pipeline.dualsense_hid.is_none(),
        VirtualGamepadBackend::XInput
        | VirtualGamepadBackend::DirectInput
        | VirtualGamepadBackend::SteamInput
        | VirtualGamepadBackend::Dsu => false,
    };
    profile.wants_native_controller() && twin_missing
}

/// Whether native motion rides a uinput sensor node beside the virtual pad. A second
/// motion device would surface as a phantom controller under SDL2's default hint.
pub(crate) fn wants_uinput_motion_node(
    motion_available: bool,
    profile: &InputProfile,
    native_passthrough: bool,
) -> bool {
    motion_available && profile.native_motion && !native_passthrough
}

pub(crate) struct VirtualStack {
    pub(crate) gamepad: VirtualGamepad,
    pub(crate) keyboard: Option<VirtualKeyboard>,
    pub(crate) mouse: Option<VirtualMouse>,
    /// The cemuhook server: the DSU backend itself, or a motion-only
    /// companion stream next to a real virtual pad.
    pub(crate) motion: Option<MotionServer>,
    pub(crate) motion_device: Option<VirtualMotionSensor>,
    pub(crate) ds4_hid: Option<Ds4UhidDevice>,
    pub(crate) dualsense_hid: Option<DualsenseUhidDevice>,
    pub(crate) switch_pro_hid: Option<SwitchProUhidDevice>,
    pub(crate) imu_hid: Option<ImuUhidDevice>,
    /// Whether the cemuhook stream is on at all: the `--motion-port 0` kill
    /// switch can turn it off for the whole session.
    pub(crate) motion_enabled: bool,
}

/// Whether a reload from `old` to `new` must rebuild the whole output stack
/// instead of hot-applying mappings: the controller kind changed, or the
/// native uhid transport toggled.
pub(crate) fn reload_needs_full_rebuild(old: &InputProfile, new: &InputProfile) -> bool {
    old.backend != new.backend
        || old.wants_native_controller() != new.wants_native_controller()
}

fn twin_uniq() -> String {
    format!(
        "{}{}-{}",
        crate::physical::TWIN_UNIQ_PREFIX,
        std::process::id(),
        TWIN_SERIAL.fetch_add(1, Ordering::Relaxed)
    )
}

/// Builds every virtual output device a session needs for `profile`. Each
/// piece degrades to nothing (or a shadow device) on failure: a missing
/// uinput or uhid must never keep the game from starting.
pub(crate) fn build_virtual_stack(
    motion_available: bool,
    motion_allowed: bool,
    profile: &InputProfile,
    native_passthrough: bool,
    mirror: Option<crate::MirroredIdentity>,
) -> VirtualStack {
    let keyboard = match create_keyboard(profile.keyboard_keycodes()) {
        Ok(keyboard) => keyboard,
        Err(error) => {
            eprintln!("ira-input: {error}; keyboard output disabled, the game still launches");
            None
        }
    };
    let mouse = match create_mouse(profile.uses_mouse()) {
        Ok(mouse) => mouse,
        Err(error) => {
            eprintln!("ira-input: {error}; mouse output disabled, the game still launches");
            None
        }
    };
    let backend = profile.backend;
    // The native twin exists whenever the profile wants one, motion or not: gating on
    // a live source left sessions with a visible physical pad and no virtual pad at all.
    let native_transport = profile.wants_native_controller() && !native_passthrough;
    let gamepad_passthrough = native_passthrough;
    let native_ds4 = native_transport && backend == VirtualGamepadBackend::DualShock4;
    let native_switch_pro = native_transport && backend == VirtualGamepadBackend::SwitchPro;
    let native_dualsense = native_transport && backend == VirtualGamepadBackend::DualSense;
    let gamepad = if gamepad_passthrough {
        eprintln!("ira-input: passthrough session; the physical pad is the controller");
        VirtualGamepad::shadow_only(backend)
    } else if native_ds4 || native_switch_pro || native_dualsense {
        eprintln!("ira-input: uinput gamepad suppressed; the uhid controller is the controller");
        VirtualGamepad::shadow_only(backend)
    } else {
        match VirtualGamepad::create_for_backend(backend, mirror) {
            Ok(virtual_gamepad) => virtual_gamepad,
            Err(error) => {
                eprintln!(
                    "ira-input: failed to create virtual gamepad: {error}; \
                     gamepad output disabled, the game still launches"
                );
                VirtualGamepad::shadow_only(backend)
            }
        }
    };
    // The cemuhook stream is the DSU backend itself, or a motion-only companion next
    // to a real virtual pad (XInput/DirectInput carry no motion of their own).
    let motion_enabled = motion_allowed
        && (backend == VirtualGamepadBackend::Dsu
            || (motion_available && profile.native_motion));
    let motion = if motion_enabled {
        MotionServer::bind()
    } else {
        None
    };
    match (&motion, motion_available) {
        (Some(_), true) => eprintln!(
            "ira-input: motion passthrough on udp/{MOTION_PORT} for emulators (cemuhook)"
        ),
        (None, true) => {
            eprintln!(
                "ira-input: udp/{MOTION_PORT} busy; motion passthrough disabled"
            )
        }
        _ => {}
    }
    // The motion node must exist before the game opens the virtual pad: SDL pairs
    // sensor nodes with a pad at open time only.
    let motion_device = if wants_uinput_motion_node(motion_available, profile, native_passthrough)
    {
        open_motion_node(backend)
    } else {
        None
    };
    // Whole-HID DualShock4: reports carry buttons, sticks, triggers and motion, so
    // SDL's DS4 driver reads our sensors natively; the paired motion HID is flatpak-visible.
    let mut ds4_hid = None;
    let mut dualsense_hid = None;
    let mut switch_pro_hid = None;
    let mut imu_hid = None;
    if native_ds4 {
        let uniq = twin_uniq();
        match Ds4UhidDevice::create(&uniq) {
            Ok(device) => {
                eprintln!("ira-input: experimental native-motion DS4 exposed over hidraw");
                ds4_hid = Some(device);
                imu_hid = spawn_paired_imu(&uniq);
            }
            Err(error) => {
                eprintln!(
                    "ira-input: failed to create virtual DS4: {error}; \
                     /dev/uhid is root-only unless a uaccess rule grants it"
                );
            }
        }
    }
    // A virtual *real* Switch Pro over USB identity: the only bus SDL2's hidapi Switch
    // driver claims; hid-nintendo binds alongside as for a wired real Pro Controller.
    // Motion comes straight from the 0x30 reports (the only motion path that
    // reaches a hidapi-claimed pad). No paired IMU: it would get udev's
    // accelerometer tag, which SDL2 lists as a second joystick whose axes
    // barely move — the exact "gyro as a second analog stick" confusion
    // this backend exists to avoid.
    if native_switch_pro {
        match SwitchProUhidDevice::create(&twin_uniq()) {
            Ok(device) => {
                eprintln!("ira-input: virtual Switch Pro exposed over hidraw");
                switch_pro_hid = Some(device);
            }
            Err(error) => {
                eprintln!("ira-input: failed to create virtual Switch Pro: {error}");
            }
        }
    }
    // Same whole-HID approach as the DS4, on SDL's PS5 third-party path with the
    // licensed HORI PID and identity-calibrated sensor replies.
    if native_dualsense {
        let uniq = twin_uniq();
        match DualsenseUhidDevice::create(&uniq) {
            Ok(device) => {
                eprintln!("ira-input: experimental native-motion DualSense exposed over hidraw");
                dualsense_hid = Some(device);
                imu_hid = spawn_paired_imu(&uniq);
            }
            Err(error) => {
                eprintln!(
                    "ira-input: failed to create virtual DualSense: {error}; \
                     /dev/uhid is root-only unless a uaccess rule grants it"
                );
            }
        }
    }
    VirtualStack {
        gamepad,
        keyboard,
        mouse,
        motion,
        motion_device,
        ds4_hid,
        dualsense_hid,
        switch_pro_hid,
        imu_hid,
        motion_enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::super::sensor::SensorPipeline;
    use super::*;
    use crate::{ControllerCalibration, GyroProcessingOptions, GyroProcessor};

    fn bare_pipeline() -> SensorPipeline {
        SensorPipeline {
            motion_available: false,
            gyro_processor: GyroProcessor::new(
                ControllerCalibration::default(),
                GyroProcessingOptions::default(),
            ),
            last_sensor_us: None,
            motion: None,
            motion_device: None,
            ds4_hid: None,
            dualsense_hid: None,
            switch_pro_hid: None,
            imu_hid: None,
            ever_had_sensor: false,
            last_dsu_ts: 0,
        }
    }

    fn motion_profile(backend: VirtualGamepadBackend) -> InputProfile {
        InputProfile {
            backend,
            native_motion: true,
            ..InputProfile::default_gamepad()
        }
    }

    #[test]
    fn test_reload_needs_full_rebuild_truth_table() {
        let base = InputProfile::default_gamepad();
        // Mapping-only edits stay on the cheap reload path.
        assert!(!reload_needs_full_rebuild(&base, &base));
        // A different controller kind rebuilds the output stack.
        let direct_input = InputProfile::default_gamepad_for_backend(VirtualGamepadBackend::DirectInput);
        assert!(reload_needs_full_rebuild(&base, &direct_input));
        assert!(reload_needs_full_rebuild(&direct_input, &base));
    }

    #[test]
    fn test_passthrough_never_spawns_a_uinput_motion_node() {
        let motion = motion_profile(VirtualGamepadBackend::XInput);
        // A uinput-backed session builds the sensor node beside its pad.
        assert!(wants_uinput_motion_node(true, &motion, false));
        // A passthrough leaves motion to the pad's own sensor node.
        assert!(!wants_uinput_motion_node(true, &motion, true));
        // No motion source or no native-motion request, no node either way.
        assert!(!wants_uinput_motion_node(false, &motion, false));
        let plain = InputProfile::default_gamepad();
        assert!(!wants_uinput_motion_node(true, &plain, false));
    }

    #[test]
    fn test_motion_outputs_missing_detects_a_late_motion_node() {
        // A uinput-backed session built before the pad appeared has no
        // motion node; the profile wants one.
        let profile = motion_profile(VirtualGamepadBackend::XInput);
        let pipeline = bare_pipeline();
        assert!(motion_outputs_missing(&pipeline, &profile));
        // A profile without native motion never needs one.
        let plain = InputProfile::default_gamepad();
        assert!(!motion_outputs_missing(&pipeline, &plain));
    }

    #[test]
    fn test_motion_outputs_missing_detects_missing_twins() {
        // A native-controller profile whose stack was built motion-less
        // got a plain uinput pad instead of the uhid twin.
        let profile = motion_profile(VirtualGamepadBackend::DualShock4);
        let pipeline = bare_pipeline();
        assert!(motion_outputs_missing(&pipeline, &profile));
        // Twin backends with motion entirely off want nothing.
        let plain = InputProfile {
            backend: VirtualGamepadBackend::DualShock4,
            ..InputProfile::default_gamepad()
        };
        assert!(!motion_outputs_missing(&pipeline, &plain));
    }
}
