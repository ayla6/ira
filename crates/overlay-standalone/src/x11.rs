//! XCB connection + window for the standalone overlay under gamescope.
//!
//! Creates a borderless X11 window via XCB on gamescope's XWayland server.
//! The window is marked as a `GAMESCOPE_EXTERNAL_OVERLAY` so gamescope
//! composites it on top of the game as a separate plane (like mangoapp).
//!
//! The overlay runs under the Gamescope WSI layer, which intercepts
//! `vkCreateXcbSurfaceKHR` and presents frames to gamescope via Wayland
//! (bypassing XWayland) with pre-multiplied alpha. The X11 window itself is a
//! plain depth-24 window — it exists so gamescope can match the overlay's
//! Wayland surface to a window and read the overlay/opacity properties.
//!
//! Visibility is toggled via the `_NET_WM_WINDOW_OPACITY` and
//! `GAMESCOPE_EXTERNAL_OVERLAY` properties:
//!   - visible:  opacity 0xFFFFFFFF, external overlay 1
//!   - hidden:   opacity 0, external overlay 0
//!     Both are set BEFORE the window is mapped so gamescope's initial focus
//!     reroll picks the overlay up; toggling the external-overlay property
//!     later re-triggers that reroll (see `set_visible`).
//!
//! A passive root-window grab receives the toggle chord while hidden. When
//! visible, the keyboard is grabbed so navigation events go to the overlay.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicU32, Ordering};

use ira_overlay::ui::{push_event, Event};
use ira_overlay_ipc::{ShmHeader, X11_KEYCODE_OFFSET};

// ─── XCB constants ───

const XCB_CW_BACK_PIXEL: u32 = 0x00000002;
const XCB_CW_EVENT_MASK: u32 = 0x00000800;
const XCB_EVENT_MASK_KEY_PRESS: u32 = 0x00000001;
const XCB_EVENT_MASK_KEY_RELEASE: u32 = 0x00000002;

const XCB_KEY_PRESS: u8 = 2;

const XCB_ATOM_CARDINAL: u32 = 6;
const XCB_PROP_MODE_REPLACE: u8 = 0;
const XCB_GRAB_MODE_ASYNC: c_int = 1;
const XCB_TIME_CURRENT_TIME: u32 = 0;

// XCB event field offsets (xcb_key_press_event_t)
const OFF_DETAIL: usize = 1;
const OFF_STATE: usize = 28;

// Modifier masks (same as Xlib)
const MOD_SHIFT: u16 = 0x01;

// Navigation keys — X11-domain codes, ordered like
// ShmHeader::NAV_KEYCODES_X11 ([Return, Up, Down, Left, Right]).
const KC_UP: u8 = ShmHeader::NAV_KEYCODES_X11[1] as u8;
const KC_DOWN: u8 = ShmHeader::NAV_KEYCODES_X11[2] as u8;
const KC_LEFT: u8 = ShmHeader::NAV_KEYCODES_X11[3] as u8;
const KC_RIGHT: u8 = ShmHeader::NAV_KEYCODES_X11[4] as u8;
const KC_RETURN: u8 = ShmHeader::NAV_KEYCODES_X11[0] as u8;

// ─── XCB FFI types ───

#[repr(C)]
struct XcbSetup {
    _status: u8,
    _pad0: u8,
    _protocol_major_version: u16,
    _protocol_minor_version: u16,
    _length: u16,
    _release_number: u32,
    resource_id_base: u32,
    resource_id_mask: u32,
    _motion_buffer_size: u32,
    vendor_len: u16,
    _maximum_request_length: u16,
    roots_len: u8,
    _pixmap_formats_len: u8,
    _image_byte_order: u8,
    _bitmap_format_bit_order: u8,
    _bitmap_format_scanline_unit: u8,
    _bitmap_format_scanline_pad: u8,
    _min_keycode: u8,
    _max_keycode: u8,
    _pad1: [u8; 4],
}

#[repr(C)]
struct XcbScreenIterator {
    data: *mut XcbScreen,
    rem: c_int,
    index: c_int,
}

