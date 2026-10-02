//! Overlay chord authority for gamescope launches.
//!
//! Every in-process toggle path stands down in gamescope mode
//! (`IRA_OVERLAY_DISABLE_UI`: shim hooks, layer evdev), so the daemon —
//! which owns the physical pad — watches the toggle chord from game SHM,
//! swallows the completing press so the game never sees it, and reports it
//! to the app, which flips the SHM visibility flag. Flatpak games get this
//! with no special-casing: they never had in-process hooks to stand down.
//!
//! Screenshot/record chords are daemon-owned the same way: the daemon pushes
//! host→layer canvas actions the in-process layer drains and executes
//! game-side, mirroring the overlay host's menu buttons.

use std::collections::HashSet;
use std::sync::atomic::Ordering;

use ira_overlay_ipc::{
    canvas_shm_path, CanvasAction, CanvasShm, MappedShm, ACT_SCREENSHOT, ACT_TOGGLE_RECORD,
    DAEMON_TOGGLE_ENV, GAMEPAD_A, GAMEPAD_B, GAMEPAD_DPAD_DOWN, GAMEPAD_DPAD_LEFT,
    GAMEPAD_DPAD_RIGHT, GAMEPAD_DPAD_UP, GAMEPAD_GUIDE, GAMEPAD_L1, GAMEPAD_L2, GAMEPAD_L3,
    GAMEPAD_R1, GAMEPAD_R2, GAMEPAD_R3, GAMEPAD_SELECT, GAMEPAD_START, GAMEPAD_X, GAMEPAD_Y,
};

use crate::mapping::InputEvent;
use crate::{GamepadButton, InputSource};

/// Press threshold, mirroring the mapping engine's button edge.
const PRESSED_ABOVE: f32 = 0.5;

/// What the session loop should do with a physical input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToggleAction {
    /// Forward to the mapper untouched.
    Forward,
    /// Drop silently: grip/settle press, never a real toggle intent.
    Swallow,
    /// Drop and flip the overlay: a genuine toggle release.
    Fire,
    /// Drop and screenshot: the screenshot chord newly completed.
    FireScreenshot,
    /// Drop and flip recording: the record chord newly completed.
    FireRecord,
}

/// Grip-settle window after a pad (re)connect: buttons held while
/// plugging in are not toggle intent.
const SETTLE_AFTER_CONNECT: std::time::Duration = std::time::Duration::from_millis(500);

pub(crate) struct OverlayToggle {
    toggle: u32,
    screenshot: u32,
    record: u32,
    held: HashSet<u32>,
    /// The toggle chord completed on press and is waiting for release:
    /// like the in-process paths, the flip fires when the chord clears so
    /// a Guide-based toggle cannot pre-empt a Guide-based screenshot or
    /// record chord that is still being formed.
    toggle_pending: bool,
    canvas: Option<CanvasShm>,
    shm: Option<MappedShm>,
    settle_until: Option<std::time::Instant>,
}

impl OverlayToggle {
    /// Arms from the session env: needs the launcher marker plus a readable
    /// game SHM to decode the chords. Silent `None` otherwise. The mapping
    /// is kept so firing flips it directly — no IPC round trip.
    pub(crate) fn arm(env: Option<&[(String, String)]>) -> Option<Self> {
        let env = env?;
        if !env
            .iter()
            .any(|(k, v)| k == DAEMON_TOGGLE_ENV && v == "1")
        {
            return None;
        }
        let shm_name = env
            .iter()
            .find(|(k, _)| k == "IRA_OVERLAY_SHM")
            .map(|(_, v)| v.as_str())?;
        // Read-write: firing flips the flag through this same mapping, and
        // a read-only mapping would segfault on the atomic write.
        let shm = MappedShm::open_rw(shm_name).ok()?;
        let (toggle, screenshot, record) = shm.header().gamepad_hotkeys();
        if toggle == 0 {
            return None;
        }
        Some(Self {
            toggle,
            screenshot,
            record,
            held: HashSet::new(),
            toggle_pending: false,
            canvas: None,
            shm: Some(shm),
            settle_until: None,
        })
    }

