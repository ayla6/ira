//! Gamepad navigation for the overlay in gamescope/flatpak launches.
//!
//! In-process navigation (layer evdev → canvas commands) is dead in the
//! sandbox, and the daemon drops pad input while the overlay is up —
//! without this the overlay is view-only. While visible, dpad, left
//! stick, A, and shoulders translate to the same canvas commands the
//! layer would have pushed (`NAV_*`, `ACTIVATE`, scroll); B backs out.
//! Press edges only, tracked per source: wireless pads re-send held
//! state, and duplicates must not double-step.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ira_overlay_ipc::{
    canvas_shm_path, CanvasCommand, CanvasShm, MappedShm, CMD_ACTIVATE, CMD_BACK, CMD_NAV_DOWN,
    CMD_NAV_LEFT, CMD_NAV_RIGHT, CMD_NAV_UP,
};

use crate::{AxisDirection, GamepadAxis, GamepadButton, InputEvent, InputSource};

/// Press threshold, mirroring the mapping engine's button edge.
const PRESSED_ABOVE: f32 = 0.5;
/// Left-stick deflection that counts as a press; dropping below
/// `STICK_RELEASE` counts as a release (hysteresis against hover jitter).
const STICK_PRESS: f32 = 0.6;
const STICK_RELEASE: f32 = 0.4;

/// What one navigation press does. Tested below.
fn command_for_source(source: InputSource) -> Option<CanvasCommand> {
    let nav = |kind| Some(CanvasCommand::nav(kind));
    match source {
        InputSource::Button(button) => match button {
            GamepadButton::DpadUp => nav(CMD_NAV_UP),
            GamepadButton::DpadDown => nav(CMD_NAV_DOWN),
            GamepadButton::DpadLeft => nav(CMD_NAV_LEFT),
            GamepadButton::DpadRight => nav(CMD_NAV_RIGHT),
            GamepadButton::A => nav(CMD_ACTIVATE),
            // Back out one screen (submenu → menu, menu → close, decided host-side).
            GamepadButton::B => nav(CMD_BACK),
            GamepadButton::LeftShoulder => Some(CanvasCommand::scroll(-1.0)),
            GamepadButton::RightShoulder => Some(CanvasCommand::scroll(1.0)),
            _ => None,
        },
        InputSource::AxisDirection { axis, direction } => match (axis, direction) {
            (GamepadAxis::LeftX, AxisDirection::Negative) => nav(CMD_NAV_LEFT),
            (GamepadAxis::LeftX, AxisDirection::Positive) => nav(CMD_NAV_RIGHT),
            (GamepadAxis::LeftY, AxisDirection::Negative) => nav(CMD_NAV_UP),
            (GamepadAxis::LeftY, AxisDirection::Positive) => nav(CMD_NAV_DOWN),
            _ => None,
        },
        _ => None,
    }
}

/// Left-stick halves with press/release hysteresis, so hovering at the edge
/// cannot machine-gun the menu.
fn stick_halves(
    axis: GamepadAxis,
    value: f32,
    held: &HashMap<InputSource, Instant>,
) -> Vec<(InputSource, bool)> {
    use AxisDirection::{Negative, Positive};
    let (neg, pos) = match axis {
        GamepadAxis::LeftX | GamepadAxis::LeftY => (
            InputSource::AxisDirection {
                axis,
                direction: Negative,
            },
            InputSource::AxisDirection {
                axis,
                direction: Positive,
            },
        ),
        _ => return Vec::new(),
    };
    let latched = |source: &InputSource, beyond: bool, back: bool| {
        beyond || (held.contains_key(source) && !back)
    };
    vec![
        (neg, latched(&neg, value < -STICK_PRESS, value > -STICK_RELEASE)),
        (pos, latched(&pos, value > STICK_PRESS, value < STICK_RELEASE)),
    ]
}

pub(crate) struct OverlayNav {
    canvas: CanvasShm,
    held: HashMap<InputSource, Instant>,
    last_repeat: Instant,
}

