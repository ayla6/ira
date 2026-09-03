//! Flick Stick: stick direction snaps the camera by the same angle, and
//! holding a direction keeps turning. Splatoon-style gyro companion — point
//! the stick where you want to look, flick to turn 180°.

use super::{MappingEngine, OutputEvent, VALUE_EPSILON};
use crate::profile::{GamepadAxis, InputSource, MouseAxis, SourceMode};

/// Stick deflection that counts as "pointing": crossing this from centered
/// starts a flick.
const FLICK_ENGAGE: f32 = 0.85;
/// An engaged stick ends its flick below this softer line instead of the
/// engage point — re-crossing the rim would otherwise re-flick on every
/// wobble.
const FLICK_RELEASE: f32 = 0.75;

#[derive(Default)]
pub(crate) struct FlickState {
    /// Angle of the stick while engaged, radians.
    last_angle: Option<f32>,
    /// In-flight flick: tick-clock start time.
    flick_start_us: Option<u64>,
    /// Signed flick angle in radians (our convention: clockwise positive).
    flick_angle: f32,
    /// Duration of this flick, seconds — scaled by flick size per the
    /// reference: a 90° snap plays in half the time of a 180°.
    flick_duration_s: f32,
    /// Eased progress already emitted, 0..1.
    last_shaped: f32,
}

impl MappingEngine {
    /// Emit flick/rotation mouse motion for every Flickstick-mode input;
    /// called from tick() with the elapsed time.
    pub(crate) fn emit_flick_motion(&mut self, now_us: u64) -> Vec<OutputEvent> {
        let mut modes: Vec<(InputSource, SourceMode)> = Vec::new();
        for (source, mode) in self.mode_inputs() {
            let is_flick = matches!(mode, SourceMode::Flickstick { .. });
            if is_flick && !modes.iter().any(|(seen, _)| *seen == source) {
                modes.push((source, mode));
            }
        }
        let mut output = Vec::new();
        for (source, mode) in modes {
            let SourceMode::Flickstick {
                rotation_sensitivity,
                flick_duration_ms,
            } = mode
            else {
                continue;
            };
            let Some((x, y)) = self.stick_pair_for(source) else {
                continue;
            };
            let state = self.flick_states.entry(source).or_default();
            // Steam's shared calibration: one 360° sweep moves this many
            // mouse pixels at 1x sweep sensitivity.
            let cfg = FlickConfig {
                sensitivity: rotation_sensitivity,
                dots_per_360: self.profile.gyro.dots_per_360,
                flick_seconds: flick_duration_ms as f32 / 1000.0,
            };
            emit_flick_for_stick(state, x, y, now_us, &cfg, &mut output);
        }
        output
    }