    /// Notes a pad (re)connect: held state clears (physical state is
    /// unknown across the gap) and chord presses within the settle window
    /// are swallowed without firing (gripping buttons while plugging in).
    pub(crate) fn note_connect(&mut self) {
        self.held.clear();
        self.toggle_pending = false;
        self.settle_until = Some(std::time::Instant::now() + SETTLE_AFTER_CONNECT);
    }

    fn settling(&self) -> bool {
        self.settle_until
            .is_some_and(|until| std::time::Instant::now() < until)
    }

    /// Whether the overlay is currently shown (the game should not receive
    /// pad input while it is).
    pub(crate) fn is_visible(&self) -> bool {
        self.shm
            .as_ref()
            .is_some_and(|shm| shm.header().overlay_visible.load(Ordering::SeqCst) != 0)
    }

    /// Publishes the physical controller family for the overlay's footer
    /// glyphs. Best-effort: unarmed (no SHM) sessions skip silently.
    pub(crate) fn publish_family(&self, family: crate::ControllerFamily) {
        if let Some(shm) = self.shm.as_ref() {
            shm.header()
                .controller_family
                .store(family.as_u32(), Ordering::SeqCst);
        }
    }

    /// Flips the SHM visibility flag. Returns the new visibility on a
    /// committed flip, `None` when debounced — same contract as the
    /// header's toggle, surfaced for logging.
    pub(crate) fn flip(&self) -> Option<bool> {
        self.shm.as_ref()?.header().toggle_visible()
    }

    /// Filters one physical input event. Presses that newly complete the
    /// screenshot/record chords come back `FireScreenshot`/`FireRecord`
    /// (swallow + capture action); a press newly completing the toggle
    /// chord arms `toggle_pending` and the flip fires on the release that
    /// clears the chord — the same release-edge the in-process paths use,
    /// so a shared Guide cannot pre-empt the longer chords. Taps while a
    /// chord is already held are incidental and forward. Everything else
    /// is `Forward`. Held tracking also makes multi-key chords work and
    /// key-repeat never re-fire.
    pub(crate) fn filter(&mut self, event: &InputEvent) -> ToggleAction {
        if self.toggle == 0 {
            return ToggleAction::Forward;
        }
        let InputSource::Button(button) = event.source else {
            return ToggleAction::Forward;
        };
        let Some(bit) = mask_of(button) else {
            return ToggleAction::Forward;
        };
        if event.value > PRESSED_ABOVE {
            let before = self.held_bits();
            if !self.held.insert(bit) {
                // Repeat of a held button: chord members were consumed on
                // their press, so swallow further repeats; anything else
                // forwards untouched.
                return if bit & self.chord_bits() != 0 {
                    ToggleAction::Swallow
                } else {
                    ToggleAction::Forward
                };
            }
            let after = self.held_bits();
            let newly = |mask: u32| after & mask == mask && before & mask != mask;
            // Screenshot/record win over the toggle they share a Guide with.
            if newly(self.screenshot) {
                self.toggle_pending = false;
                return if self.settling() {
                    ToggleAction::Swallow
                } else {
                    ToggleAction::FireScreenshot
                };
            }
            if newly(self.record) {
                self.toggle_pending = false;
                return if self.settling() {
                    ToggleAction::Swallow
                } else {
                    ToggleAction::FireRecord
                };
            }
            if newly(self.toggle) {
                return if self.settling() {
                    ToggleAction::Swallow
                } else {
                    self.toggle_pending = true;
                    ToggleAction::Swallow
                };
            }
            return ToggleAction::Forward;
        }
        self.held.remove(&bit);
        // Releases always forward (the close-edge swallow set needs to see
        // them to retire consumed buttons); a pending toggle fires when the
        // chord itself breaks — releases of bystander buttons held alongside
        // must not cancel it.
        if self.toggle_pending && self.held_bits() & self.toggle != self.toggle {
            self.toggle_pending = false;
            return ToggleAction::Fire;
        }
        ToggleAction::Forward
    }

    fn held_bits(&self) -> u32 {
        self.held.iter().fold(0, |acc, b| acc | b)
    }

    fn chord_bits(&self) -> u32 {
        self.toggle | self.screenshot | self.record
    }

