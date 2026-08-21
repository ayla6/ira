mod gyro;
mod mapping;
mod physical;
mod profile;
mod registry;
mod sensor;
mod virtual_gamepad;
mod virtual_keyboard;
mod virtual_mouse;

pub use mapping::{InputEvent, MappingEngine, OutputEvent};
pub use physical::{
    discover_gamepads, ControllerFamily, DeviceInfo, PhysicalGamepad, ReportedInputMode,
};
pub use profile::{
    Activation, AxisDirection, AxisTransform, Binding, ChordMode, GamepadAxis, GamepadButton,
    GyroActivation, GyroCalibration, GyroConfig, GyroOutput, InputCategory, InputProfile,
    InputSource, MouseAxis, MouseButton, OutputAction, VirtualGamepadBackend, PROFILE_VERSION,
};
pub use gyro::{GyroProcessingOptions, GyroProcessor, GyroRates};
pub use registry::ControllerRegistry;
pub use sensor::{discover_sdl_gamepads, SensorSample, Sdl3SensorBackend, SdlGamepadInfo};
pub use virtual_gamepad::VirtualGamepad;
pub use virtual_keyboard::VirtualKeyboard;
pub use virtual_mouse::VirtualMouse;
