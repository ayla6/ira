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
    GamepadAxis, GyroActivation, GyroOutput, InputProfile, InputSource, MouseAxis, TriggerDampening,
};
use crate::OutputEvent;

/// Gyro-to-mouse conversion at sensitivity 1.0: relative counts per radian of
/// rotation (≈30 counts per degree, a medium-sensitivity mouse feel).
const GYRO_MOUSE_COUNTS_PER_RADIAN: f32 = 1718.87;
/// Stick-to-mouse velocity at full deflection and sensitivity 1.0, in counts
/// per second.
pub(crate) const STICK_MOUSE_COUNTS_PER_SECOND: f32 = 2000.0;
/// Stick-to-wheel velocity at full deflection, in detents per second.
/// Gyro-to-stick scaling: rotation rate (rad/s) that drives the stick to full
/// deflection at sensitivity 1.0. Without scaling, an ordinary hand turn
/// (1-3 rad/s) would slam the stick to its rail.
const GYRO_STICK_RADS_PER_UNIT: f32 = 4.0;
/// Trigger travel that counts as a soft pull for trigger dampening.
const DAMPENING_SOFT_PULL: f32 = 0.5;
/// Trigger travel that counts as a full pull: physical triggers report ~1.0
/// at the bottom; a little slack absorbs sensor wear.
const DAMPENING_FULL_PULL: f32 = 0.95;
/// Angular rate (rad/s) under which the momentum glide is considered spent.
const MOMENTUM_MIN_RATE: f32 = 0.01;

impl MappingEngine {
    /// Feed the latest player-space rates from the gyro processor. Smoothed
    /// bias-corrected rates in rad/s; see `crate::gyro`.
    pub fn update_gyro(&mut self, rates: GyroRates) {
        self.gyro_rates = rates;
    }

    /// Recomputes the rates the output paths consume. While the gyro is
    /// active they are the live rates; once it deactivates, momentum (if
    /// enabled) keeps outputting the last rates while friction decays them;
    /// without momentum output stops dead.
    pub(crate) fn refresh_gyro_effective(&mut self, dt: f32) {
        let momentum = &self.profile.gyro.momentum;
        if self.gyro_active() {
            self.gyro_effective = self.gyro_rates;
            if momentum.enabled {
                self.gyro_momentum = self.gyro_rates;
            }
            return;
        }
        let gliding = momentum.enabled
            && (self.gyro_momentum.yaw.abs() >= MOMENTUM_MIN_RATE
                || self.gyro_momentum.pitch.abs() >= MOMENTUM_MIN_RATE);
        if !gliding {
            self.gyro_momentum = GyroRates::default();
            self.gyro_effective = GyroRates::default();
            return;
        }
        let decay = (-momentum.friction * dt).exp();
        self.gyro_momentum.yaw *= decay;
        self.gyro_momentum.pitch *= decay;
        if self.gyro_momentum.yaw.abs() < MOMENTUM_MIN_RATE
            && self.gyro_momentum.pitch.abs() < MOMENTUM_MIN_RATE
        {
            self.gyro_momentum = GyroRates::default();
        }
        self.gyro_effective = self.gyro_momentum;
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
        self.refresh_gyro_effective(dt);
        let mut output = Vec::new();
        if !self.profile.action_sets.is_empty() {
            output.extend(self.advance_set_activators(now_us));
            output.extend(self.take_pending_releases());
            self.emit_mode_mouse_motion(dt, &mut output);
            self.emit_mode_dpad(&mut output);
            self.emit_mode_outer_ring(&mut output);
            output.extend(self.emit_flick_motion(dt));
        }
        self.emit_mouse_motion(dt, &mut output);
        self.emit_axis_outputs(&mut output);
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
        if gyro.enabled && gyro.output == GyroOutput::Mouse {
            let scale =
                GYRO_MOUSE_COUNTS_PER_RADIAN * gyro.sensitivity * dt * self.gyro_dampening_scale();
            push_mouse(
                output,
                MouseAxis::X,
                self.gyro_effective.yaw * scale * sign(gyro.invert_x),
            );
            // Screen Y grows downward while positive pitch aims up.
            push_mouse(
                output,
                MouseAxis::Y,
                -self.gyro_effective.pitch * scale * sign(gyro.invert_y),
            );
        }
    }

