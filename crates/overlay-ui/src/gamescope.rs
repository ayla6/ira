//! Gamescope mode: the host window itself is the overlay plane.
//!
//! Marks the (X11) window as `GAMESCOPE_EXTERNAL_OVERLAY` so gamescope
//! composites it above the game as a separate plane, like mangoapp.
//! Without the atom gamescope would treat the window as the main plane and
//! take over the display. Falls back to a plain fullscreen window when not
//! on X11 (Wayland backend, nested session without XWayland).

use gdk4::prelude::ToplevelExt;
use glib::object::Cast as _;
use gtk4::prelude::{GtkWindowExt, NativeExt};

pub fn is_gamescope_mode() -> bool {
    std::env::var_os("IRA_OVERLAY_GAMESCOPE").is_some()
}

/// Picks the gamescope Xwayland display from connectable candidates:
/// newest first (the just-launched session wins), ties broken by lowest
/// number. Pure for testability; IO lives in `discover_xwayland_display`.
fn pick_display(candidates: &[(u32, std::time::SystemTime)]) -> Option<u32> {
    let mut best: Option<(u32, std::time::SystemTime)> = None;
    for (num, time) in candidates {
        let better = match best {
            None => true,
            Some((best_num, best_time)) => {
                *time > best_time || (*time == best_time && *num < best_num)
            }
        };
        if better {
            best = Some((*num, *time));
        }
    }
    best.map(|(num, _)| num)
}

/// Finds the gamescope Xwayland display for this host to live on. Nested
/// gamescope serves its own X server (usually `:2`); inheriting the
/// desktop `:0` leaves the overlay composited nowhere, buried under the
/// fullscreen session window. Desktop `:0`/`:1` are never candidates (in
/// an embedded session only `:0` exists and is kept). Returns `None` to
/// keep the inherited `DISPLAY`.
pub fn discover_xwayland_display() -> Option<u32> {
    // Override for discovery (e.g. `:3`).
    if let Ok(display) = std::env::var("IRA_OVERLAY_XWAYLAND_DISPLAY") {
        return display
            .trim_start_matches(':')
            .parse()
            .ok()
            .filter(|num| *num > 1);
    }
    let mut candidates = Vec::new();
    let entries = std::fs::read_dir("/tmp/.X11-unix").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(num) = name
            .to_str()
            .and_then(|n| n.strip_prefix('X'))
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        // Desktop servers live on :0/:1; embedded sessions have nothing
        // else and keep the inherited display via the None below.
        if num < 2 {
            continue;
        }
        let path = format!("/tmp/.X11-unix/X{num}");
        if std::os::unix::net::UnixStream::connect(&path).is_err() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        candidates.push((num, modified));
    }
    pick_display(&candidates)
}

/// Pushes the injected-mode helper window out of the user's way: lowered in
/// stacking order everywhere, and out of the taskbar/pager on X11. (GTK4
/// dropped the old skip-taskbar/keep-below window hints.)
pub fn background_helper(window: &gtk4::Window) {
    let surface = window.surface();
    if let Some(toplevel) = surface
        .as_ref()
        .and_then(|s| s.downcast_ref::<gdk4::Toplevel>())
    {
        toplevel.lower();
    }
    if let Some(xid) = x11_id(window)
        && let Err(e) = set_net_wm_state(xid)
    {
        eprintln!("ira-overlay-ui: failed to set taskbar atoms: {e}");
    }
}

