//! XCB event hooking via LD_PRELOAD symbol interposition.
//!
//! Qt6's XCB platform plugin uses `xcb_poll_for_event` / `xcb_wait_for_event`
//! directly, bypassing Xlib. The Xlib hooks in `hooks_x11.rs` won't see
//! KeyPress events from Qt6 apps. This module interposes the XCB event
//! polling functions to catch those events.
//!
//! XCB event structs have a different layout from Xlib:
//!   - `response_type` at byte 0 (lower 7 bits = event type, bit 8 = generated)
//!   - For KeyPress (type 2 = XCB_KEY_PRESS):
//!     - offset 0:  uint8_t  response_type
//!     - offset 1:  uint8_t  detail (keycode)
//!     - offset 2:  uint16_t sequence
//!     - offset 4:  uint32_t time
//!     - offset 8:  uint32_t root
//!     - offset 12: uint32_t event
//!     - offset 16: uint32_t child
//!     - offset 20: int16_t  root_x
//!     - offset 22: int16_t  root_y
//!     - offset 24: int16_t  event_x
//!     - offset 26: int16_t  event_y
//!     - offset 28: uint16_t state (modifier mask)
//!     - offset 30: uint8_t  same_screen
//!
//! The modifier masks are the same as Xlib (Shift=0x01, Ctrl=0x04, etc.)
//! and the keycodes are the same (evdev keycodes on modern Linux).

use std::ffi::c_void;
use std::sync::OnceLock;

use ira_overlay_ipc::InputEventRaw;
use ira_overlay_ipc::X11_KEYCODE_OFFSET;

use crate::state;

// XCB event type constants
const XCB_KEY_PRESS: u8 = 2;
const XCB_KEY_RELEASE: u8 = 3;
const XCB_BUTTON_PRESS: u8 = 4;
const XCB_BUTTON_RELEASE: u8 = 5;
const XCB_MOTION_NOTIFY: u8 = 6;

// XCB button IDs (scroll)
const XCB_BUTTON_INDEX_4: u8 = 4;
const XCB_BUTTON_INDEX_5: u8 = 5;

// XCB event field offsets (xcb_key_press_event_t / xcb_button_press_event_t)
const XCB_RESPONSE_TYPE: usize = 0;
const XCB_DETAIL: usize = 1;
const XCB_STATE: usize = 28;
const XCB_EVENT_X: usize = 24;
const XCB_EVENT_Y: usize = 26;

type XcbPollForEventFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type XcbWaitForEventFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

fn resolve(name: &std::ffi::CStr) -> *mut c_void {
    unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) }
}

static REAL_XCB_POLL: OnceLock<Option<XcbPollForEventFn>> = OnceLock::new();
static REAL_XCB_WAIT: OnceLock<Option<XcbWaitForEventFn>> = OnceLock::new();

fn xcb_poll() -> Option<XcbPollForEventFn> {
    *REAL_XCB_POLL.get_or_init(|| {
        let p = resolve(c"xcb_poll_for_event");
        (!p.is_null()).then(|| cast_fn(p))
    })
}

fn xcb_wait() -> Option<XcbWaitForEventFn> {
    *REAL_XCB_WAIT.get_or_init(|| {
        let p = resolve(c"xcb_wait_for_event");
        (!p.is_null()).then(|| cast_fn(p))
    })
}

fn cast_fn<T>(p: *mut c_void) -> T {
    unsafe { std::mem::transmute_copy(&p) }
}

// --- XCB event field readers ---

fn xcb_response_type(ev: *const c_void) -> u8 {
    unsafe { *(ev as *const u8).add(XCB_RESPONSE_TYPE) & 0x7f }
}

fn xcb_detail(ev: *const c_void) -> u8 {
    unsafe { *(ev as *const u8).add(XCB_DETAIL) }
}

fn xcb_state(ev: *const c_void) -> u16 {
    unsafe { *((ev as *const u8).add(XCB_STATE) as *const u16) }
}

fn xcb_event_xy(ev: *const c_void) -> (i16, i16) {
    unsafe {
        let x = *((ev as *const u8).add(XCB_EVENT_X) as *const i16);
        let y = *((ev as *const u8).add(XCB_EVENT_Y) as *const i16);
        (x, y)
    }
}

// --- Event consumption logic ---

