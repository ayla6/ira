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

impl InputProfile {
    /// Parse a profile from JSON. Files written by older Ira versions are
    /// not migrated; unknown fields are ignored and missing ones take their
    /// serde defaults, and `validate` rejects versions it cannot read.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|error| format!("invalid profile: {error}"))
    }
}