/// Big Picture behavior: held directions auto-repeat after a short delay.
/// Buttons (activate, close, scroll) always single-fire.
const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_RATE: Duration = Duration::from_millis(120);

fn repeats(source: InputSource) -> bool {
    match source {
        InputSource::Button(button) => matches!(
            button,
            GamepadButton::DpadUp
                | GamepadButton::DpadDown
                | GamepadButton::DpadLeft
                | GamepadButton::DpadRight
        ),
        InputSource::AxisDirection { axis, .. } => {
            matches!(axis, GamepadAxis::LeftX | GamepadAxis::LeftY)
        }
        _ => false,
    }
}

/// Directions held past the initial delay re-fire at the repeat rate.
fn repeat_due(
    held: &HashMap<InputSource, Instant>,
    last_repeat: Instant,
    now: Instant,
) -> Vec<InputSource> {
    if now.duration_since(last_repeat) < REPEAT_RATE {
        return Vec::new();
    }
    held.iter()
        .filter(|(source, since)| {
            repeats(**source) && now.duration_since(**since) >= REPEAT_DELAY
        })
        .map(|(source, _)| *source)
        .collect()
}

impl OverlayNav {
    /// Arms from the session env. Silent `None` until the host created its
    /// canvas (callers retry lazily).
    pub(crate) fn arm(env: Option<&[(String, String)]>) -> Option<Self> {
        let shm_name = super::daemon_toggle_shm_name(env)?;
        let shm = MappedShm::open(&shm_name).ok()?;
        let canvas = CanvasShm::open_rw(&canvas_shm_path(shm.header().game_db_id)).ok()?;
        Some(Self {
            canvas,
            held: HashMap::new(),
            last_repeat: Instant::now(),
        })
    }

    /// Translates one pad event while the overlay is visible; the session drops
    /// everything afterwards regardless, so the game never sees it.
    pub(crate) fn feed(&mut self, event: &InputEvent) {
        match event.source {
            InputSource::Button(button) => {
                let source = InputSource::Button(button);
                if event.value <= PRESSED_ABOVE {
                    self.held.remove(&source);
                    return;
                }
                if self.held.insert(source, Instant::now()).is_some() {
                    return;
                }
                self.act(source);
            }
            InputSource::Axis(axis) => {
                for (dir, active) in stick_halves(axis, event.value, &self.held) {
                    if active {
                        if self.held.insert(dir, Instant::now()).is_none() {
                            self.act(dir);
                        }
                    } else {
                        self.held.remove(&dir);
                    }
                }
            }
            _ => {}
        }
    }

    fn act(&mut self, source: InputSource) {
        if let Some(cmd) = command_for_source(source) {
            self.canvas.push_command(cmd);
        }
    }

    /// Re-fires held directions past the repeat delay. Called every pass while visible.
    pub(crate) fn poll_repeat(&mut self) {
        self.poll_repeat_at(Instant::now());
    }