/// Inspects an XCB event and decides whether to consume it.
/// Returns `true` if consumed (caller should free the event and return NULL).
unsafe fn maybe_consume_xcb_event(ev: *mut c_void) -> bool {
    if !state::overlay_active() {
        return false;
    }

    static FIRST_EVENT: std::sync::Once = std::sync::Once::new();
    FIRST_EVENT.call_once(|| {
        eprintln!(
            "ira-overlay-shim: intercepting XCB events in pid {}",
            std::process::id()
        );
    });

    let event_type = xcb_response_type(ev);

    // Always check for hotkeys, even when overlay is hidden.
    if event_type == XCB_KEY_PRESS {
        let mods = xcb_state(ev) as u32;
        let keycode = xcb_detail(ev) as u32;

        let (tog_kc, tog_mods, ss_kc, ss_mods, rec_kc, rec_mods) = state::hotkeys();
        // XCB keycodes are the same as X11 keycodes (evdev + 8).
        // SHM stores evdev keycodes, so add 8 for comparison.
        let tog_x11 = tog_kc + X11_KEYCODE_OFFSET;
        let ss_x11 = ss_kc + X11_KEYCODE_OFFSET;
        let rec_x11 = rec_kc + X11_KEYCODE_OFFSET;

        if (mods & tog_mods) == tog_mods && keycode == tog_x11 {
            state::toggle_visible();
            return true;
        }
        if (mods & ss_mods) == ss_mods && keycode == ss_x11 {
            state::push_event(InputEventRaw {
                event_type: 5,
                x: 0,
                y: 0,
                button: 0,
                keycode: 0,
            });
            return true;
        }
        if (mods & rec_mods) == rec_mods && keycode == rec_x11 {
            state::push_event(InputEventRaw {
                event_type: 6,
                x: 0,
                y: 0,
                button: 0,
                keycode: 0,
            });
            return true;
        }
    }

    // When overlay is visible, consume all mouse and keyboard events.
    if state::is_visible() {
        match event_type {
            XCB_MOTION_NOTIFY => {
                let (x, y) = xcb_event_xy(ev);
                state::set_mouse_pos(x as i32, y as i32);
                state::push_event(InputEventRaw {
                    event_type: 0,
                    x: x as i32,
                    y: y as i32,
                    button: 0,
                    keycode: 0,
                });
                return true;
            }
            XCB_BUTTON_PRESS => {
                let (x, y) = xcb_event_xy(ev);
                let button = xcb_detail(ev);
                if button == XCB_BUTTON_INDEX_4 || button == XCB_BUTTON_INDEX_5 {
                    let delta = if button == XCB_BUTTON_INDEX_4 { -1 } else { 1 };
                    state::push_event(InputEventRaw {
                        event_type: 7,
                        x: 0,
                        y: delta,
                        button: button as u32,
                        keycode: 0,
                    });
                    return true;
                }
                state::push_event(InputEventRaw {
                    event_type: 1,
                    x: x as i32,
                    y: y as i32,
                    button: button as u32,
                    keycode: 0,
                });
                return true;
            }
            XCB_BUTTON_RELEASE => {
                let (x, y) = xcb_event_xy(ev);
                let button = xcb_detail(ev);
                if button == XCB_BUTTON_INDEX_4 || button == XCB_BUTTON_INDEX_5 {
                    return true;
                }
                state::push_event(InputEventRaw {
                    event_type: 2,
                    x: x as i32,
                    y: y as i32,
                    button: button as u32,
                    keycode: 0,
                });
                return true;
            }
            XCB_KEY_PRESS => {
                let keycode = xcb_detail(ev) as u32;
                state::push_event(InputEventRaw {
                    event_type: 3,
                    x: 0,
                    y: 0,
                    button: 0,
                    keycode,
                });
                return true;
            }
            XCB_KEY_RELEASE => {
                let keycode = xcb_detail(ev) as u32;
                state::push_event(InputEventRaw {
                    event_type: 4,
                    x: 0,
                    y: 0,
                    button: 0,
                    keycode,
                });
                return true;
            }
            _ => {}
        }
    }

    false
}

// --- LD_PRELOAD hooks ---

/// Non-blocking event poll. Used by Qt6/XCB for event processing.
#[no_mangle]
pub unsafe extern "C" fn xcb_poll_for_event(c: *mut c_void) -> *mut c_void {
    let Some(real_fn) = xcb_poll() else {
        return std::ptr::null_mut();
    };
    let event = real_fn(c);
    if !event.is_null() && maybe_consume_xcb_event(event) {
        libc::free(event);
        return std::ptr::null_mut();
    }
    event
}

/// Blocking event wait. Loops internally to skip consumed events.
#[no_mangle]
pub unsafe extern "C" fn xcb_wait_for_event(c: *mut c_void) -> *mut c_void {
    let Some(real_fn) = xcb_wait() else {
        return std::ptr::null_mut();
    };
    loop {
        let event = real_fn(c);
        if event.is_null() {
            return event;
        }
        if !state::overlay_active() {
            return event;
        }
        if !maybe_consume_xcb_event(event) {
            return event;
        }
        // Event was consumed — free it and loop to get the next one.
        libc::free(event);
    }
}
