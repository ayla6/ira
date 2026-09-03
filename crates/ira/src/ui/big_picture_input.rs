//! Controller navigation for big-picture mode: a background reader opens
//! gamepads without grabbing them (couch mode runs without the input daemon,
//! so the devices are free) and translates the left stick, the dpad and the
//! A button into navigation commands delivered on the GTK main loop.

use super::state::SharedState;
use ira_input::{discover_gamepads, PhysicalGamepad};
use ira_input::{GamepadAxis, GamepadButton, InputSource};
use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// Stick deflection that starts a navigation; releasing below
/// `STICK_RELEASE` stops it, so rim jitter around the engage point doesn't
/// stutter the selection.
const STICK_ENGAGE: f32 = 0.6;
const STICK_RELEASE: f32 = 0.4;
/// Hold-repeat pacing, close to the desktop keyboard's feel.
const REPEAT_DELAY_MS: u64 = 450;
const REPEAT_EVERY_MS: u64 = 140;
/// Per-pad event wait and how often disconnected pads are re-discovered.
const POLL_MS: u64 = 10;
const RESCAN_EVERY_MS: u64 = 2_000;
const IDLE_SLEEP_MS: u64 = 100;

/// One step of big-picture UI navigation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NavCommand {
    Left,
    Right,
    Confirm,
}

/// Held-direction tracking shared by the stick and the dpad: whichever
/// source commanded last wins, and holding it repeats after a delay.
#[derive(Default)]
struct NavState {
    stick: Option<NavCommand>,
    dpad: Option<NavCommand>,
    active: Option<NavCommand>,
    next_repeat_ms: u64,
}

impl NavState {
    /// Re-derive the held command from the stick and dpad state. Returns a
    /// command when a direction just engaged or the stick rolled straight
    /// from one direction into another; holding steady returns nothing.
    fn update(&mut self, now_ms: u64) -> Option<NavCommand> {
        let desired = self.stick.or(self.dpad);
        if desired == self.active {
            return None;
        }
        self.active = desired;
        self.next_repeat_ms = now_ms + REPEAT_DELAY_MS;
        desired
    }

    /// The held command again once its repeat delay has elapsed.
    fn repeat_due(&mut self, now_ms: u64) -> Option<NavCommand> {
        let cmd = self.active?;
        if now_ms < self.next_repeat_ms {
            return None;
        }
        self.next_repeat_ms = now_ms + REPEAT_EVERY_MS;
        Some(cmd)
    }

    fn apply_stick(&mut self, x: f32) {
        let threshold = if self.stick.is_some() {
            STICK_RELEASE
        } else {
            STICK_ENGAGE
        };
        self.stick = if x >= threshold {
            Some(NavCommand::Right)
        } else if x <= -threshold {
            Some(NavCommand::Left)
        } else {
            None
        };
    }

    fn apply_dpad(&mut self, left: bool, pressed: bool) {
        let cmd = if left {
            NavCommand::Left
        } else {
            NavCommand::Right
        };
        let matches = self.dpad == Some(cmd);
        self.dpad = match (pressed, matches) {
            (true, _) => Some(cmd),
            (false, true) => None,
            (false, false) => self.dpad,
        };
    }
}

/// Spawn the reader thread and drain its command channel on the main loop.
/// A fixed poll follows the calibration dialog's pattern; at 30 ms it is
/// invisible next to the 450 ms repeat delay.
pub(super) fn start(state: &SharedState) {
    let (tx, rx) = std::sync::mpsc::channel::<NavCommand>();
    let save_dir = state.borrow().save_dir.clone();
    let nav_state = state.clone();
    glib::timeout_add_local(Duration::from_millis(30), move || loop {
        match rx.try_recv() {
            Ok(command) => super::big_picture_view::handle_nav(&nav_state, command),
            Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return glib::ControlFlow::Break,
        }
    });
    std::thread::Builder::new()
        .name("big-picture-nav".to_string())
        .spawn(move || reader_loop(tx, save_dir))
        .expect("spawn big-picture navigation thread");
}

fn reader_loop(tx: Sender<NavCommand>, save_dir: String) {
    let calibration_path = ira_input::calibration_store_path(&save_dir);
    let mut pads: Vec<PhysicalGamepad> = Vec::new();
    let mut nav = NavState::default();
    let started = Instant::now();
    let mut last_rescan = 0u64;
    loop {
        let now_ms = started.elapsed().as_millis() as u64;
        if now_ms.saturating_sub(last_rescan) >= RESCAN_EVERY_MS {
            last_rescan = now_ms;
            rescan(&mut pads, &calibration_path);
        }
        if pads.is_empty() {
            std::thread::sleep(Duration::from_millis(IDLE_SLEEP_MS));
            continue;
        }
        // A pad the daemon grabbed (a game just launched) goes silent for us
        // without erroring; the timeout below keeps the loop alive until it
        // comes back or is gone for good.
        let mut live = Vec::with_capacity(pads.len());
        for mut pad in pads.drain(..) {
            let _ = pad.wait_for_event(Some(Duration::from_millis(POLL_MS)));
            let Ok(events) = pad.fetch_events() else {
                // gone; the next rescan reopens a replacement
                continue;
            };
            for event in events {
                fold_event(&mut nav, event, &tx, now_ms);
            }
            live.push(pad);
        }
        pads = live;
        if let Some(command) = nav.repeat_due(now_ms) {
            if tx.send(command).is_err() {
                return; // receiver dropped: the app is shutting down
            }
        }
    }
}

