//! Session-display chord fallback (Shift+Tab + navigation).
//!
//! The :2 server delivers effectively no input to external clients, but the
//! chord demonstrably fires there — so this thread grabs the chord on the
//! session display, and while visible widens to every keycode with each
//! press edge translated to the canvas commands the gamepad drives.
//! Single-fire for activate/back/exit, Big Picture repeat for directions.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use x11rb::connection::Connection as _;

use super::keys::{
    ALT_F4_X11, REPEAT_DELAY, REPEAT_RATE, grab_all_keys, grab_modifiers, grab_one, nav_command,
    push_command_kind, repeats, ungrab_all_keys,
};

struct ChordCtx {
    shm: ira_overlay_ipc::MappedShm,
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
    toggle: u32,
    tog_mods: u32,
}

#[derive(Default)]
struct ChordState {
    pressed: bool,
    wide: bool,
    nav_held: HashMap<u32, (Instant, Instant)>,
    canvas: Option<ira_overlay_ipc::CanvasShm>,
}

/// Chord-plus-navigation watch on the session display. Never returns
/// except when the X connection dies.
pub(super) fn chord_loop(shm_name: &str) {
    let Some((ctx, mut st)) = chord_setup(shm_name) else {
        return;
    };
    loop {
        update_chord_wide(&ctx, &mut st);
        repeat_held(&ctx, &mut st);
        if !poll_chord_events(&ctx, &mut st) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn chord_setup(shm_name: &str) -> Option<(ChordCtx, ChordState)> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt as _, EventMask};

    let shm = match ira_overlay_ipc::MappedShm::open_rw(shm_name) {
        Ok(shm) => shm,
        Err(error) => {
            eprintln!("ira-overlay-ui: chord fallback disabled, no game SHM: {error}");
            return None;
        }
    };
    let (tog_kc, tog_mods, _, _, _, _) = shm.header().hotkeys();
    let toggle = tog_kc + 8;

    let conn = match x11rb::connect(None) {
        Ok((conn, _)) => conn,
        Err(error) => {
            eprintln!("ira-overlay-ui: chord fallback disabled, no session X connection: {error}");
            return None;
        }
    };
    let root = match conn.setup().roots.first() {
        Some(screen) => screen.root,
        None => {
            eprintln!("ira-overlay-ui: chord fallback disabled, no root window");
            return None;
        }
    };
    let mut landed = 0;
    for locked in grab_modifiers(tog_mods) {
        landed += usize::from(grab_one(&conn, root, locked, toggle));
    }
    if landed == 0 {
        eprintln!("ira-overlay-ui: chord fallback disabled, grabs refused");
        return None;
    }
    use x11rb::protocol::xproto::ChangeWindowAttributesAux;
    if conn
        .change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::KEY_RELEASE),
        )
        .is_err()
    {
        eprintln!("ira-overlay-ui: chord fallback disabled, cannot watch releases");
        return None;
    }
    conn.flush().ok();
    eprintln!("ira-overlay-ui: session chord grabbed (evdev {tog_kc})");
    Some((
        ChordCtx {
            shm,
            conn,
            root,
            toggle,
            tog_mods,
        },
        ChordState::default(),
    ))
}

/// Widens the grab to every keycode while visible (and shrinks it back on
/// hide, re-asserting the chord the mass ungrab released too).
fn update_chord_wide(ctx: &ChordCtx, st: &mut ChordState) {
    let visible = ctx.shm.header().overlay_visible.load(Ordering::SeqCst) != 0;
    if visible && !st.wide {
        grab_all_keys(&ctx.conn, ctx.root);
        st.wide = true;
        eprintln!("ira-overlay-ui: session keys stolen while visible");
    } else if !visible && st.wide {
        ungrab_all_keys(&ctx.conn, ctx.root);
        // Re-assert the chord: the mass ungrab above released it too.
        for locked in grab_modifiers(ctx.tog_mods) {
            grab_one(&ctx.conn, ctx.root, locked, ctx.toggle);
        }
        ctx.conn.flush().ok();
        st.wide = false;
        st.nav_held.clear();
        eprintln!("ira-overlay-ui: session keys released (overlay hidden)");
    }
}

/// Held direction repeat, Big Picture cadence.
fn repeat_held(ctx: &ChordCtx, st: &mut ChordState) {
    if !st.wide {
        return;
    }
    let now = Instant::now();
    for (key, (since, last)) in st.nav_held.iter_mut() {
        if repeats(*key)
            && now.duration_since(*since) >= REPEAT_DELAY
            && now.duration_since(*last) >= REPEAT_RATE
            && let Some(kind) = nav_command(*key)
        {
            *last = now;
            st.canvas = push_command_kind(&ctx.shm, st.canvas.take(), kind);
        }
    }
}

/// Drains one poll of session events. False when the X connection died.
fn poll_chord_events(ctx: &ChordCtx, st: &mut ChordState) -> bool {
    let event = match ctx.conn.poll_for_event() {
        Ok(event) => event,
        Err(error) => {
            eprintln!("ira-overlay-ui: session input loop ended: {error}");
            return false;
        }
    };
    use x11rb::protocol::Event as XEvent;
    match event {
        Some(XEvent::KeyPress(event)) if event.detail as u32 == ctx.toggle && !st.pressed => {
            st.pressed = true;
            match ctx.shm.header().toggle_visible() {
                Some(visible) => {
                    eprintln!("ira-overlay-ui: session chord flipped overlay to {visible}")
                }
                None => eprintln!("ira-overlay-ui: session chord debounced"),
            }
        }
        Some(XEvent::KeyRelease(event)) if event.detail as u32 == ctx.toggle => {
            st.pressed = false;
        }
        // Navigation while visible: Alt+F4 exits (no window manager
        // exists to honor it), arrows/enter/escape drive the overlay.
        Some(XEvent::KeyPress(event)) if st.wide => {
            use std::collections::hash_map::Entry;
            use x11rb::protocol::xproto::KeyButMask;
            let key = event.detail as u32;
            let kind = if key == ALT_F4_X11 && event.state.contains(KeyButMask::MOD1) {
                Some(ira_overlay_ipc::CMD_EXIT_GAME)
            } else {
                nav_command(key)
            };
            if let Some(kind) = kind
                && let Entry::Vacant(slot) = st.nav_held.entry(key)
            {
                let now = Instant::now();
                slot.insert((now, now));
                st.canvas = push_command_kind(&ctx.shm, st.canvas.take(), kind);
            }
        }
        Some(XEvent::KeyRelease(event)) => {
            st.nav_held.remove(&(event.detail as u32));
        }
        _ => {}
    }
    true
}
