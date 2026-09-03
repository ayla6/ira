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

/// Whether a profile's gyro starts out engaged: Always and Suppress
/// (active-unless-held) are active from the start, so the engine must not
/// treat its first tick as a re-engagement and discard fresh rates.
fn initial_gyro_active(profile: &InputProfile) -> bool {
    profile.gyro.enabled
        && matches!(
            profile.gyro.activation,
            GyroActivation::Always | GyroActivation::Suppress(_)
        )
}

pub struct MappingEngine {
    pub(crate) profile: InputProfile,
    pub(crate) values: HashMap<InputSource, f32>,
    pub(crate) toggles: HashMap<InputSource, bool>,
    pub(crate) chord_toggles: HashMap<Vec<InputSource>, bool>,
    /// Last emitted composed value per gamepad output axis. Several bindings
    /// (e.g. physical stick passthrough plus gyro) can target the same axis;
    /// their contributions are summed and this stores the last value emitted.
    pub(crate) axis_outputs: HashMap<GamepadAxis, f32>,
    /// Latest player-space rotation rates, fed by the gyro processor.
    pub(crate) gyro_rates: GyroRates,
    /// Tick-clock time of the most recent `update_gyro` call, so rates that
    /// stopped being refreshed (a sensor that went quiet) can age out.
    pub(crate) last_gyro_sample_us: Option<u64>,
    /// Whether the gyro was active at the last refresh; its rising edge
    /// clears carried rates so re-engaging starts from rest.
    pub(crate) gyro_was_active: bool,
    /// Laser Pointer angle deltas drained from the gyro processor since
    /// the last tick, emitted directly as cursor position deltas.
    pub(crate) laser_yaw_delta: f32,
    pub(crate) laser_pitch_delta: f32,
    /// Rates the output paths consume: live rates while the gyro is active,
    /// the momentum glide while it decays, zero otherwise.
    pub(crate) gyro_effective: GyroRates,
    /// Carried rates powering the momentum glide after deactivation.
    pub(crate) gyro_momentum: GyroRates,
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
    /// Stick deadzones calibrated for the connected controller's two
    /// sticks; Joystick modes with
    /// [`crate::profile::StickDeadzone::Controller`] read the value for
    /// their stick.
    pub(crate) controller_deadzone_left: f32,
    pub(crate) controller_deadzone_right: f32,
    /// Outer Ring Commands currently held, per stick (the X axis identifies
    /// the pair), with the output to release.
    pub(crate) outer_ring_pressed: Vec<(GamepadAxis, OutputAction)>,
}

impl MappingEngine {
    pub fn new(profile: InputProfile) -> Result<Self, String> {
        profile.validate()?;
        let gyro_was_active = initial_gyro_active(&profile);
        Ok(Self {
            profile,
            values: HashMap::new(),
            toggles: HashMap::new(),
            chord_toggles: HashMap::new(),
            axis_outputs: HashMap::new(),
            gyro_rates: GyroRates::default(),
            last_gyro_sample_us: None,
            gyro_was_active,
            laser_yaw_delta: 0.0,
            laser_pitch_delta: 0.0,
            gyro_effective: GyroRates::default(),
            gyro_momentum: GyroRates::default(),
            last_tick_us: None,
            activator_states: ActivatorStates::default(),
            active_set: 0,
            toggled_layers: Vec::new(),
            pending_releases: Vec::new(),
            mode_dpad_pressed: Vec::new(),
            flick_states: std::collections::HashMap::new(),
            controller_deadzone_left: 0.0,
            controller_deadzone_right: 0.0,
            outer_ring_pressed: Vec::new(),
        })
    }

    /// Store the stick deadzones calibrated for the connected controller's
    /// left and right sticks. Joystick modes whose deadzone source is
    /// `Controller` scale their raw input by their stick's value.
    pub fn set_controller_deadzones(&mut self, left: f32, right: f32) {
        self.controller_deadzone_left = left.clamp(0.0, 0.9);
        self.controller_deadzone_right = right.clamp(0.0, 0.9);
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
        // Between ticks the carried momentum does not decay (dt 0); this
        // only refreshes which rates the output paths see.
        self.refresh_gyro_effective(0.0);
        let mut output = Vec::new();
        output.extend(self.run_activators(event.source, event.value, event.timestamp_us));
        output.extend(self.take_pending_releases());
        output
    }

