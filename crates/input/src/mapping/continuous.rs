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
    GamepadAxis, GyroActivation, GyroOrientation, GyroOutput, GyroStickResponseStyle,
    GyroStickSettings, InputProfile, InputSource, MouseAxis, TriggerDampening,
};
use crate::OutputEvent;

/// Gyro-to-mouse conversion at sensitivity 1.0: relative counts per radian of
/// rotation (≈30 counts per degree, a medium-sensitivity mouse feel).
/// Stick-to-mouse velocity at full deflection and sensitivity 1.0, in counts
/// per second.
pub(crate) const STICK_MOUSE_COUNTS_PER_SECOND: f32 = 2000.0;
/// Time constant for spreading Laser Pointer deltas across ticks, in
/// seconds. The pad delivers IMU samples in ~30 Hz bursts even though its
/// button reports run at 1000 Hz; this spreads each burst so the cursor
/// glides instead of stepping. Wider = smoother but laggier. JSM has no
/// laser mode to copy; its comparable flick-rotation smoothing targets
/// 64 ms — this stays well under that for pointer responsiveness.
const LASER_EMIT_TC: f32 = 0.02;
/// Stick-to-wheel velocity at full deflection, in detents per second.
/// Gyro-to-stick scaling: rotation rate (rad/s) that drives the stick to full
/// deflection at sensitivity 1.0. Steam's camera feel lands near a
/// moderate hand turn (~85 deg/s) reaching full deflection; the previous
/// 4.0 made ordinary turns crawl ("barely moves").
const GYRO_STICK_RADS_PER_UNIT: f32 = 1.5;
/// Trigger travel that counts as a soft pull for trigger dampening.
const DAMPENING_SOFT_PULL: f32 = 0.5;
/// Trigger travel that counts as a full pull: physical triggers report ~1.0
/// at the bottom; a little slack absorbs sensor wear.
const DAMPENING_FULL_PULL: f32 = 0.95;
/// Angular rate (rad/s) under which the momentum glide is considered spent.
const MOMENTUM_MIN_RATE: f32 = 0.01;
/// Sample gap under which the sensor stream counts as live and rates pass
/// through untouched. Real motion always brings follow-up reports (the
/// pads stream at 30 Hz+ while moving); only stillness and sub-threshold
/// slow aiming produce silence.
const GYRO_STREAM_CONFIRM_US: u64 = 120_000;


impl MappingEngine {
    /// Feed the latest player-space rates from the gyro processor, stamped
    /// with the tick clock at which the sample arrived. Smoothed
    /// bias-corrected rates in rad/s; see `crate::gyro`.
    pub fn update_gyro(&mut self, rates: GyroRates, now_us: u64) {
        self.gyro_rates = rates;
        self.last_gyro_sample_us = Some(now_us);
    }

    /// Stop the rates once the sensor stream goes quiet past the confirm
    /// window. This pad streams while anything moves and goes silent when
    /// it stops, so silence past the window means the hand stopped: the
    /// cursor must stop with it. Carrying or gliding the last rate through
    /// silence read as the cursor sliding on ice after every motion.
    fn stop_quiet_gyro_rates(&mut self, now_us: u64) {
        let quiet = self
            .last_gyro_sample_us
            .map_or(u64::MAX, |last| now_us.saturating_sub(last));
        if quiet >= GYRO_STREAM_CONFIRM_US {
            self.gyro_rates = GyroRates::default();
        }
    }

    /// Accumulate Laser Pointer angle deltas (radians) drained from the
    /// gyro processor; the next tick emits them as cursor position deltas.
    pub fn update_gyro_position(&mut self, yaw_delta: f32, pitch_delta: f32) {
        self.laser_yaw_delta += yaw_delta;
        self.laser_pitch_delta += pitch_delta;
    }

