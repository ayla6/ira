//! Wayland keyboard + pointer callbacks — split from `wayland.rs`.
//!
//! `wayland.rs` owns the connection, registry/seat binding, and dispatch;
//! this file owns input state and the listener callbacks that feed the
//! canvas command ring. No Wayland protocol calls here, only event mapping.

use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

use ira_overlay::capture;
use ira_overlay_ipc::{
    CanvasCommand, ShmHeader, CMD_ACTIVATE, CMD_NAV_DOWN, CMD_NAV_LEFT, CMD_NAV_RIGHT, CMD_NAV_UP,
};

pub static HAS_FOCUS: AtomicBool = AtomicBool::new(false);

// --- Tracked input state (all accessed from dispatch(), single-threaded) ---

static MODS_DEPRESSED: AtomicU32 = AtomicU32::new(0);
/// Toggle key currently held (u32::MAX = none): XKB/Wayland re-fires
/// key-press for held keys, which would strobe the overlay without this.
static TOGGLE_HELD: AtomicU32 = AtomicU32::new(u32::MAX);
static MOUSE_SX: AtomicI32 = AtomicI32::new(0);
static MOUSE_SY: AtomicI32 = AtomicI32::new(0);

// --- Evdev keycodes (Wayland uses evdev directly, no +8 offset like X11) ---
// Navigation order matches ShmHeader::NAV_KEYCODES_EVDEV:
// [Return, Up, Down, Left, Right].
const KC_RETURN: u32 = ShmHeader::NAV_KEYCODES_EVDEV[0];
const KC_UP: u32 = ShmHeader::NAV_KEYCODES_EVDEV[1];
const KC_DOWN: u32 = ShmHeader::NAV_KEYCODES_EVDEV[2];
const KC_LEFT: u32 = ShmHeader::NAV_KEYCODES_EVDEV[3];
const KC_RIGHT: u32 = ShmHeader::NAV_KEYCODES_EVDEV[4];

const KEY_PRESSED: u32 = 1;
const AXIS_VERTICAL: u32 = 0;

// --- Keyboard callbacks ---

pub(crate) extern "C" fn keyboard_enter(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    _: *mut c_void,
    _: *mut c_void,
) {
    HAS_FOCUS.store(true, Ordering::Relaxed);
    eprintln!("ira-overlay: keyboard enter (focus=true)");
}

pub(crate) extern "C" fn keyboard_leave(_: *mut c_void, _: *mut c_void, _: u32, _: *mut c_void) {
    HAS_FOCUS.store(false, Ordering::Relaxed);
    eprintln!("ira-overlay: keyboard leave (focus=false)");
}

pub(crate) extern "C" fn keyboard_key(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    _: u32,
    key: u32,
    state: u32,
) {
    if !overlay_active() {
        return;
    }
    let pressed = state == KEY_PRESSED;
    let mods = MODS_DEPRESSED.load(Ordering::Relaxed);

    // Hotkeys work even when overlay is hidden. Edge-triggered: held
    // keys re-fire press events, which must not re-toggle.
    if pressed {
        match hotkey_action(key, mods) {
            Some(KeyAction::Toggle) => {
                if !crate::shim_bridge::ready_for_overlay() {
                    return;
                }
                if TOGGLE_HELD
                    .compare_exchange(u32::MAX, key, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    let visible = crate::shim_bridge::is_visible();
                    crate::shim_bridge::set_visible(!visible);
                }
                return;
            }
            Some(KeyAction::Screenshot) => {
                capture::request_screenshot();
                return;
            }
            Some(KeyAction::Record) => {
                capture::toggle_recording();
                return;
            }
            None => {}
        }
    }
    if !pressed {
        let _ = TOGGLE_HELD.compare_exchange(key, u32::MAX, Ordering::SeqCst, Ordering::SeqCst);
    }

    // Navigation keys and debug toggles only when overlay is visible.
    if !crate::shim_bridge::is_visible() {
        return;
    }
    if pressed {
        if let Some(kind) = nav_kind(key) {
            crate::shim_bridge::push_canvas_command(CanvasCommand::nav(kind));
        }
    }
}

enum KeyAction {
    Toggle,
    Screenshot,
    Record,
}

