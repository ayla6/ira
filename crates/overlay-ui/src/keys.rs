//! Shared X11 key/button grab primitives for the input threads.
//!
//! Both the session chord watch (`chord.rs`) and the desktop pointer watch
//! (`desktop.rs`) grab the same toggle chord and push the same navigation
//! commands; the helpers here keep the two policies identical.

use std::time::Duration;

/// X11 lock masks a grab must cover: each hotkey needs a grab per
/// NumLock/CapsLock combination or it silently misses.
pub(super) fn grab_modifiers(base: u32) -> [u32; 4] {
    const LOCK: u32 = 0x02;
    const MOD2: u32 = 0x10;
    [base, base | MOD2, base | LOCK, base | MOD2 | LOCK]
}

/// One passive chord grab. Checked: the toggle must land, everything
/// else is best-effort.
pub(super) fn grab_one(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    mods: u32,
    key: u32,
) -> bool {
    use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask};
    conn.grab_key(
        false,
        root,
        ModMask::from(mods as u16),
        key as u8,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
    )
    .map(|cookie| cookie.check().is_ok())
    .unwrap_or(false)
}

/// X11 `AnyModifier`: one grab per keycode covers every modifier
/// combination, locks included.
const ANY_MODS: u16 = 0x8000;

/// X11 `AnyButton` (0): one button grab covers every mouse button.
const ANY_BUTTON: u8 = 0;

/// Steal the pointer buttons while engaged: presses land here instead of
/// the desktop or the game.
pub(super) fn grab_buttons(conn: &impl x11rb::connection::Connection, root: u32) {
    use x11rb::protocol::xproto::{ButtonIndex, ConnectionExt as _, EventMask, GrabMode, ModMask};
    let _ = conn.grab_button(
        false,
        root,
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
        root,
        0u32,
        ButtonIndex::from(ANY_BUTTON),
        ModMask::from(ANY_MODS),
    );
    conn.flush().ok();
}

pub(super) fn ungrab_buttons(conn: &impl x11rb::connection::Connection, root: u32) {
    use x11rb::protocol::xproto::{ButtonIndex, ConnectionExt as _, ModMask};
    let _ = conn.ungrab_button(ButtonIndex::from(ANY_BUTTON), root, ModMask::from(ANY_MODS));
    conn.flush().ok();
}

/// Steal everything on the session display: passive grabs on the whole
/// keycode range, async and unchecked (a refused key just keeps flowing;
/// the chord is the only grab that must land).
pub(super) fn grab_all_keys(conn: &impl x11rb::connection::Connection, root: u32) {
    use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask};
    for key in 8..=255u8 {
        let _ = conn.grab_key(
            false,
            root,
            ModMask::from(ANY_MODS),
            key,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        );
    }
    conn.flush().ok();
}

pub(super) fn ungrab_all_keys(conn: &impl x11rb::connection::Connection, root: u32) {
    use x11rb::protocol::xproto::{ConnectionExt as _, ModMask};
    for key in 8..=255u8 {
        let _ = conn.ungrab_key(key, root, ModMask::from(ANY_MODS));
    }
    conn.flush().ok();
}

/// Big Picture repeat cadence for held keys, mirroring the gamepad path.
pub(super) const REPEAT_DELAY: Duration = Duration::from_millis(400);
pub(super) const REPEAT_RATE: Duration = Duration::from_millis(120);

/// X11 keycodes are evdev + 8; these are the evdev codes translated.
const EVDEV_ESC: u32 = 1;
const EVDEV_ENTER: u32 = 28;
const EVDEV_KP_ENTER: u32 = 96;
const EVDEV_F4: u32 = 62;
const EVDEV_LEFT: u32 = 105;
const EVDEV_RIGHT: u32 = 106;
const EVDEV_UP: u32 = 103;
const EVDEV_DOWN: u32 = 108;
/// X11 keycode of F4 (evdev 62 + 8): with Alt held, exits the game.
/// Matched by keycode in the press handler (the wide grab already owns
/// the key); Alt is read from the event's modifier state.
pub(super) const ALT_F4_X11: u32 = EVDEV_F4 + 8;

