//! Shim state — input event queue, visibility flag, and mouse position.
//! All state is process-local (the shim and Vulkan layer share the same process).

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use ira_overlay_ipc::{InputEventRaw, MappedShm, ShmHeader};

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
static INJECTED_UI_DISABLED: OnceLock<bool> = OnceLock::new();

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

/// The game can keep injected helpers active while the standalone Gamescope
/// overlay owns all UI rendering and navigation.
pub fn injected_ui_disabled() -> bool {
    *INJECTED_UI_DISABLED.get_or_init(|| std::env::var_os("IRA_OVERLAY_DISABLE_UI").is_some())
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
    if v {
        crate::cursor::force_cursor_visible();
    }
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
            if let Some(visible) = shm.header().toggle_visible() {
                OVERLAY_VISIBLE.store(visible, Ordering::SeqCst);
                eprintln!("ira-overlay-shim: toggle -> {visible}");
                if visible {
                    crate::cursor::force_cursor_visible();
                }
            }
            return;
        }
    }
    // Fallback: local toggle (no SHM available).
    let v = !OVERLAY_VISIBLE.load(Ordering::SeqCst);
    OVERLAY_VISIBLE.store(v, Ordering::SeqCst);
    if v {
        crate::cursor::force_cursor_visible();
    }
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

static LAST_TOGGLE_KEY: AtomicU32 = AtomicU32::new(u32::MAX);

/// Edge-trigger for hotkey toggles: X11 auto-repeat re-fires KeyPress for
/// a held combination, and the SHM debounce only rate-limits (300ms), so a
/// held Shift+Tab would strobe the overlay and land randomly. Only the
/// first press counts until that keycode is released.
pub fn toggle_edge(keycode: u32) -> bool {
    LAST_TOGGLE_KEY
        .compare_exchange(u32::MAX, keycode, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// Re-arms the toggle edge on key release (same keycode only; other
/// releases must not re-arm a still-held toggle key).
pub fn release_edge(keycode: u32) {
    let _ = LAST_TOGGLE_KEY.compare_exchange(keycode, u32::MAX, Ordering::SeqCst, Ordering::SeqCst);
}

/// Desktop-level combos that must always reach the compositor, even with
/// the overlay up: close window, switcher, overview, fullscreen toggle,
/// terminal. Takes X11 keycodes (evdev + 8) and X11 modifier state.
pub fn is_desktop_combo(keycode: u32, mods: u32) -> bool {
    const ALT: u32 = 0x08;
    const SUPER: u32 = 0x40;
    const CTRL: u32 = 0x04;
    const TAB: u32 = 15 + 8;
    const F4: u32 = 62 + 8;
    const ENTER: u32 = 28 + 8;
    const T: u32 = 20 + 8;
    const SUPER_L: u32 = 125 + 8;
    const SUPER_R: u32 = 126 + 8;
    if keycode == SUPER_L || keycode == SUPER_R || (mods & SUPER) != 0 {
        return true;
    }
    if (mods & ALT) == 0 {
        return false;
    }
    if keycode == F4 || keycode == TAB || keycode == ENTER {
        return true;
    }
    (mods & CTRL) != 0 && keycode == T
}

pub fn initialize() {
    initialize_visibility();
    static LOGGED: OnceLock<()> = OnceLock::new();
    LOGGED.get_or_init(|| {
        eprintln!("ira-overlay-shim: input hooks live (edge-toggle, grab-neuter, desktop-combos)");
    });
}

/// Reads hotkey config from SHM via the canonical decoder, falling back to
/// the default table when SHM is unavailable.
/// Returns (toggle_kc, toggle_mods, screenshot_kc, screenshot_mods, record_kc, record_mods).
pub fn hotkeys() -> (u32, u32, u32, u32, u32, u32) {
    let Some(shm) = shm().and_then(|m| m.lock().ok()) else {
        return ShmHeader::default_hotkeys();
    };
    shm.header().hotkeys()
}

/// Reads gamepad hotkey masks from SHM via the canonical decoder, falling
/// back to the default table when SHM is unavailable.
pub fn gamepad_hotkeys() -> (u32, u32, u32) {
    let Some(shm) = shm().and_then(|m| m.lock().ok()) else {
        return ShmHeader::default_gamepad_hotkeys();
    };
    shm.header().gamepad_hotkeys()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toggle_edge_fires_once_until_release() {
        let k = 900_001;
        release_edge(k);
        assert!(toggle_edge(k));
        assert!(!toggle_edge(k));
        assert!(!toggle_edge(k));
        release_edge(k + 1);
        assert!(!toggle_edge(k));
        release_edge(k);
        assert!(toggle_edge(k));
        release_edge(k);
    }

    #[test]
    fn test_is_desktop_combo() {
        // X11 keycodes are evdev + 8; Alt = 0x08, Super = 0x40.
        assert!(is_desktop_combo(62 + 8, 0x08)); // Alt+F4
        assert!(is_desktop_combo(15 + 8, 0x08)); // Alt+Tab
        assert!(is_desktop_combo(15 + 8, 0x08 | 0x01)); // Shift+Alt+Tab
        assert!(is_desktop_combo(28 + 8, 0x08)); // Alt+Enter
        assert!(is_desktop_combo(20 + 8, 0x08 | 0x04)); // Ctrl+Alt+T
        assert!(is_desktop_combo(125 + 8, 0)); // Super_L alone
        assert!(is_desktop_combo(30, 0x40)); // Super+A
        assert!(!is_desktop_combo(28 + 8, 0)); // plain Enter
        assert!(!is_desktop_combo(15 + 8, 0)); // plain Tab
        assert!(!is_desktop_combo(30, 0x08)); // Alt+A stays consumed
        assert!(!is_desktop_combo(28 + 8, 0x04)); // Ctrl+Enter stays consumed
    }
}
