//! Input profile model. `model.rs` holds the types; this module re-exports
//! them and owns cross-cutting load-time normalization.

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
    /// Parse a profile from JSON and normalize shapes older editors wrote:
    /// per-axis stick mappings collapse onto their X axis, and version-1
    /// profiles — whose deadzone source defaulted to raw passthrough —
    /// switch to the controller-preference default. Profiles from before
    /// the action-set model carry a flat `bindings` list serde ignores —
    /// Ira is pre-release, those simply start empty.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut profile: InputProfile =
            serde_json::from_str(json).map_err(|error| format!("invalid profile: {error}"))?;
        normalize_stick_mappings(&mut profile);
        normalize_gyro_orientation(&mut profile);
        if profile.version < 2 {
            migrate_raw_deadzone_to_controller(&mut profile);
            profile.version = PROFILE_VERSION;
        }
        Ok(profile)
    }
}

/// Version 1 shipped with `StickDeadzone::None` as the default, so every
/// stick serialized `deadzone: "none"` even though the user never chose it.
/// The product default is now the controller's calibrated deadzone: flip
/// every un-chosen raw passthrough over, leaving explicit Custom alone.
fn migrate_raw_deadzone_to_controller(profile: &mut InputProfile) {
    let processing_lists = profile
        .action_sets
        .iter_mut()
        .map(|set| set.inputs.iter_mut().filter_map(inputs_processing_mut))
        .chain(
            profile
                .action_layers
                .iter_mut()
                .map(|layer| layer.inputs.iter_mut().filter_map(inputs_processing_mut)),
        )
        .flatten();
    for processing in processing_lists {
        if processing.deadzone == StickDeadzone::None {
            processing.deadzone = StickDeadzone::Controller;
        }
    }
}

/// The stick processing a mapping carries, for the modes that have one.
fn inputs_processing_mut(input: &mut InputMapping) -> Option<&mut StickProcessing> {
    match &mut input.mode {
        Some(SourceMode::Joystick(settings)) => Some(&mut settings.processing),
        Some(SourceMode::Mouse { stick, .. }) => Some(stick),
        _ => None,
    }
}

/// "Passthrough" (raw local axes) was a failed native-gyro experiment and
/// is no longer offered: profiles carrying it load as Player Space, the
/// gravity-corrected default that behaves at any hold angle.
fn normalize_gyro_orientation(profile: &mut InputProfile) {
    if profile.gyro.orientation == GyroOrientation::Local {
        profile.gyro.orientation = GyroOrientation::PlayerSpace;
    }
}

/// Older editors wrote one mapping per stick axis; a stick's behavior now
/// lives only on its X axis. Move each Y half's mode onto its X counterpart
/// and drop the leftover — unless the Y half carries activators or shifts
/// the editor could have added, which stay put (the runtime then ignores the
/// Y mode in favor of the X half's).
fn normalize_stick_mappings(profile: &mut InputProfile) {
    let inputs_lists = profile
        .action_sets
        .iter_mut()
        .map(|set| &mut set.inputs)
        .chain(
            profile
                .action_layers
                .iter_mut()
                .map(|layer| &mut layer.inputs),
        );
    for inputs in inputs_lists {
        for (x_axis, y_axis) in [
            (GamepadAxis::LeftX, GamepadAxis::LeftY),
            (GamepadAxis::RightX, GamepadAxis::RightY),
        ] {
            let Some(y_index) = inputs
                .iter()
                .position(|input| input.source == InputSource::Axis(y_axis))
            else {
                continue;
            };
            if !inputs[y_index].activators.is_empty() || !inputs[y_index].mode_shifts.is_empty() {
                continue;
            }
            let y_mode = inputs[y_index].mode.take();
            let x_source = InputSource::Axis(x_axis);
            match inputs.iter_mut().find(|input| input.source == x_source) {
                Some(x_input) => {
                    if x_input.mode.is_none() {
                        x_input.mode = y_mode;
                    }
                }
                None => {
                    if let Some(mode) = y_mode {
                        inputs.push(mapping_with_mode(x_source, mode));
                    }
                }
            }
            inputs.remove(y_index);
        }
    }
}

fn mapping_with_mode(source: InputSource, mode: SourceMode) -> InputMapping {
    let mut mapping = InputMapping::new(source);
    mapping.mode = Some(mode);
    mapping
}
