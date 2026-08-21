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
use crate::profile::{GamepadAxis, GamepadButton, InputSource, MouseAxis, SourceMode, StickOutput};

impl MappingEngine {
    /// Collect the mode-driven mappings that currently apply (active set +
    /// layers), with mode shifts replacing the mode while held.
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
        if let Some(set) = self.profile.action_sets.get(self.active_set) {
            for mapping in &set.inputs {
                push(mapping);
            }
        }
        for index in self.active_layer_indexes() {
            if let Some(layer) = self.profile.action_layers.get(index) {
                for mapping in &layer.inputs {
                    push(mapping);
                }
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
        for (source, mode) in self.mode_inputs() {
            match mode {
                SourceMode::Joystick {
                    output,
                    deadzone_inner,
                    deadzone_outer,
                    curve,
                } => {
                    let (x_axis, y_axis) = stick_axes(source);
                    let Some((x, y)) = self.stick_pair(x_axis, y_axis) else {
                        continue;
                    };
                    let (x, y) =
                        apply_radial_deadzone(x, y, deadzone_inner, deadzone_outer, curve);
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
                    if value <= threshold {
                        continue;
                    }
                    // Rescale so the threshold reads as zero and full pull
                    // saturates.
                    let deflection = (value - threshold) / (1.0 - threshold).max(VALUE_EPSILON);
                    if !totals.contains_key(&axis) {
                        order.push(axis);
                    }
                    *totals.entry(axis).or_insert(0.0) += deflection.clamp(0.0, 1.0);
                }
                _ => {}
            }
        }
    }

    /// Relative mouse motion from Mouse-mode sticks, emitted per tick.
    pub(crate) fn emit_mode_mouse_motion(&self, dt: f32, output: &mut Vec<OutputEvent>) {
        for (source, mode) in self.mode_inputs() {
            let SourceMode::Mouse { sensitivity } = mode else {
                continue;
            };
            let (x_axis, y_axis) = stick_axes(source);
            let Some((x, y)) = self.stick_pair(x_axis, y_axis) else {
                continue;
            };
            push_mouse_axis(output, MouseAxis::X, x * sensitivity * STICK_MOUSE_COUNTS_PER_SECOND * dt);
            push_mouse_axis(output, MouseAxis::Y, y * sensitivity * STICK_MOUSE_COUNTS_PER_SECOND * dt);
        }
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
                    self.mode_dpad_pressed.retain(|candidate| *candidate != button);
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

/// Radial deadzone: deflection below the inner radius reads as zero, the
/// outer radius saturates, and between them the magnitude is rescaled and
/// curved while the direction is preserved.
fn apply_radial_deadzone(
    x: f32,
    y: f32,
    inner: f32,
    outer: f32,
    curve: f32,
) -> (f32, f32) {
    let magnitude = (x * x + y * y).sqrt();
    if magnitude <= inner || magnitude < VALUE_EPSILON {
        return (0.0, 0.0);
    }
    let scaled = ((magnitude - inner) / (outer - inner).max(VALUE_EPSILON)).clamp(0.0, 1.0);
    let curved = scaled.powf(curve) / magnitude.max(VALUE_EPSILON);
    (x * curved, y * curved)
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
        ActionSet, Activator, InputMapping, InputProfile, OutputAction, SourceMode,
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
                    activators: vec![Activator::full_press(vec![
                        OutputAction::GamepadButton(GamepadButton::DpadUp),
                    ])],
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
        let profile = mode_profile(SourceMode::Joystick {
            output: StickOutput::Left,
            deadzone_inner: 0.2,
            deadzone_outer: 1.0,
            curve: 1.0,
        });
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
        let profile = mode_profile(SourceMode::Mouse { sensitivity: 1.0 });
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
            OutputEvent::GamepadButton { button: GamepadButton::DpadUp, pressed: true }
        )));
        // Sustained deflection emits nothing further.
        assert!(engine.tick(8_000).iter().all(|event| !matches!(
            event,
            OutputEvent::GamepadButton { .. }
        )));
        // Return to center releases.
        engine.process(stick(InputSource::Axis(GamepadAxis::LeftY), 0.0));
        let events = engine.tick(12_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadButton { button: GamepadButton::DpadUp, pressed: false }
        )));
    }
}
