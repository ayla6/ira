//! Input profile model. `model.rs` holds the types; this module re-exports
//! them and owns cross-cutting helpers (JSON loading with legacy stripping
//! and flat-binding unification).

mod model;

pub use model::{
    Activator, ActivatorKind, ActivatorSettings, ActionSet, ActionSetLayer, Activation,
    AxisDirection, AxisTransform, Binding, ChordMode, GamepadAxis, GamepadButton, GyroActivation,
    GyroCalibration, GyroConfig, GyroOrientation, GyroOutput, InputCategory, InputMapping,
    InputProfile, InputSource, ModeShift, MouseAxis, MouseButton, OutputAction, SourceMode,
    StickOutput, VirtualGamepadBackend, PROFILE_VERSION,
};

use InputSource::Axis;

impl InputProfile {
    /// Parse a profile from JSON, dropping legacy entries that no longer have
    /// a meaning (per-axis gyro bindings, recenter bindings) so profiles from
    /// before the gyro rework keep loading, and converting the flat binding
    /// list into a default action set. Ira is pre-release: nothing tries to
    /// translate old gyro setups, they are simply removed.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut value: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| format!("invalid profile JSON: {error}"))?;
        if let Some(bindings) = value.get_mut("bindings").and_then(|b| b.as_array_mut()) {
            let dropped = strip_legacy_gyro_bindings(bindings);
            if dropped > 0 {
                eprintln!("ira-input: dropped {dropped} legacy gyro/recenter binding(s)");
            }
        }
        let mut profile: InputProfile =
            serde_json::from_value(value).map_err(|error| format!("invalid profile: {error}"))?;
        convert_bindings_to_action_sets(&mut profile);
        Ok(profile)
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

/// Unify storage: flat-binding profiles become a single "Default" action set
/// so the engine and editor only deal with the set model. Lossless for the
/// shapes the old editor could express; anything exotic is dropped loudly.
pub(super) fn convert_bindings_to_action_sets(profile: &mut InputProfile) {
    if !profile.action_sets.is_empty() || profile.bindings.is_empty() {
        return;
    }
    let mut inputs: Vec<InputMapping> = Vec::new();
    // Sticks are converted as whole units: pull out each stick pair first so
    // X/Y halves never become duplicate mode mappings.
    let mut bindings = std::mem::take(&mut profile.bindings);
    for (x_axis, y_axis) in [
        (GamepadAxis::LeftX, GamepadAxis::LeftY),
        (GamepadAxis::RightX, GamepadAxis::RightY),
    ] {
        let stick = take_matching(&mut bindings, &[Axis(x_axis), Axis(y_axis)]);
        if !stick.is_empty() {
            if let Some(mapping) = convert_stick(&stick) {
                merge_mapping(&mut inputs, mapping);
            }
        }
    }
    let triggers = take_matching(
        &mut bindings,
        &[Axis(GamepadAxis::LeftTrigger), Axis(GamepadAxis::RightTrigger)],
    );
    for binding in &triggers {
        if let Some(mode) = trigger_mode(binding) {
            let mut mapping = InputMapping::new(binding.source);
            mapping.mode = Some(mode);
            merge_mapping(&mut inputs, mapping);
        }
    }
    for binding in &bindings {
        match (&binding.source, &binding.output) {
            (InputSource::Button(source), output) => {
                let mut mapping = InputMapping::new(InputSource::Button(*source));
                let mut activator = Activator::full_press(vec![output.clone()]);
                activator.activation = binding.activation.clone();
                mapping.activators.push(activator);
                merge_mapping(&mut inputs, mapping);
            }
            (
                InputSource::AxisDirection { axis, .. },
                OutputAction::GamepadButton(_),
            ) => {
                // Stick-as-dpad presets collapse into one Dpad mode per stick.
                if !inputs
                    .iter()
                    .any(|candidate| candidate.source == Axis(*axis))
                {
                    let mut mapping = InputMapping::new(Axis(*axis));
                    mapping.mode = Some(SourceMode::Dpad { threshold: 0.5 });
                    inputs.push(mapping);
                }
            }
            (source, output) => {
                eprintln!(
                    "ira-input: dropping unconvertible binding {:?} -> {:?}",
                    source, output
                );
            }
        }
    }
    if inputs.is_empty() {
        return;
    }
    profile.action_sets = vec![model::ActionSet {
        name: "Default".to_string(),
        inputs,
    }];
}

/// Remove and return every binding whose source is one of `sources`.
fn take_matching(bindings: &mut Vec<Binding>, sources: &[InputSource]) -> Vec<Binding> {
    let mut matched = Vec::new();
    let mut kept = Vec::new();
    for binding in std::mem::take(bindings) {
        if sources.contains(&binding.source) {
            matched.push(binding);
        } else {
            kept.push(binding);
        }
    }
    *bindings = kept;
    matched
}

/// Convert one stick's bindings (X and/or Y half) into its SourceMode.
/// A joystick passthrough wins over mouse output when both exist.
fn convert_stick(stick: &[Binding]) -> Option<InputMapping> {
    let source = stick.first()?.source;
    let passthrough = stick.iter().find(|binding| {
        matches!(
            (&binding.source, &binding.output),
            (Axis(GamepadAxis::LeftX | GamepadAxis::RightX), OutputAction::GamepadAxis(_))
        )
    });
    if let Some(binding) = passthrough {
        let OutputAction::GamepadAxis(output_axis) = &binding.output else {
            return None;
        };
        return Some(mapping_with_mode(
            source,
            SourceMode::Joystick {
                output: match output_axis {
                    GamepadAxis::RightX | GamepadAxis::RightY => StickOutput::Right,
                    _ => StickOutput::Left,
                },
                deadzone_inner: binding.transform.dead_zone,
                deadzone_outer: 1.0,
                curve: binding.transform.exponent,
            },
        ));
    }
    let mouse = stick.iter().find(|binding| {
        matches!(
            &binding.output,
            OutputAction::MouseAxis(MouseAxis::X | MouseAxis::Y)
        )
    });
    if let Some(binding) = mouse {
        return Some(mapping_with_mode(
            source,
            SourceMode::Mouse {
                sensitivity: binding.transform.sensitivity,
            },
        ));
    }
    for binding in stick {
        eprintln!(
            "ira-input: dropping unconvertible stick binding {:?} -> {:?}",
            binding.source, binding.output
        );
    }
    None
}

fn mapping_with_mode(source: InputSource, mode: SourceMode) -> InputMapping {
    let mut mapping = InputMapping::new(source);
    mapping.mode = Some(mode);
    mapping
}

/// Trigger passthrough becomes a thresholded Trigger mode.
fn trigger_mode(binding: &Binding) -> Option<SourceMode> {
    match (&binding.source, &binding.output) {
        (
            Axis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger),
            OutputAction::GamepadAxis(_),
        ) => Some(SourceMode::Trigger {
            threshold: binding.transform.dead_zone,
        }),
        (source, output) => {
            eprintln!(
                "ira-input: dropping unconvertible trigger binding {:?} -> {:?}",
                source, output
            );
            None
        }
    }
}

fn merge_mapping(inputs: &mut Vec<InputMapping>, mapping: InputMapping) {
    if let Some(existing) = inputs
        .iter_mut()
        .find(|candidate| candidate.source == mapping.source)
    {
        existing.activators.extend(mapping.activators);
        if existing.mode.is_none() {
            existing.mode = mapping.mode;
        }
    } else {
        inputs.push(mapping);
    }
}
