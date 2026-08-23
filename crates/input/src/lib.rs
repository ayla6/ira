mod calibration;
mod gyro;
mod mapping;
mod physical;
mod profile;
mod registry;
mod report_rate;
mod sensor;
mod virtual_gamepad;
mod virtual_keyboard;
mod virtual_mouse;

pub use mapping::{InputEvent, MappingEngine, OutputEvent};
pub use physical::{
    discover_gamepads, ControllerFamily, DeviceInfo, PhysicalGamepad, ReportedInputMode,
};
pub use profile::{
    Activator, ActivatorKind, ActivatorSettings, ActionSet, ActionSetLayer, Activation,
    AnalogCondition, AxisDirection, AxisTransform, Binding, ChordMode, GamepadAxis, GamepadButton,
    GyroActivation, GyroCalibration, GyroConfig, GyroOrientation, GyroOutput, InputCategory,
    InputMapping, InputProfile, InputSource, ModeShift, MouseAxis, MouseButton, OutputAction,
    SourceMode, StickOutput, VirtualGamepadBackend, PROFILE_VERSION,
};
pub use calibration::{
    calibration_store_path, device_key, load_calibration, remove_calibration, save_calibration,
};
pub use gyro::{GyroProcessingOptions, GyroProcessor, GyroRates};
pub use registry::ControllerRegistry;
pub use report_rate::ReportRateEstimator;
mod focus;
pub use focus::FocusWatcher;
mod motion_udp;
pub use motion_udp::{
    sensor_to_motion, MotionSample, MotionServer, PadState, MOTION_PORT,
};
mod motion_device;
pub use motion_device::VirtualMotionSensor;
pub mod vdf;
pub use vdf::{import_vdf, import_vdf_file, ImportReport};
pub use sensor::{discover_sdl_gamepads, SensorSample, Sdl3SensorBackend, SdlGamepadInfo};
pub use virtual_gamepad::VirtualGamepad;
pub use virtual_keyboard::VirtualKeyboard;
pub use virtual_mouse::VirtualMouse;
