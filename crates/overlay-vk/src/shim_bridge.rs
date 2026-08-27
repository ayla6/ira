//! Bridge to the LD_PRELOAD input shim.
//!
//! The shim (`libira_overlay_shim.so`) is loaded via `LD_PRELOAD` and exports
//! C functions (`ira_overlay_poll_events`, `ira_overlay_is_visible`, etc.).
//! This module resolves them via `dlsym(RTLD_DEFAULT, ...)` at first use,
//! then calls them each frame to poll input events and sync visibility.
//!
//! If the shim is not loaded (dlsym returns NULL), all functions return
//! defaults — the overlay simply won't receive input.

use std::ffi::c_int;
use std::sync::{Mutex, OnceLock};

use ira_overlay::ui::{capture, push_event, Event};
use ira_overlay_ipc::{InputEventRaw, MappedShm, ShmHeader};

type PollEventsFn = unsafe extern "C" fn(*mut InputEventRaw, usize) -> usize;
type IsVisibleFn = unsafe extern "C" fn() -> c_int;
type SetVisibleFn = unsafe extern "C" fn(c_int);
type HasSdlFn = unsafe extern "C" fn() -> c_int;
type IncrementPresentFn = unsafe extern "C" fn();
type ResetPresentFn = unsafe extern "C" fn();
type ReadyForOverlayFn = unsafe extern "C" fn() -> c_int;

static POLL_EVENTS: OnceLock<Option<PollEventsFn>> = OnceLock::new();
static IS_VISIBLE: OnceLock<Option<IsVisibleFn>> = OnceLock::new();
static SET_VISIBLE: OnceLock<Option<SetVisibleFn>> = OnceLock::new();
static HAS_SDL: OnceLock<Option<HasSdlFn>> = OnceLock::new();
static INCREMENT_PRESENT: OnceLock<Option<IncrementPresentFn>> = OnceLock::new();
static RESET_PRESENT: OnceLock<Option<ResetPresentFn>> = OnceLock::new();
static READY_FOR_OVERLAY: OnceLock<Option<ReadyForOverlayFn>> = OnceLock::new();

