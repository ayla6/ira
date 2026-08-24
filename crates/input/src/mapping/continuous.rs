//! Tick-driven continuous outputs.
//!
//! Relative mouse motion and gyro-driven stick deflection cannot be emitted
//! from input events alone: evdev only reports *changes*, so a stick held
//! deflected stops producing events while the cursor must keep moving. The
//! daemon calls [`MappingEngine::tick`] on a fixed schedule (the controller's
//! report rate) and every emission here is computed from current state times
//! the elapsed time, making cursor speed independent of event rates.

use std::collections::HashMap;

use super::{MappingEngine, DEFAULT_TICK_INTERVAL};
use crate::gyro::GyroRates;
use crate::profile::{
    GamepadAxis, GyroActivation, GyroOutput, InputProfile, InputSource, MouseAxis, OutputAction,
};
use crate::OutputEvent;

/// Gyro-to-mouse conversion at sensitivity 1.0: relative counts per radian of
/// rotation (≈30 counts per degree, a medium-sensitivity mouse feel).
const GYRO_MOUSE_COUNTS_PER_RADIAN: f32 = 1718.87;
/// Stick-to-mouse velocity at full deflection and sensitivity 1.0, in counts
/// per second.
pub(crate) const STICK_MOUSE_COUNTS_PER_SECOND: f32 = 2000.0;
/// Stick-to-wheel velocity at full deflection, in detents per second.
pub(crate) const STICK_WHEEL_DETENTS_PER_SECOND: f32 = 8.0;
/// Gyro-to-stick scaling: rotation rate (rad/s) that drives the stick to full
/// deflection at sensitivity 1.0. Without scaling, an ordinary hand turn
/// (1-3 rad/s) would slam the stick to its rail.
const GYRO_STICK_RADS_PER_UNIT: f32 = 4.0;

impl MappingEngine {
    /// Feed the latest player-space rates from the gyro processor. Smoothed
    /// bias-corrected rates in rad/s; see `crate::gyro`.
    pub fn update_gyro(&mut self, rates: GyroRates) {
        self.gyro_rates = rates;
    }

    /// Whether any continuous (tick-driven) output is configured, so the
    /// daemon knows to keep ticking even without a gyro sensor attached.
    pub fn has_continuous_outputs(&self) -> bool {
        continuous_outputs_configured(&self.profile)
    }

    pub fn gyro_active(&self) -> bool {
        let gyro = &self.profile.gyro;
        if !gyro.enabled {
            return false;
        }
        match gyro.activation {
            GyroActivation::Always => true,
            GyroActivation::Hold(button) => {
                self.source_value(InputSource::Button(button)) > super::BUTTON_THRESHOLD
            }
            GyroActivation::Toggle(button) => self
                .toggles
                .get(&InputSource::Button(button))
                .copied()
                .unwrap_or(false),
        }
    }

    pub fn tick(&mut self, now_us: u64) -> Vec<OutputEvent> {
        let dt = self.tick_delta(now_us);
        let mut output = Vec::new();
        if !self.profile.action_sets.is_empty() {
            output.extend(self.advance_set_activators(now_us));
            output.extend(self.take_pending_releases());
            self.emit_mode_mouse_motion(dt, &mut output);
            self.emit_mode_dpad(&mut output);
            output.extend(self.emit_flick_motion(dt));
        }
        self.emit_mouse_motion(dt, &mut output);
        let computed = self.compute_values();
        self.emit_axis_outputs(&computed, &mut output);
        output
    }

    fn tick_delta(&mut self, now_us: u64) -> f32 {
        let delta = self
            .last_tick_us
            .map(|last| now_us.saturating_sub(last) as f32 / 1_000_000.0)
            .unwrap_or(DEFAULT_TICK_INTERVAL)
            .clamp(0.0005, 0.05);
        self.last_tick_us = Some(now_us);
        delta
    }

    fn emit_mouse_motion(&self, dt: f32, output: &mut Vec<OutputEvent>) {
        let gyro = &self.profile.gyro;
        if gyro.enabled && gyro.output == GyroOutput::Mouse && self.gyro_active() {
            let scale = GYRO_MOUSE_COUNTS_PER_RADIAN * gyro.sensitivity * dt;
            push_mouse(
                output,
                MouseAxis::X,
                self.gyro_rates.yaw * scale * sign(gyro.invert_x),
            );
            // Screen Y grows downward while positive pitch aims up.
            push_mouse(
                output,
                MouseAxis::Y,
                -self.gyro_rates.pitch * scale * sign(gyro.invert_y),
            );
        }
        for binding in &self.profile.bindings {
            let OutputAction::MouseAxis(axis) = &binding.output else {
                continue;
            };
            if !self.activation_active(&binding.activation) {
                continue;
            }
            let velocity = binding
                .transform
                .apply_unbounded(self.source_value(binding.source));
            let delta = match axis {
                MouseAxis::X | MouseAxis::Y => velocity * STICK_MOUSE_COUNTS_PER_SECOND * dt,
                MouseAxis::Wheel | MouseAxis::WheelX => {
                    velocity * STICK_WHEEL_DETENTS_PER_SECOND * dt
                }
            };
            push_mouse(output, *axis, delta);
        }
    }

