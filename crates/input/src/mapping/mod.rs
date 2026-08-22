mod activators;
mod continuous;
mod flick;
mod modes;
mod sets;

use std::collections::HashMap;

use crate::gyro::GyroRates;
use crate::profile::{
    Activation, ChordMode, GamepadAxis, GamepadButton, GyroActivation, InputProfile, InputSource,
    MouseButton, OutputAction,
};
use activators::ActivatorStates;

pub(crate) const BUTTON_THRESHOLD: f32 = 0.5;
pub(crate) const VALUE_EPSILON: f32 = 0.0001;
/// Tick interval assumed before the first tick reports a real one.
pub(crate) const DEFAULT_TICK_INTERVAL: f32 = 1.0 / 250.0;

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
        axis: GamepadAxis,
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
        axis: crate::MouseAxis,
        value: f32,
    },
    /// One discrete wheel detent batch; the daemon routes it to the virtual
    /// mouse wheel accumulators.
    WheelClick {
        axis: crate::MouseAxis,
        amount: i32,
    },
}

pub struct MappingEngine {
    pub(crate) profile: InputProfile,
    pub(crate) values: HashMap<InputSource, f32>,
    pub(crate) toggles: HashMap<InputSource, bool>,
    pub(crate) chord_toggles: HashMap<Vec<InputSource>, bool>,
    pub(crate) last_outputs: Vec<f32>,
    /// Last emitted composed value per gamepad output axis. Several bindings
    /// (e.g. physical stick passthrough plus gyro) can target the same axis;
    /// their contributions are summed and this stores the last value emitted.
    pub(crate) axis_outputs: HashMap<GamepadAxis, f32>,
    /// Latest player-space rotation rates, fed by the gyro processor.
    pub(crate) gyro_rates: GyroRates,
    pub(crate) last_tick_us: Option<u64>,
    /// Action-set engine state: press-pattern tracking per source, the
    /// active set index, toggled layers, and releases waiting to be merged
    /// into the next event batch.
    pub(crate) activator_states: ActivatorStates,
    pub(crate) active_set: usize,
    pub(crate) toggled_layers: Vec<usize>,
    pub(crate) pending_releases: Vec<OutputEvent>,
    /// Dpad directions currently held by stick-as-dpad modes.
    pub(crate) mode_dpad_pressed: Vec<GamepadButton>,
    /// Flick Stick state per stick source (angle, in-flight flick).
    pub(crate) flick_states:
        std::collections::HashMap<InputSource, crate::mapping::flick::FlickState>,
}

impl MappingEngine {
    pub fn new(profile: InputProfile) -> Result<Self, String> {
        profile.validate()?;
        let last_outputs = vec![0.0; profile.bindings.len()];
        Ok(Self {
            profile,
            values: HashMap::new(),
            toggles: HashMap::new(),
            chord_toggles: HashMap::new(),
            last_outputs,
            axis_outputs: HashMap::new(),
            gyro_rates: GyroRates::default(),
            last_tick_us: None,
            activator_states: ActivatorStates::default(),
            active_set: 0,
            toggled_layers: Vec::new(),
            pending_releases: Vec::new(),
            mode_dpad_pressed: Vec::new(),
            flick_states: std::collections::HashMap::new(),
        })
    }

    pub fn profile(&self) -> &InputProfile {
        &self.profile
    }

    /// Discrete, event-driven outputs: buttons, keys, and mouse buttons.
    /// Continuous outputs (relative mouse motion, gyro-driven axes) are
    /// emitted by [`MappingEngine::tick`] instead, because they must progress
    /// even when no input events arrive (e.g. a held stick stops reporting).
    pub fn process(&mut self, event: InputEvent) -> Vec<OutputEvent> {
        let previous = self.values.insert(event.source, event.value).unwrap_or(0.0);
        if previous <= BUTTON_THRESHOLD && event.value > BUTTON_THRESHOLD {
            self.toggle_activations(event.source);
        }
        let mut output = Vec::new();
        if self.profile.action_sets.is_empty() {
            let computed = self.compute_values();
            let discrete: Vec<(usize, OutputAction)> = self
                .profile
                .bindings
                .iter()
                .enumerate()
                .filter(|(_, binding)| {
                    !matches!(
                        &binding.output,
                        OutputAction::GamepadAxis(_) | OutputAction::MouseAxis(_)
                    )
                })
                .map(|(index, binding)| (index, binding.output.clone()))
                .collect();
            for (index, action) in discrete {
                self.emit_changed(index, &action, computed[index], &mut output);
            }
            self.emit_axis_outputs(&computed, &mut output);
        } else {
            output.extend(self.run_activators(event.source, event.value, event.timestamp_us));
            output.extend(self.take_pending_releases());
        }
        output
    }

