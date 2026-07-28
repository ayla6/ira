//! X11 event hooking via LD_PRELOAD symbol interposition.
//!
//! When loaded via `LD_PRELOAD`, the dynamic linker resolves calls to
//! `XCheckIfEvent`, `XCheckWindowEvent`, etc. to our versions instead of
//! the real libX11 functions. We resolve the real functions via
//! `dlsym(RTLD_NEXT, ...)` and chain through to them.
//!
//! When the overlay is visible, mouse and keyboard events are consumed
//! (removed from the queue) and pushed to our input queue. The Vulkan layer
//! polls this queue via the exported C API (`ira_overlay_poll_events`).
//!
//! When the overlay is hidden, all events pass through to the game unmodified.
//!
//! The toggle hotkey (Shift+Tab) is always detected, regardless of visibility.

use std::ffi::c_void;
use std::sync::OnceLock;

use ira_overlay_ipc::InputEventRaw;
use ira_overlay_ipc::X11_KEYCODE_OFFSET;

use crate::state;

// X11 event type constants (from /usr/include/X11/X.h)
const KEYPRESS: i32 = 2;
const KEYRELEASE: i32 = 3;
const BUTTONPRESS: i32 = 4;
const BUTTONRELEASE: i32 = 5;
const MOTIONNOTIFY: i32 = 6;

// X11 event struct field offsets (64-bit Linux).
// Confirmed against /usr/include/X11/Xlib.h — XKeyEvent, XButtonEvent,
// and XMotionEvent share the same layout for these fields.
//   offset 0:  int type
//   offset 8:  unsigned long serial (8-byte aligned)
//   offset 16: Bool send_event
//   offset 24: Display *display (8-byte aligned)
//   offset 32: Window window
//   offset 40: Window root
//   offset 48: Window subwindow
//   offset 56: Time time
//   offset 64: int x
//   offset 68: int y
//   offset 72: int x_root
//   offset 76: int y_root
//   offset 80: unsigned int state
//   offset 84: unsigned int keycode (XKeyEvent) / button (XButtonEvent)
const EV_TYPE: usize = 0;
const EV_X: usize = 64;
const EV_Y: usize = 68;
const EV_STATE: usize = 80;
const EV_DETAIL: usize = 84;

// --- Real function pointer resolution ---

type XCheckIfEventFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32;
type XCheckWindowEventFn = unsafe extern "C" fn(*mut c_void, u64, i64, *mut c_void) -> i32;
type XCheckTypedEventFn = unsafe extern "C" fn(*mut c_void, i32, *mut c_void) -> i32;
type XCheckTypedWindowEventFn = unsafe extern "C" fn(*mut c_void, u64, i32, *mut c_void) -> i32;
type XCheckMaskEventFn = unsafe extern "C" fn(*mut c_void, i64, *mut c_void) -> i32;
type XNextEventFn = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;

fn resolve(name: &std::ffi::CStr) -> *mut c_void {
    unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) }
}

static REAL_XCHECK_IF_EVENT: OnceLock<Option<XCheckIfEventFn>> = OnceLock::new();
static REAL_XCHECK_WINDOW_EVENT: OnceLock<Option<XCheckWindowEventFn>> = OnceLock::new();
static REAL_XCHECK_TYPED_EVENT: OnceLock<Option<XCheckTypedEventFn>> = OnceLock::new();
static REAL_XCHECK_TYPED_WINDOW_EVENT: OnceLock<Option<XCheckTypedWindowEventFn>> = OnceLock::new();
static REAL_XCHECK_MASK_EVENT: OnceLock<Option<XCheckMaskEventFn>> = OnceLock::new();
static REAL_XNEXT_EVENT: OnceLock<Option<XNextEventFn>> = OnceLock::new();

