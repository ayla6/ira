//! Force-cursor: release game pointer capture when the overlay opens.
//!
//! Games hide the pointer (grabs, blank cursors, confinement); like Steam
//! we break the capture so the compositor draws the real cursor again.
//! Implemented on raw xcb — xcb is thread-safe by design, while Xlib
//! requires process-wide `XInitThreads` that a game may never have called,
//! which makes Xlib calls from our threads a heap-corruption hazard.
//! Best-effort: private connection, every failure silent. No X server
//! (Wayland-native games) means nothing to do — the canvas pointer covers
//! those.

use std::ffi::c_void;
use std::os::raw::c_int;
use std::sync::OnceLock;

// xcb_window_t values and value mask for change_window_attributes.
const XCB_WINDOW_NONE: u32 = 0;
const XCB_CW_CURSOR: u32 = 1 << 14;
const XCB_CURSOR_NONE: u32 = 0;

type ConnectFn = unsafe extern "C" fn(*const std::os::raw::c_char, *mut c_int) -> *mut c_void;
type HasErrorFn = unsafe extern "C" fn(*mut c_void) -> c_int;
type DisconnectFn = unsafe extern "C" fn(*mut c_void);
type FlushFn = unsafe extern "C" fn(*mut c_void) -> c_int;
type GetInputFocusFn = unsafe extern "C" fn(*mut c_void) -> u32;
type GetInputFocusReplyFn =
    unsafe extern "C" fn(*mut c_void, u32, *mut *mut c_void) -> *mut c_void;
type UngrabPointerFn = unsafe extern "C" fn(*mut c_void) -> u32;
type UngrabKeyboardFn = unsafe extern "C" fn(*mut c_void) -> u32;
type ChangeAttrsFn = unsafe extern "C" fn(*mut c_void, u32, u32, *const u32) -> u32;

fn resolve(name: &std::ffi::CStr) -> *mut c_void {
    unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) }
}

macro_rules! sym {
    ($name:literal, $ty:ty) => {{
        static CELL: OnceLock<Option<$ty>> = OnceLock::new();
        *CELL.get_or_init(|| {
            let p = resolve($name);
            (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<_, $ty>(&p) })
        })
    }};
}

/// Focus window from a get_input_focus reply (`focus` u32 at offset 8).
/// Pure read for testability; the caller frees the reply.
fn parse_focus(reply: *const c_void) -> u32 {
    if reply.is_null() {
        return XCB_WINDOW_NONE;
    }
    unsafe { *((reply as *const u8).add(8) as *const u32) }
}

/// Releases pointer capture on the focused window: drops any grab and
/// clears a blanked cursor. Called when the overlay opens and periodically
/// while visible; games re-grab on their own when it closes.
pub fn force_cursor_visible() {
    let Some(connect) = sym!(c"xcb_connect", ConnectFn) else {
        return;
    };
    let Some(has_error) = sym!(c"xcb_connection_has_error", HasErrorFn) else {
        return;
    };
    let Some(disconnect) = sym!(c"xcb_disconnect", DisconnectFn) else {
        return;
    };
    let Some(flush) = sym!(c"xcb_flush", FlushFn) else {
        return;
    };
    let Some(get_focus) = sym!(c"xcb_get_input_focus", GetInputFocusFn) else {
        return;
    };
    let Some(get_focus_reply) = sym!(c"xcb_get_input_focus_reply", GetInputFocusReplyFn)
    else {
        return;
    };
    let Some(ungrab) = sym!(c"xcb_ungrab_pointer", UngrabPointerFn) else {
        return;
    };
    let ungrab_keyboard: Option<UngrabKeyboardFn> =
        sym!(c"xcb_ungrab_keyboard", UngrabKeyboardFn);
    let Some(change_attrs) = sym!(c"xcb_change_window_attributes", ChangeAttrsFn) else {
        return;
    };

    let conn = unsafe { connect(std::ptr::null(), std::ptr::null_mut()) };
    if conn.is_null() || unsafe { has_error(conn) } != 0 {
        if !conn.is_null() {
            unsafe { disconnect(conn) };
        }
        return;
    }
    let cookie = unsafe { get_focus(conn) };
    let mut err: *mut c_void = std::ptr::null_mut();
    let reply = unsafe { get_focus_reply(conn, cookie, &mut err) };
    if !err.is_null() {
        unsafe { libc::free(err) };
    }
    let window = parse_focus(reply);
    unsafe { libc::free(reply) };
    if window != XCB_WINDOW_NONE {
        let none = XCB_CURSOR_NONE;
        let _ = unsafe { change_attrs(conn, window, XCB_CW_CURSOR, &none) };
        let _ = unsafe { ungrab(conn) };
        // Keyboard too: an active game grab blinds the compositor to
        // Alt+Tab, Super and Alt+F4. Absent on old servers: skip silently.
        if let Some(ungrab_keyboard) = ungrab_keyboard {
            let _ = unsafe { ungrab_keyboard(conn) };
        }
        let _ = unsafe { flush(conn) };
    }
    unsafe { disconnect(conn) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_focus_reads_window_id() {
        let mut buf = [0u8; 12];
        buf[8..12].copy_from_slice(&0x00ABCDEFu32.to_le_bytes());
        assert_eq!(parse_focus(buf.as_ptr() as *const c_void), 0x00ABCDEF);
        assert_eq!(parse_focus(std::ptr::null()), XCB_WINDOW_NONE);
    }
}
