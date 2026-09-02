pub mod daemon;
mod calibration;
mod gyro;
mod hid_ds4;
mod hid_dualsense;
mod mapping;
mod physical;
mod profile;
mod registry;
mod report_rate;
mod sensor;
mod uhid;
mod virtual_gamepad;
mod virtual_keyboard;
mod virtual_mouse;

pub use calibration::{
    calibration_store_path, default_calibration_for, device_key, load_calibration,
    remove_calibration, resolved_nintendo_layout, save_calibration,
};
pub use gyro::{GyroProcessingOptions, GyroProcessor, GyroRates};
pub use mapping::{InputEvent, MappingEngine, OutputEvent};
pub use physical::{
    discover_gamepads, ControllerFamily, DeviceInfo, PhysicalGamepad, ReportedInputMode,
};
pub use profile::{
    ActionSet, ActionSetLayer, Activation, Activator, ActivatorKind, ActivatorSettings,
    AnalogCondition, AxisDirection, ChordMode, ControllerCalibration, GamepadAxis, GamepadButton,
    GyroActivation, GyroConfig, GyroMomentum, GyroOrientation, GyroOutput, GyroStickResponseStyle,
    GyroStickSettings, InputCategory,
    InputMapping, InputProfile, InputSource, JoystickSettings, ModeShift, MouseAxis, MouseButton,
    OuterRingCommand, OutputAction, ResponseAxisStyle, SourceMode, StickDeadzone, StickOutput,
    StickOutputAxis, StickProcessing, TriggerDampening, VirtualGamepadBackend, PROFILE_VERSION,
};
pub use registry::ControllerRegistry;
mod rumble;
pub use report_rate::ReportRateEstimator;
pub use rumble::{rumble_report_8bitdo, PhysicalRumble, RumbleCommand, VENDOR_8BITDO};
mod cursor;
mod evdev_imu;
mod focus;
mod switch_hidraw;
mod switch_rumble;
pub use cursor::CursorWatcher;
pub use evdev_imu::{discover_imu_node, sensor_node_names, EvdevImu};
pub use focus::FocusWatcher;
pub use switch_hidraw::SwitchHidrawPad;
mod motion_udp;
pub use motion_udp::{
    sensor_to_dsu_frame, sensor_to_motion, MotionSample, MotionServer, PadState, MOTION_PORT,
};
mod motion_device;
pub use motion_device::VirtualMotionSensor;
mod hid_imu;
mod hid_switch_pro;
pub use hid_ds4::Ds4UhidDevice;
pub use hid_dualsense::DualsenseUhidDevice;
pub use hid_imu::ImuUhidDevice;
pub use hid_switch_pro::SwitchProUhidDevice;
pub use uhid::{UhidDevice, BUS_USB};
pub mod vdf;
pub use sensor::{discover_sdl_gamepads, Sdl3SensorBackend, SdlGamepadInfo, SensorSample};
pub use vdf::{import_vdf, import_vdf_file, ImportReport};
pub use virtual_gamepad::VirtualGamepad;
pub use virtual_keyboard::VirtualKeyboard;
pub use virtual_mouse::VirtualMouse;
