//! X11 window translation for input mapping.
//!
//! Split out of `canvas.rs` (which owns the SHM upload + quad compositing):
//! X11 reports pointer motion in root (screen) coordinates, but the panel
//! rect lives in the swapchain's client space. This module remembers which
//! X11 window sits behind each swapchain and subtracts its root-space
//! origin. Non-X11 surfaces (Wayland, XCB) leave no entry and coordinates
//! pass through untouched.

use std::collections::HashMap;
use std::sync::Mutex;

/// Game X11 window behind a swapchain, with its cached root-space origin.
/// For fullscreen games at the origin client and root coincide; for moved
/// windows they differ by the origin. The origin refreshes about once a
/// second (windows rarely move); a failed refresh keeps the previous one.
/// No entry (non-X11 surfaces) means root coordinates pass through untouched.
struct Xwin {
    dpy: usize,
    window: u64,
    ox: i32,
    oy: i32,
    age: u32,
}

/// Created VkSurface → (Display*, Window), filled by the surface hooks.
static SURFACE_XWIN: Mutex<Option<HashMap<u64, (usize, u64)>>> = Mutex::new(None);
static SWAPCHAIN_XWIN: Mutex<Option<HashMap<u64, Xwin>>> = Mutex::new(None);

/// Remembers which X11 window a created VkSurface wraps. Called from the
/// surface-creation hooks with the raw create-info pointers.
pub fn record_xlib_surface(surface: u64, dpy: usize, window: u64) {
    SURFACE_XWIN
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(surface, (dpy, window));
}

/// Binds a swapchain to its surface's X11 window for input translation.
/// Unknown surfaces (Wayland, XCB) leave no entry: coordinates pass
/// through exactly as before.
pub fn bind_swapchain_window(swapchain: u64, surface: u64) {
    let xwin = SURFACE_XWIN
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&surface))
        .map(|&(dpy, window)| Xwin {
            dpy,
            window,
            ox: 0,
            oy: 0,
            // Refresh on first use: windowed games need their origin
            // before the first mapped event, not a second later.
            age: 60,
        });
    if let Some(xwin) = xwin {
        SWAPCHAIN_XWIN
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(swapchain, xwin);
    }
}

/// Pure core of the translation (unit-tested): subtract the origin.
fn apply_origin(rx: f32, ry: f32, ox: i32, oy: i32) -> (f32, f32) {
    (rx - ox as f32, ry - oy as f32)
}

type TranslateCoordsFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    u64,
    u64,
    i32,
    i32,
    *mut i32,
    *mut i32,
    *mut u64,
) -> i32;
type DefaultRootFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> u64;

fn x11_fns() -> Option<(TranslateCoordsFn, DefaultRootFn)> {
    static FNS: std::sync::OnceLock<Option<(TranslateCoordsFn, DefaultRootFn)>> =
        std::sync::OnceLock::new();
    *FNS.get_or_init(|| unsafe {
        let translate = libc::dlsym(libc::RTLD_DEFAULT, c"XTranslateCoordinates".as_ptr());
        let root = libc::dlsym(libc::RTLD_DEFAULT, c"XDefaultRootWindow".as_ptr());
        if translate.is_null() || root.is_null() {
            return None;
        }
        let translate: TranslateCoordsFn = std::mem::transmute(translate);
        let root: DefaultRootFn = std::mem::transmute(root);
        Some((translate, root))
    })
}

/// Queries a window's root-space origin: where the window's own (0,0)
/// lands in root coordinates. `None` when Xlib is absent or the query
/// fails — callers keep the previous origin (or none).
fn query_origin(dpy: usize, window: u64) -> Option<(i32, i32)> {
    let (translate, root_of) = x11_fns()?;
    unsafe {
        let dpy = dpy as *mut std::ffi::c_void;
        let root = root_of(dpy);
        let (mut x, mut y) = (0i32, 0i32);
        let mut child = 0u64;
        // Direction matters: source is the window, destination is root.
        // (The reverse answers "where is root-(0,0) in window space",
        // i.e. the negated origin — verified against a live server.)
        let ok = translate(dpy, window, root, 0, 0, &mut x, &mut y, &mut child);
        (ok != 0).then_some((x, y))
    }
}

/// Translates root-space pointer coordinates into the swapchain's client
/// space for input mapping (cursor + commands share the result, so they
/// can never disagree). Fullscreen games sit at the origin and translate
/// to themselves; unknown swapchains pass through untouched.
pub fn translate_to_client(swapchain: u64, rx: f32, ry: f32) -> (f32, f32) {
    let mut map = SWAPCHAIN_XWIN.lock().unwrap();
    let Some(xwin) = map.as_mut().and_then(|m| m.get_mut(&swapchain)) else {
        return (rx, ry);
    };
    // Refresh about once a second (60 presents): windows rarely move, and
    // every refresh is a synchronous X round-trip on the game thread.
    xwin.age = xwin.age.wrapping_add(1);
    if xwin.age >= 60 {
        if let Some((ox, oy)) = query_origin(xwin.dpy, xwin.window) {
            if (ox, oy) != (xwin.ox, xwin.oy) && crate::types::debug_log() {
                eprintln!("ira-overlay: window origin now ({ox},{oy})");
            }
            xwin.ox = ox;
            xwin.oy = oy;
        }
        xwin.age = 0;
    }
    apply_origin(rx, ry, xwin.ox, xwin.oy)
}

#[cfg(test)]
mod tests {
    use super::apply_origin;

    #[test]
    fn test_apply_origin_fullscreen_at_origin_is_identity() {
        assert_eq!(apply_origin(398.0, 21.0, 0, 0), (398.0, 21.0));
    }

    #[test]
    fn test_apply_origin_subtracts_window_offset() {
        // Windowed game at root offset (100, 50): root (398, 21) is
        // client (298, -29).
        assert_eq!(apply_origin(398.0, 21.0, 100, 50), (298.0, -29.0));
    }
}
