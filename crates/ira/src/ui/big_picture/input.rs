//! Controller navigation for big-picture mode: a background reader opens
//! gamepads without grabbing them (big-picture mode runs without the input daemon,
//! so the devices are free) and translates sticks, dpads and the A/B buttons
//! into navigation messages delivered on the GTK main loop.

use crate::ui::state::SharedState;
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
    Up,
    Down,
    Confirm,
    Back,
    /// The Options/Start key: a page-level action (sorting, grouping).
    Options,
    /// The shoulders switch the page's tabs (Software / Groups).
    PrevTab,
    NextTab,
    /// The X button: a secondary action on the focused thing (delete a
    /// group on the Groups tiles).
    Secondary,
}

/// What the bottom rail shows about connected gamepads: the count for the
/// dots and the leading pad's family, so the button prompts draw that
/// controller's glyphs. With no pads the prompts stay Xbox-style.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct PadStatus {
    pub count: usize,
    pub family: ira_input::ControllerFamily,
}

/// Everything the reader thread reports: navigation steps and pad status.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NavMsg {
    Nav(NavCommand),
    Pads(PadStatus),
}

/// Held-direction tracking for one axis: whichever source commanded last
/// wins, and holding it repeats after a delay. The stick and the dpad feed
/// it; both use engage/release hysteresis so rim jitter doesn't stutter.
#[derive(Default)]
struct AxisNav {
    stick: Option<NavCommand>,
    dpad: Option<NavCommand>,
    active: Option<NavCommand>,
    next_repeat_ms: u64,
}

impl AxisNav {
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

    /// A stick axis value mapped onto the axis's two commands. `positive`
    /// is the command for deflection past +1.
    fn apply_stick(&mut self, value: f32, positive: NavCommand, negative: NavCommand) {
        let threshold = if self.stick.is_some() {
            STICK_RELEASE
        } else {
            STICK_ENGAGE
        };
        self.stick = if value >= threshold {
            Some(positive)
        } else if value <= -threshold {
            Some(negative)
        } else {
            None
        };
    }

    /// A dpad button press/release for one of the axis's two directions.
    fn apply_button(&mut self, cmd: NavCommand, pressed: bool) {
        if pressed {
            self.dpad = Some(cmd);
        } else if self.dpad == Some(cmd) {
            self.dpad = None;
        }
    }
}

/// Both axes of directional navigation, plus the moment they last ticked.
#[derive(Default)]
struct NavState {
    h: AxisNav,
    v: AxisNav,
}

impl NavState {
    fn update(&mut self, now_ms: u64) -> Option<NavCommand> {
        self.h.update(now_ms).or_else(|| self.v.update(now_ms))
    }

    fn repeat_due(&mut self, now_ms: u64) -> Option<NavCommand> {
        self.h.repeat_due(now_ms).or_else(|| self.v.repeat_due(now_ms))
    }
}

/// Spawn the reader thread and drain its message channel on the main loop.
/// A fixed poll follows the calibration dialog's pattern; at 30 ms it is
/// invisible next to the 450 ms repeat delay.
pub(super) fn start(state: &SharedState) {
    let (tx, rx) = std::sync::mpsc::channel::<NavMsg>();
    let save_dir = state.borrow().save_dir.clone();
    let nav_state = state.clone();
    glib::timeout_add_local(Duration::from_millis(30), move || loop {
        match rx.try_recv() {
            Ok(msg) => super::view::handle_msg(&nav_state, msg),
            Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return glib::ControlFlow::Break,
        }
    });
    std::thread::Builder::new()
        .name("big-picture-nav".to_string())
        .spawn(move || reader_loop(tx, save_dir))
        .expect("spawn big-picture navigation thread");
}

