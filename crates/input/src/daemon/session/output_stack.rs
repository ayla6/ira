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

/// Whether the built stack lacks motion outputs the profile wants while
/// motion is available — the state a session is left in when its stack was
/// built before the controller appeared, because [`build_virtual_stack`]
/// gates the motion node and the uhid twins on motion availability.
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
        | VirtualGamepadBackend::Dsu => false,
    };
    profile.wants_native_controller() && twin_missing
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
        "ira-virtual-{}-{}",
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
    let native_transport = motion_available && profile.wants_native_controller();
    let native_ds4 = native_transport && backend == VirtualGamepadBackend::DualShock4;
    let native_switch_pro = native_transport && backend == VirtualGamepadBackend::SwitchPro;
    let native_dualsense = native_transport && backend == VirtualGamepadBackend::DualSense;
    let gamepad = if native_ds4 || native_switch_pro || native_dualsense {
        eprintln!("ira-input: uinput gamepad suppressed; the uhid controller is the controller");
        VirtualGamepad::shadow_only(backend)
    } else {
        match VirtualGamepad::create_for_backend(backend) {
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
    // The cemuhook stream exists as the DSU backend itself — picking that
    // output mode always streams — and doubles as a motion-only companion
    // next to a real virtual pad: XInput and DirectInput carry no motion of
    // their own, so a native-motion profile streams to emulators over
    // cemuhook instead.
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
    // The motion node must exist before the game opens the virtual pad:
    // SDL pairs sensor nodes with a pad at open time only. A node built
    // during a mid-session reload waits for the game's next pad (re)open.
    let motion_device = if motion_available && profile.native_motion {
        open_motion_node(backend)
    } else {
        None
    };
    // Experimental whole-HID DualShock4: a real hidraw device whose reports
    // carry buttons, sticks, triggers AND motion, so SDL's DS4 driver reads
    // our sensors natively. A companion motion-only HID shares its serial:
    // SDL's evdev backend pairs them, which is the half flatpaks can see.
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
    // A virtual *real* Switch Pro over USB identity: SDL2's hidapi skips
    // hidraw devices on any other bus, so USB is the only surface its
    // Switch driver can claim — motion included, read straight from the
    // 0x30 reports (the only motion path that reaches a hidapi-claimed
    // pad). hid-nintendo binds alongside, exactly as for a wired real Pro
    // Controller, and SDL hides its evdev nodes by the pad's identity. No
    // paired IMU: it gets udev's accelerometer tag, which SDL2 lists as a
    // second joystick whose axes barely move — the exact "gyro as a
    // second analog stick" confusion this backend exists not to cause.
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
    // Same whole-HID approach as the DS4, on SDL's PS5 third-party path:
    // the licensed HORI PID is typed PS5 in SDL's table and feature replies
    // enable sensors with an identity calibration.
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