    pub fn reset(&mut self) -> Vec<OutputEvent> {
        self.values.clear();
        self.toggles.clear();
        self.chord_toggles.clear();
        self.gyro_rates = GyroRates::default();
        self.last_tick_us = None;
        self.active_set = 0;
        self.toggled_layers.clear();
        self.activator_states.clear();
        self.pending_releases.clear();
        self.mode_dpad_pressed.clear();
        self.flick_states.clear();
        let outputs: Vec<OutputAction> = self
            .profile
            .bindings
            .iter()
            .map(|binding| binding.output.clone())
            .collect();
        let mut output = Vec::new();
        for (index, action) in outputs.into_iter().enumerate() {
            match action {
                OutputAction::GamepadAxis(axis) => {
                    let previous = self.axis_outputs.remove(&axis).unwrap_or(0.0);
                    if changed(previous, 0.0) {
                        output.push(OutputEvent::GamepadAxis { axis, value: 0.0 });
                    }
                }
                OutputAction::MouseAxis(_) => {}
                discrete => self.emit_changed(index, &discrete, 0.0, &mut output),
            }
        }
        for axis in [
            GamepadAxis::LeftX,
            GamepadAxis::LeftY,
            GamepadAxis::RightX,
            GamepadAxis::RightY,
        ] {
            let previous = self.axis_outputs.remove(&axis).unwrap_or(0.0);
            if changed(previous, 0.0) {
                output.push(OutputEvent::GamepadAxis { axis, value: 0.0 });
            }
        }
        // Set-driven profiles can hold activator outputs anywhere; release
        // them bluntly by iterating every mapping in every set and layer.
        self.release_all_activator_states();
        output.extend(self.take_pending_releases());
        output
    }

