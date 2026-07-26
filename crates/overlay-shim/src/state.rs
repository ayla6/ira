//! Shim state — input event queue, visibility flag, and mouse position.
//! All state is process-local (the shim and Vulkan layer share the same process).

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;

use ira_overlay_ipc::InputEventRaw;

pub static OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);
static HAS_SDL: AtomicBool = AtomicBool::new(false);
static SDL_CHECKED: AtomicBool = AtomicBool::new(false);
static PRESENT_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const MIN_PRESENTS: u64 = 60;

static MOUSE_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_Y: AtomicI32 = AtomicI32::new(0);
static INPUT_QUEUE: Mutex<Vec<InputEventRaw>> = Mutex::new(Vec::new());

/// Returns true if the overlay system is active (IRA_OVERLAY_SHM env var is set).
/// When false, all hooks pass events through unmodified.
/// Cached after first call — env vars don't change at runtime.
static OVERLAY_ACTIVE_CACHED: AtomicBool = AtomicBool::new(false);
static OVERLAY_ACTIVE_INIT: AtomicBool = AtomicBool::new(false);

pub fn overlay_active() -> bool {
    if !OVERLAY_ACTIVE_INIT.load(Ordering::Relaxed) {
        let active = std::env::var_os("IRA_OVERLAY_SHM").is_some();
        OVERLAY_ACTIVE_CACHED.store(active, Ordering::Relaxed);
        OVERLAY_ACTIVE_INIT.store(true, Ordering::Relaxed);
        active
    } else {
        OVERLAY_ACTIVE_CACHED.load(Ordering::Relaxed)
    }
}

pub fn push_event(event: InputEventRaw) {
    if let Ok(mut q) = INPUT_QUEUE.lock() {
        q.push(event);
    }
}

pub fn drain_events(out: &mut [InputEventRaw]) -> usize {
    INPUT_QUEUE.lock().map_or(0, |mut q| {
        let n = q.len().min(out.len());
        out[..n].copy_from_slice(&q[..n]);
        q.drain(..n);
        n
    })
}

pub fn set_visible(v: bool) {
    OVERLAY_VISIBLE.store(v, Ordering::SeqCst);
}

pub fn is_visible() -> bool {
    OVERLAY_VISIBLE.load(Ordering::SeqCst)
}

pub fn increment_present_count() {
    PRESENT_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn reset_present_count() {
    PRESENT_COUNT.store(0, Ordering::Relaxed);
}

pub fn ready_for_overlay() -> bool {
    PRESENT_COUNT.load(Ordering::Relaxed) >= MIN_PRESENTS
}

/// Set by SDL hooks when SDL2 is detected. When true, the Vulkan layer
/// skips evdev gamepad polling (SDL hooks handle it and can consume events).
pub fn set_has_sdl(v: bool) {
    HAS_SDL.store(v, Ordering::SeqCst);
}

/// Returns true if SDL is loaded. On first call, detects SDL by checking
/// if SDL functions are available via dlsym(RTLD_DEFAULT, ...).
/// This catches games that use SDL without calling SDL_PollEvent (e.g. shadPS4
/// uses SDL_GameControllerGetButton directly), which would otherwise cause
/// evdev to poll and conflict with SDL's gamepad handling.
pub fn has_sdl() -> bool {
    if HAS_SDL.load(Ordering::SeqCst) {
        return true;
    }
    if !SDL_CHECKED.swap(true, Ordering::SeqCst) {
        // Use dlsym instead of dlopen(RTLD_NOLOAD) — dlsym(RTLD_DEFAULT, ...)
        // searches all loaded libraries regardless of their path, which is
        // more reliable for AppImages that load SDL from non-standard paths.
        let sdl_init = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"SDL_Init".as_ptr()) };
        let sdl_gamepad = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"SDL_GameControllerOpen".as_ptr()) };
        let sdl3_gamepad = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"SDL_OpenGamepad".as_ptr()) };
        let found = !sdl_init.is_null() || !sdl_gamepad.is_null() || !sdl3_gamepad.is_null();
        if found {
            eprintln!("ira-overlay: SDL detected via dlsym, disabling evdev");
            HAS_SDL.store(true, Ordering::SeqCst);
        } else {
            eprintln!("ira-overlay: SDL not detected (dlsym found nothing)");
        }
    }
    HAS_SDL.load(Ordering::SeqCst)
}

pub fn set_mouse_pos(x: i32, y: i32) {
    MOUSE_X.store(x, Ordering::Relaxed);
    MOUSE_Y.store(y, Ordering::Relaxed);
}

pub fn mouse_pos() -> (i32, i32) {
    (
        MOUSE_X.load(Ordering::Relaxed),
        MOUSE_Y.load(Ordering::Relaxed),
    )
}
