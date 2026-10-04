//! Desktop pointer watch for gamescope mode.
//!
//! The gamescope X server delivers effectively no input to external
//! clients, but the desktop server always can — the user types and points
//! there no matter what the session shows. This thread grabs the toggle
//! chord plus (while engaged) the pointer buttons on the desktop display,
//! maps pointer positions into overlay space through the gamescope window
//! rect, and reports them to the GTK loop.
//!
//! Narrow grabs only: the game keeps every other key, and the desktop
//! keeps everything the moment focus leaves the gamescope window (or the
//! overlay hides; button releases get a 300ms grace so a closing click
//! never leaks its release anywhere). Navigation keys live on the session
//! display instead (proven delivery; duplicating them here would
//! double-step).

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use x11rb::connection::Connection as _;

use super::keys::{grab_buttons, grab_modifiers, grab_one, ungrab_buttons};
use super::xauth::connect_desktop;

struct GrabCtx {
    shm: ira_overlay_ipc::MappedShm,
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
    root_w: u32,
    root_h: u32,
    screen: (u32, u32),
    toggle: u32,
    tx: std::sync::mpsc::Sender<super::grab::PointerEvent>,
}

struct GrabState {
    pressed: bool,
    /// Pointer buttons grabbed (engaged). The chord stays grabbed regardless.
    narrow: bool,
    buttons_held: bool,
    /// Hide timestamp: the button grab lingers 300ms past hiding to eat
    /// the closing click's release.
    hide_at: Option<Instant>,
    pointer: (i32, i32),
    game_xid: Option<u32>,
    rect: Option<(i32, i32, u32, u32)>,
    rect_at: Instant,
    focused_logged: bool,
    on_game: bool,
    tick: u64,
}

/// Desktop input watch. Never returns except on setup failure, a dead
/// receiver, or a dead X connection.
pub(super) fn grab_loop(
    shm_name: &str,
    desktop_display: &str,
    desktop_auth: Option<&str>,
    tx: std::sync::mpsc::Sender<super::grab::PointerEvent>,
) {
    let Some((ctx, mut st)) = grab_setup(shm_name, desktop_display, desktop_auth, tx) else {
        return;
    };
    loop {
        st.tick += 1;
        let visible = ctx.shm.header().overlay_visible.load(Ordering::SeqCst) != 0;
        refresh_window(&ctx, &mut st);
        refresh_focus(&ctx, &mut st);
        update_engage(&ctx, &mut st, visible);
        update_grace(&ctx, &mut st);
        if !poll_pointer(&ctx, &mut st, visible) {
            return;
        }
        if !poll_grab_events(&ctx, &mut st) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn grab_setup(
    shm_name: &str,
    desktop_display: &str,
    desktop_auth: Option<&str>,
    tx: std::sync::mpsc::Sender<super::grab::PointerEvent>,
) -> Option<(GrabCtx, GrabState)> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt as _, EventMask};

    // Read-write: firing flips the flag through this mapping, and a
    // read-only mapping would segfault on the atomic write.
    let shm = match ira_overlay_ipc::MappedShm::open_rw(shm_name) {
        Ok(shm) => shm,
        Err(error) => {
            eprintln!("ira-overlay-ui: desktop input disabled, no game SHM: {error}");
            return None;
        }
    };
    let (tog_kc, tog_mods, _, _, _, _) = shm.header().hotkeys();
    // X11 keycodes are evdev + 8 (same domain as the shim hooks).
    let toggle = tog_kc + 8;

    let conn = match connect_desktop(desktop_display, desktop_auth) {
        Some(conn) => conn,
        None => {
            eprintln!("ira-overlay-ui: desktop input disabled, cannot reach {desktop_display}");
            return None;
        }
    };
    let (root, root_w, root_h) = match conn.setup().roots.first() {
        Some(screen) => {
            let geom = conn
                .get_geometry(screen.root)
                .ok()
                .and_then(|cookie| cookie.reply().ok());
            let (w, h) = geom
                .map(|g| (u32::from(g.width), u32::from(g.height)))
                .unwrap_or((1920, 1080));
            (screen.root, w, h)
        }
        None => {
            eprintln!("ira-overlay-ui: desktop input disabled, no root window");
            return None;
        }
    };
    for locked in grab_modifiers(tog_mods) {
        if !grab_one(&conn, root, locked, toggle) {
            // A refused chord only loses keyboard toggling (the gamepad
            // still flips); navigation and pointer stay valuable.
            eprintln!(
                "ira-overlay-ui: chord grab refused for key {toggle} mods {locked:#x}; continuing without it"
            );
            break;
        }
    }
    // Release notification for edge tracking: held keys re-fire presses,
    // which must not re-toggle (same rationale as the shim's toggle_edge).
    use x11rb::protocol::xproto::ChangeWindowAttributesAux;
    if conn
        .change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::KEY_RELEASE),
        )
        .is_err()
    {
        eprintln!("ira-overlay-ui: desktop input disabled, cannot watch releases");
        return None;
    }
    conn.flush().ok();
    eprintln!("ira-overlay-ui: desktop input on {desktop_display} (chord evdev {tog_kc})");
    Some((
        GrabCtx {
            shm,
            conn,
            root,
            root_w,
            root_h,
            screen: super::screen_size(),
            toggle,
            tx,
        },
        GrabState {
            pressed: false,
            narrow: false,
            buttons_held: false,
            hide_at: None,
            pointer: (0, 0),
            game_xid: None,
            rect: None,
            rect_at: Instant::now() - Duration::from_secs(60),
            focused_logged: false,
            on_game: true,
            tick: 0,
        },
    ))
}

