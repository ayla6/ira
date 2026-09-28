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
const EV_X_ROOT: usize = 72;
const EV_Y_ROOT: usize = 76;
const EV_STATE: usize = 80;
const EV_DETAIL: usize = 84;

// --- Real function pointer resolution ---

type XCheckIfEventFn =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32;
type XCheckWindowEventFn = unsafe extern "C" fn(*mut c_void, u64, i64, *mut c_void) -> i32;
type XCheckTypedEventFn = unsafe extern "C" fn(*mut c_void, i32, *mut c_void) -> i32;
type XCheckTypedWindowEventFn = unsafe extern "C" fn(*mut c_void, u64, i32, *mut c_void) -> i32;
type XCheckMaskEventFn = unsafe extern "C" fn(*mut c_void, i64, *mut c_void) -> i32;
type XNextEventFn = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
// GrabSuccess = 0; AlreadyGrabbed = 1 (used when the real call is missing).
type XGrabPointerFn =
    unsafe extern "C" fn(*mut c_void, u64, i32, u32, i32, i32, u64, u64, u64) -> i32;
type XIGrabDeviceFn =
    unsafe extern "C" fn(*mut c_void, i32, u64, u64, u64, i32, i32, i32, *mut c_void) -> i32;
type XGrabKeyboardFn = unsafe extern "C" fn(*mut c_void, u64, i32, i32, i32, u64) -> i32;

fn resolve(name: &std::ffi::CStr) -> *mut c_void {
    unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) }
}

