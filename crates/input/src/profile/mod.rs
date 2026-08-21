//! Input profile model. `model.rs` holds the types; this module re-exports
//! them and owns cross-cutting helpers (JSON loading with legacy stripping).

mod model;

pub use model::{
    Activator, ActivatorKind, ActivatorSettings, ActionSet, ActionSetLayer, Activation,
    AxisDirection, AxisTransform, Binding, ChordMode, GamepadAxis, GamepadButton, GyroActivation,
    GyroCalibration, GyroConfig, GyroOutput, InputCategory, InputMapping, InputProfile,
    InputSource, ModeShift, MouseAxis, MouseButton, OutputAction, SourceMode, StickOutput,
    VirtualGamepadBackend, PROFILE_VERSION,
};

impl InputProfile {
    /// Parse a profile from JSON, dropping legacy entries that no longer have
    /// a meaning (per-axis gyro bindings, recenter bindings) so profiles from
    /// before the gyro rework keep loading. Ira is pre-release: nothing tries
    /// to translate old gyro setups, they are simply removed.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut value: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| format!("invalid profile JSON: {error}"))?;
        if let Some(bindings) = value.get_mut("bindings").and_then(|b| b.as_array_mut()) {
            let dropped = strip_legacy_gyro_bindings(bindings);
            if dropped > 0 {
                eprintln!("ira-input: dropped {dropped} legacy gyro/recenter binding(s)");
            }
        }
        serde_json::from_value(value).map_err(|error| format!("invalid profile: {error}"))
    }
}

/// Remove bindings that reference removed model features, in place, and
/// return how many were dropped.
fn strip_legacy_gyro_bindings(bindings: &mut Vec<serde_json::Value>) -> usize {
    let before = bindings.len();
    bindings.retain(|binding| {
        let is_gyro_source = binding
            .get("source")
            .and_then(|source| source.as_object())
            .is_some_and(|source| source.contains_key("gyro"));
        let is_recenter_output =
            binding.get("output").and_then(|o| o.as_str()) == Some("recenter_gyro");
        !is_gyro_source && !is_recenter_output
    });
    before - bindings.len()
}