    fn poll_repeat_at(&mut self, now: Instant) {
        let due = repeat_due(&self.held, self.last_repeat, now);
        if due.is_empty() {
            return;
        }
        self.last_repeat = now;
        for source in due {
            if let Some(cmd) = command_for_source(source) {
                self.canvas.push_command(cmd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_for_covers_layer_nav_mapping() {
        // Same vocabulary as the in-process layer evdev path.
        let btn = InputSource::Button;
        assert!(matches!(
            command_for_source(btn(GamepadButton::DpadUp)),
            Some(cmd) if cmd.kind == CMD_NAV_UP
        ));
        assert!(matches!(
            command_for_source(btn(GamepadButton::DpadDown)),
            Some(cmd) if cmd.kind == CMD_NAV_DOWN
        ));
        assert!(matches!(
            command_for_source(btn(GamepadButton::DpadLeft)),
            Some(cmd) if cmd.kind == CMD_NAV_LEFT
        ));
        assert!(matches!(
            command_for_source(btn(GamepadButton::DpadRight)),
            Some(cmd) if cmd.kind == CMD_NAV_RIGHT
        ));
        assert!(matches!(
            command_for_source(btn(GamepadButton::A)),
            Some(cmd) if cmd.kind == CMD_ACTIVATE
        ));
        // B backs out one screen (submenu → menu, menu → close).
        assert!(matches!(
            command_for_source(btn(GamepadButton::B)),
            Some(cmd) if cmd.kind == CMD_BACK
        ));
        assert!(command_for_source(btn(GamepadButton::LeftShoulder)).is_some());
        assert!(command_for_source(btn(GamepadButton::RightShoulder)).is_some());
        // Guide stays the daemon toggle's (never navigation).
        assert!(command_for_source(btn(GamepadButton::Guide)).is_none());
        assert!(command_for_source(btn(GamepadButton::Start)).is_none());
        // Left stick mirrors the dpad; right stick stays out of menus.
        use AxisDirection::{Negative, Positive};
        let dir = |axis, direction| InputSource::AxisDirection { axis, direction };
        assert!(matches!(
            command_for_source(dir(GamepadAxis::LeftX, Negative)),
            Some(cmd) if cmd.kind == CMD_NAV_LEFT
        ));
        assert!(matches!(
            command_for_source(dir(GamepadAxis::LeftY, Positive)),
            Some(cmd) if cmd.kind == CMD_NAV_DOWN
        ));
        assert!(command_for_source(dir(GamepadAxis::RightX, Positive)).is_none());
    }

    #[test]
    fn test_repeat_due_fires_directions_past_delay_at_rate() {
        // Arrange: DpadDown held 500ms, A held 500ms, last repeat long ago.
        let now = Instant::now();
        let held: HashMap<InputSource, Instant> = [
            (
                InputSource::Button(GamepadButton::DpadDown),
                now - Duration::from_millis(500),
            ),
            (
                InputSource::Button(GamepadButton::A),
                now - Duration::from_millis(500),
            ),
        ]
        .into_iter()
        .collect();
        // Act + Assert: only the direction re-fires…
        assert_eq!(
            repeat_due(&held, now - Duration::from_secs(1), now),
            vec![InputSource::Button(GamepadButton::DpadDown)]
        );
        // …nothing before the initial delay…
        let fresh: HashMap<InputSource, Instant> = [(
            InputSource::Button(GamepadButton::DpadDown),
            now,
        )]
        .into_iter()
        .collect();
        assert!(repeat_due(&fresh, now - Duration::from_secs(1), now).is_empty());
        // …and rate-limiting holds back a second immediate pass.
        assert!(repeat_due(&held, now, now).is_empty());
    }

    #[test]
    fn test_stick_halves_latch_with_hysteresis() {
        // Arrange: nothing held.
        let held: HashMap<InputSource, Instant> = HashMap::new();
        let left = |d| InputSource::AxisDirection {
            axis: GamepadAxis::LeftX,
            direction: d,
        };
        use AxisDirection::{Negative, Positive};
        // Act + Assert: full deflection latches its half…
        let halves = stick_halves(GamepadAxis::LeftX, 0.8, &held);
        assert!(halves.contains(&(left(Positive), true)));
        assert!(halves.contains(&(left(Negative), false)));
        // …hovering at the edge without a latch stays quiet…
        let halves = stick_halves(GamepadAxis::LeftX, 0.5, &held);
        assert!(halves.iter().all(|(_, active)| !active));
        // …a latched half survives inside the release band…
        let mut held = held;
        held.insert(left(Positive), Instant::now());
        let halves = stick_halves(GamepadAxis::LeftX, 0.5, &held);
        assert!(halves.contains(&(left(Positive), true)));
        // …and clears past it.
        let halves = stick_halves(GamepadAxis::LeftX, 0.2, &held);
        assert!(halves.iter().all(|(_, active)| !active));
        // Non-sticks never navigate.
        assert!(stick_halves(GamepadAxis::RightX, 1.0, &held).is_empty());
    }
}