    /// Adds gyro stick deflection into the axis composition so physical-stick
    /// and gyro contributions sum on the shared output axes.
    pub(crate) fn add_gyro_axis_deflections(
        &self,
        totals: &mut HashMap<GamepadAxis, f32>,
        order: &mut Vec<GamepadAxis>,
    ) {
        let gyro = &self.profile.gyro;
        if !gyro.enabled || gyro.output == GyroOutput::Mouse || !self.gyro_active() {
            return;
        }
        let (x_axis, y_axis) = match gyro.output {
            GyroOutput::LeftStick => (GamepadAxis::LeftX, GamepadAxis::LeftY),
            GyroOutput::RightStick => (GamepadAxis::RightX, GamepadAxis::RightY),
            GyroOutput::Mouse => return,
        };
        let scale = GYRO_STICK_RADS_PER_UNIT / gyro.sensitivity;
        let x = (self.gyro_rates.yaw / scale).clamp(-1.0, 1.0) * sign(gyro.invert_x);
        let y = (self.gyro_rates.pitch / scale).clamp(-1.0, 1.0) * sign(gyro.invert_y);
        for (axis, value) in [(x_axis, x), (y_axis, y)] {
            if !totals.contains_key(&axis) {
                order.push(axis);
            }
            *totals.entry(axis).or_insert(0.0) += value;
        }
    }
}

fn sign(inverted: bool) -> f32 {
    if inverted {
        -1.0
    } else {
        1.0
    }
}

fn push_mouse(output: &mut Vec<OutputEvent>, axis: MouseAxis, value: f32) {
    if value.abs() > super::VALUE_EPSILON {
        output.push(OutputEvent::MouseMotion { axis, value });
    }
}