    fn stick_pair_for(&self, source: InputSource) -> Option<(f32, f32)> {
        let (x_axis, y_axis) = match source {
            InputSource::Axis(GamepadAxis::RightX) | InputSource::Axis(GamepadAxis::RightY) => {
                (GamepadAxis::RightX, GamepadAxis::RightY)
            }
            _ => (GamepadAxis::LeftX, GamepadAxis::LeftY),
        };
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

/// Per-stick constants for the flick path.
struct FlickConfig {
    /// Wheel-mode rotation scale (the snap always covers the stick angle
    /// exactly).
    sensitivity: f32,
    /// Mouse counts for one full camera turn.
    dots_per_360: f32,
    /// How long a flick is spread, in seconds.
    flick_seconds: f32,
}

fn emit_flick_for_stick(
    state: &mut FlickState,
    x: f32,
    y: f32,
    now_us: u64,
    cfg: &FlickConfig,
    output: &mut Vec<OutputEvent>,
) {
    let magnitude = (x * x + y * y).sqrt();
    let threshold = if state.last_angle.is_some() {
        FLICK_RELEASE
    } else {
        FLICK_ENGAGE
    };
    if magnitude < threshold {
        // Released: forget the direction so re-engaging starts a fresh
        // flick — but an in-flight flick keeps playing to completion, as
        // in the reference; canceling it ate the tail of every snap.
        state.last_angle = None;
    } else {
        let angle = f32::atan2(x, -y); // 0 = up, growing clockwise.
        if state.last_angle.is_none() {
            // Bam! New flick: snap the camera by the full stick angle.
            state.last_angle = Some(angle);
            state.flick_angle = angle;
            state.last_shaped = 0.0;
            state.flick_duration_s =
                (cfg.flick_seconds * (angle.abs() / std::f32::consts::PI)).max(0.01);
            state.flick_start_us = Some(now_us);
        } else if let Some(previous) = state.last_angle {
            let delta = wrap_angle(angle - previous);
            if delta.abs() > VALUE_EPSILON {
                // Wheel mode: while engaged the camera turns with the
                // stick, scaled by the rotation sensitivity.
                push_yaw(
                    output,
                    delta * cfg.dots_per_360 / std::f32::consts::TAU * cfg.sensitivity,
                );
                state.last_angle = Some(angle);
            }
        }
    }

    // The flick's progress is time-based and eased (fast start, gliding
    // into the target) — the reference's shape; our old linear pacing
    // crawled through small flicks at constant speed.
    if let Some(started) = state.flick_start_us {
        let elapsed = now_us.saturating_sub(started) as f32 / 1_000_000.0;
        let progress = (elapsed / state.flick_duration_s).clamp(0.0, 1.0);
        let shaped = 1.0 - (1.0 - progress) * (1.0 - progress);
        let delta_shaped = shaped - state.last_shaped;
        state.last_shaped = shaped;
        push_yaw(output, delta_shaped * state.flick_angle * cfg.dots_per_360 / std::f32::consts::TAU);
        if progress >= 1.0 {
            state.flick_start_us = None;
        }
    }
}

fn push_yaw(output: &mut Vec<OutputEvent>, counts: f32) {
    if counts.abs() > VALUE_EPSILON {
        output.push(OutputEvent::MouseMotion {
            axis: MouseAxis::X,
            value: counts,
        });
    }
}

fn wrap_angle(delta: f32) -> f32 {
    let mut delta = delta + std::f32::consts::PI;
    delta -= std::f32::consts::TAU * (delta / std::f32::consts::TAU).floor();
    delta - std::f32::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::MappingEngine;
    use crate::profile::{
        ActionSet, Activator, GamepadAxis, GamepadButton, InputMapping, InputProfile, OutputAction,
    };

    fn flick_profile() -> InputProfile {
        InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping {
                    source: InputSource::Axis(GamepadAxis::LeftX),
                    mode: Some(SourceMode::Flickstick {
                        rotation_sensitivity: 1.0,
                        flick_duration_ms: 100,
                    }),
                    mode_shifts: Vec::new(),
                    activators: vec![Activator::full_press(vec![OutputAction::GamepadButton(
                        GamepadButton::DpadUp,
                    )])],
                }],
            }],
            ..InputProfile::default()
        }
    }

    fn stick(engine: &mut MappingEngine, x: f32, y: f32) {
        engine.process(crate::InputEvent {
            source: InputSource::Axis(GamepadAxis::LeftX),
            value: x,
            timestamp_us: 0,
        });
        engine.process(crate::InputEvent {
            source: InputSource::Axis(GamepadAxis::LeftY),
            value: y,
            timestamp_us: 0,
        });
    }

    fn yaw_total(events: &[OutputEvent]) -> f32 {
        events
            .iter()
            .filter_map(|event| match event {
                OutputEvent::MouseMotion {
                    axis: MouseAxis::X,
                    value,
                } => Some(*value),
                _ => None,
            })
            .sum()
    }

    #[test]
    fn test_flick_up_to_down_turns_180_degrees_over_time() {
        let profile = flick_profile();
        let dots = profile.gyro.dots_per_360;
        let mut engine = MappingEngine::new(profile).unwrap();
        stick(&mut engine, 0.0, -1.0); // up
        assert!(yaw_total(&engine.tick(4_000)).abs() < 1.0);

        stick(&mut engine, 0.0, 1.0); // down: half-turn flick starts
        let first = yaw_total(&engine.tick(5_000));
        assert!(first < -10.0, "down from up must flick, got {first}");

        let mut total = first;
        for i in 1..20 {
            total += yaw_total(&engine.tick(5_000 + i * 4_000));
        }
        // A full half turn is dots_per_360 / 2.
        let expected = dots / 2.0;
        assert!(
            (total.abs() - expected).abs() < expected * 0.2,
            "total {total} vs expected ~{expected}"
        );
    }

    #[test]
    fn test_centering_resets_and_reengage_flicks_again() {
        let profile = flick_profile();
        let dots = profile.gyro.dots_per_360;
        let mut engine = MappingEngine::new(profile).unwrap();
        stick(&mut engine, 1.0, 0.0); // right: quarter-turn flick starts
        engine.tick(4_000);
        // Centering mid-flick must not eat it: the reference plays flicks
        // to completion. A quarter turn drains in half the base time.
        let mut total = 0.0;
        for i in 1..25 {
            total += yaw_total(&engine.tick(4_000 + i * 4_000));
        }
        let expected = dots / 4.0;
        assert!(
            (total - expected).abs() < expected * 0.2,
            "flick must finish after re-centering: {total} vs ~{expected}"
        );

        // Release, then flick the other way: a fresh snap of the same size.
        stick(&mut engine, 0.0, 0.0);
        engine.tick(104_000);
        stick(&mut engine, -1.0, 0.0); // left
        let mut motion = yaw_total(&engine.tick(108_000));
        for i in 1..25 {
            motion += yaw_total(&engine.tick(108_000 + i * 4_000));
        }
        assert!(
            (motion.abs() - expected).abs() < expected * 0.2,
            "got {motion}"
        );
    }

    #[test]
    fn test_swinging_the_stick_rotates_continuously() {
        let profile = flick_profile();
        let dots = profile.gyro.dots_per_360;
        let mut engine = MappingEngine::new(profile).unwrap();
        stick(&mut engine, 0.0, -1.0); // engage pointing up
        engine.tick(4_000);
        // Swing 90° clockwise over several ticks.
        let mut total = 0.0;
        for step in 1..=8 {
            let angle = std::f32::consts::FRAC_PI_2 * (step as f32 / 8.0);
            stick(&mut engine, angle.sin(), -angle.cos());
            total += yaw_total(&engine.tick(4_000 + step * 4_000));
        }
        let expected = dots / 4.0; // quarter turn
        assert!(
            (total - expected).abs() < expected * 0.25,
            "total {total} vs expected ~{expected}"
        );
    }

    #[test]
    fn test_flick_from_neutral_down_snaps_half_circle() {
        // The defining flick-stick move: from a centered stick, pointing
        // down must flick the camera 180°. The old snap covered only the
        // remainder past the nearest half turn and turned nothing.
        let profile = flick_profile();
        let dots = profile.gyro.dots_per_360;
        let mut engine = MappingEngine::new(profile).unwrap();
        stick(&mut engine, 0.0, 1.0); // down, from neutral
        let mut total = yaw_total(&engine.tick(4_000));
        for i in 1..30 {
            total += yaw_total(&engine.tick(4_000 + i * 4_000));
        }
        let expected = dots / 2.0;
        assert!(
            (total - expected).abs() < expected * 0.2,
            "total {total} vs expected ~{expected}"
        );
    }

    #[test]
    fn test_flick_duration_setting_paces_the_snap() {
        let mut profile = flick_profile();
        profile.action_sets[0].inputs[0].mode = Some(SourceMode::Flickstick {
            rotation_sensitivity: 1.0,
            flick_duration_ms: 200,
        });
        let dots = profile.gyro.dots_per_360;
        let mut engine = MappingEngine::new(profile).unwrap();
        stick(&mut engine, 0.0, 1.0); // down: half-circle flick, 200 ms

        // Halfway through the flick only half the turn has been emitted.
        let mut halfway = yaw_total(&engine.tick(4_000));
        for i in 1..20 {
            halfway += yaw_total(&engine.tick(4_000 + i * 4_000));
        }
        assert!(
            halfway < dots * 0.35,
            "flick must still be in flight at 100 ms, got {halfway}"
        );

        // After the full 200 ms it lands on the same half circle.
        let mut total = halfway;
        for i in 20..45 {
            total += yaw_total(&engine.tick(4_000 + i * 4_000));
        }
        let expected = dots / 2.0;
        assert!(
            (total - expected).abs() < expected * 0.2,
            "total {total} vs expected ~{expected}"
        );
    }

    #[test]
    fn test_rim_wobble_does_not_reflick() {
        // Dropping just below the engage point while engaged must not end
        // the flick, or re-crossing the rim would snap all over again.
        let profile = flick_profile();
        let dots = profile.gyro.dots_per_360;
        let mut engine = MappingEngine::new(profile).unwrap();
        stick(&mut engine, 0.0, 1.0); // down: 180° flick
        let mut total = yaw_total(&engine.tick(4_000));
        for i in 1..30 {
            total += yaw_total(&engine.tick(4_000 + i * 4_000));
        }

        // Wobble between the release and engage lines at the same angle.
        for magnitude in [0.8f32, 0.95, 0.8, 1.0] {
            stick(&mut engine, 0.0, magnitude);
            total += yaw_total(&engine.tick(40_000));
        }
        assert!(
            total < dots * 0.75,
            "wobbling at the rim re-flicked: total {total}"
        );
    }
}