    /// Recomputes the rates the output paths consume. While the gyro is
    /// active they are the live rates; once it deactivates, momentum (if
    /// enabled) keeps outputting the last rates while friction decays them;
    /// without momentum output stops dead.
    pub(crate) fn refresh_gyro_effective(&mut self, dt: f32) {
        let momentum = &self.profile.gyro.momentum;
        let active = self.gyro_active();
        if active && !self.gyro_was_active {
            // Re-engaging must start from rest. The sensor kept reporting
            // through the pause — a fling putting the pad down, a wake-up
            // burst — and none of that belongs to the moment the user
            // chose to aim again.
            self.gyro_rates = GyroRates::default();
            self.gyro_momentum = GyroRates::default();
            self.laser_yaw_delta = 0.0;
            self.laser_pitch_delta = 0.0;
        }
        self.gyro_was_active = active;
        if active {
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
            self.laser_yaw_delta = 0.0;
            self.laser_pitch_delta = 0.0;
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
            // Steam's "Hold to Suppress": on unless the button is held.
            GyroActivation::Suppress(button) => {
                self.source_value(InputSource::Button(button)) <= super::BUTTON_THRESHOLD
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
        self.stop_quiet_gyro_rates(now_us);
        self.refresh_gyro_effective(dt);
        let mut output = Vec::new();
        if !self.profile.action_sets.is_empty() {
            output.extend(self.advance_set_activators(now_us));
            output.extend(self.take_pending_releases());
            self.emit_mode_mouse_motion(dt, &mut output);
            self.emit_mode_dpad(&mut output);
            self.emit_mode_outer_ring(&mut output);
            output.extend(self.emit_flick_motion(now_us));
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

    fn emit_mouse_motion(&mut self, dt: f32, output: &mut Vec<OutputEvent>) {
        let gyro = &self.profile.gyro;
        if gyro.enabled
            && gyro.output == GyroOutput::Mouse
            && gyro.orientation == GyroOrientation::LaserPointer
        {
            // Laser Pointer is position-based: the accumulated angle
            // deltas ARE the cursor movement. They bypass the rate
            // pipeline entirely — no smoothing, no decay, nothing that
            // could keep sliding after the hand stops. A still hand
            // produces zero deltas and a still cursor.
            if self.gyro_active() {
                // The pad delivers motion samples in bursts; emitting the
                // accumulated deltas verbatim made the cursor teleport in
                // chunks. A 20 ms leaky bucket spreads each burst over a
                // few ticks — imperceptible latency, no jumps.
                let share = 1.0 - (-dt / LASER_EMIT_TC).exp();
                let yaw = self.laser_yaw_delta * share;
                let pitch = self.laser_pitch_delta * share;
                self.laser_yaw_delta -= yaw;
                self.laser_pitch_delta -= pitch;
                let scale = gyro.dots_per_360 / std::f32::consts::TAU
                    * gyro.sensitivity
                    * self.gyro_dampening_scale();
                let (yaw, pitch) = rotate_gyro_output(yaw, pitch, gyro.rotate_output);
                push_mouse(output, MouseAxis::X, yaw * scale * sign(gyro.invert_x));
                push_mouse(output, MouseAxis::Y, -pitch * scale * sign(gyro.invert_y));
            } else {
                self.laser_yaw_delta = 0.0;
                self.laser_pitch_delta = 0.0;
            }
            return;
        }
        if gyro.enabled && gyro.output == GyroOutput::Mouse {
            // Steam's "Dots Per 360°": one full physical turn produces this
            // many mouse pixels at 1x sensitivity.
            let counts_per_radian = gyro.dots_per_360 / std::f32::consts::TAU;
            let scale =
                counts_per_radian * gyro.sensitivity * dt * self.gyro_dampening_scale();
            let (yaw, pitch) = rotate_gyro_output(
                self.gyro_effective.yaw,
                self.gyro_effective.pitch,
                gyro.rotate_output,
            );
            push_mouse(output, MouseAxis::X, yaw * scale * sign(gyro.invert_x));
            // Screen Y grows downward while positive pitch aims up.
            push_mouse(output, MouseAxis::Y, -pitch * scale * sign(gyro.invert_y));
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
            // Native motion flows through the uhid controller's own driver,
            // never through the mapping engine.
            GyroOutput::Mouse | GyroOutput::NativeMotion => return,
        };
        let (yaw, pitch) = rotate_gyro_output(
            self.gyro_effective.yaw,
            self.gyro_effective.pitch,
            gyro.rotate_output,
        );
        let (x, y) = gyro_stick_deflection(yaw, pitch, &gyro.stick, gyro.sensitivity);
        let x = x * sign(gyro.invert_x);
        let y = y * sign(gyro.invert_y);
        for (axis, value) in [(x_axis, x), (y_axis, y)] {
            if !totals.contains_key(&axis) {
                order.push(axis);
            }
            *totals.entry(axis).or_insert(0.0) += value;
        }
    }
}

/// Steam's Gyro-To-Joystick pipeline. `yaw`/`pitch` are the desired camera
/// turn rates in rad/s; the output is stick deflection in −1..=1.
///
/// Speeds below the deadzone output nothing (with recovery: the cutoff
/// scales the surviving rate so fast motions lose nothing overall).
/// Surviving rates are normalized against the full-deflection turn rate,
/// shaped by the power curve (per axis or on the deflection magnitude),
/// scaled to the maximum output, and optionally locked inside the unit
/// circle.
fn gyro_stick_deflection(
    yaw: f32,
    pitch: f32,
    settings: &GyroStickSettings,
    sensitivity: f32,
) -> (f32, f32) {
    const MAX_TURN_RATE: f32 = GYRO_STICK_RADS_PER_UNIT;
    let deadzone = settings.deadzone_dps.to_radians();
    let recover = |rate: f32| -> f32 {
        let magnitude = rate.abs();
        if magnitude <= deadzone || magnitude <= f32::EPSILON {
            return 0.0;
        }
        // Recover what the deadzone removed: scale by in/(in − dz) so a
        // rate far above the cutoff passes nearly untouched.
        rate * (magnitude / (magnitude - deadzone))
    };
    let yaw = recover(yaw) * sensitivity;
    let pitch = recover(pitch) * sensitivity;

    let curve = |normalized: f32| -> f32 {
        normalized.clamp(0.0, 1.0).powf(settings.power_curve) * settings.max_output
    };
    let (x, y) = match settings.response_style {
        GyroStickResponseStyle::PerAxis => (
            curve(yaw / MAX_TURN_RATE),
            curve(pitch / MAX_TURN_RATE),
        ),
        GyroStickResponseStyle::Circular => {
            let magnitude = (yaw * yaw + pitch * pitch).sqrt();
            if magnitude <= f32::EPSILON {
                (0.0, 0.0)
            } else {
                let shaped = curve(magnitude / MAX_TURN_RATE) / magnitude;
                (yaw * shaped, pitch * shaped)
            }
        }
    };
    if settings.lock_at_edges {
        let magnitude = (x * x + y * y).sqrt();
        if magnitude > settings.max_output && magnitude > f32::EPSILON {
            return (x / magnitude * settings.max_output, y / magnitude * settings.max_output);
        }
    }
    (x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0))
}

/// Rotates the gyro's 2D output clockwise by `degrees`, Steam's "Rotate
/// Output": a favorite diagonal hold angle can be straightened without
/// changing how the pad is held.
fn rotate_gyro_output(yaw: f32, pitch: f32, degrees: f32) -> (f32, f32) {
    if degrees.abs() < 0.01 {
        return (yaw, pitch);
    }
    let (sin, cos) = degrees.to_radians().sin_cos();
    (yaw * cos - pitch * sin, yaw * sin + pitch * cos)
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
        }, 0);
        let events = engine.tick(4_000);
        let dt = 0.004;
        assert!(
            (motion(&events, MouseAxis::X) - 2.0 * (6545.0 / std::f32::consts::TAU) * dt).abs() < 0.5
        );
        // Positive pitch aims up, which is negative screen Y.
        assert!(
            (motion(&events, MouseAxis::Y) - 1.0 * (6545.0 / std::f32::consts::TAU) * dt).abs() < 0.5
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
        }, 0);
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
        }, 0);
        assert!(engine.tick(4_000).is_empty());
        // Pressing the activation re-engages from rest; the next sensor
        // sample drives output again.
        engine.process(event(InputSource::Button(GamepadButton::LeftTrigger), 1.0));
        engine.update_gyro(GyroRates {
            yaw: 1.0,
            pitch: 0.0,
            gravity_locked: true,
        }, 6_000);
        assert!(!engine.tick(8_000).is_empty());
        engine.process(event(InputSource::Button(GamepadButton::LeftTrigger), 0.0));
        assert!(engine.tick(12_000).is_empty());
    }

    #[test]
    fn test_gyro_output_stops_when_the_stream_goes_quiet() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        let base = 1_000_000u64;
        let rate = GyroRates {
            yaw: 2.0,
            pitch: 0.0,
            gravity_locked: true,
        };
        engine.update_gyro(rate, base);
        // A second report within the confirm window establishes the stream.
        engine.update_gyro(rate, base + 50_000);
        assert!(motion(&engine.tick(base + 54_000), MouseAxis::X) > 0.0);
        // Quiet gaps shorter than the grace window (a live 50 ms report
        // stream) keep the full rate: the cursor must not freeze mid-aim.
        // The long gap clamps to the 50 ms tick-dt ceiling.
        let within_grace = base + 50_000 + GYRO_STREAM_CONFIRM_US - 4_000;
        let held = motion(&engine.tick(within_grace), MouseAxis::X);
        assert!(
            (held - 2.0 * (6545.0 / std::f32::consts::TAU) * 0.05).abs() < 0.5,
            "held: {held}"
        );
        // Past the confirm window the stream has stopped and so must the
        // cursor: no clamp, no glide — carried rates read as the cursor
        // sliding on ice after every motion.
        let now = within_grace + 50_000;
        assert_eq!(motion(&engine.tick(now), MouseAxis::X), 0.0);
        assert_eq!(motion(&engine.tick(now + 50_000), MouseAxis::X), 0.0);
        // A fresh sample restores full output immediately.
        engine.update_gyro(rate, now + 100_000);
        assert!(
            motion(&engine.tick(now + 104_000), MouseAxis::X)
                > 2.0 * (6545.0 / std::f32::consts::TAU) * 0.004 * 0.5,
            "fresh sample must resume output"
        );
    }

    #[test]
    fn test_laser_pointer_emits_position_deltas_once() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                orientation: GyroOrientation::LaserPointer,
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        // A turn right and a nose-down tilt, drained as position deltas.
        // Burst-delivered deltas spread across a few ticks (the 20 ms
        // emitter) instead of teleporting in one chunk, and land at the
        // exact target: a still hand (no fresh deltas) stops emitting.
        engine.update_gyro_position(0.1, -0.2);
        let scale = 6545.0f32 / std::f32::consts::TAU;
        let first = engine.tick(4_000);
        let first_x = motion(&first, MouseAxis::X);
        assert!(
            first_x > 0.0 && first_x < 0.1 * scale,
            "first tick must emit a partial share: {first_x}"
        );
        let mut total_x = first_x;
        let mut total_y = motion(&first, MouseAxis::Y);
        for i in 1..70 {
            let events = engine.tick(4_000 + i * 4_000);
            total_x += motion(&events, MouseAxis::X);
            total_y += motion(&events, MouseAxis::Y);
        }
        assert!((total_x - 0.1 * scale).abs() < 0.5, "x total: {total_x}");
        assert!((total_y - 0.2 * scale).abs() < 0.5, "y total: {total_y}");
        // Consumed: nothing further can slide.
        let mut tail_x;
        let mut tail_y;
        loop {
            let tail = engine.tick(40_000_000);
            tail_x = motion(&tail, MouseAxis::X);
            tail_y = motion(&tail, MouseAxis::Y);
            if tail_x == 0.0 && tail_y == 0.0 {
                break;
            }
        }
    }

    #[test]
    fn test_reengaging_gyro_starts_from_rest() {
        // While the gyro is paused the sensor keeps reporting — a fling
        // putting the pad down, a wake-up burst. Re-engaging must discard
        // all of that and start from rest instead of jerking the cursor
        // with the paused stretch's last motion.
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                activation: GyroActivation::Hold(GamepadButton::LeftShoulder),
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        let base = 1_000_000u64;
        let rate = GyroRates {
            yaw: 2.0,
            pitch: 0.0,
            gravity_locked: true,
        };
        engine.process(event(InputSource::Button(GamepadButton::LeftShoulder), 1.0));
        engine.update_gyro(rate, base);
        assert!(motion(&engine.tick(base + 4_000), MouseAxis::X) > 0.0);
        // Pause: output stops dead.
        engine.process(event(InputSource::Button(GamepadButton::LeftShoulder), 0.0));
        assert_eq!(motion(&engine.tick(base + 8_000), MouseAxis::X), 0.0);
        // During the pause the pad is flung — a confirmed stream of big
        // rates lands in the engine.
        engine.update_gyro(
            GyroRates {
                yaw: 6.0,
                pitch: 0.0,
                gravity_locked: true,
            },
            base + 20_000,
        );
        engine.update_gyro(
            GyroRates {
                yaw: 4.0,
                pitch: 0.0,
                gravity_locked: true,
            },
            base + 60_000,
        );
        // Re-engage: the paused stretch's rates are discarded, output
        // starts from rest.
        engine.process(event(InputSource::Button(GamepadButton::LeftShoulder), 1.0));
        assert_eq!(motion(&engine.tick(base + 64_000), MouseAxis::X), 0.0);
        // A fresh sample aims normally again.
        engine.update_gyro(rate, base + 100_000);
        assert!(motion(&engine.tick(base + 104_000), MouseAxis::X) > 0.0);
    }

    #[test]
    fn test_isolated_gyro_sample_does_not_drift_through_silence() {
        // The pad emits a lone reading as its report stream wakes (any input
        // event pulls one out) and then goes quiet while nothing moves.
        // Carrying that reading through the silence was the cursor drifting
        // away the moment the paddle was pressed.
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        let base = 1_000_000u64;
        let rate = GyroRates {
            yaw: 0.2,
            pitch: 0.0,
            gravity_locked: false,
        };
        engine.update_gyro(rate, base);
        // The lone sample outputs for its own moment — it is a real reading.
        assert!(motion(&engine.tick(base + 4_000), MouseAxis::X) > 0.0);
        // Silence past the confirm window with no partner sample: stop dead,
        // no clamp, no glide.
        assert_eq!(
            motion(
                &engine.tick(base + GYRO_STREAM_CONFIRM_US + 4_000),
                MouseAxis::X
            ),
            0.0
        );
        // A paired arrival resumes output normally.
        engine.update_gyro(rate, base + GYRO_STREAM_CONFIRM_US + 100_000);
        engine.update_gyro(rate, base + GYRO_STREAM_CONFIRM_US + 150_000);
        assert!(
            motion(
                &engine.tick(base + GYRO_STREAM_CONFIRM_US + 154_000),
                MouseAxis::X
            ) > 0.0
        );
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
        }, 0);
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
            yaw: 0.75,
            pitch: 0.0,
            gravity_locked: true,
        }, 0);
        let events = engine.tick(4_000);
        // 0.75 rad/s over 1.5 rad/s-per-unit = half deflection.
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis { axis: GamepadAxis::RightX, value } if (value - 0.5).abs() < 0.001
        )));
        // 3 rad/s is double the full-deflection rate and clamps.
        engine.update_gyro(GyroRates {
            yaw: 0.0,
            pitch: -3.0,
            gravity_locked: true,
        }, 0);
        let events = engine.tick(8_000);
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
            yaw: 0.75,
            pitch: 0.0,
            gravity_locked: true,
        }, 0);
        let events = engine.tick(4_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadAxis { axis: GamepadAxis::RightX, value } if (value - 0.75).abs() < 0.001
        )));
    }

    #[test]
    fn test_gyro_stick_pipeline_deadzone_curve_and_clamp() {
        use crate::profile::GyroStickSettings;
        // Deadzone: a rate under the cutoff outputs nothing...
        let (x, _) = super::gyro_stick_deflection(
            0.1,
            0.0,
            &GyroStickSettings {
                deadzone_dps: 10.0, // ~0.17 rad/s
                ..GyroStickSettings::default()
            },
            1.0,
        );
        assert_eq!(x, 0.0);
        // ...but recovery preserves rotation far above the cutoff.
        let (x, _) = super::gyro_stick_deflection(
            1.5,
            0.0,
            &GyroStickSettings {
                deadzone_dps: 10.0,
                ..GyroStickSettings::default()
            },
            1.0,
        );
        assert!((x - 1.0).abs() < 0.001, "{x}");
        // Relaxed curve: half-rate input deflects much less than half.
        let (relaxed, _) = super::gyro_stick_deflection(
            0.75,
            0.0,
            &GyroStickSettings {
                power_curve: 4.0,
                ..GyroStickSettings::default()
            },
            1.0,
        );
        assert!(relaxed < 0.1, "relaxed {relaxed}");
        // Maximum output caps full deflection.
        let (capped, _) = super::gyro_stick_deflection(
            3.0,
            0.0,
            &GyroStickSettings {
                max_output: 0.4,
                ..GyroStickSettings::default()
            },
            1.0,
        );
        assert!((capped - 0.4).abs() < 0.001, "{capped}");
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
        }, 0);
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
        }, 0);
        let dt = 0.004f32;
        let held = motion(&engine.tick(4_000), MouseAxis::X);
        assert!(
            (held - 4.0 * (6545.0 / std::f32::consts::TAU) * dt).abs() < 1.0,
            "held: {held}"
        );

        // Releasing the activation button must glide, not stop dead: the
        // first tick after release outputs the friction-decayed rate.
        engine.process(event(InputSource::Button(GamepadButton::LeftShoulder), 0.0));
        let glide = motion(&engine.tick(8_000), MouseAxis::X);
        let expected = 4.0 * (-2.0 * dt).exp() * (6545.0 / std::f32::consts::TAU) * dt;
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
        }, 0);
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
        }, 0);
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
        }, 0);
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
            yaw: 0.75,
            pitch: 0.0,
            gravity_locked: true,
        }, 0);
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
