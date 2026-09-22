//! Canvas command + action types for the out-of-process GTK overlay.
//!
//! The Vulkan layer captures input in the game process and pushes
//! [`CanvasCommand`]s (layer → host). The GTK host (`ira-overlay-ui`)
//! applies them to real widgets and pushes [`CanvasAction`]s back
//! (host → layer) for things that must execute game-side
//! (screenshot readback, record toggle).
//!
//! Transport lives in [`crate::canvas`] — this file is pure types only,
//! no SHM dependency. Rings are SPSC with a single atomic write index;
//! each side tracks its own read index locally (same pattern as the
//! notification ring in `protocol.rs`).

/// Layer → host command kinds.
pub const CMD_NAV_UP: u32 = 0;
pub const CMD_NAV_DOWN: u32 = 1;
pub const CMD_NAV_LEFT: u32 = 2;
pub const CMD_NAV_RIGHT: u32 = 3;
pub const CMD_ACTIVATE: u32 = 4;
pub const CMD_SCROLL: u32 = 5;
pub const CMD_MOUSE_MOVE: u32 = 6;
pub const CMD_MOUSE_DOWN: u32 = 7;
pub const CMD_MOUSE_UP: u32 = 8;
pub const CMD_SHOW: u32 = 9;
pub const CMD_HIDE: u32 = 10;

/// Host → layer action kinds.
pub const ACT_SCREENSHOT: u32 = 0;
pub const ACT_TOGGLE_RECORD: u32 = 1;
pub const ACT_HIDE_OVERLAY: u32 = 2;

/// Ring capacities. Commands burst per frame (one per input event);
/// 64 covers a full `InputEventRaw` poll batch. Actions are rare.
pub const MAX_CANVAS_COMMANDS: usize = 64;
pub const MAX_CANVAS_ACTIONS: usize = 16;

/// One layer → host input command.
///
/// `a`/`b` carry coordinates (mouse) or deltas (scroll `a` = dy);
/// `c` carries the button index for mouse down/up. Nav/activate/show/hide
/// ignore all payload fields.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CanvasCommand {
    pub kind: u32,
    pub a: i32,
    pub b: i32,
    pub c: u32,
}

impl CanvasCommand {
    pub fn nav(kind: u32) -> Self {
        Self {
            kind,
            a: 0,
            b: 0,
            c: 0,
        }
    }

    pub fn scroll(delta_y: f32) -> Self {
        Self {
            kind: CMD_SCROLL,
            a: delta_y as i32,
            b: 0,
            c: 0,
        }
    }

    pub fn mouse_move(x: f32, y: f32) -> Self {
        Self {
            kind: CMD_MOUSE_MOVE,
            a: x as i32,
            b: y as i32,
            c: 0,
        }
    }

    pub fn mouse_down(x: f32, y: f32, button: u32) -> Self {
        Self {
            kind: CMD_MOUSE_DOWN,
            a: x as i32,
            b: y as i32,
            c: button,
        }
    }

    pub fn mouse_up(x: f32, y: f32, button: u32) -> Self {
        Self {
            kind: CMD_MOUSE_UP,
            a: x as i32,
            b: y as i32,
            c: button,
        }
    }
}

/// One host → layer action. Payload-free for now; the kind is the message.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CanvasAction {
    pub kind: u32,
    pub _pad: [u8; 12],
}

impl CanvasAction {
    pub fn of(kind: u32) -> Self {
        Self { kind, _pad: [0; 12] }
    }
}

/// Converts an [`crate::protocol::InputEventRaw`] shim event into the
/// equivalent canvas command. Only press/move/scroll cross the ring —
/// releases (4) and capture hotkeys (5/6) are handled game-side by the layer.
/// Returns `None` for anything else.
pub fn command_from_raw(
    event_type: u32,
    x: i32,
    y: i32,
    button: u32,
    keycode: u32,
) -> Option<CanvasCommand> {
    match event_type {
        0 => Some(CanvasCommand::mouse_move(x as f32, y as f32)),
        1 => Some(CanvasCommand::mouse_down(x as f32, y as f32, button)),
        2 => Some(CanvasCommand::mouse_up(x as f32, y as f32, button)),
        3 => nav_from_keycode(keycode),
        7 => Some(CanvasCommand::scroll(y as f32)),
        _ => None,
    }
}

fn nav_from_keycode(keycode: u32) -> Option<CanvasCommand> {
    use crate::protocol::ShmHeader;
    let codes = ShmHeader::NAV_KEYCODES_X11;
    let idx = codes.iter().position(|&c| c == keycode)?;
    // Canonical order: [Return, Up, Down, Left, Right].
    let kind = match idx {
        0 => CMD_ACTIVATE,
        1 => CMD_NAV_UP,
        2 => CMD_NAV_DOWN,
        3 => CMD_NAV_LEFT,
        4 => CMD_NAV_RIGHT,
        _ => return None,
    };
    Some(CanvasCommand::nav(kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_sizes_are_fixed() {
        assert_eq!(std::mem::size_of::<CanvasCommand>(), 16);
        assert_eq!(std::mem::size_of::<CanvasAction>(), 16);
    }

    #[test]
    fn test_mouse_command_roundtrip() {
        let c = CanvasCommand::mouse_down(10.0, 20.0, 0);
        assert_eq!(c.kind, CMD_MOUSE_DOWN);
        assert_eq!((c.a, c.b, c.c), (10, 20, 0));
    }

    #[test]
    fn test_raw_mouse_maps_to_command() {
        let c = command_from_raw(0, 5, 6, 0, 0).unwrap();
        assert_eq!(c.kind, CMD_MOUSE_MOVE);
        assert_eq!((c.a, c.b), (5, 6));
    }

    #[test]
    fn test_raw_unknown_key_maps_to_none() {
        assert_eq!(command_from_raw(3, 0, 0, 0, 999_999), None);
        assert_eq!(command_from_raw(4, 0, 0, 0, 0), None);
        assert_eq!(command_from_raw(5, 0, 0, 0, 0), None);
        assert_eq!(command_from_raw(6, 0, 0, 0, 0), None);
        assert_eq!(command_from_raw(99, 0, 0, 0, 0), None);
    }

    #[test]
    fn test_raw_nav_key_maps_to_command() {
        use crate::protocol::ShmHeader;
        let codes = ShmHeader::NAV_KEYCODES_X11;
        assert_eq!(
            command_from_raw(3, 0, 0, 0, codes[1]).unwrap().kind,
            CMD_NAV_UP
        );
        assert_eq!(
            command_from_raw(3, 0, 0, 0, codes[0]).unwrap().kind,
            CMD_ACTIVATE
        );
    }
}