/// Matches a (key, mods) press against the three configured hotkeys.
/// Toggle first: a Super-based toggle must win over desktop combos.
fn hotkey_action(key: u32, mods: u32) -> Option<KeyAction> {
    let (tog_kc, tog_mods, ss_kc, ss_mods, rec_kc, rec_mods) = crate::shim_bridge::hotkeys();
    [
        ((tog_kc, tog_mods), KeyAction::Toggle),
        ((ss_kc, ss_mods), KeyAction::Screenshot),
        ((rec_kc, rec_mods), KeyAction::Record),
    ]
    .into_iter()
    .find(|((kc, m), _)| (mods & m) == *m && key == *kc)
    .map(|(_, action)| action)
}

/// Overlay nav keys (evdev codes; Wayland needs no +8 offset).
fn nav_kind(key: u32) -> Option<u32> {
    match key {
        KC_UP => Some(CMD_NAV_UP),
        KC_DOWN => Some(CMD_NAV_DOWN),
        KC_LEFT => Some(CMD_NAV_LEFT),
        KC_RIGHT => Some(CMD_NAV_RIGHT),
        KC_RETURN => Some(CMD_ACTIVATE),
        _ => None,
    }
}

pub(crate) extern "C" fn keyboard_modifiers(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    mods_depressed: u32,
    _: u32,
    _: u32,
    _: u32,
) {
    MODS_DEPRESSED.store(mods_depressed, Ordering::Relaxed);
}

pub(crate) extern "C" fn nop_keymap(_: *mut c_void, _: *mut c_void, _: u32, _: c_int, _: u32) {}
pub(crate) extern "C" fn nop_repeat(_: *mut c_void, _: *mut c_void, _: c_int, _: c_int) {}

// --- Pointer callbacks ---

fn store_mouse(sx: i32, sy: i32) {
    MOUSE_SX.store(sx, Ordering::Relaxed);
    MOUSE_SY.store(sy, Ordering::Relaxed);
}

pub(crate) extern "C" fn pointer_enter(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    _: *mut c_void,
    sx: i32,
    sy: i32,
) {
    store_mouse(sx, sy);
}

pub(crate) extern "C" fn pointer_motion(_: *mut c_void, _: *mut c_void, _: u32, sx: i32, sy: i32) {
    store_mouse(sx, sy);
    if crate::shim_bridge::is_visible() {
        let (x, y) = fixed_to_f32(sx, sy);
        // Second-source tracing: proves whether layer-Wayland pointer
        // events fire alongside shim events in the same session.
        if crate::types::debug_log() {
            static LAST_WL_MS: AtomicU32 = AtomicU32::new(0);
            let now = crate::canvas::now_ms();
            if now.wrapping_sub(LAST_WL_MS.load(Ordering::Relaxed)) > 2000 {
                LAST_WL_MS.store(now, Ordering::Relaxed);
                eprintln!("ira-overlay: wayland motion surface=({x:.0},{y:.0})");
            }
        }
        crate::canvas::set_last_mouse(x, y);
        if let Some((cx, cy)) = crate::canvas::screen_to_canvas(x, y) {
            crate::shim_bridge::push_canvas_command(CanvasCommand::mouse_move(cx, cy));
        }
    }
}

pub(crate) extern "C" fn pointer_button(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    _: u32,
    _button: u32,
    state: u32,
) {
    if !crate::shim_bridge::is_visible() {
        return;
    }
    let (x, y) = fixed_to_f32(
        MOUSE_SX.load(Ordering::Relaxed),
        MOUSE_SY.load(Ordering::Relaxed),
    );
    crate::canvas::set_last_mouse(x, y);
    let Some((cx, cy)) = crate::canvas::screen_to_canvas(x, y) else {
        return;
    };
    if state == KEY_PRESSED {
        crate::shim_bridge::push_canvas_command(CanvasCommand::mouse_down(cx, cy, 0));
    } else {
        crate::shim_bridge::push_canvas_command(CanvasCommand::mouse_up(cx, cy, 0));
    }
}

pub(crate) extern "C" fn pointer_axis(_: *mut c_void, _: *mut c_void, _: u32, axis: u32, value: i32) {
    if !crate::shim_bridge::is_visible() {
        return;
    }
    if axis != AXIS_VERTICAL {
        return;
    }
    let delta_y = if value > 0 { 1.0 } else { -1.0 };
    crate::shim_bridge::push_canvas_command(CanvasCommand::scroll(delta_y));
}

fn fixed_to_f32(sx: i32, sy: i32) -> (f32, f32) {
    (sx as f32 / 256.0, sy as f32 / 256.0)
}

fn overlay_active() -> bool {
    std::env::var_os("IRA_OVERLAY_SHM").is_some()
}