pub fn setup_external_overlay(window: &gtk4::Window) {
    window.fullscreen();
    let Some(xid) = x11_id(window) else {
        eprintln!("ira-overlay-ui: not on X11, gamescope overlay plane unavailable");
        return;
    };
    if let Err(e) = set_external_overlay(xid) {
        eprintln!("ira-overlay-ui: failed to set external overlay atom: {e}");
    } else {
        eprintln!("ira-overlay-ui: external overlay plane ready (xid={xid})");
    }
    // Start fully hidden: gamescope paints the external plane whenever our
    // opacity is nonzero and never checks map state on the paint path, so a
    // merely unmapped window leaves a frozen frame on screen forever. The
    // tick restores opacity on show. (Upstream notes opacity=0 as the hide
    // signal for its own overlay planes.)
    if let Err(e) = set_window_opacity(xid, 0) {
        eprintln!("ira-overlay-ui: failed to zero initial opacity: {e}");
    }
}

/// Opaque `_NET_WM_WINDOW_OPACITY`: gamescope's paint and focus paths key
/// the external plane off nonzero opacity, ignoring unmap.
pub const OPAQUE: u32 = 0xFFFF_FFFF;

/// Signals (un)hidden to gamescope through the sanctioned channel: nonzero
/// opacity paints the plane, zero drops it — independent of map state.
pub(crate) fn set_window_opacity(xid: u32, opacity: u32) -> Result<(), String> {
    set_cardinal(xid, b"_NET_WM_WINDOW_OPACITY", opacity)
}

pub(crate) fn x11_id(window: &gtk4::Window) -> Option<u32> {
    let surface = window.surface()?;
    let x11 = surface.downcast_ref::<gdkx11::X11Surface>()?;
    Some(x11.xid() as u32)
}

fn set_external_overlay(xid: u32) -> Result<(), String> {
    set_cardinal(xid, b"GAMESCOPE_EXTERNAL_OVERLAY", 1)
}

/// Appends `_NET_WM_STATE_SKIP_TASKBAR`/`_SKIP_PAGER` to the window state.
fn set_net_wm_state(xid: u32) -> Result<(), String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
    let (conn, _) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let state = intern(&conn, b"_NET_WM_STATE")?;
    let skip_taskbar = intern(&conn, b"_NET_WM_STATE_SKIP_TASKBAR")?;
    let skip_pager = intern(&conn, b"_NET_WM_STATE_SKIP_PAGER")?;
    let data = [skip_taskbar.to_ne_bytes(), skip_pager.to_ne_bytes()].concat();
    conn.change_property(PropMode::APPEND, xid, state, AtomEnum::ATOM, 32, 2, &data)
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn set_cardinal(xid: u32, name: &[u8], value: u32) -> Result<(), String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
    let (conn, _) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let atom = intern(&conn, name)?;
    conn.change_property(
        PropMode::REPLACE,
        xid,
        atom,
        AtomEnum::CARDINAL,
        32,
        1,
        &value.to_ne_bytes(),
    )
    .map_err(|e| e.to_string())?
    .check()
    .map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn intern(conn: &x11rb::rust_connection::RustConnection, name: &[u8]) -> Result<u32, String> {
    use x11rb::protocol::xproto::ConnectionExt as _;
    conn.intern_atom(false, name)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())
        .map(|r| r.atom)
}

/// Persistent X connection for unmap/remap/focus queries. Gamescope never
/// focuses external overlay planes itself and re-asserts the game on every
/// commit, so steals never stick (and are not attempted); an unmapped game
/// window drops out of the focus list instead. One connection kept for the
/// session: per-call connects were slow and spammed connections.
pub(crate) struct FocusConn {
    conn: x11rb::rust_connection::RustConnection,
}

impl FocusConn {
    /// Connects to the gamescope Xwayland server (`None` when unreachable;
    /// callers treat that as "cannot own focus" and carry on).
    pub(crate) fn connect() -> Option<Self> {
        let (conn, _) = x11rb::connect(None).ok()?;
        Some(Self { conn })
    }

    /// Current X input focus window, if queryable.
    pub(crate) fn current_focus(&self) -> Option<u32> {
        use x11rb::protocol::xproto::ConnectionExt as _;
        self.conn
            .get_input_focus()
            .ok()?
            .reply()
            .ok()
            .map(|reply| reply.focus)
    }

