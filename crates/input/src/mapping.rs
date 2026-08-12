use std::collections::HashMap;

use crate::profile::{
    Activation, ChordMode, GamepadAxis, GamepadButton, InputProfile, InputSource, MouseAxis,
    MouseButton, OutputAction, RecenterMode,
};

const BUTTON_THRESHOLD: f32 = 0.5;
const VALUE_EPSILON: f32 = 0.0001;
/// SDL3 reports gyro as angular velocity in radians per second, but a gamepad
/// stick axis is a [-1, 1] deflection. Without scaling, a realistic rotation
/// (1-3 rad/s for an ordinary hand turn) slams the stick to full deflection.
/// Scale rad/s down so a brisk rotation of `GYRO_STICK_RADS_PER_UNIT` rad/s
/// drives the stick to full range; the binding's sensitivity still applies.
const GYRO_STICK_RADS_PER_UNIT: f32 = 4.0;
/// Position mode integrates angle. A 45-degree wrist rotation reaches full
/// deflection so ordinary movement clears typical in-game stick dead zones.
const GYRO_POSITION_RADS_PER_UNIT: f32 = std::f32::consts::FRAC_PI_4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputEvent {
    pub source: InputSource,
    pub value: f32,
    pub timestamp_us: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OutputEvent {
    GamepadButton {
        button: GamepadButton,
        pressed: bool,
    },
    GamepadAxis {
        axis: crate::GamepadAxis,
        value: f32,
    },
    Key {
        keycode: u16,
        pressed: bool,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    MouseMotion {
        axis: MouseAxis,
        value: f32,
    },
    RecenterGyro,
}

pub struct MappingEngine {
    profile: InputProfile,
    values: HashMap<InputSource, f32>,
    toggles: HashMap<InputSource, bool>,
    chord_toggles: HashMap<Vec<InputSource>, bool>,
    last_outputs: Vec<f32>,
    /// Last emitted composed value per gamepad output axis. Several bindings
    /// (e.g. physical stick passthrough plus gyro) can target the same axis;
    /// their contributions are summed and this stores the last value emitted.
    axis_outputs: HashMap<GamepadAxis, f32>,
    gyro_positions: Vec<f32>,
    last_gyro_timestamp: Option<u64>,
    last_gyro_delta: f32,
}

impl MappingEngine {
    pub fn new(profile: InputProfile) -> Result<Self, String> {
        profile.validate()?;
        let binding_count = profile.bindings.len();
        let last_outputs = vec![0.0; binding_count];
        Ok(Self {
            profile,
            values: HashMap::new(),
            toggles: HashMap::new(),
            chord_toggles: HashMap::new(),
            last_outputs,
            axis_outputs: HashMap::new(),
            gyro_positions: vec![0.0; binding_count],
            last_gyro_timestamp: None,
            last_gyro_delta: 1.0 / 250.0,
        })
    }

    pub fn process(&mut self, event: InputEvent) -> Vec<OutputEvent> {
        let gyro_delta = match event.source {
            InputSource::Gyro(_) => Some(self.gyro_delta(event.timestamp_us)),
            _ => None,
        };
        let previous_activation: Vec<bool> = self
            .profile
            .bindings
            .iter()
            .map(|binding| self.activation_active(&binding.activation))
            .collect();
        let previous = self.values.insert(event.source, event.value).unwrap_or(0.0);
        if previous <= BUTTON_THRESHOLD && event.value > BUTTON_THRESHOLD {
            self.toggle_activations(event.source);
        }

        let mut output = Vec::new();
        let mut should_recenter = false;
        for (index, binding) in self.profile.bindings.iter().enumerate() {
            let active = self.activation_active(&binding.activation);
            if recenter_on_transition(binding.recenter, previous_activation[index], active) {
                should_recenter = true;
            }
            if matches!(&binding.output, OutputAction::RecenterGyro)
                && source_matches(event.source, binding.source)
                && previous <= BUTTON_THRESHOLD
                && event.value > BUTTON_THRESHOLD
                && active
            {
                should_recenter = true;
            }
        }
        if should_recenter {
            self.gyro_positions.fill(0.0);
            output.push(OutputEvent::RecenterGyro);
        }
        let computed = self.compute_values(event.source, gyro_delta);
        let emissions: Vec<(usize, OutputAction, bool, f32)> = self
            .profile
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, binding)| {
                !matches!(
                    &binding.output,
                    OutputAction::RecenterGyro | OutputAction::GamepadAxis(_)
                )
            })
            .map(|(index, binding)| {
                (
                    index,
                    binding.output.clone(),
                    source_matches(event.source, binding.source),
                    computed[index],
                )
            })
            .collect();
        for (index, output_action, source_changed, value) in emissions {
            self.emit_changed(index, &output_action, value, source_changed, &mut output);
        }
        self.emit_gamepad_axes(&computed, &mut output);
        output
    }

    pub fn profile(&self) -> &InputProfile {
        &self.profile
    }

    pub fn reset(&mut self) -> Vec<OutputEvent> {
        self.values.clear();
        self.toggles.clear();
        self.chord_toggles.clear();
        self.gyro_positions.fill(0.0);
        self.last_gyro_timestamp = None;
        let mut output = Vec::new();
        for index in 0..self.profile.bindings.len() {
            let output_action = self.profile.bindings[index].output.clone();
            match output_action {
                OutputAction::RecenterGyro => {}
                OutputAction::GamepadAxis(axis) => {
                    let previous = self.axis_outputs.remove(&axis).unwrap_or(0.0);
                    if changed(previous, 0.0) {
                        output.push(OutputEvent::GamepadAxis { axis, value: 0.0 });
                    }
                }
                _ => self.emit_changed(index, &output_action, 0.0, true, &mut output),
            }
        }
        output
    }

    fn toggle_activations(&mut self, source: InputSource) {
        let has_toggle = self.profile.bindings.iter().any(|binding| {
            matches!(&binding.activation, Activation::Toggle(activator) if source_matches(source, *activator))
        });
        if has_toggle {
            let enabled = self.toggles.entry(source).or_insert(false);
            *enabled = !*enabled;
        }
        let chords: Vec<Vec<InputSource>> = self
            .profile
            .bindings
            .iter()
            .filter_map(|binding| match &binding.activation {
                Activation::Chord { sources, mode }
                    if *mode == ChordMode::Toggle
                        && sources
                            .iter()
                            .any(|candidate| source_matches(source, *candidate))
                        && self.chord_active(sources) =>
                {
                    Some(sources.clone())
                }
                _ => None,
            })
            .collect();
        for chord in chords {
            let enabled = self.chord_toggles.entry(chord).or_insert(false);
            *enabled = !*enabled;
        }
    }

    fn activation_active(&self, activation: &Activation) -> bool {
        match activation {
            Activation::Always => true,
            Activation::Hold(source) => self.source_value(*source) > BUTTON_THRESHOLD,
            Activation::Toggle(source) => self.toggles.get(source).copied().unwrap_or(false),
            Activation::DisableWhile(source) => self.source_value(*source) <= BUTTON_THRESHOLD,
            Activation::Chord { sources, mode } => match mode {
                ChordMode::Hold => self.chord_active(sources),
                ChordMode::Toggle => self.chord_toggles.get(sources).copied().unwrap_or(false),
            },
        }
    }

    fn chord_active(&self, sources: &[InputSource]) -> bool {
        !sources.is_empty()
            && sources
                .iter()
                .all(|source| self.source_value(*source) > BUTTON_THRESHOLD)
    }

    fn source_value(&self, source: InputSource) -> f32 {
        let value = self.values.get(&source).copied().unwrap_or(0.0);
        match source {
            InputSource::Gyro(axis) => value - self.profile.gyro_calibration.axis_value(axis),
            InputSource::AxisDirection { axis, direction } => {
                let value = self.source_value(InputSource::Axis(axis));
                match direction {
                    crate::AxisDirection::Negative => (-value).max(0.0),
                    crate::AxisDirection::Positive => value.max(0.0),
                }
            }
            _ => value,
        }
    }

    fn compute_values(&mut self, event_source: InputSource, gyro_delta: Option<f32>) -> Vec<f32> {
        let mut computed = Vec::with_capacity(self.profile.bindings.len());
        for (index, binding) in self.profile.bindings.iter().enumerate() {
            if matches!(&binding.output, OutputAction::RecenterGyro)
                || !self.activation_active(&binding.activation)
            {
                computed.push(0.0);
                continue;
            }
            let raw = self.source_value(binding.source);
            let mut value = if matches!(&binding.output, OutputAction::MouseAxis(_)) {
                let value = binding.transform.apply_unbounded(raw);
                if matches!(binding.source, InputSource::Gyro(_)) {
                    value * gyro_delta.unwrap_or(self.last_gyro_delta)
                } else {
                    value
                }
            } else if matches!(binding.source, InputSource::Gyro(_)) {
                binding.transform.apply(raw / GYRO_STICK_RADS_PER_UNIT)
            } else {
                binding.transform.apply(raw)
            };
            if binding.gyro_mode == crate::GyroMode::HoldLast
                && matches!(binding.source, InputSource::Gyro(_))
                && matches!(&binding.output, OutputAction::GamepadAxis(_))
            {
                if source_matches(event_source, binding.source) {
                    let step = binding
                        .transform
                        .apply_unbounded(raw / GYRO_POSITION_RADS_PER_UNIT)
                        * gyro_delta.unwrap_or(self.last_gyro_delta);
                    self.gyro_positions[index] =
                        (self.gyro_positions[index] + step).clamp(-1.0, 1.0);
                }
                value = self.gyro_positions[index];
            }
            computed.push(value);
        }
        computed
    }

    /// Composes every active binding targeting a given gamepad output axis into
    /// a single value. A physical stick contributes its own deflection and a
    /// gyro binding contributes its scaled rate; summing them gives additive,
    /// deterministic control instead of the last-written binding zeroing the
    /// other (which happened when each binding emitted to the axis directly).
    fn emit_gamepad_axes(&mut self, computed: &[f32], output: &mut Vec<OutputEvent>) {
        let mut totals: HashMap<GamepadAxis, f32> = HashMap::new();
        let mut order: Vec<GamepadAxis> = Vec::new();
        for (index, binding) in self.profile.bindings.iter().enumerate() {
            if let OutputAction::GamepadAxis(axis) = &binding.output {
                let axis = *axis;
                if !totals.contains_key(&axis) {
                    order.push(axis);
                }
                *totals.entry(axis).or_insert(0.0) += computed[index];
            }
        }
        for axis in order {
            let value = totals[&axis].clamp(-1.0, 1.0);
            let previous = self.axis_outputs.get(&axis).copied().unwrap_or(0.0);
            if changed(previous, value) {
                self.axis_outputs.insert(axis, value);
                output.push(OutputEvent::GamepadAxis { axis, value });
            }
        }
    }

    fn gyro_delta(&mut self, timestamp_us: u64) -> f32 {
        if timestamp_us == 0 {
            return 1.0;
        }
        if self.last_gyro_timestamp == Some(timestamp_us) {
            return self.last_gyro_delta;
        }
        let delta = self
            .last_gyro_timestamp
            .map(|previous| timestamp_us.saturating_sub(previous) as f32 / 1_000_000.0)
            .unwrap_or(1.0 / 250.0)
            .clamp(0.0005, 0.05);
        self.last_gyro_timestamp = Some(timestamp_us);
        self.last_gyro_delta = delta;
        delta
    }

    fn emit_changed(
        &mut self,
        index: usize,
        output_action: &OutputAction,
        value: f32,
        source_changed: bool,
        output: &mut Vec<OutputEvent>,
    ) {
        let is_mouse_axis = matches!(output_action, OutputAction::MouseAxis(_));
        if is_mouse_axis && !source_changed {
            return;
        }
        if !is_mouse_axis && !changed(self.last_outputs[index], value) {
            return;
        }
        self.last_outputs[index] = value;
        match output_action {
            OutputAction::GamepadButton(button) => output.push(OutputEvent::GamepadButton {
                button: *button,
                pressed: value > BUTTON_THRESHOLD,
            }),
            OutputAction::GamepadAxis(_) => {
                // Gamepad axes are emitted compositely in emit_gamepad_axes();
                // this arm is never reached from process()/reset().
            }
            OutputAction::Keyboard { keycode } => output.push(OutputEvent::Key {
                keycode: *keycode,
                pressed: value > BUTTON_THRESHOLD,
            }),
            OutputAction::MouseButton(button) => output.push(OutputEvent::MouseButton {
                button: *button,
                pressed: value > BUTTON_THRESHOLD,
            }),
            OutputAction::MouseAxis(axis) => {
                if value.abs() > VALUE_EPSILON {
                    output.push(OutputEvent::MouseMotion { axis: *axis, value });
                }
            }
            OutputAction::RecenterGyro => {}
        }
    }
}

