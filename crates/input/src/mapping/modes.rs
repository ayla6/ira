//! Mode-driven analog outputs for action-set profiles.
//!
//! In the set model a stick or trigger expresses its analog behavior through
//! a [`SourceMode`] instead of per-axis bindings: joystick with deadzones and
//! curve, mouse velocity, or four digital dpad directions. All of them are
//! tick-driven (positions must keep producing output while held) and
//! integrate into the same axis-composition and mouse-motion paths as the
//! legacy bindings.

use super::continuous::STICK_MOUSE_COUNTS_PER_SECOND;
use super::{MappingEngine, OutputEvent, VALUE_EPSILON};
use crate::profile::{
    GamepadAxis, GamepadButton, InputSource, JoystickSettings, MouseAxis, OuterRingCommand,
    OutputAction, ResponseAxisStyle, SourceMode, StickDeadzone, StickOutput, StickOutputAxis,
    StickProcessing,
};

impl MappingEngine {
    /// Collect the mode-driven mappings that currently apply (active set +
    /// layers), with mode shifts replacing the mode while held. Layers come
    /// first so "first wins" consumers resolve overrides the way
    /// `resolve_mapping` documents: active layers beat the base set.
    fn mode_inputs(&self) -> Vec<(InputSource, SourceMode)> {
        let mut result = Vec::new();
        let mut push = |mapping: &crate::profile::InputMapping| {
            let mode = match self.active_shift(mapping) {
                Some(shift) => shift.mode.clone().or_else(|| mapping.mode.clone()),
                None => mapping.mode.clone(),
            };
            if let Some(mode) = mode {
                result.push((mapping.source, mode));
            }
        };
        for index in self.active_layer_indexes() {
            if let Some(layer) = self.profile.action_layers.get(index) {
                for mapping in &layer.inputs {
                    push(mapping);
                }
            }
        }
        if let Some(set) = self.profile.action_sets.get(self.active_set) {
            for mapping in &set.inputs {
                push(mapping);
            }
        }
        result
    }

    /// Axis-pair contributions from Joystick/Dpad modes, folded into the
    /// composition totals during emit_axis_outputs.
    pub(crate) fn add_mode_axis_deflections(
        &self,
        totals: &mut std::collections::HashMap<GamepadAxis, f32>,
        order: &mut Vec<GamepadAxis>,
    ) {
        // Stick pairs whose Joystick mode already applied: older profiles
        // carry the same mode on both axes of a stick, and the pair is one
        // input — it must contribute once, not twice.
        let mut applied_sticks: Vec<(GamepadAxis, GamepadAxis)> = Vec::new();
        for (source, mode) in self.mode_inputs() {
            match mode {
                SourceMode::Joystick(settings) => {
                    let JoystickSettings { output, processing } = settings;
                    let (x_axis, y_axis) = stick_axes(source);
                    if applied_sticks.contains(&(x_axis, y_axis)) {
                        continue;
                    }
                    let Some((x, y)) = self.stick_pair(x_axis, y_axis) else {
                        continue;
                    };
                    applied_sticks.push((x_axis, y_axis));
                    let (x, y) = apply_stick_processing(
                        processing,
                        x,
                        y,
                        self.controller_deadzone_for(x_axis),
                    );
                    let (target_x, target_y) = match output {
                        StickOutput::Left => (GamepadAxis::LeftX, GamepadAxis::LeftY),
                        StickOutput::Right => (GamepadAxis::RightX, GamepadAxis::RightY),
                    };
                    for (axis, value) in [(target_x, x), (target_y, y)] {
                        if !totals.contains_key(&axis) {
                            order.push(axis);
                        }
                        *totals.entry(axis).or_insert(0.0) += value;
                    }
                }
                SourceMode::Trigger { threshold } => {
                    let Some(axis) = trigger_axis(source) else {
                        continue;
                    };
                    let value = self.source_value(source);
                    // Below threshold the trigger is fully released; emit
                    // 0 so a previous above-threshold value cannot stay
                    // latched on the virtual pad.
                    let deflection = if value <= threshold {
                        0.0
                    } else {
                        // Rescale so the threshold reads as zero and full
                        // pull saturates.
                        ((value - threshold) / (1.0 - threshold).max(VALUE_EPSILON)).clamp(0.0, 1.0)
                    };
                    if !totals.contains_key(&axis) {
                        order.push(axis);
                    }
                    *totals.entry(axis).or_insert(0.0) += deflection;
                }
                _ => {}
            }
        }
    }

