//! Shim state — input event queue, visibility flag, and mouse position.
//! All state is process-local (the shim and Vulkan layer share the same process).

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use ira_overlay_ipc::{InputEventRaw, MappedShm};
use ira_overlay_ipc::{
    DEFAULT_RECORD_GAMEPAD_HOTKEY, DEFAULT_RECORD_KEYCODE, DEFAULT_RECORD_MODS,
    DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY, DEFAULT_SCREENSHOT_KEYCODE, DEFAULT_SCREENSHOT_MODS,
    DEFAULT_TOGGLE_GAMEPAD_HOTKEY, DEFAULT_TOGGLE_KEYCODE, DEFAULT_TOGGLE_MODS,
};

pub static OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);
static VISIBILITY_INITIALIZED: AtomicBool = AtomicBool::new(false);
static HAS_SDL: AtomicBool = AtomicBool::new(false);
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
        eprintln!(
            "ira-overlay-shim: overlay_active={} shm={:?}",
            active,
            std::env::var_os("IRA_OVERLAY_SHM")
        );
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

/// SHM mapping for cross-process visibility flag (used by standalone overlay).
/// Lazily opened on first `set_visible` call.
static SHM: OnceLock<Option<Mutex<MappedShm>>> = OnceLock::new();

fn shm() -> Option<&'static Mutex<MappedShm>> {
    let opt = SHM.get_or_init(|| {
        let path = std::env::var_os("IRA_OVERLAY_SHM")?;
        match MappedShm::open_rw(&path.to_string_lossy()) {
            Ok(shm) => Some(Mutex::new(shm)),
            Err(e) => {
                eprintln!("ira-overlay-shim: failed to open SHM for visibility: {e}");
                None
            }
        }
    });
    opt.as_ref()
}

pub fn set_visible(v: bool) {
    eprintln!("ira-overlay-shim: set_visible({v})");
    VISIBILITY_INITIALIZED.store(true, Ordering::Release);
    OVERLAY_VISIBLE.store(v, Ordering::SeqCst);
    if let Some(shm) = shm() {
        if let Ok(shm) = shm.lock() {
            shm.header()
                .overlay_visible
                .store(if v { 1 } else { 0 }, Ordering::SeqCst);
        }
    }
}

/// Atomically toggles overlay visibility via compare_exchange on SHM.
/// Includes a 300ms cross-process debounce to prevent multiple child processes
/// (game + zenity dialogs, etc.) from toggling simultaneously on the same key event.
pub fn toggle_visible() {
    VISIBILITY_INITIALIZED.store(true, Ordering::Release);
    if let Some(shm) = shm() {
        if let Ok(shm) = shm.lock() {
            let now = now_ms();
            let last = shm.header().last_toggle_ms.load(Ordering::SeqCst);
            // Debounce: ignore toggles within 300ms of the last one.
            if now.wrapping_sub(last) < 300 {
                return;
            }
            let current = shm.header().overlay_visible.load(Ordering::SeqCst);
            let new_val: u32 = if current != 0 { 0 } else { 1 };
            match shm.header().overlay_visible.compare_exchange(
                current,
                new_val,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    shm.header().last_toggle_ms.store(now, Ordering::SeqCst);
                    OVERLAY_VISIBLE.store(new_val != 0, Ordering::SeqCst);
                    eprintln!("ira-overlay-shim: toggle -> {}", new_val != 0);
                }
                Err(_) => {
                    // Another process toggled first — skip.
                }
            }
            return;
        }
    }
    // Fallback: local toggle (no SHM available).
    let v = !OVERLAY_VISIBLE.load(Ordering::SeqCst);
    OVERLAY_VISIBLE.store(v, Ordering::SeqCst);
}

fn initialize_visibility() {
    if VISIBILITY_INITIALIZED.swap(true, Ordering::AcqRel) {
        return;
    }
    let requested = std::env::var_os("IRA_OVERLAY_START_VISIBLE")
        .is_some_and(|value| value == "1" || value == "true");
    if requested {
        set_visible(true);
    }
}

pub fn initialize() {
    initialize_visibility();
}

fn now_ms() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0)
}

/// Reads hotkey config from SHM, falling back to defaults if fields are 0.
/// Returns (toggle_kc, toggle_mods, screenshot_kc, screenshot_mods, record_kc, record_mods).
pub fn hotkeys() -> (u32, u32, u32, u32, u32, u32) {
    let Some(shm) = shm() else {
        return (
            DEFAULT_TOGGLE_KEYCODE,
            DEFAULT_TOGGLE_MODS,
            DEFAULT_SCREENSHOT_KEYCODE,
            DEFAULT_SCREENSHOT_MODS,
            DEFAULT_RECORD_KEYCODE,
            DEFAULT_RECORD_MODS,
        );
    };
    let Ok(shm) = shm.lock() else {
        return (
            DEFAULT_TOGGLE_KEYCODE,
            DEFAULT_TOGGLE_MODS,
            DEFAULT_SCREENSHOT_KEYCODE,
            DEFAULT_SCREENSHOT_MODS,
            DEFAULT_RECORD_KEYCODE,
            DEFAULT_RECORD_MODS,
        );
    };
    let hdr = shm.header();
    let tog_kc = if hdr.toggle_keysym == 0 {
        DEFAULT_TOGGLE_KEYCODE
    } else {
        hdr.toggle_keysym
    };
    let tog_mods = if hdr.toggle_keysym == 0 {
        DEFAULT_TOGGLE_MODS
    } else {
        hdr.toggle_mods
    };
    let ss_kc = if hdr.screenshot_keysym == 0 {
        DEFAULT_SCREENSHOT_KEYCODE
    } else {
        hdr.screenshot_keysym
    };
    let ss_mods = if hdr.screenshot_keysym == 0 {
        DEFAULT_SCREENSHOT_MODS
    } else {
        hdr.screenshot_mods
    };
    let rec_kc = if hdr.record_keysym == 0 {
        DEFAULT_RECORD_KEYCODE
    } else {
        hdr.record_keysym
    };
    let rec_mods = if hdr.record_keysym == 0 {
        DEFAULT_RECORD_MODS
    } else {
        hdr.record_mods
    };
    (tog_kc, tog_mods, ss_kc, ss_mods, rec_kc, rec_mods)
}

pub fn gamepad_hotkeys() -> (u32, u32, u32) {
    let Some(shm) = shm() else {
        return (
            DEFAULT_TOGGLE_GAMEPAD_HOTKEY,
            DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY,
            DEFAULT_RECORD_GAMEPAD_HOTKEY,
        );
    };
    let Ok(shm) = shm.lock() else {
        return (
            DEFAULT_TOGGLE_GAMEPAD_HOTKEY,
            DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY,
            DEFAULT_RECORD_GAMEPAD_HOTKEY,
        );
    };
    let header = shm.header();
    (
        header.toggle_gamepad,
        header.screenshot_gamepad,
        header.record_gamepad,
    )
}

pub fn is_visible() -> bool {
    if OVERLAY_VISIBLE.load(Ordering::SeqCst) {
        return true;
    }
    // Also check SHM — the standalone overlay may have toggled visibility
    // directly (e.g., via its own keyboard handler) without going through
    // the shim's set_visible().
    let Some(shm) = shm() else {
        return false;
    };
    let Ok(shm) = shm.lock() else {
        return false;
    };
    shm.header().overlay_visible.load(Ordering::SeqCst) != 0
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

/// Returns true once the SDL event hook has received an event. Merely loading
/// SDL is insufficient because direct-state games never call `SDL_PollEvent`.
pub fn has_sdl() -> bool {
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