/// Resolves the gamescope window (cached, refreshed every 20th tick once
/// older than 5s): desktop focus and pointer mapping both key off it.
fn refresh_window(ctx: &GrabCtx, st: &mut GrabState) {
    let stale = st.game_xid.is_none() || st.rect_at.elapsed() >= Duration::from_secs(5);
    if stale && st.tick.is_multiple_of(20) {
        st.rect_at = Instant::now();
        let pid = super::proc::session_leader_pid();
        st.game_xid = pid.and_then(|pid| find_window_by_pid(&ctx.conn, ctx.root, pid));
        st.rect = st
            .game_xid
            .and_then(|xid| window_rect(&ctx.conn, ctx.root, xid))
            .or(Some((0, 0, ctx.root_w, ctx.root_h)));
    }
}

/// Desktop focus gate: full authority only while the pointer's owner (the
/// user) sits on the gamescope window. Unknown window fails open — a
/// fullscreen session owns the screen anyway.
fn refresh_focus(ctx: &GrabCtx, st: &mut GrabState) {
    if !st.tick.is_multiple_of(20) {
        return;
    }
    let active = active_window(&ctx.conn, ctx.root);
    st.on_game = match (active, st.game_xid) {
        (Some(a), Some(xid)) => a == xid,
        _ => true,
    };
    if !st.on_game && !st.focused_logged {
        st.focused_logged = true;
        eprintln!("ira-overlay-ui: desktop focus left the game; pointer released");
    } else if st.on_game && st.focused_logged {
        st.focused_logged = false;
        eprintln!("ira-overlay-ui: desktop focus back on the game");
    }
}

fn update_engage(ctx: &GrabCtx, st: &mut GrabState, visible: bool) {
    let engaged = visible && st.on_game;
    if engaged && !st.narrow {
        grab_buttons(&ctx.conn, ctx.root);
        st.narrow = true;
        st.buttons_held = true;
        eprintln!("ira-overlay-ui: input grabbed (overlay input stolen)");
    } else if !engaged && st.narrow {
        st.narrow = false;
        if visible {
            // Focus left but the overlay is up: the desktop needs its
            // pointer back immediately.
            ungrab_buttons(&ctx.conn, ctx.root);
            st.buttons_held = false;
        } else {
            // Hidden: keep the buttons briefly to eat the closing
            // click's release, then let go.
            st.hide_at = Some(Instant::now());
        }
        eprintln!("ira-overlay-ui: input released (overlay hidden)");
    }
}

/// Releases the button grab 300ms after hiding. There is deliberately no
/// "clear while hidden" reset here: one used to sit below this check and
/// it strangled the grace on the very next tick, wedging the buttons
/// grabbed (and the desktop clickless) until the next show.
fn update_grace(ctx: &GrabCtx, st: &mut GrabState) {
    if st.buttons_held
        && let Some(at) = st.hide_at
        && at.elapsed() >= Duration::from_millis(300)
    {
        ungrab_buttons(&ctx.conn, ctx.root);
        st.buttons_held = false;
        st.hide_at = None;
    }
}

/// Server pointer, on change and only while engaged: mapped into overlay
/// space for the composited cursor. False when the receiver died.
fn poll_pointer(ctx: &GrabCtx, st: &mut GrabState, visible: bool) -> bool {
    let engaged = visible && st.on_game;
    if !engaged {
        return true;
    }
    let pos = pointer_raw(&ctx.conn, ctx.root);
    if let Some(pos) = pos
        && pos != st.pointer
    {
        st.pointer = pos;
        let rect = st.rect.unwrap_or((0, 0, ctx.root_w, ctx.root_h));
        let (x, y) = map_to_overlay(pos.0, pos.1, rect, ctx.screen);
        if ctx.tx.send(super::grab::PointerEvent::Move(x, y)).is_err() {
            return false;
        }
    }
    true
}

/// Root-absolute pointer position straight off the server, if it answers.
fn pointer_raw(conn: &impl x11rb::connection::Connection, root: u32) -> Option<(i32, i32)> {
    use x11rb::protocol::xproto::ConnectionExt as _;
    let reply = conn.query_pointer(root).ok()?.reply().ok()?;
    Some((i32::from(reply.root_x), i32::from(reply.root_y)))
}