    /// Unmaps a window (makes it unfocusable and invisible). Used on the
    /// game window while the overlay is up: racing gamescope for X focus
    /// never sticks, but an unmapped window cannot hold focus at all, so
    /// focus-pausing games pause deterministically. The game process is
    /// untouched (no signals); remap brings it back.
    pub(crate) fn unmap_window(&self, xid: u32) -> Result<(), String> {
        use x11rb::protocol::xproto::ConnectionExt as _;
        self.conn
            .unmap_window(xid)
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| e.to_string())?;
        self.flush()?;
        Ok(())
    }

    /// Remaps a window unmapped by [`FocusConn::unmap_window`].
    pub(crate) fn map_window(&self, xid: u32) -> Result<(), String> {
        use x11rb::protocol::xproto::ConnectionExt as _;
        self.conn
            .map_window(xid)
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| e.to_string())?;
        self.flush()?;
        Ok(())
    }

    fn flush(&self) -> Result<(), String> {
        use x11rb::connection::Connection;
        self.conn.flush().map_err(|e| e.to_string())
    }

    /// Whether the window is currently viewable (mapped). Lets the host
    /// retreat if something (gamescope) remaps the game behind its back
    /// instead of fighting it forever.
    pub(crate) fn is_viewable(&self, xid: u32) -> Option<bool> {
        use x11rb::protocol::xproto::{ConnectionExt as _, MapState};
        let attrs = self.conn.get_window_attributes(xid).ok()?.reply().ok()?;
        Some(attrs.map_state == MapState::VIEWABLE)
    }

    /// Identifies a window for the log: (window-manager name, owning pid).
    /// Proves an unmap/steal target really is the game and not some helper
    /// window that happened to hold focus.
    pub(crate) fn window_info(&self, xid: u32) -> (Option<String>, Option<u32>) {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
        let atom = |name: &[u8]| intern(&self.conn, name).ok();
        let utf8 = atom(b"UTF8_STRING").unwrap_or(AtomEnum::STRING.into());
        let name = atom(b"_NET_WM_NAME")
            .and_then(|a| {
                self.conn
                    .get_property(false, xid, a, utf8, 0, 256)
                    .ok()?
                    .reply()
                    .ok()
            })
            .and_then(|reply| String::from_utf8(reply.value8()?.collect()).ok());
        let pid = atom(b"_NET_WM_PID").and_then(|a| {
            self.conn
                .get_property(false, xid, a, AtomEnum::CARDINAL, 0, 1)
                .ok()?
                .reply()
                .ok()?
                .value32()
                .and_then(|mut values| values.next())
        });
        (name, pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gamescope_mode_follows_env() {
        let prev = std::env::var_os("IRA_OVERLAY_GAMESCOPE");
        unsafe { std::env::remove_var("IRA_OVERLAY_GAMESCOPE") };
        assert!(!is_gamescope_mode());
        unsafe { std::env::set_var("IRA_OVERLAY_GAMESCOPE", "1") };
        assert!(is_gamescope_mode());
        match prev {
            Some(v) => unsafe { std::env::set_var("IRA_OVERLAY_GAMESCOPE", v) },
            None => unsafe { std::env::remove_var("IRA_OVERLAY_GAMESCOPE") },
        }
    }

    #[test]
    fn test_pick_display_prefers_newest_then_lowest() {
        use std::time::{Duration, UNIX_EPOCH};
        // Arrange
        let t0 = UNIX_EPOCH;
        let t1 = UNIX_EPOCH + Duration::from_secs(10);
        // Act + Assert: empty picks nothing, newest wins, ties go low.
        assert_eq!(pick_display(&[]), None);
        assert_eq!(pick_display(&[(2, t0)]), Some(2));
        assert_eq!(pick_display(&[(2, t0), (3, t1)]), Some(3));
        assert_eq!(pick_display(&[(3, t1), (2, t1)]), Some(2));
    }
}