    /// Pushes a screenshot capture action to the canvas ring, where the
    /// in-process layer drains and executes it game-side. Opens the canvas
    /// lazily: the host creates it after the session starts.
    pub(crate) fn push_screenshot(&mut self) -> bool {
        self.push_capture(ACT_SCREENSHOT, "screenshot")
    }

    /// Pushes a record-toggle capture action to the canvas ring.
    pub(crate) fn push_record_toggle(&mut self) -> bool {
        self.push_capture(ACT_TOGGLE_RECORD, "record toggle")
    }

    fn push_capture(&mut self, kind: u32, label: &str) -> bool {
        let canvas = match self.ensure_canvas() {
            Some(canvas) => canvas,
            None => return false,
        };
        canvas.push_action(CanvasAction::of(kind));
        eprintln!("ira-input: overlay {label} chord fired");
        true
    }

    fn ensure_canvas(&mut self) -> Option<&mut CanvasShm> {
        if self.canvas.is_none() {
            let db_id = self.shm.as_ref()?.header().game_db_id;
            match CanvasShm::open_rw(&canvas_shm_path(db_id)) {
                Ok(canvas) => self.canvas = Some(canvas),
                Err(error) => {
                    eprintln!("ira-input: capture chord dropped, no canvas yet: {error}");
                    return None;
                }
            }
        }
        self.canvas.as_mut()
    }
}