fn reader_loop(tx: Sender<NavMsg>, save_dir: String) {
    let calibration_path = ira_input::calibration_store_path(&save_dir);
    let mut pads: Vec<PhysicalGamepad> = Vec::new();
    let mut nav = NavState::default();
    let started = Instant::now();
    let mut last_rescan = 0u64;
    let mut pads_ui = PadUi::default();
    loop {
        let now_ms = started.elapsed().as_millis() as u64;
        if now_ms.saturating_sub(last_rescan) >= RESCAN_EVERY_MS {
            last_rescan = now_ms;
            rescan(&mut pads, &calibration_path);
            pads_ui.after_rescan(&pads);
        }
        if pads.is_empty() {
            if !pads_ui.maybe_send(&tx) {
                return;
            }
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
        if !pads_ui.maybe_send(&tx) {
            return;
        }
        if let Some(command) = nav.repeat_due(now_ms) {
            if tx.send(NavMsg::Nav(command)).is_err() {
                return; // receiver dropped: the app is shutting down
            }
        }
    }
}

/// Pad-status bookkeeping for the reader thread: changes go out as
/// `NavMsg::Pads`.
#[derive(Default)]
struct PadUi {
    count: usize,
    family: Option<ira_input::ControllerFamily>,
    sent: Option<PadStatus>,
}

impl PadUi {
    fn after_rescan(&mut self, pads: &[PhysicalGamepad]) {
        self.count = pads.len();
        self.family = pads.first().map(|pad| pad.info().family());
    }

    /// Push the current status when it changed; false when the receiver is
    /// gone (the app is shutting down).
    fn maybe_send(&mut self, tx: &Sender<NavMsg>) -> bool {
        let status = PadStatus {
            count: self.count,
            family: self.family.unwrap_or(ira_input::ControllerFamily::Xbox),
        };
        if self.sent == Some(status) {
            return true;
        }
        if tx.send(NavMsg::Pads(status)).is_err() {
            return false;
        }
        self.sent = Some(status);
        true
    }
}

fn fold_event(
    nav: &mut NavState,
    event: ira_input::InputEvent,
    tx: &Sender<NavMsg>,
    now_ms: u64,
) {
    let pressed = event.value > 0.5;
    match event.source {
        InputSource::Button(GamepadButton::DpadLeft) => {
            nav.h.apply_button(NavCommand::Left, pressed)
        }
        InputSource::Button(GamepadButton::DpadRight) => {
            nav.h.apply_button(NavCommand::Right, pressed)
        }
        InputSource::Button(GamepadButton::DpadUp) => nav.v.apply_button(NavCommand::Up, pressed),
        InputSource::Button(GamepadButton::DpadDown) => {
            nav.v.apply_button(NavCommand::Down, pressed)
        }
        InputSource::Button(GamepadButton::A) if pressed => {
            let _ = tx.send(NavMsg::Nav(NavCommand::Confirm));
        }
        InputSource::Button(GamepadButton::B) if pressed => {
            let _ = tx.send(NavMsg::Nav(NavCommand::Back));
        }
        InputSource::Button(GamepadButton::Start) if pressed => {
            let _ = tx.send(NavMsg::Nav(NavCommand::Options));
        }
        InputSource::Button(GamepadButton::LeftShoulder) if pressed => {
            let _ = tx.send(NavMsg::Nav(NavCommand::PrevTab));
        }
        InputSource::Button(GamepadButton::RightShoulder) if pressed => {
            let _ = tx.send(NavMsg::Nav(NavCommand::NextTab));
        }
        InputSource::Button(GamepadButton::X) if pressed => {
            let _ = tx.send(NavMsg::Nav(NavCommand::Secondary));
        }
        InputSource::Axis(GamepadAxis::LeftX) => {
            nav.h.apply_stick(event.value, NavCommand::Right, NavCommand::Left)
        }
        InputSource::Axis(GamepadAxis::LeftY) => {
            nav.v.apply_stick(event.value, NavCommand::Down, NavCommand::Up)
        }
        _ => {}
    }
    if let Some(command) = nav.update(now_ms) {
        let _ = tx.send(NavMsg::Nav(command));
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
        nav.h.apply_stick(0.9, NavCommand::Right, NavCommand::Left);
        assert_eq!(nav.update(10), Some(NavCommand::Right));
        assert_eq!(nav.update(20), None, "holding steady must not re-emit");
    }

    #[test]
    fn test_stick_uses_release_hysteresis() {
        let mut nav = NavState::default();
        nav.h.apply_stick(0.9, NavCommand::Right, NavCommand::Left);
        assert_eq!(nav.update(0), Some(NavCommand::Right));
        // Between engage and release the direction is kept.
        nav.h.apply_stick(0.5, NavCommand::Right, NavCommand::Left);
        assert_eq!(nav.update(10), None);
        nav.h.apply_stick(0.2, NavCommand::Right, NavCommand::Left);
        assert_eq!(nav.update(20), None, "returning to center releases");
        assert_eq!(nav.h.active, None);
    }

    #[test]
    fn test_rolling_between_directions_reengages() {
        let mut nav = NavState::default();
        nav.h.apply_stick(-0.9, NavCommand::Right, NavCommand::Left);
        assert_eq!(nav.update(0), Some(NavCommand::Left));
        nav.h.apply_stick(0.9, NavCommand::Right, NavCommand::Left);
        assert_eq!(nav.update(10), Some(NavCommand::Right));
    }

    #[test]
    fn test_dpad_drives_and_releases() {
        let mut nav = NavState::default();
        nav.h.apply_button(NavCommand::Left, true);
        assert_eq!(nav.update(0), Some(NavCommand::Left));
        assert_eq!(nav.update(10), None);
        nav.h.apply_button(NavCommand::Left, false);
        assert_eq!(nav.update(20), None);
        assert_eq!(nav.h.active, None);
    }

    #[test]
    fn test_vertical_axis_maps_down_positive() {
        let mut nav = NavState::default();
        nav.v.apply_stick(0.9, NavCommand::Down, NavCommand::Up);
        assert_eq!(nav.update(0), Some(NavCommand::Down));
        nav.v.apply_stick(-0.9, NavCommand::Down, NavCommand::Up);
        assert_eq!(nav.update(10), Some(NavCommand::Up));
    }

    #[test]
    fn test_vertical_dpad_drives_and_releases() {
        let mut nav = NavState::default();
        nav.v.apply_button(NavCommand::Down, true);
        assert_eq!(nav.update(0), Some(NavCommand::Down));
        nav.v.apply_button(NavCommand::Down, false);
        assert_eq!(nav.update(10), None);
    }

    #[test]
    fn test_released_direction_does_not_repeat() {
        let mut nav = NavState::default();
        nav.h.apply_stick(1.0, NavCommand::Right, NavCommand::Left);
        assert_eq!(nav.update(0), Some(NavCommand::Right));
        nav.h.apply_stick(0.0, NavCommand::Right, NavCommand::Left);
        assert_eq!(nav.update(10), None);
        assert_eq!(nav.repeat_due(5_000), None);
    }

    #[test]
    fn test_repeat_waits_then_repeats() {
        let mut nav = NavState::default();
        nav.h.apply_stick(1.0, NavCommand::Right, NavCommand::Left);
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
    fn test_axes_report_independently() {
        let mut nav = NavState::default();
        nav.h.apply_stick(1.0, NavCommand::Right, NavCommand::Left);
        nav.v.apply_stick(-1.0, NavCommand::Down, NavCommand::Up);
        assert_eq!(nav.update(0), Some(NavCommand::Right));
        assert_eq!(nav.update(1), Some(NavCommand::Up));
        // Each held axis repeats on its own schedule.
        assert_eq!(nav.repeat_due(REPEAT_DELAY_MS), Some(NavCommand::Right));
        assert_eq!(nav.repeat_due(REPEAT_DELAY_MS + 1), Some(NavCommand::Up));
        // Releasing the stick silences only the horizontal axis.
        nav.h.apply_stick(0.0, NavCommand::Right, NavCommand::Left);
        assert_eq!(nav.update(REPEAT_DELAY_MS + 2), None);
        assert_eq!(nav.repeat_due(REPEAT_DELAY_MS + 2), None);
        assert_eq!(nav.v.repeat_due(1 + REPEAT_DELAY_MS + REPEAT_EVERY_MS), Some(NavCommand::Up));
    }
}