/// Whether any continuous output is configured, so the daemon knows to run
/// ticks even without a gyro sensor present.
fn continuous_outputs_configured(profile: &InputProfile) -> bool {
    // Set-driven profiles always tick: activator deadlines and mode-driven
    // axes need time even without mouse or gyro.
    !profile.action_sets.is_empty()
        || profile.gyro.enabled
        || profile
            .bindings
            .iter()
            .any(|binding| matches!(binding.output, OutputAction::MouseAxis(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Binding, GyroConfig, InputSource};
    use crate::{GamepadButton, GyroActivation, GyroOutput};

    fn event(source: InputSource, value: f32) -> crate::InputEvent {
        crate::InputEvent {
            source,
            value,
            timestamp_us: 0,
        }
    }

    fn motion(events: &[OutputEvent], axis: MouseAxis) -> f32 {
        events
            .iter()
            .find_map(|event| match event {
                OutputEvent::MouseMotion { axis: a, value } if *a == axis => Some(*value),
                _ => None,
            })
            .unwrap_or(0.0)
    }

    #[test]
    fn test_held_stick_produces_continuous_mouse_motion() {
        // The original bug: motion only fired on events, so a held stick
        // stopped the cursor. Now a single deflection event followed by
        // ticks keeps producing equal deltas.
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Axis(GamepadAxis::LeftX),
                OutputAction::MouseAxis(MouseAxis::X),
            )],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        assert!(engine
            .process(event(InputSource::Axis(GamepadAxis::LeftX), 1.0))
            .is_empty());
        let first = engine.tick(4_000);
        let second = engine.tick(8_000);
        let third = engine.tick(12_000);
        let expected = STICK_MOUSE_COUNTS_PER_SECOND * 0.004;
        for tick in [first, second, third] {
            assert!((motion(&tick, MouseAxis::X) - expected).abs() < 0.5);
        }
    }

    #[test]
    fn test_released_stick_stops_mouse_motion() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Axis(GamepadAxis::LeftX),
                OutputAction::MouseAxis(MouseAxis::X),
            )],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(event(InputSource::Axis(GamepadAxis::LeftX), 0.8));
        engine.tick(4_000);
        engine.process(event(InputSource::Axis(GamepadAxis::LeftX), 0.0));
        assert_eq!(motion(&engine.tick(8_000), MouseAxis::X), 0.0);
    }

    #[test]
    fn test_gyro_mouse_scales_with_rate_and_time() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.update_gyro(GyroRates {
            yaw: 2.0,
            pitch: -1.0,
            gravity_locked: true,
        });
        let events = engine.tick(4_000);
        let dt = 0.004;
        assert!(
            (motion(&events, MouseAxis::X) - 2.0 * GYRO_MOUSE_COUNTS_PER_RADIAN * dt).abs() < 0.5
        );
        // Positive pitch aims up, which is negative screen Y.
        assert!(
            (motion(&events, MouseAxis::Y) - 1.0 * GYRO_MOUSE_COUNTS_PER_RADIAN * dt).abs() < 0.5
        );
    }

    #[test]
    fn test_gyro_mouse_respects_inverts() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                invert_x: true,
                invert_y: true,
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.update_gyro(GyroRates {
            yaw: 1.0,
            pitch: 1.0,
            gravity_locked: true,
        });
        let events = engine.tick(4_000);
        assert!(motion(&events, MouseAxis::X) < 0.0);
        // Double negation: pitch up is negative Y, inverted back to positive.
        assert!(motion(&events, MouseAxis::Y) > 0.0);
    }

    #[test]
    fn test_gyro_mouse_requires_activation() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                activation: GyroActivation::Hold(GamepadButton::LeftTrigger),
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.update_gyro(GyroRates {
            yaw: 1.0,
            pitch: 0.0,
            gravity_locked: true,
        });
        assert!(engine.tick(4_000).is_empty());
        engine.process(event(InputSource::Button(GamepadButton::LeftTrigger), 1.0));
        assert!(!engine.tick(8_000).is_empty());
        engine.process(event(InputSource::Button(GamepadButton::LeftTrigger), 0.0));
        assert!(engine.tick(12_000).is_empty());
    }

    #[test]
    fn test_gyro_toggle_activation_flips_on_button_press() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                activation: GyroActivation::Toggle(GamepadButton::Guide),
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.update_gyro(GyroRates {
            yaw: 1.0,
            pitch: 0.0,
            gravity_locked: true,
        });
        assert!(!engine.gyro_active());
        engine.process(event(InputSource::Button(GamepadButton::Guide), 1.0));
        engine.process(event(InputSource::Button(GamepadButton::Guide), 0.0));
        assert!(engine.gyro_active());
        engine.process(event(InputSource::Button(GamepadButton::Guide), 1.0));
        engine.process(event(InputSource::Button(GamepadButton::Guide), 0.0));
        assert!(!engine.gyro_active());
    }

    #[test]
    fn test_gyro_stick_deflection_scales_and_clamps() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                output: GyroOutput::RightStick,
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.update_gyro(GyroRates {
            yaw: 2.0,
            pitch: -8.0,
            gravity_locked: true,
        });
        let events = engine.tick(4_000);
        // 2 rad/s over 4 rad/s-per-unit = half deflection.
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis { axis: GamepadAxis::RightX, value } if (value - 0.5).abs() < 0.001
        )));
        // 8 rad/s clamps to full deflection.
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis { axis: GamepadAxis::RightY, value } if (value + 1.0).abs() < 0.001
        )));
    }

    #[test]
    fn test_gyro_stick_composes_with_physical_stick() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Axis(GamepadAxis::RightX),
                OutputAction::GamepadAxis(GamepadAxis::RightX),
            )],
            gyro: GyroConfig {
                enabled: true,
                output: GyroOutput::RightStick,
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(event(InputSource::Axis(GamepadAxis::RightX), 0.25));
        engine.update_gyro(GyroRates {
            yaw: 2.0,
            pitch: 0.0,
            gravity_locked: true,
        });
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis { axis: GamepadAxis::RightX, value } if (value - 0.75).abs() < 0.001
        )));
    }

    #[test]
    fn test_disabled_gyro_produces_no_output() {
        let profile = InputProfile {
            gyro: GyroConfig::default(),
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.update_gyro(GyroRates {
            yaw: 5.0,
            pitch: 5.0,
            gravity_locked: true,
        });
        assert!(engine.tick(4_000).is_empty());
    }

    #[test]
    fn test_tick_delta_clamps_outliers() {
        let profile = InputProfile::default();
        let mut engine = MappingEngine::new(profile).unwrap();
        // First tick uses the default interval; a huge jump clamps to 50 ms.
        engine.tick(0);
        engine.tick(10_000_000);
        assert_eq!(engine.last_tick_us, Some(10_000_000));
    }

    #[test]
    fn test_mouse_wheel_velocity_uses_detents() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Axis(GamepadAxis::RightY),
                OutputAction::MouseAxis(MouseAxis::Wheel),
            )],
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.tick(0);
        engine.process(event(InputSource::Axis(GamepadAxis::RightY), 1.0));
        let events = engine.tick(4_000);
        // Full deflection for 4 ms at 8 detents/s.
        assert!((motion(&events, MouseAxis::Wheel) - 0.032).abs() < 0.001);
    }

    #[test]
    fn test_continuous_outputs_configured_detects_mouse_usage() {
        let mut profile = InputProfile::default();
        assert!(!continuous_outputs_configured(&profile));
        profile.bindings.push(Binding::new(
            InputSource::Axis(GamepadAxis::LeftX),
            OutputAction::MouseAxis(MouseAxis::X),
        ));
        assert!(continuous_outputs_configured(&profile));
        profile.bindings.clear();
        profile.gyro.enabled = true;
        assert!(continuous_outputs_configured(&profile));
    }
}