    /// Fraction of gyro mouse output that survives trigger dampening: while
    /// the configured trigger state is held, output scales down by the
    /// dampening amount (1.0 → frozen, 0.0 → untouched).
    fn gyro_dampening_scale(&self) -> f32 {
        let gyro = &self.profile.gyro;
        if !self.trigger_dampening_active() {
            return 1.0;
        }
        (1.0 - gyro.dampening_amount).clamp(0.0, 1.0)
    }

    fn trigger_dampening_active(&self) -> bool {
        let left = self.source_value(InputSource::Axis(GamepadAxis::LeftTrigger));
        let right = self.source_value(InputSource::Axis(GamepadAxis::RightTrigger));
        match self.profile.gyro.trigger_dampening {
            TriggerDampening::Off => false,
            TriggerDampening::LeftTriggerSoftPull => left >= DAMPENING_SOFT_PULL,
            TriggerDampening::LeftTriggerFullPull => left >= DAMPENING_FULL_PULL,
            TriggerDampening::RightTriggerSoftPull => right >= DAMPENING_SOFT_PULL,
            TriggerDampening::RightTriggerFullPull => right >= DAMPENING_FULL_PULL,
            TriggerDampening::BothTriggersFullPull => {
                left >= DAMPENING_FULL_PULL || right >= DAMPENING_FULL_PULL
            }
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
        if !gyro.enabled || gyro.output == GyroOutput::Mouse {
            return;
        }
        let (x_axis, y_axis) = match gyro.output {
            GyroOutput::LeftStick => (GamepadAxis::LeftX, GamepadAxis::LeftY),
            GyroOutput::RightStick => (GamepadAxis::RightX, GamepadAxis::RightY),
            GyroOutput::Mouse => return,
        };
        let scale = GYRO_STICK_RADS_PER_UNIT / gyro.sensitivity;
        let x = (self.gyro_effective.yaw / scale).clamp(-1.0, 1.0) * sign(gyro.invert_x);
        let y = (self.gyro_effective.pitch / scale).clamp(-1.0, 1.0) * sign(gyro.invert_y);
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
    !profile.action_sets.is_empty() || profile.gyro.enabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{
        ActionSet, GyroConfig, InputMapping, InputProfile, InputSource, SourceMode, StickProcessing,
    };
    use crate::{GamepadButton, GyroActivation, GyroOutput};

    fn event(source: InputSource, value: f32) -> crate::InputEvent {
        crate::InputEvent {
            source,
            value,
            timestamp_us: 0,
        }
    }

    fn set_profile(mapping: InputMapping) -> InputProfile {
        InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![mapping],
            }],
            ..InputProfile::default()
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
        let profile = set_profile(InputMapping {
            mode: Some(SourceMode::Mouse {
                sensitivity: 1.0,
                stick: StickProcessing::default(),
            }),
            ..InputMapping::new(InputSource::Axis(GamepadAxis::LeftX))
        });
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
        let profile = set_profile(InputMapping {
            mode: Some(SourceMode::Mouse {
                sensitivity: 1.0,
                stick: StickProcessing::default(),
            }),
            ..InputMapping::new(InputSource::Axis(GamepadAxis::LeftX))
        });
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
        let mut profile = set_profile(InputMapping {
            mode: Some(SourceMode::joystick(crate::profile::StickOutput::Right)),
            ..InputMapping::new(InputSource::Axis(GamepadAxis::RightX))
        });
        profile.gyro = GyroConfig {
            enabled: true,
            output: GyroOutput::RightStick,
            ..GyroConfig::default()
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
    fn test_gyro_momentum_glides_after_deactivation() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                activation: GyroActivation::Hold(GamepadButton::LeftShoulder),
                momentum: crate::profile::GyroMomentum {
                    enabled: true,
                    friction: 2.0,
                },
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(event(InputSource::Button(GamepadButton::LeftShoulder), 1.0));
        engine.update_gyro(GyroRates {
            yaw: 4.0,
            pitch: 0.0,
            gravity_locked: true,
        });
        let dt = 0.004f32;
        let held = motion(&engine.tick(4_000), MouseAxis::X);
        assert!(
            (held - 4.0 * GYRO_MOUSE_COUNTS_PER_RADIAN * dt).abs() < 1.0,
            "held: {held}"
        );

        // Releasing the activation button must glide, not stop dead: the
        // first tick after release outputs the friction-decayed rate.
        engine.process(event(InputSource::Button(GamepadButton::LeftShoulder), 0.0));
        let glide = motion(&engine.tick(8_000), MouseAxis::X);
        let expected = 4.0 * (-2.0 * dt).exp() * GYRO_MOUSE_COUNTS_PER_RADIAN * dt;
        assert!((glide - expected).abs() < 1.0, "glide: {glide}");

        // After a few time constants the glide is spent and output stops.
        let mut now = 12_000u64;
        for _ in 0..(250 * 4) {
            engine.tick(now);
            now += 4_000;
        }
        assert_eq!(motion(&engine.tick(now), MouseAxis::X), 0.0);
    }

    #[test]
    fn test_gyro_without_momentum_stops_dead_on_deactivation() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                activation: GyroActivation::Hold(GamepadButton::LeftShoulder),
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(event(InputSource::Button(GamepadButton::LeftShoulder), 1.0));
        engine.update_gyro(GyroRates {
            yaw: 4.0,
            pitch: 0.0,
            gravity_locked: true,
        });
        assert!(motion(&engine.tick(4_000), MouseAxis::X) > 0.0);
        engine.process(event(InputSource::Button(GamepadButton::LeftShoulder), 0.0));
        assert_eq!(motion(&engine.tick(8_000), MouseAxis::X), 0.0);
    }

