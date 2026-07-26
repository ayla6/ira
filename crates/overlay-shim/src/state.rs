//! Shim state — input event queue, visibility flag, and mouse position.
//! All state is process-local (the shim and Vulkan layer share the same process).

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;

use ira_overlay_ipc::InputEventRaw;

pub static OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);
static HAS_SDL: AtomicBool = AtomicBool::new(false);

static MOUSE_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_Y: AtomicI32 = AtomicI32::new(0);
static INPUT_QUEUE: Mutex<Vec<InputEventRaw>> = Mutex::new(Vec::new());

/// Returns true if the overlay system is active (IRA_OVERLAY_SHM env var is set).
/// When false, all hooks pass events through unmodified.
pub fn overlay_active() -> bool {
    std::env::var_os("IRA_OVERLAY_SHM").is_some()
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

/// Set by SDL hooks when SDL2 is detected. When true, the Vulkan layer
/// skips evdev gamepad polling (SDL hooks handle it and can consume events).
pub fn set_has_sdl(v: bool) {
    HAS_SDL.store(v, Ordering::SeqCst);
}

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
