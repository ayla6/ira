//! Pause injection while the launched game window is unfocused.
//!
//! Wine games run under XWayland, so the EWMH property chain on the X server
//! tells us which window is active and which pid owns it. On setups without
//! an X server (native Wayland games, headless tests) [`FocusWatcher`]
//! cannot be built and callers treat that as always-focused — the feature
//! degrades to off rather than blocking input.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, Window};
use x11rb::rust_connection::RustConnection;

/// How many transient-parent hops to walk looking for the game pid; wine
/// wraps game windows in a small chain.
const MAX_PARENT_HOPS: usize = 8;

pub struct FocusWatcher {
    conn: RustConnection,
    root: Window,
    child_pid: u32,
    active_atom: u32,
    pid_atom: u32,
    transient_atom: u32,
}

impl FocusWatcher {
    /// None when there is no X server to ask; callers treat that as
    /// always-focused.
    pub fn for_child(child_pid: u32) -> Option<Self> {
        let (conn, screen) = RustConnection::connect(None).ok()?;
        let root = conn.setup().roots.get(screen).map(|screen| screen.root)?;
        let (active_atom, pid_atom, transient_atom) = {
            let atom = |name: &[u8]| -> Option<u32> {
                Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
            };
            (
                atom(b"_NET_ACTIVE_WINDOW")?,
                atom(b"_NET_WM_PID")?,
                atom(b"WM_TRANSIENT_FOR")?,
            )
        };
        Some(Self {
            conn,
            root,
            child_pid,
            active_atom,
            pid_atom,
            transient_atom,
        })
    }

    /// Whether the game's window currently has input focus. Unknown states
    /// count as focused so a WM without these hints never pauses the game.
    pub fn game_is_focused(&self) -> bool {
        let window = match self
            .conn
            .get_property(false, self.root, self.active_atom, AtomEnum::WINDOW, 0, 1)
        {
            Ok(cookie) => match cookie.reply() {
                Ok(reply) => reply
                    .value32()
                    .and_then(|mut values| values.next())
                    .unwrap_or(0),
                Err(_) => return true,
            },
            Err(_) => return true,
        };
        // 0 / root means the desktop itself is focused.
        if window == 0 || window == self.root {
            return false;
        }
        let mut current = Some(window);
        for _ in 0..MAX_PARENT_HOPS {
            let Some(candidate) = current else {
                return true;
            };
            if self.window_belongs_to_child(candidate) {
                return true;
            }
            current = self.transient_parent(candidate);
        }
        true
    }

    fn window_belongs_to_child(&self, window: Window) -> bool {
        match self
            .conn
            .get_property(false, window, self.pid_atom, AtomEnum::CARDINAL, 0, 1)
        {
            Ok(cookie) => match cookie.reply() {
                Ok(reply) => reply
                    .value32()
                    .and_then(|mut values| values.next())
                    .is_some_and(|pid| pid == self.child_pid),
                Err(_) => false,
            },
            Err(_) => false,
        }
    }

    fn transient_parent(&self, window: Window) -> Option<Window> {
        let cookie = self
            .conn
            .get_property(false, window, self.transient_atom, AtomEnum::WINDOW, 0, 1)
            .ok()?;
        let reply = cookie.reply().ok()?;
        reply
            .value32()
            .and_then(|mut values| values.next())
    }
}