    /// The calibrated deadzone of the physical stick a pair belongs to;
    /// backs the `Controller` deadzone source.
    fn controller_deadzone_for(&self, x_axis: GamepadAxis) -> f32 {
        match x_axis {
            GamepadAxis::RightX | GamepadAxis::RightY => self.controller_deadzone_right,
            _ => self.controller_deadzone_left,
        }
    }

    /// Relative mouse motion from Mouse-mode sticks, emitted per tick.
    pub(crate) fn emit_mode_mouse_motion(&self, dt: f32, output: &mut Vec<OutputEvent>) {
        let mut applied_sticks: Vec<(GamepadAxis, GamepadAxis)> = Vec::new();
        for (source, mode) in self.mode_inputs() {
            let SourceMode::Mouse { sensitivity, stick } = mode else {
                continue;
            };
            let (x_axis, y_axis) = stick_axes(source);
            if applied_sticks.contains(&(x_axis, y_axis)) {
                continue;
            }
            let Some((x, y)) = self.stick_pair(x_axis, y_axis) else {
                continue;
            };
            applied_sticks.push((x_axis, y_axis));
            let (x, y) = apply_stick_processing(stick, x, y, self.controller_deadzone_for(x_axis));
            push_mouse_axis(
                output,
                MouseAxis::X,
                x * sensitivity * STICK_MOUSE_COUNTS_PER_SECOND * dt,
            );
            push_mouse_axis(
                output,
                MouseAxis::Y,
                y * sensitivity * STICK_MOUSE_COUNTS_PER_SECOND * dt,
            );
        }
    }

    /// Outer Ring Commands from Joystick/Mouse modes: the command is held
    /// while the raw stick deflection sits past the ring radius (inside it,
    /// when inverted), emitted on crossings like the dpad directions.
    pub(crate) fn emit_mode_outer_ring(&mut self, output: &mut Vec<OutputEvent>) {
        let mut active: Vec<(GamepadAxis, OuterRingCommand)> = Vec::new();
        for (source, mode) in self.mode_inputs() {
            let processing = match mode {
                SourceMode::Joystick(settings) => settings.processing,
                SourceMode::Mouse { stick, .. } => stick,
                _ => continue,
            };
            let Some(ring) = processing.outer_ring else {
                continue;
            };
            let (x_axis, y_axis) = stick_axes(source);
            if active.iter().any(|(axis, _)| *axis == x_axis) {
                continue;
            }
            let (x, y) = self.stick_pair(x_axis, y_axis).unwrap_or((0.0, 0.0));
            let magnitude = (x * x + y * y).sqrt();
            if ring.invert != (magnitude >= ring.radius) {
                active.push((x_axis, ring));
            }
        }
        // Newly covered rings press their command.
        for (axis, ring) in &active {
            if self.outer_ring_pressed.iter().any(|(held, _)| held == axis) {
                continue;
            }
            if let Some(event) = discrete_press_event(&ring.output, true) {
                self.outer_ring_pressed.push((*axis, ring.output.clone()));
                output.push(event);
            }
        }
        // Rings no longer covered — or no longer configured — release.
        let mut kept = Vec::new();
        for (axis, held_output) in std::mem::take(&mut self.outer_ring_pressed) {
            if active.iter().any(|(candidate, _)| *candidate == axis) {
                kept.push((axis, held_output));
            } else if let Some(event) = discrete_press_event(&held_output, false) {
                output.push(event);
            }
        }
        self.outer_ring_pressed = kept;
    }

