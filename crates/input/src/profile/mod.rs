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
            let (dropped, legacy_gyro) = strip_legacy_gyro_bindings(bindings);
            if dropped > 0 {
                eprintln!("ira-input: dropped {dropped} legacy gyro/recenter binding(s)");
            }
            // The user's gyro lived in per-axis bindings before the config
            // card existed; keep it working instead of dropping their setup.
            // An explicit new-format gyro config in the file wins.
            if value.get("gyro").is_none() {
                if let Some(config) = synthesize_gyro_config(&legacy_gyro) {
                    eprintln!("ira-input: converted legacy gyro bindings to gyro config");
                    match serde_json::to_value(config) {
                        Ok(config) => value["gyro"] = config,
                        Err(error) => return Err(format!("invalid profile: {error}")),
                    }
                }
            }
        }
        let mut profile: InputProfile =
            serde_json::from_value(value).map_err(|error| format!("invalid profile: {error}"))?;
        convert_bindings_to_action_sets(&mut profile);
        Ok(profile)
    }
}

/// Remove bindings that reference removed model features, in place, and
/// return how many were dropped. Keeps a copy of the removed gyro bindings so
/// [`synthesize_gyro_config`] can translate them into the config card model.
fn strip_legacy_gyro_bindings(
    bindings: &mut Vec<serde_json::Value>,
) -> (usize, Vec<serde_json::Value>) {
    let mut legacy_gyro = Vec::new();
    let before = bindings.len();
    bindings.retain(|binding| {
        let is_gyro_source = binding
            .get("source")
            .and_then(|source| source.as_object())
            .is_some_and(|source| source.contains_key("gyro"));
        let is_recenter_output =
            binding.get("output").and_then(|o| o.as_str()) == Some("recenter_gyro");
        if is_gyro_source {
            legacy_gyro.push(binding.clone());
        }
        !is_gyro_source && !is_recenter_output
    });
    (before - bindings.len(), legacy_gyro)
}

/// Translate pre-card per-axis gyro bindings into a [`GyroConfig`]: output by
/// majority target, activation from any Hold/Toggle gate, sensitivity and
/// invert flags from the yaw/pitch transforms.
fn synthesize_gyro_config(legacy: &[serde_json::Value]) -> Option<GyroConfig> {
    if legacy.is_empty() {
        return None;
    }
    let mut config = GyroConfig {
        enabled: true,
        ..GyroConfig::default()
    };
    let mut counts = [(GyroOutput::Mouse, 0), (GyroOutput::LeftStick, 0), (GyroOutput::RightStick, 0)];
    for binding in legacy {
        if let Some(kind) = legacy_output_kind(binding.get("output")) {
            for slot in counts.iter_mut() {
                if slot.0 == kind {
                    slot.1 += 1;
                }
            }
        }
        if config.activation == GyroActivation::Always {
            if let Some(activation) = legacy_activation(binding.get("activation")) {
                config.activation = activation;
            }
        }
        let transform = binding.get("transform");
        let axis = binding
            .get("source")
            .and_then(|source| source.get("gyro"))
            .and_then(|axis| axis.as_str())
            .unwrap_or("z");
        if let Some(sensitivity) = transform
            .and_then(|t| t.get("sensitivity"))
            .and_then(|s| s.as_f64())
            .map(|s| s as f32)
            .filter(|s| (0.05..=20.0).contains(s))
        {
            config.sensitivity = sensitivity;
        }
        let invert = transform
            .and_then(|t| t.get("invert"))
            .and_then(|i| i.as_bool())
            .unwrap_or(false);
        match axis {
            "z" => config.invert_x = invert,
            "x" => config.invert_y = invert,
            _ => {}
        }
    }
    config.output = counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(output, _)| output)
        .unwrap_or(GyroOutput::Mouse);
    Some(config)
}

fn legacy_output_kind(output: Option<&serde_json::Value>) -> Option<GyroOutput> {
    let output = output?.as_object()?;
    if output.contains_key("mouse_axis") {
        Some(GyroOutput::Mouse)
    } else {
        match output.get("gamepad_axis")?.as_str()? {
            "left_x" | "left_y" => Some(GyroOutput::LeftStick),
            "right_x" | "right_y" => Some(GyroOutput::RightStick),
            _ => None,
        }
    }
}

fn legacy_activation(activation: Option<&serde_json::Value>) -> Option<GyroActivation> {
    let activation = activation?.as_object()?;
    for kind in ["hold", "toggle"] {
        if let Some(button) = activation.get(kind).and_then(|b| b.as_str()) {
            if let Ok(button) = serde_json::from_value::<GamepadButton>(button.into()) {
                return Some(if kind == "hold" {
                    GyroActivation::Hold(button)
                } else {
                    GyroActivation::Toggle(button)
                });
            }
        }
    }
    None
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
