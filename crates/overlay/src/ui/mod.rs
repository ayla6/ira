mod atlas;
pub(crate) mod capture;
mod focus;
mod renderer;
mod resources;
pub(crate) mod text;
mod vertex;
pub(crate) mod widget;
mod widgets;

pub use atlas::cleanup_old_staging;
pub use renderer::UiRenderer;
pub use widget::Event;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::Mutex;

static INPUT_EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());

pub fn push_event(event: Event) {
    if let Ok(mut events) = INPUT_EVENTS.lock() {
        events.push(event);
    }
}

pub fn take_events() -> Vec<Event> {
    std::mem::take(&mut *INPUT_EVENTS.lock().unwrap())
}

static MOUSE_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_Y: AtomicI32 = AtomicI32::new(0);
static MOUSE_ACTIVE: AtomicBool = AtomicBool::new(false);
static SCREEN_W: AtomicU32 = AtomicU32::new(1);
static SCREEN_H: AtomicU32 = AtomicU32::new(1);

pub(crate) fn set_screen_size(w: u32, h: u32) {
    SCREEN_W.store(w, Ordering::Relaxed);
    SCREEN_H.store(h, Ordering::Relaxed);
}

pub(crate) fn update_mouse(dx: i32, dy: i32) -> (f32, f32) {
    let w = SCREEN_W.load(Ordering::Relaxed) as i32;
    let h = SCREEN_H.load(Ordering::Relaxed) as i32;
    let x = (MOUSE_X.load(Ordering::Relaxed) + dx).clamp(0, w - 1);
    let y = (MOUSE_Y.load(Ordering::Relaxed) + dy).clamp(0, h - 1);
    MOUSE_X.store(x, Ordering::Relaxed);
    MOUSE_Y.store(y, Ordering::Relaxed);
    MOUSE_ACTIVE.store(true, Ordering::Relaxed);
    (x as f32, y as f32)
}

pub(crate) fn mouse_pos() -> (f32, f32) {
    (MOUSE_X.load(Ordering::Relaxed) as f32, MOUSE_Y.load(Ordering::Relaxed) as f32)
}

pub(crate) fn mouse_active() -> bool {
    MOUSE_ACTIVE.load(Ordering::Relaxed)
}