    pub fn reset(&mut self) -> Vec<OutputEvent> {
        self.values.clear();
        self.toggles.clear();
        self.chord_toggles.clear();
        self.gyro_rates = GyroRates::default();
        self.last_gyro_sample_us = None;
        self.gyro_was_active = initial_gyro_active(&self.profile);
        self.laser_yaw_delta = 0.0;
        self.laser_pitch_delta = 0.0;
        self.gyro_effective = GyroRates::default();
        self.gyro_momentum = GyroRates::default();
        self.last_tick_us = None;
        self.active_set = 0;
        self.toggled_layers.clear();
        // Held activator outputs must release before the state is wiped,
        // otherwise a held button sticks on across profile switches.
        self.pending_releases.clear();
        self.release_all_activator_states();
        self.activator_states.clear();
        self.mode_dpad_pressed.clear();
        self.flick_states.clear();
        self.outer_ring_pressed.clear();
        let mut output = Vec::new();
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
        let has_toggle = set_toggle
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
            .action_sets
            .iter()
            .flat_map(|set| set.inputs.iter())
            .chain(
                self.profile
                    .action_layers
                    .iter()
                    .flat_map(|layer| layer.inputs.iter()),
            )
            .flat_map(|input| input.activators.iter())
            .filter_map(|activator| match &activator.activation {
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

    /// Composes every active contribution targeting each gamepad output axis
    /// — gyro deflection, mode-driven sticks — into one value and emits
    /// changes.
    pub(crate) fn emit_axis_outputs(&mut self, output: &mut Vec<OutputEvent>) {
        let mut totals: HashMap<GamepadAxis, f32> = HashMap::new();
        let mut order: Vec<GamepadAxis> = Vec::new();
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
    use crate::profile::{ActionSet, InputMapping, SourceMode};
    use crate::GamepadAxis;

    fn event(source: InputSource, value: f32) -> InputEvent {
        InputEvent {
            source,
            value,
            timestamp_us: 0,
        }
    }

    fn button_profile(source: GamepadButton, output: OutputAction) -> InputProfile {
        InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping::simple(InputSource::Button(source), output)],
            }],
            ..InputProfile::default()
        }
    }

    #[test]
    fn test_mapping_engine_maps_button_press_and_release() {
        let mut engine = MappingEngine::new(button_profile(
            GamepadButton::A,
            OutputAction::GamepadButton(GamepadButton::B),
        ))
        .unwrap();

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
        let mut engine = MappingEngine::new(button_profile(
            GamepadButton::B,
            OutputAction::GamepadButton(GamepadButton::A),
        ))
        .unwrap();
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
        let mut engine = MappingEngine::new(button_profile(
            GamepadButton::A,
            OutputAction::WheelClick {
                axis: crate::MouseAxis::Wheel,
                amount: -1,
            },
        ))
        .unwrap();
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
    fn test_mapping_engine_toggle_activation_flips_on_press() {
        let mut mapping = InputMapping::simple(
            InputSource::Button(GamepadButton::A),
            OutputAction::GamepadButton(GamepadButton::B),
        );
        mapping.activators[0].activation =
            Activation::Toggle(InputSource::Button(GamepadButton::Guide));
        let profile = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![mapping],
            }],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        // Toggle starts off: the press does nothing, and its release is
        // swallowed too.
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::A), 1.0))
            .is_empty());
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::A), 0.0))
            .is_empty());
        // Guide flips the toggle on without firing anything itself.
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::Guide), 1.0))
            .is_empty());
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
        // A second Guide press flips the toggle back off.
        engine.process(event(InputSource::Button(GamepadButton::Guide), 0.0));
        engine.process(event(InputSource::Button(GamepadButton::Guide), 1.0));
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::A), 1.0))
            .is_empty());
    }

    #[test]
    fn test_mapping_engine_disable_while_restores_output_on_release() {
        let mut mapping = InputMapping::simple(
            InputSource::Button(GamepadButton::A),
            OutputAction::GamepadButton(GamepadButton::B),
        );
        mapping.activators[0].activation =
            Activation::DisableWhile(InputSource::Button(GamepadButton::Back));
        let profile = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![mapping],
            }],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();

        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::A), 1.0)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true,
            }]
        );
        // Holding Back disables the mapping; the held output releases on the
        // next tick and further presses are swallowed.
        engine.process(event(InputSource::Button(GamepadButton::Back), 1.0));
        assert_eq!(
            engine.tick(4_000),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: false,
            }]
        );
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::A), 1.0))
            .is_empty());
        // The swallowed press ends when the physical button releases; the
        // gate being closed means nothing emits.
        assert!(engine
            .process(event(InputSource::Button(GamepadButton::A), 0.0))
            .is_empty());
        // Releasing Back restores the mapping.
        engine.process(event(InputSource::Button(GamepadButton::Back), 0.0));
        assert_eq!(
            engine.process(event(InputSource::Button(GamepadButton::A), 1.0)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true,
            }]
        );
    }

    #[test]
    fn test_reset_emits_single_zero_for_composed_axis() {
        let profile = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping {
                    mode: Some(SourceMode::joystick(crate::profile::StickOutput::Right)),
                    ..InputMapping::new(InputSource::Axis(GamepadAxis::RightX))
                }],
            }],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(event(InputSource::Axis(GamepadAxis::RightX), 0.5));
        engine.tick(4_000);
        assert_eq!(
            engine.reset(),
            vec![OutputEvent::GamepadAxis {
                axis: GamepadAxis::RightX,
                value: 0.0,
            }]
        );
    }
}