fn recenter_on_transition(mode: RecenterMode, was_active: bool, is_active: bool) -> bool {
    if was_active == is_active {
        return false;
    }
    match mode {
        RecenterMode::Never => false,
        RecenterMode::OnEnable => !was_active && is_active,
        RecenterMode::OnDisable => was_active && !is_active,
        RecenterMode::OnEnableOrDisable => true,
    }
}

fn changed(previous: f32, current: f32) -> bool {
    previous.is_nan() || (previous - current).abs() > VALUE_EPSILON
}

fn source_matches(event: InputSource, binding: InputSource) -> bool {
    event == binding
        || matches!(
            (event, binding),
            (
                InputSource::Axis(event_axis),
                InputSource::AxisDirection { axis: binding_axis, .. }
            ) if event_axis == binding_axis
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AxisDirection, Binding, GamepadAxis, GyroAxis, RecenterMode};

    fn event(source: InputSource, value: f32) -> InputEvent {
        InputEvent {
            source,
            value,
            timestamp_us: 0,
        }
    }

    fn event_at(source: InputSource, value: f32, timestamp_us: u64) -> InputEvent {
        InputEvent {
            source,
            value,
            timestamp_us,
        }
    }

    fn assert_axis_value(events: Vec<OutputEvent>, expected: f32) {
        assert_eq!(events.len(), 1);
        let Some(OutputEvent::GamepadAxis { value, .. }) = events.first() else {
            panic!("expected one gamepad axis output");
        };
        assert!((*value - expected).abs() < 0.00001);
    }

    #[test]
    fn test_mapping_engine_maps_button_press_and_release() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Button(GamepadButton::A),
                OutputAction::GamepadButton(GamepadButton::B),
            )],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::A), 1.0)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::A), 0.0)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: false
            }]
        );
    }

    #[test]
    fn test_mapping_engine_reset_releases_active_outputs() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Button(GamepadButton::B),
                OutputAction::GamepadButton(GamepadButton::A),
            )],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::B), 1.0)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::A,
                pressed: true,
            }]
        );
        assert_eq!(
            engine.reset(),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::A,
                pressed: false,
            }]
        );
    }

    #[test]
    fn test_mapping_engine_maps_negative_axis_direction_to_button() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::AxisDirection {
                    axis: GamepadAxis::LeftX,
                    direction: AxisDirection::Negative,
                },
                OutputAction::GamepadButton(GamepadButton::DpadLeft),
            )],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        assert_eq!(
            engine.process(event(InputSource::Axis(GamepadAxis::LeftX), -1.0)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::DpadLeft,
                pressed: true,
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Axis(GamepadAxis::LeftX), 0.0)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::DpadLeft,
                pressed: false,
            }]
        );
    }

    #[test]
    fn test_mapping_engine_toggle_activation_flips_on_press() {
        let mut binding = Binding::new(
            InputSource::Axis(GamepadAxis::RightX),
            OutputAction::GamepadAxis(GamepadAxis::RightY),
        );
        binding.activation = Activation::Toggle(InputSource::Button(GamepadButton::Guide));
        let profile = InputProfile {
            bindings: vec![binding],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert!(engine
            .process(event(InputSource::Axis(GamepadAxis::RightX), 0.5))
            .is_empty());
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::Guide), 1.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightY,
                value: 0.5
            }]
        );
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::Guide), 0.0))
            .is_empty());
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::Guide), 1.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightY,
                value: 0.0
            }]
        );
    }

    #[test]
    fn test_mapping_engine_recenter_emits_on_rising_edge() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Button(GamepadButton::Back),
                OutputAction::RecenterGyro,
            )],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::Back), 1.0)),
            vec![OutputEvent::RecenterGyro]
        );
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::Back), 1.0))
            .is_empty());
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::A), 1.0))
            .is_empty());
    }

    #[test]
    fn test_mapping_engine_rate_gyro_recenter_does_not_create_offset() {
        let mut binding = Binding::new(
            InputSource::Gyro(GyroAxis::X),
            OutputAction::GamepadAxis(GamepadAxis::LeftX),
        );
        binding.activation = Activation::Hold(InputSource::Button(GamepadButton::A));
        binding.recenter = RecenterMode::OnEnableOrDisable;
        let profile = InputProfile {
            bindings: vec![binding],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert!(engine
            .process(event(InputSource::Gyro(GyroAxis::X), 0.5))
            .is_empty());
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::A), 1.0)),
            vec![
                OutputEvent::RecenterGyro,
                OutputEvent::GamepadAxis {
                    axis: GamepadAxis::LeftX,
                    value: 0.125,
                },
            ]
        );
        assert_eq!(
            engine.process(event(InputSource::Gyro(GyroAxis::X), 0.75)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                value: 0.1875,
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::A), 0.0)),
            vec![
                OutputEvent::RecenterGyro,
                OutputEvent::GamepadAxis {
                    axis: GamepadAxis::LeftX,
                    value: 0.0,
                },
            ]
        );
    }

    #[test]
    fn test_mapping_engine_disable_while_restores_gyro_output_on_release() {
        let mut binding = Binding::new(
            InputSource::Gyro(GyroAxis::Y),
            OutputAction::GamepadAxis(GamepadAxis::RightY),
        );
        binding.activation = Activation::DisableWhile(InputSource::Button(GamepadButton::Back));
        let profile = InputProfile {
            bindings: vec![binding],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert_eq!(
            engine.process(event(InputSource::Gyro(GyroAxis::Y), 2.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightY,
                value: 0.5,
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::Back), 1.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightY,
                value: 0.0,
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::Back), 0.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightY,
                value: 0.5,
            }]
        );
    }

    #[test]
    fn test_gyro_to_gamepad_scales_realistic_rad_per_sec() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Gyro(GyroAxis::X),
                OutputAction::GamepadAxis(GamepadAxis::RightX),
            )],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert_eq!(
            engine.process(event(InputSource::Gyro(GyroAxis::X), 0.5)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.125,
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Gyro(GyroAxis::X), 1.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.25,
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Gyro(GyroAxis::X), 8.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 1.0,
            }]
        );
    }

    #[test]
    fn test_gyro_to_gamepad_preserves_sign() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Gyro(GyroAxis::Y),
                OutputAction::GamepadAxis(GamepadAxis::RightX),
            )],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert_eq!(
            engine.process(event(InputSource::Gyro(GyroAxis::Y), -2.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: -0.5,
            }]
        );
    }

    #[test]
    fn test_gyro_position_moves_stick_and_keeps_position() {
        let mut binding = Binding::new(
            InputSource::Gyro(GyroAxis::X),
            OutputAction::GamepadAxis(GamepadAxis::RightX),
        );
        binding.gyro_mode = crate::GyroMode::HoldLast;
        let profile = InputProfile {
            bindings: vec![binding],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert_axis_value(
            engine.process(event_at(
                InputSource::Gyro(GyroAxis::X),
                std::f32::consts::FRAC_PI_4,
                1_000_000,
            )),
            0.004,
        );
        assert_axis_value(
            engine.process(event_at(
                InputSource::Gyro(GyroAxis::X),
                std::f32::consts::FRAC_PI_4,
                1_010_000,
            )),
            0.014,
        );
        assert!(engine
            .process(event_at(InputSource::Gyro(GyroAxis::X), 0.0, 1_020_000))
            .is_empty());
        assert_axis_value(
            engine.process(event_at(
                InputSource::Gyro(GyroAxis::X),
                -std::f32::consts::FRAC_PI_4,
                1_030_000,
            )),
            0.004,
        );
    }

    #[test]
    fn test_stick_and_gyro_compose_on_shared_axis() {
        let profile = InputProfile {
            bindings: vec![
                Binding::new(
                    InputSource::Axis(GamepadAxis::RightX),
                    OutputAction::GamepadAxis(GamepadAxis::RightX),
                ),
                Binding::new(
                    InputSource::Gyro(GyroAxis::Y),
                    OutputAction::GamepadAxis(GamepadAxis::RightX),
                ),
            ],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert_eq!(
            engine.process(event(InputSource::Axis(GamepadAxis::RightX), 0.25)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.25,
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Gyro(GyroAxis::Y), 0.5)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.375,
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Gyro(GyroAxis::Y), 0.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.25,
            }]
        );
    }

    #[test]
    fn test_toggle_off_removes_only_gyro_contribution() {
        let mut gyro_binding = Binding::new(
            InputSource::Gyro(GyroAxis::Y),
            OutputAction::GamepadAxis(GamepadAxis::RightX),
        );
        gyro_binding.activation = Activation::Toggle(InputSource::Button(GamepadButton::Guide));
        let profile = InputProfile {
            bindings: vec![
                Binding::new(
                    InputSource::Axis(GamepadAxis::RightX),
                    OutputAction::GamepadAxis(GamepadAxis::RightX),
                ),
                gyro_binding,
            ],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        engine.process(event(InputSource::Axis(GamepadAxis::RightX), 0.5));
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::Guide), 1.0))
            .is_empty());
        assert_eq!(
            engine.process(event(InputSource::Gyro(GyroAxis::Y), 0.5)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.625,
            }]
        );
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::Guide), 0.0))
            .is_empty());
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::Guide), 1.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.5,
            }]
        );
    }

    #[test]
    fn test_reset_emits_single_zero_for_composed_axis() {
        let profile = InputProfile {
            bindings: vec![
                Binding::new(
                    InputSource::Axis(GamepadAxis::RightX),
                    OutputAction::GamepadAxis(GamepadAxis::RightX),
                ),
                Binding::new(
                    InputSource::Gyro(GyroAxis::Y),
                    OutputAction::GamepadAxis(GamepadAxis::RightX),
                ),
            ],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        engine.process(event(InputSource::Axis(GamepadAxis::RightX), 0.5));
        engine.process(event(InputSource::Gyro(GyroAxis::Y), 0.5));
        assert_eq!(
            engine.reset(),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.0,
            }]
        );
    }
}