fn xcheck_if_event() -> Option<XCheckIfEventFn> {
    *REAL_XCHECK_IF_EVENT.get_or_init(|| {
        let p = resolve(c"XCheckIfEvent");
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

fn xcheck_window_event() -> Option<XCheckWindowEventFn> {
    *REAL_XCHECK_WINDOW_EVENT.get_or_init(|| {
        let p = resolve(c"XCheckWindowEvent");
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

fn xcheck_typed_event() -> Option<XCheckTypedEventFn> {
    *REAL_XCHECK_TYPED_EVENT.get_or_init(|| {
        let p = resolve(c"XCheckTypedEvent");
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

fn xcheck_typed_window_event() -> Option<XCheckTypedWindowEventFn> {
    *REAL_XCHECK_TYPED_WINDOW_EVENT.get_or_init(|| {
        let p = resolve(c"XCheckTypedWindowEvent");
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

fn xcheck_mask_event() -> Option<XCheckMaskEventFn> {
    *REAL_XCHECK_MASK_EVENT.get_or_init(|| {
        let p = resolve(c"XCheckMaskEvent");
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

fn xnext_event() -> Option<XNextEventFn> {
    *REAL_XNEXT_EVENT.get_or_init(|| {
        let p = resolve(c"XNextEvent");
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

// --- X11 event field readers ---

fn read_type(ev: *const c_void) -> i32 {
    unsafe { *((ev as *const u8).add(EV_TYPE) as *const i32) }
}

fn read_xy(ev: *const c_void) -> (i32, i32) {
    unsafe {
        let x = *((ev as *const u8).add(EV_X) as *const i32);
        let y = *((ev as *const u8).add(EV_Y) as *const i32);
        (x, y)
    }
}

fn read_state(ev: *const c_void) -> u32 {
    unsafe { *((ev as *const u8).add(EV_STATE) as *const u32) }
}

fn read_detail(ev: *const c_void) -> u32 {
    unsafe { *((ev as *const u8).add(EV_DETAIL) as *const u32) }
}

// --- Event consumption logic ---

/// Inspects an event and decides whether to consume it.
/// Returns `true` if the event was consumed (caller should return 0/False).
///
/// Handles:
/// - Shift+Tab: toggles overlay visibility (always, even when hidden)
/// - F12: triggers screenshot (always, when overlay system is active)
/// - F11: toggles recording (always, when overlay system is active)
/// - Mouse/keyboard: consumed when overlay is visible
unsafe fn maybe_consume_event(ev: *mut c_void) -> bool {
    if !state::overlay_active() {
        return false;
    }

    static FIRST_EVENT: std::sync::Once = std::sync::Once::new();
    FIRST_EVENT.call_once(|| {
        eprintln!("ira-overlay-shim: intercepting X11 events in pid {}", std::process::id());
    });

    let event_type = read_type(ev);

    // Always check for hotkeys, even when overlay is hidden.
    if event_type == KEYPRESS {
        let mods = read_state(ev);
        let keycode = read_detail(ev);

        let (tog_kc, tog_mods, ss_kc, ss_mods, rec_kc, rec_mods) = state::hotkeys();
        // X11 keycodes are evdev + 8.
        let tog_x11 = tog_kc + X11_KEYCODE_OFFSET;
        let ss_x11 = ss_kc + X11_KEYCODE_OFFSET;
        let rec_x11 = rec_kc + X11_KEYCODE_OFFSET;

        if (mods & tog_mods) == tog_mods && keycode == tog_x11 {
            state::toggle_visible();
            return true;
        }
        if (mods & ss_mods) == ss_mods && keycode == ss_x11 {
            state::push_event(InputEventRaw {
                event_type: 5, // screenshot request
                x: 0, y: 0, button: 0, keycode: 0,
            });
            return true;
        }
        if (mods & rec_mods) == rec_mods && keycode == rec_x11 {
            state::push_event(InputEventRaw {
                event_type: 6, // recording toggle
                x: 0, y: 0, button: 0, keycode: 0,
            });
            return true;
        }
    }

    // When overlay is visible, consume all mouse and keyboard events.
    if state::is_visible() {
        match event_type {
            MOTIONNOTIFY => {
                let (x, y) = read_xy(ev);
                state::set_mouse_pos(x, y);
                state::push_event(InputEventRaw {
                    event_type: 0, x, y, button: 0, keycode: 0,
                });
                return true;
            }
            BUTTONPRESS => {
                let (x, y) = read_xy(ev);
                let button = read_detail(ev);
                // X11 scroll: button 4 = up, button 5 = down
                if button == 4 || button == 5 {
                    let delta = if button == 4 { -1 } else { 1 };
                    state::push_event(InputEventRaw {
                        event_type: 7, x: 0, y: delta, button, keycode: 0,
                    });
                    return true;
                }
                state::push_event(InputEventRaw {
                    event_type: 1, x, y, button, keycode: 0,
                });
                return true;
            }
            BUTTONRELEASE => {
                let (x, y) = read_xy(ev);
                let button = read_detail(ev);
                // Scroll buttons don't have meaningful release events
                if button == 4 || button == 5 {
                    return true;
                }
                state::push_event(InputEventRaw {
                    event_type: 2, x, y, button, keycode: 0,
                });
                return true;
            }
            KEYPRESS => {
                let keycode = read_detail(ev);
                state::push_event(InputEventRaw {
                    event_type: 3, x: 0, y: 0, button: 0, keycode,
                });
                return true;
            }
            KEYRELEASE => {
                let keycode = read_detail(ev);
                state::push_event(InputEventRaw {
                    event_type: 4, x: 0, y: 0, button: 0, keycode,
                });
                return true;
            }
            _ => {}
        }
    }

    false
}

// --- LD_PRELOAD hooks ---

/// Non-blocking event check with a predicate. Used by SDL2 and most game engines.
#[no_mangle]
pub unsafe extern "C" fn XCheckIfEvent(
    display: *mut c_void,
    event_return: *mut c_void,
    predicate: *mut c_void,
    arg: *mut c_void,
) -> i32 {
    let Some(real_fn) = xcheck_if_event() else {
        return 0;
    };
    let result = real_fn(display, event_return, predicate, arg);
    if result != 0 && maybe_consume_event(event_return) {
        return 0;
    }
    result
}

/// Non-blocking check for events matching a window and event mask.
#[no_mangle]
pub unsafe extern "C" fn XCheckWindowEvent(
    display: *mut c_void,
    w: u64,
    event_mask: i64,
    event_return: *mut c_void,
) -> i32 {
    let Some(real_fn) = xcheck_window_event() else {
        return 0;
    };
    let result = real_fn(display, w, event_mask, event_return);
    if result != 0 && maybe_consume_event(event_return) {
        return 0;
    }
    result
}

/// Non-blocking check for events matching a specific type.
#[no_mangle]
pub unsafe extern "C" fn XCheckTypedEvent(
    display: *mut c_void,
    event_type: i32,
    event_return: *mut c_void,
) -> i32 {
    let Some(real_fn) = xcheck_typed_event() else {
        return 0;
    };
    let result = real_fn(display, event_type, event_return);
    if result != 0 && maybe_consume_event(event_return) {
        return 0;
    }
    result
}

/// Non-blocking check for events matching a window and specific type.
#[no_mangle]
pub unsafe extern "C" fn XCheckTypedWindowEvent(
    display: *mut c_void,
    w: u64,
    event_type: i32,
    event_return: *mut c_void,
) -> i32 {
    let Some(real_fn) = xcheck_typed_window_event() else {
        return 0;
    };
    let result = real_fn(display, w, event_type, event_return);
    if result != 0 && maybe_consume_event(event_return) {
        return 0;
    }
    result
}

/// Non-blocking check for events matching an event mask.
#[no_mangle]
pub unsafe extern "C" fn XCheckMaskEvent(
    display: *mut c_void,
    event_mask: i64,
    event_return: *mut c_void,
) -> i32 {
    let Some(real_fn) = xcheck_mask_event() else {
        return 0;
    };
    let result = real_fn(display, event_mask, event_return);
    if result != 0 && maybe_consume_event(event_return) {
        return 0;
    }
    result
}

/// Blocking event wait. Loops internally to skip consumed events.
#[no_mangle]
pub unsafe extern "C" fn XNextEvent(
    display: *mut c_void,
    event_return: *mut c_void,
) -> i32 {
    let Some(real_fn) = xnext_event() else {
        return 0;
    };
    loop {
        let result = real_fn(display, event_return);
        if !state::overlay_active() {
            return result;
        }
        if !maybe_consume_event(event_return) {
            return result;
        }
        // Event was consumed — loop to get the next one.
    }
}