#[repr(C)]
struct XcbScreen {
    root: u32,
    _default_colormap: u32,
    _white_pixel: u32,
    _black_pixel: u32,
    _current_input_masks: u32,
    width_in_pixels: u16,
    height_in_pixels: u16,
    _width_in_millimeters: u16,
    _height_in_millimeters: u16,
    _min_installed_maps: u16,
    _max_installed_maps: u16,
    root_visual: u32,
    _backing_stores: u8,
    _save_unders: u8,
    root_depth: u8,
    _allowed_depths_len: u8,
}

#[repr(C)]
struct XcbGenericEvent {
    response_type: u8,
    _pad: [u8; 31],
}

#[repr(C)]
struct XcbGetGeometryReply {
    _response_type: u8,
    _depth: u8,
    _sequence: u16,
    _length: u32,
    _root: u32,
    _x: i16,
    _y: i16,
    width: u16,
    height: u16,
    _border_width: u16,
    _pad0: [u8; 2],
}

#[repr(C)]
struct XcbInternAtomReply {
    _response_type: u8,
    _pad0: u8,
    _sequence: u16,
    _length: u32,
    atom: u32,
}

// ─── XCB FFI functions (linked at build time via pkg-config) ───

extern "C" {
    fn xcb_connect(display: *const c_char, screen: *mut *mut c_char) -> *mut c_void;
    fn xcb_disconnect(c: *mut c_void);
    fn xcb_connection_has_error(c: *mut c_void) -> c_int;
    fn xcb_get_setup(c: *mut c_void) -> *const XcbSetup;
    fn xcb_setup_roots_iterator(setup: *const XcbSetup) -> XcbScreenIterator;
    fn xcb_generate_id(c: *mut c_void) -> u32;
    fn xcb_create_window(
        c: *mut c_void,
        depth: u8,
        wid: u32,
        parent: u32,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        border_width: u16,
        class: u16,
        visual: u32,
        value_mask: u32,
        value_list: *const u32,
    );
    fn xcb_map_window(c: *mut c_void, window: u32);
    fn xcb_destroy_window(c: *mut c_void, window: u32);
    fn xcb_flush(c: *mut c_void) -> c_int;
    fn xcb_get_geometry(c: *mut c_void, drawable: u32) -> u32;
    fn xcb_get_geometry_reply(
        c: *mut c_void,
        cookie: u32,
        error: *mut *mut c_void,
    ) -> *mut XcbGetGeometryReply;
    fn xcb_poll_for_event(c: *mut c_void) -> *mut XcbGenericEvent;
    fn xcb_intern_atom(
        c: *mut c_void,
        only_if_exists: u8,
        name_len: u16,
        name: *const c_char,
    ) -> u32;
    fn xcb_intern_atom_reply(
        c: *mut c_void,
        cookie: u32,
        error: *mut *mut c_void,
    ) -> *mut XcbInternAtomReply;
    fn xcb_change_property(
        c: *mut c_void,
        mode: u8,
        window: u32,
        property: u32,
        type_: u32,
        format: u8,
        data_len: u32,
        data: *const u32,
    ) -> u32;
    fn xcb_grab_keyboard(
        c: *mut c_void,
        owner_events: u8,
        grab_window: u32,
        time: u32,
        pointer_mode: c_int,
        keyboard_mode: c_int,
    ) -> u32;
    fn xcb_grab_key(
        c: *mut c_void,
        owner_events: u8,
        grab_window: u32,
        modifiers: u16,
        key: u8,
        pointer_mode: u8,
        keyboard_mode: u8,
    ) -> u32;
    fn xcb_ungrab_keyboard(c: *mut c_void, time: u32) -> u32;
}

// ─── Helper functions ───

fn intern_atom(conn: *mut c_void, name: &CStr) -> u32 {
    let cookie = unsafe { xcb_intern_atom(conn, 0, name.to_bytes().len() as u16, name.as_ptr()) };
    let reply = unsafe { xcb_intern_atom_reply(conn, cookie, std::ptr::null_mut()) };
    if reply.is_null() {
        return 0;
    }
    let atom = unsafe { (*reply).atom };
    unsafe { libc::free(reply as *mut c_void) };
    atom
}

fn set_cardinal(conn: *mut c_void, window: u32, property: u32, value: u32) {
    unsafe {
        xcb_change_property(
            conn,
            XCB_PROP_MODE_REPLACE,
            window,
            property,
            XCB_ATOM_CARDINAL,
            32,
            1,
            &value,
        );
    }
}

