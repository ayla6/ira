//! Flick Stick: stick direction snaps the camera by the same angle, and
//! holding a direction keeps turning. Splatoon-style gyro companion — point
//! the stick where you want to look, flick to turn 180°.

use super::{MappingEngine, OutputEvent, VALUE_EPSILON};
use crate::profile::{GamepadAxis, InputSource, MouseAxis, SourceMode};

/// Stick deflection that counts as "pointing" (below this the stick is
/// centered and no flick or rotation happens).
const FLICK_ENGAGE: f32 = 0.85;
/// Base camera turn for a full 180° flick, in relative mouse counts at
/// sensitivity 1.0 (tuned like Steam's default; sensitivity scales it).
const FLICK_COUNTS_PER_360: f32 = 7200.0;
/// How long a full flick takes, in seconds, at flick_duration_ms = 100.
const FLICK_BASE_SECONDS: f32 = 0.1;

#[derive(Default)]
pub(crate) struct FlickState {
    /// Angle of the stick while engaged, radians.
    last_angle: Option<f32>,
    /// Counts left to emit for the current flick.
    flick_remaining: f32,
    /// Sign of the ongoing flick (+1 clockwise, -1 counter-clockwise).
    flick_direction: f32,
}

impl MappingEngine {
    /// Emit flick/rotation mouse motion for every Flickstick-mode input;
    /// called from tick() with the elapsed time.
    pub(crate) fn emit_flick_motion(&mut self, dt: f32) -> Vec<OutputEvent> {
        let mut modes: Vec<(InputSource, SourceMode)> = Vec::new();
        if let Some(set) = self.profile.action_sets.get(self.active_set) {
            for mapping in &set.inputs {
                if let Some(mode @ SourceMode::Flickstick { .. }) = mapping.mode.clone() {
                    modes.push((mapping.source, mode));
                }
            }
        }
        let mut output = Vec::new();
        for (source, mode) in modes {
            let SourceMode::Flickstick {
                rotation_sensitivity,
                ..
            } = mode
            else {
                continue;
            };
            let Some((x, y)) = self.stick_pair_for(source) else {
                continue;
            };
            let state = self.flick_states.entry(source).or_default();
            emit_flick_for_stick(
                state,
                x,
                y,
                dt,
                rotation_sensitivity,
                &mut output,
            );
        }
        output
    }

    fn stick_pair_for(&self, source: InputSource) -> Option<(f32, f32)> {
        let (x_axis, y_axis) = match source {
            InputSource::Axis(GamepadAxis::RightX)
            | InputSource::Axis(GamepadAxis::RightY) => {
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

fn emit_flick_for_stick(
    state: &mut FlickState,
    x: f32,
    y: f32,
    dt: f32,
    sensitivity: f32,
    output: &mut Vec<OutputEvent>,
) {
    let magnitude = (x * x + y * y).sqrt();
    if magnitude < FLICK_ENGAGE {
        // Released: forget direction so re-engaging starts a fresh flick.
        state.last_angle = None;
        state.flick_remaining = 0.0;
        return;
    }

    let angle = f32::atan2(x, -y); // 0 = up, growing clockwise.

    // Engage before emitting so a fresh flick produces its first slice in
    // the same tick instead of starting a frame late.
    if state.last_angle.is_none() {
        state.last_angle = Some(angle);
        start_flick(state, angle, sensitivity);
    }

    if state.flick_remaining > VALUE_EPSILON {
        // Emit the in-flight flick burst.
        let step = state.flick_remaining.min(dt / FLICK_BASE_SECONDS);
        let counts = step * FLICK_COUNTS_PER_360 * sensitivity * state.flick_direction;
        push_yaw(output, counts);
        state.flick_remaining -= step;
    }

    if let Some(previous) = state.last_angle {
        let delta = wrap_angle(angle - previous);
        if delta.abs() > VALUE_EPSILON {
            // Continuous rotation proportional to how fast the player
            // swings the stick.
            push_yaw(output, delta * FLICK_COUNTS_PER_360 / std::f32::consts::TAU);
            state.last_angle = Some(angle);
        }
    }
}

fn start_flick(state: &mut FlickState, angle: f32, sensitivity: f32) {
    // Fractional turns past the nearest cardinal are preserved as rotation;
    // the flick itself covers the angle to the nearest half turn.
    let turns = angle / std::f32::consts::PI;
    let nearest_half = turns.round();
    let remainder = turns - nearest_half;
    let fraction = remainder.abs(); // 0..=0.5 of a half turn
    let _ = nearest_half;
    state.flick_direction = -remainder.signum();
    state.flick_remaining = fraction * 0.5 * sensitivity.max(0.05);
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
        ActionSet, Activator, GamepadAxis, GamepadButton, InputMapping, InputProfile,
        OutputAction,
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
        // A full half turn is FLICK_COUNTS_PER_360 / 2.
        let expected = FLICK_COUNTS_PER_360 / 2.0;
        assert!(
            (total.abs() - expected).abs() < expected * 0.2,
            "total {total} vs expected ~{expected}"
        );
    }

    #[test]
    fn test_centering_resets_and_reengage_flicks_again() {
        let profile = flick_profile();
        let mut engine = MappingEngine::new(profile).unwrap();
        stick(&mut engine, 1.0, 0.0); // right
        engine.tick(4_000);
        stick(&mut engine, 0.0, 0.0); // center resets
        engine.tick(8_000);
        assert!(yaw_total(&engine.tick(12_000)).abs() < 1.0);

        stick(&mut engine, -1.0, 0.0); // left: another flick
        let motion = yaw_total(&engine.tick(16_000));
        assert!(motion.abs() > 10.0, "re-engage must flick, got {motion}");
    }

    #[test]
    fn test_swinging_the_stick_rotates_continuously() {
        let profile = flick_profile();
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
        let expected = FLICK_COUNTS_PER_360 / 4.0; // quarter turn
        assert!(
            (total - expected).abs() < expected * 0.25,
            "total {total} vs expected ~{expected}"
        );
    }
}