    /// Dpad directions from stick deflection; crossings emit press/release.
    pub(crate) fn emit_mode_dpad(&mut self, output: &mut Vec<OutputEvent>) {
        let mut pressed: Vec<GamepadButton> = Vec::new();
        let modes = self.mode_inputs();
        for (source, mode) in &modes {
            let SourceMode::Dpad { threshold } = mode else {
                continue;
            };
            let (x_axis, y_axis) = stick_axes(*source);
            let Some((x, y)) = self.stick_pair(x_axis, y_axis) else {
                continue;
            };
            if y > *threshold {
                pressed.push(GamepadButton::DpadUp);
            }
            if y < -*threshold {
                pressed.push(GamepadButton::DpadDown);
            }
            if x < -*threshold {
                pressed.push(GamepadButton::DpadLeft);
            }
            if x > *threshold {
                pressed.push(GamepadButton::DpadRight);
            }
        }
        // Cross-compare with last emission; only changes emit.
        for button in [
            GamepadButton::DpadUp,
            GamepadButton::DpadDown,
            GamepadButton::DpadLeft,
            GamepadButton::DpadRight,
        ] {
            let now_pressed = pressed.contains(&button);
            let was_pressed = self.mode_dpad_pressed.contains(&button);
            if now_pressed != was_pressed {
                output.push(OutputEvent::GamepadButton {
                    button,
                    pressed: now_pressed,
                });
                if now_pressed {
                    self.mode_dpad_pressed.push(button);
                } else {
                    self.mode_dpad_pressed
                        .retain(|candidate| *candidate != button);
                }
            }
        }
    }

    fn stick_pair(&self, x_axis: GamepadAxis, y_axis: GamepadAxis) -> Option<(f32, f32)> {
        // A stick that has only reported one axis so far still has a
        // well-defined position; the other reads as centered.
        let x = self
            .values
            .get(&InputSource::Axis(x_axis))
            .copied()
            .unwrap_or(0.0);
        let y = self
            .values
            .get(&InputSource::Axis(y_axis))
            .copied()
            .unwrap_or(0.0);
        Some((x, y))
    }
}

/// The X/Y axis pair a stick source belongs to.
fn stick_axes(source: InputSource) -> (GamepadAxis, GamepadAxis) {
    match source {
        InputSource::Axis(GamepadAxis::LeftX | GamepadAxis::LeftY)
        | InputSource::AxisDirection {
            axis: GamepadAxis::LeftX | GamepadAxis::LeftY,
            ..
        } => (GamepadAxis::LeftX, GamepadAxis::LeftY),
        _ => (GamepadAxis::RightX, GamepadAxis::RightY),
    }
}

/// The output axis an analog trigger source drives, if it is one.
fn trigger_axis(source: InputSource) -> Option<GamepadAxis> {
    match source {
        InputSource::Axis(GamepadAxis::LeftTrigger) => Some(GamepadAxis::LeftTrigger),
        InputSource::Axis(GamepadAxis::RightTrigger) => Some(GamepadAxis::RightTrigger),
        _ => None,
    }
}

