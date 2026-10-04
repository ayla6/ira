//! Input profile model. `model.rs` holds the types; this module re-exports
//! them and owns JSON loading.

mod action_sets;
mod model;

pub use model::{
    ActionSet, ActionSetLayer, Activation, Activator, ActivatorKind, ActivatorSettings,
    AnalogCondition, AxisDirection, ChordMode, ControllerCalibration, GamepadAxis, GamepadButton,
    GyroActivation, GyroConfig, GyroMomentum, GyroOrientation, GyroOutput, GyroStickResponseStyle,
    GyroStickSettings, InputCategory,
    InputMapping, InputProfile, InputSource, JoystickSettings, ModeShift, MouseAxis, MouseButton,
    OuterRingCommand, OutputAction, ResponseAxisStyle, SourceMode, StickDeadzone, StickOutput,
    StickOutputAxis, StickProcessing, TriggerDampening, VirtualGamepadBackend, PROFILE_VERSION,
};
