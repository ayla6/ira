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
}

fn x11_id(window: &gtk4::Window) -> Option<u32> {
    use glib::object::Cast as _;
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
    let intern = |name: &[u8]| {
        conn.intern_atom(false, name)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())
            .map(|r| r.atom)
    };
    let state = intern(b"_NET_WM_STATE")?;
    let skip_taskbar = intern(b"_NET_WM_STATE_SKIP_TASKBAR")?;
    let skip_pager = intern(b"_NET_WM_STATE_SKIP_PAGER")?;
    let data = [skip_taskbar.to_ne_bytes(), skip_pager.to_ne_bytes()].concat();
    conn.change_property(
        PropMode::APPEND,
        xid,
        state,
        AtomEnum::ATOM,
        32,
        2,
        &data,
    )
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
    let atom = conn
        .intern_atom(false, name)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .atom;
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
}