/// The shared analog-stick pipeline, Steam's joystick settings in order:
/// rotate the input vector, apply the deadzone for its source, bend the
/// response curve (per axis or on the distance from the deadzone), scale
/// and invert each axis, and limit the output axes. `controller_deadzone`
/// backs the `Controller` deadzone source.
fn apply_stick_processing(
    processing: StickProcessing,
    x: f32,
    y: f32,
    controller_deadzone: f32,
) -> (f32, f32) {
    let StickProcessing {
        output_axis,
        rotation,
        sensitivity_x,
        sensitivity_y,
        invert_x,
        invert_y,
        deadzone,
        deadzone_inner,
        deadzone_outer,
        curve,
        response_axis_style,
        outer_ring: _,
    } = processing;
    // At 90° of rotation, pushing the stick north reads as east.
    let (sin, cos) = rotation.to_radians().sin_cos();
    let (x, y) = (x * cos - y * sin, x * sin + y * cos);
    let (inner, outer) = match deadzone {
        StickDeadzone::None => (0.0, 1.0),
        StickDeadzone::Controller => (controller_deadzone.clamp(0.0, 0.9), 1.0),
        StickDeadzone::Custom => (deadzone_inner, deadzone_outer),
    };
    let magnitude = (x * x + y * y).sqrt();
    // Radial deadzone: below the inner radius reads as zero, the outer
    // radius saturates, between them the magnitude rescales while the
    // direction is preserved.
    if magnitude <= inner || magnitude < VALUE_EPSILON {
        return (0.0, 0.0);
    }
    let scaled = ((magnitude - inner) / (outer - inner).max(VALUE_EPSILON)).clamp(0.0, 1.0);
    let (dx, dy) = (
        x / magnitude.max(VALUE_EPSILON),
        y / magnitude.max(VALUE_EPSILON),
    );
    let (x, y) = match response_axis_style {
        ResponseAxisStyle::Distance => {
            let curved = scaled.powf(curve);
            (dx * curved, dy * curved)
        }
        ResponseAxisStyle::PerAxis => {
            let curved_x = (dx.abs() * scaled).powf(curve) * dx.signum();
            let curved_y = (dy.abs() * scaled).powf(curve) * dy.signum();
            (curved_x, curved_y)
        }
    };
    let x = x * sensitivity_x * if invert_x { -1.0 } else { 1.0 };
    let y = y * sensitivity_y * if invert_y { -1.0 } else { 1.0 };
    match output_axis {
        StickOutputAxis::Both => (x, y),
        StickOutputAxis::Horizontal => (x, 0.0),
        StickOutputAxis::Vertical => (0.0, y),
    }
}

/// The press/release event a discrete outer-ring command maps to.
fn discrete_press_event(output: &OutputAction, pressed: bool) -> Option<OutputEvent> {
    match output {
        OutputAction::GamepadButton(button) => Some(OutputEvent::GamepadButton {
            button: *button,
            pressed,
        }),
        OutputAction::Keyboard { keycode } => Some(OutputEvent::Key {
            keycode: *keycode,
            pressed,
        }),
        OutputAction::MouseButton(button) => Some(OutputEvent::MouseButton {
            button: *button,
            pressed,
        }),
        _ => None,
    }
}