    pub(crate) fn toggle_activations(&mut self, source: InputSource) {
        let set_toggle = self
            .profile
            .action_sets
            .iter()
            .flat_map(|set| set.inputs.iter())
            .chain(
                self.profile
                    .action_layers
                    .iter()
                    .flat_map(|layer| layer.inputs.iter()),
            )
            .any(|input| {
                input.activators.iter().any(|activator| {
                    matches!(&activator.activation, Activation::Toggle(activator) if *activator == source)
                })
            });
        let has_toggle = self.profile.bindings.iter().any(|binding| {
            matches!(&binding.activation, Activation::Toggle(activator) if *activator == source)
        }) || set_toggle
            || matches!(
                self.profile.gyro.activation,
                GyroActivation::Toggle(button) if InputSource::Button(button) == source
            );
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
                        && sources.contains(&source)
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

    pub(crate) fn activation_active(&self, activation: &Activation) -> bool {
        match activation {
            Activation::Always => true,
            Activation::Hold(source) => self.source_value(*source) > BUTTON_THRESHOLD,
            Activation::Toggle(source) => self.toggles.get(source).copied().unwrap_or(false),
            Activation::DisableWhile(source) => self.source_value(*source) <= BUTTON_THRESHOLD,
            Activation::Chord { sources, mode } => match mode {
                ChordMode::Hold => self.chord_active(sources),
                ChordMode::Toggle => self.chord_toggles.get(sources).copied().unwrap_or(false),
            },
            Activation::Analog {
                axis,
                condition,
                threshold,
            } => {
                let magnitude = self.source_value(InputSource::Axis(*axis)).abs();
                match condition {
                    crate::profile::AnalogCondition::AtRest => magnitude <= *threshold,
                    crate::profile::AnalogCondition::Active => magnitude > *threshold,
                    crate::profile::AnalogCondition::MaxedOut => magnitude >= 1.0 - *threshold,
                }
            }
        }
    }

    fn chord_active(&self, sources: &[InputSource]) -> bool {
        !sources.is_empty()
            && sources
                .iter()
                .all(|source| self.source_value(*source) > BUTTON_THRESHOLD)
    }

    pub(crate) fn source_value(&self, source: InputSource) -> f32 {
        let value = self.values.get(&source).copied().unwrap_or(0.0);
        match source {
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

    pub(crate) fn compute_values(&self) -> Vec<f32> {
        self.profile
            .bindings
            .iter()
            .map(|binding| {
                if !self.activation_active(&binding.activation) {
                    return 0.0;
                }
                let raw = self.source_value(binding.source);
                if matches!(&binding.output, OutputAction::MouseAxis(_)) {
                    // Mouse velocity may exceed the [-1, 1] input range once
                    // sensitivity is applied; the tick loop converts it to a
                    // per-tick delta.
                    binding.transform.apply_unbounded(raw)
                } else {
                    binding.transform.apply(raw)
                }
            })
            .collect()
    }

    pub(crate) fn emit_changed(
        &mut self,
        index: usize,
        output_action: &OutputAction,
        value: f32,
        output: &mut Vec<OutputEvent>,
    ) {
        if !changed(self.last_outputs[index], value) {
            return;
        }
        self.last_outputs[index] = value;
        match output_action {
            OutputAction::GamepadButton(button) => output.push(OutputEvent::GamepadButton {
                button: *button,
                pressed: value > BUTTON_THRESHOLD,
            }),
            OutputAction::GamepadAxis(_) => {
                // Gamepad axes are emitted compositely in emit_axis_outputs();
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
            OutputAction::MouseAxis(_) => {
                // Mouse motion is tick-driven; see continuous.rs.
            }
            OutputAction::WheelClick { axis, amount } => {
                // One detent per rising edge; release produces nothing.
                if value > BUTTON_THRESHOLD {
                    output.push(OutputEvent::WheelClick {
                        axis: *axis,
                        amount: *amount,
                    });
                }
            }
            // Engine-internal actions are consumed by the activator engine,
            // never emitted to virtual devices.
            OutputAction::SwitchActionSet(_)
            | OutputAction::EnableLayer { .. }
            | OutputAction::ModeShiftActivate { .. } => {}
        }
    }

    /// Composes every active contribution targeting each gamepad output axis
    /// — physical sticks, gyro deflection — into one value and emits changes.
    pub(crate) fn emit_axis_outputs(&mut self, computed: &[f32], output: &mut Vec<OutputEvent>) {
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
        self.add_gyro_axis_deflections(&mut totals, &mut order);
        self.add_mode_axis_deflections(&mut totals, &mut order);
        for axis in order {
            let value = totals[&axis].clamp(-1.0, 1.0);
            let previous = self.axis_outputs.get(&axis).copied().unwrap_or(0.0);
            if changed(previous, value) {
                self.axis_outputs.insert(axis, value);
                output.push(OutputEvent::GamepadAxis { axis, value });
            }
        }
    }
}

pub(crate) fn changed(previous: f32, current: f32) -> bool {
    previous.is_nan() || (previous - current).abs() > VALUE_EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AxisDirection, Binding, GamepadAxis};

    fn event(source: InputSource, value: f32) -> InputEvent {
        InputEvent {
            source,
            value,
            timestamp_us: 0,
        }
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
    fn test_wheel_click_fires_once_per_press() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Button(GamepadButton::A),
                OutputAction::WheelClick {
                    axis: crate::MouseAxis::Wheel,
                    amount: -1,
                },
            )],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::A), 1.0)),
            vec![OutputEvent::WheelClick {
                axis: crate::MouseAxis::Wheel,
                amount: -1,
            }]
        );
        // Release and a repeated press each produce exactly one detent.
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::A), 0.0)),
            Vec::<OutputEvent>::new()
        );
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::A), 1.0)),
            vec![OutputEvent::WheelClick {
                axis: crate::MouseAxis::Wheel,
                amount: -1,
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
                pressed: true
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Axis(GamepadAxis::LeftX), 0.0)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::DpadLeft,
                pressed: false
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
    fn test_mapping_engine_disable_while_restores_output_on_release() {
        let mut binding = Binding::new(
            InputSource::Axis(GamepadAxis::RightY),
            OutputAction::GamepadAxis(GamepadAxis::RightX),
        );
        binding.activation = Activation::DisableWhile(InputSource::Button(GamepadButton::Back));
        let profile = InputProfile {
            bindings: vec![binding],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert_eq!(
            engine.process(event(InputSource::Axis(GamepadAxis::RightY), 0.5)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.5,
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::Back), 1.0)),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.0,
            }]
        );
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::Back), 0.0)),
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
                    InputSource::Axis(GamepadAxis::LeftX),
                    OutputAction::GamepadAxis(GamepadAxis::RightX),
                ),
            ],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        engine.process(event(InputSource::Axis(GamepadAxis::RightX), 0.5));
        assert_eq!(
            engine.reset(),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.0,
            }]
        );
    }
}