fn fold_event(
    nav: &mut NavState,
    event: ira_input::InputEvent,
    tx: &Sender<NavCommand>,
    now_ms: u64,
) {
    let pressed = event.value > 0.5;
    match event.source {
        InputSource::Button(GamepadButton::DpadLeft) => nav.apply_dpad(true, pressed),
        InputSource::Button(GamepadButton::DpadRight) => nav.apply_dpad(false, pressed),
        InputSource::Button(GamepadButton::A) if pressed => {
            let _ = tx.send(NavCommand::Confirm);
        }
        InputSource::Axis(GamepadAxis::LeftX) => nav.apply_stick(event.value),
        _ => {}
    }
    if let Some(command) = nav.update(now_ms) {
        let _ = tx.send(command);
    }
}

/// Open every gamepad that appeared since last time, applying each device's
/// resolved face-button layout so A/Confirm follows the physical marking.
fn rescan(pads: &mut Vec<PhysicalGamepad>, calibration_path: &Path) {
    pads.retain(|pad| pad.is_connected());
    let known: HashSet<_> = pads.iter().map(|pad| pad.info().path.clone()).collect();
    for device in discover_gamepads() {
        if known.contains(&device.path) {
            continue;
        }
        let Ok(mut pad) = PhysicalGamepad::open(&device.path, false) else {
            continue;
        };
        let layout = ira_input::resolved_nintendo_layout(calibration_path, &device);
        pad.set_nintendo_layout(layout);
        eprintln!("big-picture: navigating with {}", device.name);
        pads.push(pad);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_emits_once_per_engage() {
        let mut nav = NavState::default();
        assert_eq!(nav.update(0), None);
        nav.apply_stick(0.9);
        assert_eq!(nav.update(10), Some(NavCommand::Right));
        assert_eq!(nav.update(20), None, "holding steady must not re-emit");
    }

    #[test]
    fn test_stick_uses_release_hysteresis() {
        let mut nav = NavState::default();
        nav.apply_stick(0.9);
        assert_eq!(nav.update(0), Some(NavCommand::Right));
        // Between engage and release the direction is kept.
        nav.apply_stick(0.5);
        assert_eq!(nav.update(10), None);
        nav.apply_stick(0.2);
        assert_eq!(nav.update(20), None, "returning to center releases");
        assert_eq!(nav.active, None);
    }

    #[test]
    fn test_rolling_between_directions_reengages() {
        let mut nav = NavState::default();
        nav.apply_stick(-0.9);
        assert_eq!(nav.update(0), Some(NavCommand::Left));
        nav.apply_stick(0.9);
        assert_eq!(nav.update(10), Some(NavCommand::Right));
    }

    #[test]
    fn test_dpad_drives_and_releases() {
        let mut nav = NavState::default();
        nav.apply_dpad(true, true);
        assert_eq!(nav.update(0), Some(NavCommand::Left));
        assert_eq!(nav.update(10), None);
        nav.apply_dpad(true, false);
        assert_eq!(nav.update(20), None);
        assert_eq!(nav.active, None);
    }

    #[test]
    fn test_repeat_waits_then_repeats() {
        let mut nav = NavState::default();
        nav.apply_stick(1.0);
        assert_eq!(nav.update(100), Some(NavCommand::Right));
        assert_eq!(nav.repeat_due(100 + REPEAT_DELAY_MS - 1), None);
        assert_eq!(nav.repeat_due(100 + REPEAT_DELAY_MS), Some(NavCommand::Right));
        assert_eq!(nav.repeat_due(100 + REPEAT_DELAY_MS + 1), None);
        assert_eq!(
            nav.repeat_due(100 + REPEAT_DELAY_MS + REPEAT_EVERY_MS),
            Some(NavCommand::Right)
        );
    }

    #[test]
    fn test_released_direction_does_not_repeat() {
        let mut nav = NavState::default();
        nav.apply_stick(1.0);
        assert_eq!(nav.update(0), Some(NavCommand::Right));
        nav.apply_stick(0.0);
        assert_eq!(nav.update(10), None);
        assert_eq!(nav.repeat_due(5_000), None);
    }
}