fn push_mouse_axis(output: &mut Vec<OutputEvent>, axis: MouseAxis, value: f32) {
    if value.abs() > VALUE_EPSILON {
        output.push(OutputEvent::MouseMotion { axis, value });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::MappingEngine;
    use crate::profile::{
        ActionSet, ActionSetLayer, Activator, ActivatorKind, InputMapping, InputProfile,
        JoystickSettings, OuterRingCommand, OutputAction, ResponseAxisStyle, SourceMode,
        StickDeadzone, StickOutputAxis, StickProcessing,
    };
    use crate::InputEvent;

    fn mode_profile(mode: SourceMode) -> InputProfile {
        InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping {
                    source: InputSource::Axis(GamepadAxis::LeftX),
                    mode: Some(mode),
                    mode_shifts: Vec::new(),
                    activators: vec![Activator::full_press(vec![OutputAction::GamepadButton(
                        GamepadButton::DpadUp,
                    )])],
                }],
            }],
            ..InputProfile::default()
        }
    }

    fn stick(source: InputSource, value: f32) -> InputEvent {
        InputEvent {
            source,
            value,
            timestamp_us: 0,
        }
    }

    #[test]
    fn test_joystick_mode_applies_radial_deadzone_and_curve() {
        let profile = mode_profile(SourceMode::Joystick(JoystickSettings {
            processing: StickProcessing {
                deadzone: StickDeadzone::Custom,
                deadzone_inner: 0.2,
                deadzone_outer: 1.0,
                ..StickProcessing::default()
            },
            ..JoystickSettings::new(StickOutput::Left)
        }));
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.1));
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftY), 0.0));
        let events = engine.tick(4_000);
        // Below the inner deadzone: no axis output.
        assert!(!events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis { axis: GamepadAxis::LeftX, value } if value.abs() > 0.001
        )));

        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.6));
        let events = engine.tick(8_000);
        // (0.6 - 0.2) / 0.8 = 0.5 after rescale.
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis { axis: GamepadAxis::LeftX, value } if (value - 0.5).abs() < 0.001
        )));
    }

    #[test]
    fn test_mouse_mode_emits_velocity_from_position() {
        let profile = mode_profile(SourceMode::Mouse {
            sensitivity: 1.0,
            stick: StickProcessing::default(),
        });
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 1.0));
        let events = engine.tick(4_000);
        let expected = STICK_MOUSE_COUNTS_PER_SECOND * 0.004;
        assert!(events
            .iter()
            .any(|event| matches!(event, OutputEvent::MouseMotion { axis: MouseAxis::X, value } if (value - expected).abs() < 0.5)));
    }

    #[test]
    fn test_dpad_mode_emits_direction_crossings() {
        let profile = mode_profile(SourceMode::Dpad { threshold: 0.5 });
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftY), 1.0));
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadButton {
                button: GamepadButton::DpadUp,
                pressed: true
            }
        )));
        // Sustained deflection emits nothing further.
        assert!(engine
            .tick(8_000)
            .iter()
            .all(|event| !matches!(event, OutputEvent::GamepadButton { .. })));
        // Return to center releases.
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftY), 0.0));
        let events = engine.tick(12_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadButton {
                button: GamepadButton::DpadUp,
                pressed: false
            }
        )));
    }

    #[test]
    fn test_mode_shift_swaps_stick_behavior_while_held() {
        // What the mode-shift editor writes: a shifted behavior that
        // replaces the stick's base mode while the trigger is held.
        let mut profile = mode_profile(SourceMode::Joystick(JoystickSettings {
            processing: StickProcessing {
                deadzone: StickDeadzone::Custom,
                deadzone_inner: 0.1,
                deadzone_outer: 1.0,
                ..StickProcessing::default()
            },
            ..JoystickSettings::new(StickOutput::Left)
        }));
        profile.action_sets[0].inputs[0]
            .mode_shifts
            .push(crate::profile::ModeShift {
                trigger: InputSource::Button(GamepadButton::LeftTrigger),
                mode: Some(SourceMode::Dpad { threshold: 0.5 }),
                activators: Vec::new(),
            });
        let mut engine = MappingEngine::new(profile).unwrap();

        engine.process(stick(InputSource::Button(GamepadButton::LeftTrigger), 1.0));
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftY), 1.0));
        let events = engine.tick(4_000);
        // Shifted: the stick digitalizes to dpad presses, no axis output.
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadButton {
                button: GamepadButton::DpadUp,
                pressed: true
            }
        )));
        assert!(events
            .iter()
            .all(|event| !matches!(event, OutputEvent::GamepadAxis { .. })));

        // Shift released: the dpad press lets go and the joystick returns.
        engine.process(stick(InputSource::Button(GamepadButton::LeftTrigger), 0.0));
        let events = engine.tick(8_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadButton {
                button: GamepadButton::DpadUp,
                pressed: false
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftY,
                value
            } if *value > 0.5
        )));
    }

    #[test]
    fn test_joystick_mode_without_deadzone_passes_raw_input_through() {
        let profile = mode_profile(SourceMode::joystick(StickOutput::Left));
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.3));
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                value
            } if (value - 0.3).abs() < 0.001
        )));
    }

    #[test]
    fn test_joystick_mode_controller_deadzone_uses_calibrated_value() {
        let profile = mode_profile(SourceMode::Joystick(JoystickSettings {
            processing: StickProcessing {
                deadzone: StickDeadzone::Controller,
                ..StickProcessing::default()
            },
            ..JoystickSettings::new(StickOutput::Left)
        }));
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.set_controller_deadzones(0.2, 0.0);
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.6));
        let events = engine.tick(4_000);
        // (0.6 - 0.2) / 0.8 = 0.5 after rescale.
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                value
            } if (value - 0.5).abs() < 0.001
        )));
        // Below the controller's calibrated radius the stick reads zero.
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.1));
        let events = engine.tick(8_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                value
            } if value.abs() < 0.001
        )));
    }

    #[test]
    fn test_joystick_mode_rotation_turns_north_into_east() {
        let profile = mode_profile(SourceMode::Joystick(JoystickSettings {
            processing: StickProcessing {
                rotation: 90.0,
                ..StickProcessing::default()
            },
            ..JoystickSettings::new(StickOutput::Left)
        }));
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftY), -1.0));
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                value
            } if (value - 1.0).abs() < 0.001
        )));
        assert!(events.iter().all(|event| !matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftY,
                value
            } if value.abs() > 0.001
        )));
    }

    #[test]
    fn test_joystick_mode_output_axis_limits_components() {
        let profile = mode_profile(SourceMode::Joystick(JoystickSettings {
            processing: StickProcessing {
                output_axis: StickOutputAxis::Vertical,
                ..StickProcessing::default()
            },
            ..JoystickSettings::new(StickOutput::Left)
        }));
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.5));
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftY), 0.25));
        let events = engine.tick(4_000);
        assert!(events.iter().all(|event| !matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                value
            } if value.abs() > 0.001
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftY,
                value
            } if (value - 0.25).abs() < 0.001
        )));
    }

    #[test]
    fn test_joystick_mode_sensitivity_and_invert_apply_per_axis() {
        let profile = mode_profile(SourceMode::Joystick(JoystickSettings {
            processing: StickProcessing {
                sensitivity_x: 2.0,
                invert_x: true,
                ..StickProcessing::default()
            },
            ..JoystickSettings::new(StickOutput::Left)
        }));
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.25));
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                value
            } if (value + 0.5).abs() < 0.001
        )));
    }

    #[test]
    fn test_outer_ring_command_holds_while_past_radius() {
        let profile = mode_profile(SourceMode::Joystick(JoystickSettings {
            processing: StickProcessing {
                outer_ring: Some(OuterRingCommand {
                    radius: 0.5,
                    invert: false,
                    output: OutputAction::GamepadButton(GamepadButton::A),
                }),
                ..StickProcessing::default()
            },
            ..JoystickSettings::new(StickOutput::Left)
        }));
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.8));
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadButton {
                button: GamepadButton::A,
                pressed: true
            }
        )));
        // Sustained deflection emits nothing further.
        assert!(engine
            .tick(8_000)
            .iter()
            .all(|event| !matches!(event, OutputEvent::GamepadButton { .. })));
        // Back inside the radius releases.
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.3));
        let events = engine.tick(12_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadButton {
                button: GamepadButton::A,
                pressed: false
            }
        )));
    }

    #[test]
    fn test_outer_ring_command_invert_holds_inside_radius() {
        let profile = mode_profile(SourceMode::Joystick(JoystickSettings {
            processing: StickProcessing {
                outer_ring: Some(OuterRingCommand {
                    radius: 0.5,
                    invert: true,
                    output: OutputAction::GamepadButton(GamepadButton::B),
                }),
                ..StickProcessing::default()
            },
            ..JoystickSettings::new(StickOutput::Left)
        }));
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.2));
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true
            }
        )));
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.9));
        let events = engine.tick(8_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: false
            }
        )));
    }

    #[test]
    fn test_mouse_mode_applies_its_stick_deadzone() {
        let profile = mode_profile(SourceMode::Mouse {
            sensitivity: 1.0,
            stick: StickProcessing {
                deadzone: StickDeadzone::Custom,
                deadzone_inner: 0.5,
                ..StickProcessing::default()
            },
        });
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.25));
        let events = engine.tick(4_000);
        // Below the deadzone: no pointer motion.
        assert!(events
            .iter()
            .all(|event| !matches!(event, OutputEvent::MouseMotion { .. })));
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 1.0));
        let events = engine.tick(8_000);
        let expected = STICK_MOUSE_COUNTS_PER_SECOND * 0.004;
        assert!(events
            .iter()
            .any(|event| matches!(event, OutputEvent::MouseMotion { axis: MouseAxis::X, value } if (value - expected).abs() < 0.5)));
    }

    #[test]
    fn test_per_axis_curve_bends_components_independently() {
        // At full diagonal deflection the distance style preserves the
        // vector while the per-axis style bends each component.
        let build = |style| {
            mode_profile(SourceMode::Joystick(JoystickSettings {
                processing: StickProcessing {
                    curve: 2.0,
                    response_axis_style: style,
                    ..StickProcessing::default()
                },
                ..JoystickSettings::new(StickOutput::Left)
            }))
        };
        let mut engine = MappingEngine::new(build(ResponseAxisStyle::Distance)).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.6));
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftY), 0.8));
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                value
            } if (value - 0.6).abs() < 0.001
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftY,
                value
            } if (value - 0.8).abs() < 0.001
        )));

        let mut engine = MappingEngine::new(build(ResponseAxisStyle::PerAxis)).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.6));
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftY), 0.8));
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                value
            } if (value - 0.36).abs() < 0.001
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftY,
                value
            } if (value - 0.64).abs() < 0.001
        )));
    }

    #[test]
    fn test_stick_mode_applies_once_when_both_axes_carry_it() {
        // Profiles from the per-axis editor mapped X and Y separately; the
        // pair is one input and must contribute once, not double.
        let mut profile = mode_profile(SourceMode::joystick(StickOutput::Left));
        let y_mapping = InputMapping {
            source: InputSource::Axis(GamepadAxis::LeftY),
            ..profile.action_sets[0].inputs[0].clone()
        };
        profile.action_sets[0].inputs.push(y_mapping);
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 0.5));
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                value
            } if (value - 0.5).abs() < 0.001
        )));
    }

    #[test]
    fn test_trigger_mode_releases_to_zero_below_threshold() {
        let mut profile = mode_profile(SourceMode::Trigger { threshold: 0.5 });
        profile.action_sets[0].inputs[0].source = InputSource::Axis(GamepadAxis::LeftTrigger);
        profile.action_sets[0].inputs[0].activators.clear();
        let mut engine = MappingEngine::new(profile).unwrap();

        engine.process(stick(InputSource::Axis(GamepadAxis::LeftTrigger), 0.8));
        assert!(engine.tick(1_000).iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis { axis: GamepadAxis::LeftTrigger, value }
                if (value - 0.6).abs() < 0.001
        )));

        // Dropping below the threshold must emit a released axis, not keep
        // the last deflection latched.
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftTrigger), 0.2));
        assert!(engine.tick(2_000).iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftTrigger,
                value: 0.0
            }
        )));
    }

    #[test]
    fn test_layer_stick_mode_overrides_base_set_mode() {
        let mut base = ActionSet {
            name: "Default".to_string(),
            inputs: vec![InputMapping {
                mode: Some(SourceMode::joystick(StickOutput::Left)),
                ..InputMapping::new(InputSource::Axis(GamepadAxis::LeftX))
            }],
        };
        let layer = ActionSetLayer {
            name: "Layer".to_string(),
            parent_set: "Default".to_string(),
            inputs: vec![InputMapping {
                mode: Some(SourceMode::joystick(StickOutput::Right)),
                ..InputMapping::new(InputSource::Axis(GamepadAxis::LeftX))
            }],
        };
        let mut toggler = InputMapping::new(InputSource::Button(GamepadButton::Guide));
        toggler.activators.push(Activator {
            kind: ActivatorKind::FullPress,
            outputs: vec![OutputAction::EnableLayer {
                layer: 0,
                mode: crate::profile::ChordMode::Hold,
            }],
            activation: crate::profile::Activation::Always,
            settings: crate::profile::ActivatorSettings::default(),
        });
        base.inputs.push(toggler);
        let mut engine = MappingEngine::new(InputProfile {
            action_sets: vec![base],
            action_layers: vec![layer],
            ..InputProfile::default()
        })
        .unwrap();

        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 1.0));
        assert!(engine.tick(1_000).iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                ..
            }
        )));

        // While the layer is held, its Joystick output wins over the base
        // set's mapping for the same stick.
        engine.process(stick(InputSource::Button(GamepadButton::Guide), 1.0));
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftX), 1.0));
        let events = engine.tick(2_000);
        assert!(events
            .iter()
            .any(|event| matches!(event, OutputEvent::GamepadAxis { axis: GamepadAxis::RightX, value } if *value > 0.9)));
        assert!(!events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftX,
                ..
            }
        )));
    }
}