macro_rules! real_fn {
    ($name:literal, $ty:ty) => {{
        static CELL: OnceLock<Option<$ty>> = OnceLock::new();
        *CELL.get_or_init(|| {
            let p = resolve($name);
            (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
        })
    }};
}

fn xcheck_if_event() -> Option<XCheckIfEventFn> {
    real_fn!(c"XCheckIfEvent", XCheckIfEventFn)
}

fn xcheck_window_event() -> Option<XCheckWindowEventFn> {
    real_fn!(c"XCheckWindowEvent", XCheckWindowEventFn)
}

fn xcheck_typed_event() -> Option<XCheckTypedEventFn> {
    real_fn!(c"XCheckTypedEvent", XCheckTypedEventFn)
}

fn xcheck_typed_window_event() -> Option<XCheckTypedWindowEventFn> {
    real_fn!(c"XCheckTypedWindowEvent", XCheckTypedWindowEventFn)
}

fn xcheck_mask_event() -> Option<XCheckMaskEventFn> {
    real_fn!(c"XCheckMaskEvent", XCheckMaskEventFn)
}

fn xnext_event() -> Option<XNextEventFn> {
    real_fn!(c"XNextEvent", XNextEventFn)
}

fn xgrab_pointer() -> Option<XGrabPointerFn> {
    real_fn!(c"XGrabPointer", XGrabPointerFn)
}

fn xi_grab_device() -> Option<XIGrabDeviceFn> {
    real_fn!(c"XIGrabDevice", XIGrabDeviceFn)
}

fn xgrab_keyboard() -> Option<XGrabKeyboardFn> {
    real_fn!(c"XGrabKeyboard", XGrabKeyboardFn)
}

/// Overlay grabs the pointer when it is up and interactive: a game grab
/// would confine and hide the real cursor, desyncing it from the overlay
/// (and trapping it in windowed mode). Report success without grabbing —
/// the game proceeds as if it owned the pointer. Event consumption already
/// starves the game of pointer input while visible, so neutering changes
/// nothing gameplay-wise; grabs work again the moment it closes.
fn grabs_neutered() -> bool {
    state::overlay_active() && state::is_visible() && !state::injected_ui_disabled()
}

// --- X11 event field readers ---

fn read_type(ev: *const c_void) -> i32 {
    unsafe { *((ev as *const u8).add(EV_TYPE) as *const i32) }
}

fn read_xy(ev: *const c_void) -> (i32, i32) {
    // Root (screen) coordinates: unambiguous no matter which window the
    // game queried the event for. The layer translates these into the
    // game window's client space (identity for fullscreen at origin).
    unsafe {
        let x = *((ev as *const u8).add(EV_X_ROOT) as *const i32);
        let y = *((ev as *const u8).add(EV_Y_ROOT) as *const i32);
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
/// - Toggle/screenshot/record hotkeys: always, even when hidden
/// - Mouse/keyboard: consumed when overlay is visible
unsafe fn maybe_consume_event(ev: *mut c_void) -> bool {
    if ev.is_null() || !state::overlay_active() {
        return false;
    }

    static FIRST_EVENT: std::sync::Once = std::sync::Once::new();
    FIRST_EVENT.call_once(|| {
        eprintln!(
            "ira-overlay-shim: intercepting X11 events in pid {}",
            std::process::id()
        );
    });

    let event_type = read_type(ev);

    // Always check for hotkeys, even when overlay is hidden.
    if event_type == KEYPRESS && check_hotkey(ev) {
        return true;
    }

    // Desktop-level combos always reach the compositor (checked after the
    // hotkeys above so a configured Super-based toggle still wins).
    if event_type == KEYPRESS || event_type == KEYRELEASE {
        if state::is_desktop_combo(read_detail(ev), read_state(ev)) {
            return false;
        }
        if event_type == KEYRELEASE {
            state::release_edge(read_detail(ev));
        }
    }

    // When overlay is visible, consume all mouse and keyboard events, in
    // any mode: the game must not react to input meant for the overlay.
    // (Toggle handling above stays mode-independent for the same reason.)
    if state::is_visible() {
        return consume_visible(event_type, ev);
    }

    false
}

/// Hotkey match shared with the XCB backend via `state::match_hotkey`.
/// Returns true when the event was a hotkey (consumed).
unsafe fn check_hotkey(ev: *mut c_void) -> bool {
    use crate::state::HotkeyAction;
    let mods = read_state(ev);
    let keycode = read_detail(ev);
    match state::match_hotkey(mods, keycode) {
        // Keyboard toggle is always ours: no other authority handles
        // keyboard chords (the daemon only sees gamepads), in gamescope or
        // out. The game keeps focus under gamescope's external overlay, so
        // this is also the un-toggle path there.
        Some(HotkeyAction::Toggle) => {
            if state::toggle_edge(keycode) {
                state::toggle_visible();
            }
            true
        }
        Some(HotkeyAction::Screenshot) => {
            state::push_event(InputEventRaw {
                event_type: 5, // screenshot request
                x: 0,
                y: 0,
                button: 0,
                keycode: 0,
            });
            true
        }
        Some(HotkeyAction::Record) => {
            state::push_event(InputEventRaw {
                event_type: 6, // recording toggle
                x: 0,
                y: 0,
                button: 0,
                keycode: 0,
            });
            true
        }
        None => false,
    }
}

unsafe fn consume_visible(event_type: i32, ev: *mut c_void) -> bool {
    match event_type {
        MOTIONNOTIFY => {
            let (x, y) = read_xy(ev);
            state::set_mouse_pos(x, y);
            state::push_event(InputEventRaw {
                event_type: 0,
                x,
                y,
                button: 0,
                keycode: 0,
            });
            true
        }
        BUTTONPRESS => {
            let (x, y) = read_xy(ev);
            let button = read_detail(ev);
            // X11 scroll: button 4 = up, button 5 = down. Press pushes an
            // event; release is consumed with no event (no release semantics).
            if button == 4 || button == 5 {
                let delta = if button == 4 { -1 } else { 1 };
                state::push_event(InputEventRaw {
                    event_type: 7,
                    x: 0,
                    y: delta,
                    button,
                    keycode: 0,
                });
                return true;
            }
            state::push_event(InputEventRaw {
                event_type: 1,
                x,
                y,
                button,
                keycode: 0,
            });
            true
        }
        BUTTONRELEASE => {
            let (x, y) = read_xy(ev);
            let button = read_detail(ev);
            // Scroll buttons don't have meaningful release events
            if button == 4 || button == 5 {
                return true;
            }
            state::push_event(InputEventRaw {
                event_type: 2,
                x,
                y,
                button,
                keycode: 0,
            });
            true
        }
        KEYPRESS => {
            let keycode = read_detail(ev);
            state::push_event(InputEventRaw {
                event_type: 3,
                x: 0,
                y: 0,
                button: 0,
                keycode,
            });
            true
        }
        KEYRELEASE => {
            let keycode = read_detail(ev);
            state::push_event(InputEventRaw {
                event_type: 4,
                x: 0,
                y: 0,
                button: 0,
                keycode,
            });
            true
        }
        _ => false,
    }
}

// --- LD_PRELOAD hooks ---

macro_rules! check_hook {
    ($name:ident($($arg:ident : $ty:ty),*) => $real:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            $($arg: $ty),*,
            event_return: *mut c_void,
        ) -> i32 {
            let Some(real_fn) = $real() else {
                return 0;
            };
            // NOTE: the real call comes first with the caller's original args;
            // the macro only appends `event_return` last, so forwarding below
            // must repeat the exact parameter list.
            let result = real_fn($($arg),*, event_return);
            if result != 0 && maybe_consume_event(event_return) {
                return 0;
            }
            result
        }
    };
    // XCheckIfEvent has (display, event, predicate, arg): event_return is 2nd,
    // not last, so it keeps a hand-written shell (same 3-line body).
}

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

// Non-blocking checks below share one 3-line body (real call → maybe_consume
// → return 0 when consumed). XCheckIfEvent above is hand-written because its
// event pointer is 2nd, not last.
check_hook!(XCheckWindowEvent(display: *mut c_void, w: u64, event_mask: i64) => xcheck_window_event);
check_hook!(XCheckTypedEvent(display: *mut c_void, event_type: i32) => xcheck_typed_event);
check_hook!(XCheckTypedWindowEvent(display: *mut c_void, w: u64, event_type: i32) => xcheck_typed_window_event);
check_hook!(XCheckMaskEvent(display: *mut c_void, event_mask: i64) => xcheck_mask_event);

/// Blocking event wait. Loops internally to skip consumed events.
#[no_mangle]
pub unsafe extern "C" fn XNextEvent(display: *mut c_void, event_return: *mut c_void) -> i32 {
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

/// Interposed pointer grab (see `grabs_neutered`).
#[no_mangle]
pub unsafe extern "C" fn XGrabPointer(
    display: *mut c_void,
    grab_window: u64,
    owner_events: i32,
    event_mask: u32,
    pointer_mode: i32,
    keyboard_mode: i32,
    confine_to: u64,
    cursor: u64,
    time: u64,
) -> i32 {
    if grabs_neutered() {
        return 0;
    }
    let Some(real_fn) = xgrab_pointer() else {
        return 1;
    };
    real_fn(
        display,
        grab_window,
        owner_events,
        event_mask,
        pointer_mode,
        keyboard_mode,
        confine_to,
        cursor,
        time,
    )
}

/// Interposed XI2 device grab (see `grabs_neutered`).
#[no_mangle]
pub unsafe extern "C" fn XIGrabDevice(
    display: *mut c_void,
    deviceid: i32,
    grab_window: u64,
    time: u64,
    cursor: u64,
    grab_mode: i32,
    paired_device_mode: i32,
    owner_events: i32,
    mask: *mut c_void,
) -> i32 {
    if grabs_neutered() {
        return 0;
    }
    let Some(real_fn) = xi_grab_device() else {
        return 1;
    };
    real_fn(
        display,
        deviceid,
        grab_window,
        time,
        cursor,
        grab_mode,
        paired_device_mode,
        owner_events,
        mask,
    )
}

/// Interposed keyboard grab (see `grabs_neutered`): with it active the
/// compositor never sees Alt+Tab, Super or Alt+F4. Same deal as pointer.
#[no_mangle]
pub unsafe extern "C" fn XGrabKeyboard(
    display: *mut c_void,
    grab_window: u64,
    owner_events: i32,
    pointer_mode: i32,
    keyboard_mode: i32,
    time: u64,
) -> i32 {
    if grabs_neutered() {
        return 0;
    }
    let Some(real_fn) = xgrab_keyboard() else {
        return 1;
    };
    real_fn(
        display,
        grab_window,
        owner_events,
        pointer_mode,
        keyboard_mode,
        time,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_coords_match_xevent_layout() {
        // XMotionEvent/XButtonEvent: x,y at 64/68 (window-relative),
        // x_root/y_root at 72/76 (screen). The layer composites in screen
        // space, so reads must use the root pair.
        assert_eq!((EV_X_ROOT, EV_Y_ROOT), (72, 76));
        assert_eq!((EV_STATE, EV_DETAIL), (80, 84));
    }
}