    #[test]
    fn test_trigger_dampening_scales_gyro_mouse_while_held() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                trigger_dampening: crate::profile::TriggerDampening::RightTriggerSoftPull,
                dampening_amount: 0.5,
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
        let full = motion(&engine.tick(4_000), MouseAxis::X);

        // Below the soft-pull threshold nothing changes.
        engine.process(event(InputSource::Axis(GamepadAxis::RightTrigger), 0.4));
        assert!(
            (motion(&engine.tick(8_000), MouseAxis::X) - full).abs() < 1.0,
            "pre-soft-pull must be undampened"
        );
        // Past it, half the dampening amount is removed.
        engine.process(event(InputSource::Axis(GamepadAxis::RightTrigger), 0.6));
        let dampened = motion(&engine.tick(12_000), MouseAxis::X);
        assert!((dampened - full * 0.5).abs() < 1.0, "dampened: {dampened}");
        // Releasing the trigger restores full output.
        engine.process(event(InputSource::Axis(GamepadAxis::RightTrigger), 0.0));
        assert!(
            (motion(&engine.tick(16_000), MouseAxis::X) - full).abs() < 1.0,
            "released trigger must restore output"
        );
    }

    #[test]
    fn test_trigger_dampening_full_pull_needs_deep_travel() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                trigger_dampening: crate::profile::TriggerDampening::LeftTriggerFullPull,
                dampening_amount: 1.0,
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
        engine.tick(4_000);
        // A half pull is not a full pull: no dampening.
        engine.process(event(InputSource::Axis(GamepadAxis::LeftTrigger), 0.5));
        assert!(motion(&engine.tick(8_000), MouseAxis::X) > 0.0);
        // Bottoming the trigger freezes the gyro mouse entirely.
        engine.process(event(InputSource::Axis(GamepadAxis::LeftTrigger), 1.0));
        assert_eq!(motion(&engine.tick(12_000), MouseAxis::X), 0.0);
    }

    #[test]
    fn test_trigger_dampening_leaves_stick_output_alone() {
        // Steam applies trigger dampening to gyro mouse output only; the
        // stick deflection path must stay untouched even at full dampening.
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                output: GyroOutput::RightStick,
                trigger_dampening: crate::profile::TriggerDampening::RightTriggerSoftPull,
                dampening_amount: 1.0,
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.update_gyro(GyroRates {
            yaw: 2.0,
            pitch: 0.0,
            gravity_locked: true,
        });
        // Trigger past its soft pull: had dampening leaked into the stick
        // path, the deflection would be zeroed and no axis event emitted.
        engine.process(event(InputSource::Axis(GamepadAxis::RightTrigger), 0.6));
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis { axis: GamepadAxis::RightX, value } if (value - 0.5).abs() < 0.001
        )));
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
    fn test_continuous_outputs_configured_detects_set_profiles() {
        let mut profile = InputProfile::default();
        assert!(!continuous_outputs_configured(&profile));
        profile.action_sets.push(ActionSet {
            name: "Default".to_string(),
            inputs: Vec::new(),
        });
        assert!(continuous_outputs_configured(&profile));
        profile.action_sets.clear();
        profile.gyro.enabled = true;
        assert!(continuous_outputs_configured(&profile));
    }
}