/// Overlay mask bit for a gamepad button. `None` has no bit (paddles).
fn mask_of(button: GamepadButton) -> Option<u32> {
    Some(match button {
        GamepadButton::A => GAMEPAD_A,
        GamepadButton::B => GAMEPAD_B,
        GamepadButton::X => GAMEPAD_X,
        GamepadButton::Y => GAMEPAD_Y,
        GamepadButton::LeftShoulder => GAMEPAD_L1,
        GamepadButton::RightShoulder => GAMEPAD_R1,
        GamepadButton::LeftTrigger => GAMEPAD_L2,
        GamepadButton::RightTrigger => GAMEPAD_R2,
        GamepadButton::Back => GAMEPAD_SELECT,
        GamepadButton::Start => GAMEPAD_START,
        GamepadButton::Guide => GAMEPAD_GUIDE,
        GamepadButton::LeftStick => GAMEPAD_L3,
        GamepadButton::RightStick => GAMEPAD_R3,
        GamepadButton::DpadUp => GAMEPAD_DPAD_UP,
        GamepadButton::DpadDown => GAMEPAD_DPAD_DOWN,
        GamepadButton::DpadLeft => GAMEPAD_DPAD_LEFT,
        GamepadButton::DpadRight => GAMEPAD_DPAD_RIGHT,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(button: GamepadButton, value: f32) -> InputEvent {
        InputEvent {
            source: InputSource::Button(button),
            value,
            timestamp_us: 0,
        }
    }

    fn armed(toggle: u32) -> OverlayToggle {
        OverlayToggle {
            toggle,
            screenshot: GAMEPAD_GUIDE | GAMEPAD_START,
            record: GAMEPAD_GUIDE | GAMEPAD_SELECT,
            held: HashSet::new(),
            toggle_pending: false,
            canvas: None,
            shm: None,
            settle_until: None,
        }
    }

    #[test]
    fn test_single_button_chord_fires_on_release() {
        // Arrange
        let mut toggle = armed(GAMEPAD_GUIDE);
        // Act + Assert: press arms (swallowed, game never sees it)…
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 1.0)),
            ToggleAction::Swallow
        );
        // …repeat while held swallows (chord member, not new)…
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 1.0)),
            ToggleAction::Swallow
        );
        // …release with the chord clear fires…
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 0.0)),
            ToggleAction::Fire
        );
        // …and the next press arms again.
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 1.0)),
            ToggleAction::Swallow
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 0.0)),
            ToggleAction::Fire
        );
    }

    #[test]
    fn test_multi_button_chord_fires_when_chord_breaks() {
        // Arrange
        let mut toggle = armed(GAMEPAD_GUIDE | GAMEPAD_DPAD_DOWN);
        // Act + Assert: Guide alone forwards…
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 1.0)),
            ToggleAction::Forward
        );
        // …unrelated buttons forward…
        assert_eq!(
            toggle.filter(&press(GamepadButton::A, 1.0)),
            ToggleAction::Forward
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::A, 0.0)),
            ToggleAction::Forward
        );
        // …DpadDown completing it arms (swallowed)…
        assert_eq!(
            toggle.filter(&press(GamepadButton::DpadDown, 1.0)),
            ToggleAction::Swallow
        );
        // …and the first chord-member release fires.
        assert_eq!(
            toggle.filter(&press(GamepadButton::DpadDown, 0.0)),
            ToggleAction::Fire
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 0.0)),
            ToggleAction::Forward
        );
    }

    #[test]
    fn test_taps_while_chord_held_do_not_fire() {
        // Arrange: Guide held (chord armed, not yet fired).
        let mut toggle = armed(GAMEPAD_GUIDE);
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 1.0)),
            ToggleAction::Swallow
        );
        // Act + Assert: tapping anything else forwards without firing…
        assert_eq!(
            toggle.filter(&press(GamepadButton::A, 1.0)),
            ToggleAction::Forward
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::A, 0.0)),
            ToggleAction::Forward
        );
        // …and the chord release still fires exactly once.
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 0.0)),
            ToggleAction::Fire
        );
    }

    #[test]
    fn test_screenshot_chord_wins_over_pending_toggle() {
        // Arrange: default-style masks sharing a Guide.
        let mut toggle = OverlayToggle {
            screenshot: GAMEPAD_GUIDE | GAMEPAD_DPAD_DOWN,
            record: GAMEPAD_GUIDE | GAMEPAD_DPAD_UP,
            ..armed(GAMEPAD_GUIDE)
        };
        // Act + Assert: Guide arms the toggle…
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 1.0)),
            ToggleAction::Swallow
        );
        // …DpadDown completing the screenshot fires it instead…
        assert_eq!(
            toggle.filter(&press(GamepadButton::DpadDown, 1.0)),
            ToggleAction::FireScreenshot
        );
        // …and the releases never flip the overlay.
        assert_eq!(
            toggle.filter(&press(GamepadButton::DpadDown, 0.0)),
            ToggleAction::Forward
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 0.0)),
            ToggleAction::Forward
        );
    }

    #[test]
    fn test_record_chord_fires_on_press() {
        // Arrange
        let mut toggle = OverlayToggle {
            screenshot: GAMEPAD_GUIDE | GAMEPAD_DPAD_DOWN,
            record: GAMEPAD_GUIDE | GAMEPAD_DPAD_UP,
            ..armed(GAMEPAD_GUIDE)
        };
        // Act + Assert
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 1.0)),
            ToggleAction::Swallow
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::DpadUp, 1.0)),
            ToggleAction::FireRecord
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::DpadUp, 0.0)),
            ToggleAction::Forward
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 0.0)),
            ToggleAction::Forward
        );
    }

    #[test]
    fn test_unmapped_sources_never_fire() {
        // Arrange: axis events and unmapped buttons.
        let mut toggle = armed(GAMEPAD_GUIDE);
        // Act + Assert
        assert_eq!(
            toggle.filter(&InputEvent {
                source: InputSource::Axis(crate::GamepadAxis::LeftX),
                value: 1.0,
                timestamp_us: 0,
            }),
            ToggleAction::Forward
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::Paddle1, 1.0)),
            ToggleAction::Forward
        );
    }

    #[test]
    fn test_connect_settle_swallows_without_firing() {
        // Arrange: just (re)connected while gripping Guide.
        let mut toggle = armed(GAMEPAD_GUIDE);
        toggle.note_connect();
        // Act + Assert: the grip press is swallowed, never fired…
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 1.0)),
            ToggleAction::Swallow
        );
        // …other buttons still forward…
        assert_eq!(
            toggle.filter(&press(GamepadButton::A, 1.0)),
            ToggleAction::Forward
        );
        // …and release forwards too (nothing stuck).
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 0.0)),
            ToggleAction::Forward
        );
    }

    #[test]
    fn test_flip_writes_live_shm() {
        // Arrange: a real SHM region armed through the session env.
        let db_id = 1_999_999_997i64;
        let _shm = MappedShm::create(db_id).expect("create test SHM");
        let name = ira_overlay_ipc::shm_path(db_id);
        let prev = std::env::var("IRA_OVERLAY_SHM").ok();
        let prev_toggle = std::env::var(DAEMON_TOGGLE_ENV).ok();
        std::env::set_var("IRA_OVERLAY_SHM", &name);
        std::env::set_var(DAEMON_TOGGLE_ENV, "1");
        // Act: arm from a borrowed env snapshot, press, release, flip.
        let env: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| k == "IRA_OVERLAY_SHM" || k == DAEMON_TOGGLE_ENV)
            .collect();
        let mut toggle = OverlayToggle::arm(Some(&env)).expect("armed");
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 1.0)),
            ToggleAction::Swallow
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 0.0)),
            ToggleAction::Fire
        );
        // Assert: the shared flag actually flipped.
        assert_eq!(toggle.flip(), Some(true));
        // Cleanup: env restored, tmpfs file removed (Drop only unmaps).
        match prev {
            Some(v) => std::env::set_var("IRA_OVERLAY_SHM", v),
            None => std::env::remove_var("IRA_OVERLAY_SHM"),
        }
        match prev_toggle {
            Some(v) => std::env::set_var(DAEMON_TOGGLE_ENV, v),
            None => std::env::remove_var(DAEMON_TOGGLE_ENV),
        }
        let c_name = std::ffi::CString::new(name).unwrap();
        unsafe { libc::shm_unlink(c_name.as_ptr()) };
    }

    #[test]
    fn test_screenshot_chord_queues_canvas_action() {
        use ira_overlay_ipc::{ACT_SCREENSHOT, CanvasShm};
        // Arrange: game SHM plus the host canvas it points at. Built
        // directly (no process env): a second env-mutating test running in
        // parallel would race `std::env::vars()` into UB.
        let db_id = 1_999_999_991i64;
        let mut game_shm = MappedShm::create(db_id).expect("create test SHM");
        game_shm.init_header(db_id);
        let mut canvas_shm = CanvasShm::create(db_id).expect("create test canvas");
        canvas_shm.init_header();
        let (toggle_mask, screenshot_mask, record_mask) = game_shm.header().gamepad_hotkeys();
        let mut toggle = OverlayToggle {
            toggle: toggle_mask,
            screenshot: screenshot_mask,
            record: record_mask,
            held: HashSet::new(),
            toggle_pending: false,
            canvas: None,
            shm: Some(game_shm),
            settle_until: None,
        };
        // Act: the default screenshot chord (Guide+DpadDown) completes.
        assert_eq!(
            toggle.filter(&press(GamepadButton::Guide, 1.0)),
            ToggleAction::Swallow
        );
        assert_eq!(
            toggle.filter(&press(GamepadButton::DpadDown, 1.0)),
            ToggleAction::FireScreenshot
        );
        assert!(toggle.push_screenshot());
        // Assert: the layer-visible action ring carries the screenshot.
        let reader = CanvasShm::open(&ira_overlay_ipc::canvas_shm_path(db_id)).unwrap();
        let (_, acts) = reader.drain_actions(0);
        assert!(acts.iter().any(|act| act.kind == ACT_SCREENSHOT));
        // Cleanup: tmpfs files removed (Drop only unmaps).
        let c_name = std::ffi::CString::new(ira_overlay_ipc::shm_path(db_id)).unwrap();
        unsafe { libc::shm_unlink(c_name.as_ptr()) };
        let c_canvas =
            std::ffi::CString::new(ira_overlay_ipc::canvas_shm_path(db_id)).unwrap();
        unsafe { libc::shm_unlink(c_canvas.as_ptr()) };
    }

    #[test]
    fn test_arm_needs_marker_and_shm() {
        // Arrange: no env, marker off, marker without SHM.
        // Act + Assert: all silent Nones (never panics, never arms).
        assert!(OverlayToggle::arm(None).is_none());
        assert!(OverlayToggle::arm(Some(&[])).is_none());
        assert!(
            OverlayToggle::arm(Some(&[(DAEMON_TOGGLE_ENV.to_string(), "1".to_string())])).is_none()
        );
        assert!(
            OverlayToggle::arm(Some(&[
                (DAEMON_TOGGLE_ENV.to_string(), "0".to_string()),
                ("IRA_OVERLAY_SHM".to_string(), "/ira_overlay_1".to_string()),
            ]))
            .is_none()
        );
    }
}