fn poll_fn() -> Option<PollEventsFn> {
    *POLL_EVENTS.get_or_init(|| {
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ira_overlay_poll_events".as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

fn visible_fn() -> Option<IsVisibleFn> {
    *IS_VISIBLE.get_or_init(|| {
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ira_overlay_is_visible".as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

fn set_visible_fn() -> Option<SetVisibleFn> {
    *SET_VISIBLE.get_or_init(|| {
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ira_overlay_set_visible".as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

/// Returns true if the overlay is visible (synced from the shim).
/// Returns false if the shim is not loaded.
pub fn is_visible() -> bool {
    visible_fn().is_some_and(|f| unsafe { f() != 0 })
}

/// Sets the overlay visibility. Called by the Wayland input handler
/// when Shift+Tab is pressed, or by any other input path that needs
/// to toggle the overlay.
pub fn set_visible(v: bool) {
    if let Some(f) = set_visible_fn() {
        unsafe { f(if v { 1 } else { 0 }) };
    }
}

/// Returns true if SDL2 hooks are active (SDL2 detected via LD_PRELOAD).
/// When true, evdev gamepad polling is skipped since SDL hooks can consume
/// events (evdev can't).
pub fn has_sdl_hooks() -> bool {
    let f = *HAS_SDL.get_or_init(|| {
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ira_overlay_has_sdl".as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    });
    f.is_some_and(|f| unsafe { f() != 0 })
}

/// Increments the present counter in the shim. Called on every queue_present.
pub fn increment_present_count() {
    let f = *INCREMENT_PRESENT.get_or_init(|| {
        let p = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ira_overlay_increment_present_count".as_ptr(),
            )
        };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    });
    if let Some(f) = f {
        unsafe { f() };
    }
}

/// Resets the present counter to zero. Called when a new swapchain is created.
pub fn reset_present_count() {
    let f = *RESET_PRESENT.get_or_init(|| {
        let p = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ira_overlay_reset_present_count".as_ptr(),
            )
        };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    });
    if let Some(f) = f {
        unsafe { f() };
    }
}

/// Returns true if enough frames have been presented for the overlay to be safe.
/// If the shim isn't loaded (dlsym fails), returns true — no present count to wait for.
pub fn ready_for_overlay() -> bool {
    let f = *READY_FOR_OVERLAY.get_or_init(|| {
        let p = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ira_overlay_ready_for_overlay".as_ptr(),
            )
        };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    });
    f.is_none_or(|f| unsafe { f() != 0 })
}

// ─── SHM-based hotkey config ───

static SHM: OnceLock<Option<Mutex<MappedShm>>> = OnceLock::new();

fn shm() -> Option<&'static Mutex<MappedShm>> {
    SHM.get_or_init(|| {
        let path = std::env::var_os("IRA_OVERLAY_SHM")?;
        MappedShm::open_rw(&path.to_string_lossy())
            .ok()
            .map(Mutex::new)
    })
    .as_ref()
}

/// Reads hotkey config from SHM, falling back to defaults.
/// Returns (toggle_kc, toggle_mods, screenshot_kc, screenshot_mods, record_kc, record_mods).
pub fn hotkeys() -> (u32, u32, u32, u32, u32, u32) {
    let Some(shm) = shm().and_then(|m| m.lock().ok()) else {
        return ShmHeader::default_hotkeys();
    };
    shm.header().hotkeys()
}

pub fn gamepad_hotkeys() -> (u32, u32, u32) {
    let Some(shm) = shm().and_then(|m| m.lock().ok()) else {
        return ShmHeader::default_gamepad_hotkeys();
    };
    shm.header().gamepad_hotkeys()
}

/// Polls input events from the shim and forwards them to the overlay UI.
/// Call this every frame from `queue_present`.
pub fn poll_and_forward() {
    let Some(f) = poll_fn() else { return };

    let mut buf = [InputEventRaw::default(); 64];
    let count = unsafe { f(buf.as_mut_ptr(), buf.len()) };

    for raw in &buf[..count] {
        convert_and_forward(raw);
    }
}

// X11 navigation keycodes — what the shim puts on InputEventRaw.keycode.
// Single source of truth: ShmHeader::NAV_KEYCODES_X11, order
// [Return, Up, Down, Left, Right].
const KC_RETURN: u32 = ShmHeader::NAV_KEYCODES_X11[0];
const KC_UP: u32 = ShmHeader::NAV_KEYCODES_X11[1];
const KC_DOWN: u32 = ShmHeader::NAV_KEYCODES_X11[2];
const KC_LEFT: u32 = ShmHeader::NAV_KEYCODES_X11[3];
const KC_RIGHT: u32 = ShmHeader::NAV_KEYCODES_X11[4];

fn convert_and_forward(raw: &InputEventRaw) {
    match raw.event_type {
        0 => {
            // Mouse move — push event with coordinates.
            push_event(Event::MouseMove {
                x: raw.x as f32,
                y: raw.y as f32,
            });
        }
        1 => {
            push_event(Event::MouseDown {
                x: raw.x as f32,
                y: raw.y as f32,
            });
        }
        2 => {
            push_event(Event::MouseUp {
                x: raw.x as f32,
                y: raw.y as f32,
            });
        }
        3 => {
            // Key press — map navigation keys.
            let event = match raw.keycode {
                KC_UP => Some(Event::NavUp),
                KC_DOWN => Some(Event::NavDown),
                KC_LEFT => Some(Event::NavLeft),
                KC_RIGHT => Some(Event::NavRight),
                KC_RETURN => Some(Event::Activate),
                _ => None,
            };
            if let Some(e) = event {
                push_event(e);
            }
        }
        4 => {
            // Key release — not used by the current UI.
        }
        5 => {
            // Screenshot hotkey (F12).
            capture::request_screenshot();
        }
        6 => {
            // Recording toggle hotkey (F11).
            capture::toggle_recording();
        }
        7 => {
            // Mouse scroll event.
            push_event(Event::Scroll {
                delta_y: raw.y as f32,
            });
        }
        _ => {}
    }
}