/// Canvas command kind for an X11 keycode, if it navigates. Pure for
/// testability.
pub(super) fn nav_command(key: u32) -> Option<u32> {
    use ira_overlay_ipc::{
        CMD_ACTIVATE, CMD_BACK, CMD_NAV_DOWN, CMD_NAV_LEFT, CMD_NAV_RIGHT, CMD_NAV_UP,
    };
    match key.wrapping_sub(8) {
        EVDEV_UP => Some(CMD_NAV_UP),
        EVDEV_DOWN => Some(CMD_NAV_DOWN),
        EVDEV_LEFT => Some(CMD_NAV_LEFT),
        EVDEV_RIGHT => Some(CMD_NAV_RIGHT),
        EVDEV_ENTER | EVDEV_KP_ENTER => Some(CMD_ACTIVATE),
        EVDEV_ESC => Some(CMD_BACK),
        _ => None,
    }
}

/// Only directions auto-repeat; activate/back always single-fire, like
/// the gamepad path.
pub(super) fn repeats(key: u32) -> bool {
    use ira_overlay_ipc::{CMD_NAV_DOWN, CMD_NAV_LEFT, CMD_NAV_RIGHT, CMD_NAV_UP};
    matches!(
        nav_command(key),
        Some(CMD_NAV_UP) | Some(CMD_NAV_DOWN) | Some(CMD_NAV_LEFT) | Some(CMD_NAV_RIGHT)
    )
}

/// Pushes one keyboard navigation command to the host canvas, opening it
/// lazily (the host creates it after this thread starts). Returns the
/// canvas handle for reuse; failures retry on the next press.
/// Alt+F4 (exit) shares the path with navigation.
pub(super) fn push_command_kind(
    shm: &ira_overlay_ipc::MappedShm,
    canvas: Option<ira_overlay_ipc::CanvasShm>,
    kind: u32,
) -> Option<ira_overlay_ipc::CanvasShm> {
    use ira_overlay_ipc::{CanvasCommand, canvas_shm_path};
    let mut canvas = canvas;
    if canvas.is_none() {
        canvas =
            ira_overlay_ipc::CanvasShm::open_rw(&canvas_shm_path(shm.header().game_db_id)).ok();
        if canvas.is_some() {
            eprintln!("ira-overlay-ui: keyboard navigation armed");
        }
    }
    if let Some(canvas) = canvas.as_mut() {
        canvas.push_command(CanvasCommand::nav(kind));
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grab_modifiers_cover_lock_combos() {
        // Arrange: Shift+Tab chord mask.
        // Act
        let variants = grab_modifiers(0x01);
        // Assert: base plus every NumLock/CapsLock combination, no dupes.
        assert_eq!(variants, [0x01, 0x11, 0x03, 0x13]);
        let mut sorted = variants.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 4);
    }

    #[test]
    fn test_grab_modifiers_empty_base() {
        // F12-style chord with no modifiers still gets lock coverage.
        assert_eq!(grab_modifiers(0), [0, 0x10, 0x02, 0x12]);
    }

    #[test]
    fn test_nav_command_maps_x11_keycodes() {
        use ira_overlay_ipc::{
            CMD_ACTIVATE, CMD_BACK, CMD_NAV_DOWN, CMD_NAV_LEFT, CMD_NAV_RIGHT, CMD_NAV_UP,
        };
        // X11 keycodes are evdev + 8.
        assert_eq!(nav_command(103 + 8), Some(CMD_NAV_UP));
        assert_eq!(nav_command(108 + 8), Some(CMD_NAV_DOWN));
        assert_eq!(nav_command(105 + 8), Some(CMD_NAV_LEFT));
        assert_eq!(nav_command(106 + 8), Some(CMD_NAV_RIGHT));
        assert_eq!(nav_command(28 + 8), Some(CMD_ACTIVATE));
        assert_eq!(nav_command(96 + 8), Some(CMD_ACTIVATE));
        assert_eq!(nav_command(1 + 8), Some(CMD_BACK));
        // Ordinary keys never navigate.
        assert_eq!(nav_command(30 + 8), None);
        assert_eq!(nav_command(15 + 8), None);
    }

    #[test]
    fn test_repeats_only_directions() {
        assert!(repeats(103 + 8));
        assert!(repeats(108 + 8));
        assert!(!repeats(28 + 8));
        assert!(!repeats(1 + 8));
        assert!(!repeats(30 + 8));
    }
}