/// Drains one poll of desktop events. False on receiver/connection death.
fn poll_grab_events(ctx: &GrabCtx, st: &mut GrabState) -> bool {
    let event = match ctx.conn.poll_for_event() {
        Ok(event) => event,
        Err(error) => {
            eprintln!("ira-overlay-ui: desktop input loop ended: {error}");
            return false;
        }
    };
    use x11rb::protocol::Event as XEvent;
    match event {
        Some(XEvent::KeyPress(event)) if event.detail as u32 == ctx.toggle && !st.pressed => {
            st.pressed = true;
            match ctx.shm.header().toggle_visible() {
                Some(visible) => {
                    eprintln!("ira-overlay-ui: desktop toggle flipped overlay to {visible}")
                }
                None => eprintln!("ira-overlay-ui: desktop toggle debounced"),
            }
        }
        Some(XEvent::KeyRelease(event)) if event.detail as u32 == ctx.toggle => {
            st.pressed = false;
        }
        // Pointer buttons while held: position comes from a fresh
        // query (event coordinates are window-relative, the poll is
        // root-absolute).
        Some(XEvent::ButtonPress(event)) if st.buttons_held => {
            let button = event.detail;
            if let Some((x, y)) = pointer_at(
                &ctx.conn, ctx.root, st.rect, ctx.screen, ctx.root_w, ctx.root_h,
            ) && ctx
                .tx
                .send(super::grab::PointerEvent::Press(x, y, button))
                .is_err()
            {
                return false;
            }
        }
        Some(XEvent::ButtonRelease(event)) if st.buttons_held => {
            let button = event.detail;
            if let Some((x, y)) = pointer_at(
                &ctx.conn, ctx.root, st.rect, ctx.screen, ctx.root_w, ctx.root_h,
            ) && ctx
                .tx
                .send(super::grab::PointerEvent::Release(x, y, button))
                .is_err()
            {
                return false;
            }
        }
        // Anything else grabbed is consumed on the floor: the game
        // (and the desktop, while engaged) sees nothing of it.
        _ => {}
    }
    true
}

/// Maps a desktop pointer position into overlay coordinates through the
/// gamescope window rect. Pure for testability.
fn map_to_overlay(px: i32, py: i32, rect: (i32, i32, u32, u32), screen: (u32, u32)) -> (f64, f64) {
    let (rx, ry, rw, rh) = rect;
    let x = (f64::from(px - rx) * f64::from(screen.0) / f64::from(rw.max(1)))
        .clamp(0.0, f64::from(screen.0));
    let y = (f64::from(py - ry) * f64::from(screen.1) / f64::from(rh.max(1)))
        .clamp(0.0, f64::from(screen.1));
    (x, y)
}

/// Root-absolute pointer position mapped into overlay space, if the
/// server answers. Shared by the engaged poll and the click handlers so
/// cursor and commands always agree.
fn pointer_at(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    rect: Option<(i32, i32, u32, u32)>,
    screen: (u32, u32),
    root_w: u32,
    root_h: u32,
) -> Option<(f64, f64)> {
    let pos = pointer_raw(conn, root)?;
    let rect = rect.unwrap_or((0, 0, root_w, root_h));
    Some(map_to_overlay(pos.0, pos.1, rect, screen))
}

/// Active desktop window (`_NET_ACTIVE_WINDOW`), if the WM publishes it.
fn active_window(conn: &impl x11rb::connection::Connection, root: u32) -> Option<u32> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
    let atom = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .ok()?
        .reply()
        .ok()?
        .atom;
    conn.get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()
        .and_then(|mut values| values.next())
}

/// Top-level window owned by `pid` (`_NET_WM_PID` over `_NET_CLIENT_LIST`).
fn find_window_by_pid(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    pid: u32,
) -> Option<u32> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
    let list_atom = conn
        .intern_atom(false, b"_NET_CLIENT_LIST")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let pid_atom = conn
        .intern_atom(false, b"_NET_WM_PID")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let reply = conn
        .get_property(false, root, list_atom, AtomEnum::WINDOW, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    let windows = reply.value32()?;
    for window in windows {
        let owned = conn
            .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()
            .and_then(|mut values| values.next());
        if owned == Some(pid) {
            return Some(window);
        }
    }
    None
}

/// Root-space rect of a client window (position translated out of any
/// window-manager frame).
fn window_rect(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    xid: u32,
) -> Option<(i32, i32, u32, u32)> {
    use x11rb::protocol::xproto::ConnectionExt as _;
    let geom = conn.get_geometry(xid).ok()?.reply().ok()?;
    let pos = conn
        .translate_coordinates(xid, root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    Some((
        i32::from(pos.dst_x),
        i32::from(pos.dst_y),
        u32::from(geom.width),
        u32::from(geom.height),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_to_overlay_scales_and_clamps() {
        // Fullscreen 1920 window on a 1920 screen maps 1:1.
        assert_eq!(
            map_to_overlay(960, 540, (0, 0, 1920, 1080), (1920, 1080)),
            (960.0, 540.0)
        );
        // Offset window scales into overlay space.
        assert_eq!(
            map_to_overlay(100, 100, (100, 100, 960, 540), (1920, 1080)),
            (0.0, 0.0)
        );
        // Outside clamps to the edges instead of wrapping.
        assert_eq!(
            map_to_overlay(-50, 2000, (0, 0, 1920, 1080), (1920, 1080)),
            (0.0, 1080.0)
        );
    }
}