/// Configured toggle keybind (X11 keycode = evdev keycode + 8).
pub static TOGGLE_KEYCODE: AtomicU32 = AtomicU32::new(23); // default: Tab (evdev 15 + 8)
pub static TOGGLE_MODS: AtomicU32 = AtomicU32::new(MOD_SHIFT as u32);

pub struct X11State {
    conn: *mut c_void,
    window: u32,
    opacity_atom: u32,
    external_overlay_atom: u32,
}

unsafe impl Send for X11State {}

impl X11State {
    pub fn new() -> Result<Self, String> {
        let display = match std::env::var("DISPLAY") {
            Ok(d) => d,
            Err(e) => return Err(format!("failed to read DISPLAY: {e}")),
        };
        let display_c =
            CString::new(display.as_str()).map_err(|e| format!("invalid DISPLAY: {e}"))?;

        // Retry the connection until gamescope's XWayland is up and answering
        // requests. The overlay is spawned at the same time as the game, so
        // XWayland's socket may not exist yet when we start.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let conn = loop {
            let c = unsafe { xcb_connect(display_c.as_ptr(), std::ptr::null_mut()) };
            if !c.is_null() && unsafe { xcb_connection_has_error(c) } == 0 {
                break c;
            }
            if !c.is_null() {
                unsafe { xcb_disconnect(c) };
            }
            if std::time::Instant::now() > deadline {
                return Err(format!("timed out connecting to X display '{display}'"));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        eprintln!("ira-overlay-standalone: connected to X display '{display}'");

        let setup = unsafe { xcb_get_setup(conn) };
        if setup.is_null() {
            unsafe { xcb_disconnect(conn) };
            return Err("xcb_get_setup returned null".to_string());
        }

        let iter = unsafe { xcb_setup_roots_iterator(setup) };
        if iter.data.is_null() {
            unsafe { xcb_disconnect(conn) };
            return Err("no XCB screen found".to_string());
        }

        let screen = unsafe { &*iter.data };
        let width = screen.width_in_pixels;
        let height = screen.height_in_pixels;

        let window = unsafe { xcb_generate_id(conn) };

        // Plain depth-24 root-visual window. The Gamescope WSI layer presents
        // the overlay's frames to gamescope via Wayland (with alpha), so the
        // X11 window is just a handle gamescope uses to read the overlay and
        // opacity properties.
        let values: [u32; 2] = [
            0x00000000, // back pixel = black
            XCB_EVENT_MASK_KEY_PRESS | XCB_EVENT_MASK_KEY_RELEASE,
        ];
        unsafe {
            xcb_create_window(
                conn,
                screen.root_depth,
                window,
                screen.root,
                0,
                0,
                width,
                height,
                0, // border width
                1, // XCB_WINDOW_CLASS_INPUT_OUTPUT
                screen.root_visual,
                XCB_CW_BACK_PIXEL | XCB_CW_EVENT_MASK,
                values.as_ptr(),
            );
        }

        // Mark as external overlay so gamescope composites us on top of the game.
        let external_overlay_atom = intern_atom(conn, c"GAMESCOPE_EXTERNAL_OVERLAY");
        set_cardinal(conn, window, external_overlay_atom, 1);
        // Set opacity to fully visible BEFORE mapping. gamescope only copies
        // the context focus's externalOverlayWindow into the global focus
        // during a full focus reroll (MakeFocusDirty). If we map hidden, the
        // map-time reroll selects no external overlay and the later opacity
        // change never propagates to the global focus, so we'd never paint.
        let opacity_atom = intern_atom(conn, c"_NET_WM_WINDOW_OPACITY");
        set_cardinal(conn, window, opacity_atom, 0xFFFFFFFF);

        // Map the window once — it stays mapped forever.
        // Visibility is controlled via the opacity property.
        unsafe {
            xcb_grab_key(
                conn,
                1,
                screen.root,
                TOGGLE_MODS.load(Ordering::Relaxed) as u16,
                TOGGLE_KEYCODE.load(Ordering::Relaxed) as u8,
                XCB_GRAB_MODE_ASYNC as u8,
                XCB_GRAB_MODE_ASYNC as u8,
            );
            xcb_map_window(conn, window);
            xcb_flush(conn);
        }

        // Force XWayland to realize the window before we hand it to Vulkan.
        // Querying surface caps on a window XWayland hasn't processed yet can
        // fail (e.g. VK_ERROR_OUT_OF_HOST_MEMORY on a racing driver), so do a
        // geometry round-trip and retry until the window exists on the server.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let cookie = unsafe { xcb_get_geometry(conn, window) };
            let reply = unsafe { xcb_get_geometry_reply(conn, cookie, std::ptr::null_mut()) };
            if !reply.is_null() {
                let w = unsafe { (*reply).width };
                let h = unsafe { (*reply).height };
                unsafe { libc::free(reply as *mut c_void) };
                if w == width && h == height {
                    break;
                }
            }
            if std::time::Instant::now() > deadline {
                return Err(
                    "timed out waiting for XWayland to realize the overlay window".to_string(),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        eprintln!("ira-overlay-standalone: XCB window {window} created ({width}x{height})");

        Ok(X11State {
            conn,
            window,
            opacity_atom,
            external_overlay_atom,
        })
    }

    pub fn connection_ptr(&self) -> *mut c_void {
        self.conn
    }
    pub fn window_id(&self) -> u32 {
        self.window
    }

    pub fn set_visible(&self, visible: bool) {
        eprintln!("ira-overlay-standalone: set_visible({visible})");
        let opacity: u32 = if visible { 0xFFFFFFFF } else { 0 };
        set_cardinal(self.conn, self.window, self.opacity_atom, opacity);
        // Toggle the GAMESCOPE_EXTERNAL_OVERLAY property (like mangoapp).
        // gamescope only refreshes its global external overlay selection during
        // a focus reroll (triggered by MakeFocusDirty), which happens when this
        // property changes. Relying on opacity alone means the overlay shows
        // only until the first reroll after startup.
        let overlay: u32 = if visible { 1 } else { 0 };
        set_cardinal(self.conn, self.window, self.external_overlay_atom, overlay);
        if visible {
            // Grab keyboard so all key events come to the overlay window.
            unsafe {
                xcb_grab_keyboard(
                    self.conn,
                    1, // owner_events: deliver to window's event mask too
                    self.window,
                    XCB_TIME_CURRENT_TIME,
                    XCB_GRAB_MODE_ASYNC,
                    XCB_GRAB_MODE_ASYNC,
                );
            }
        } else {
            unsafe { xcb_ungrab_keyboard(self.conn, XCB_TIME_CURRENT_TIME) };
        }
        unsafe { xcb_flush(self.conn) };
    }

    /// Poll for XCB events and process key presses.
    /// Returns `true` if the toggle hotkey was pressed (caller toggles SHM).
    pub fn poll_events(&self) -> bool {
        let mut toggle_pressed = false;

        loop {
            let event = unsafe { xcb_poll_for_event(self.conn) };
            if event.is_null() {
                break;
            }

            let ev = unsafe { &*(event as *const u8) };
            let response_type = ev & 0x7f;

            if response_type == XCB_KEY_PRESS {
                let detail = unsafe { *event.cast::<u8>().add(OFF_DETAIL) };
                let state = unsafe { *event.cast::<u16>().add(OFF_STATE / 2) };
                // Check toggle key
                let tog_kc = TOGGLE_KEYCODE.load(Ordering::Relaxed) as u8;
                let tog_mods = TOGGLE_MODS.load(Ordering::Relaxed) as u16;
                if (state & tog_mods) == tog_mods && detail == tog_kc {
                    toggle_pressed = true;
                    unsafe { libc::free(event as *mut c_void) };
                    continue;
                }

                // Navigation keys (X11 keycodes = evdev + 8)
                let evdev_kc = detail.wrapping_sub(X11_KEYCODE_OFFSET as u8);
                match evdev_kc {
                    KC_UP => push_event(Event::NavUp),
                    KC_DOWN => push_event(Event::NavDown),
                    KC_LEFT => push_event(Event::NavLeft),
                    KC_RIGHT => push_event(Event::NavRight),
                    KC_RETURN => push_event(Event::Activate),
                    _ => {}
                }
            }

            unsafe { libc::free(event as *mut c_void) };
        }

        toggle_pressed
    }
}

impl Drop for X11State {
    fn drop(&mut self) {
        unsafe {
            xcb_ungrab_keyboard(self.conn, XCB_TIME_CURRENT_TIME);
            xcb_destroy_window(self.conn, self.window);
            xcb_flush(self.conn);
            xcb_disconnect(self.conn);
        }
    }
}
