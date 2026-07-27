mod atlas;
pub mod capture;
mod focus;
mod model;
mod renderer;
mod resources;
pub mod text;
mod vertex;
pub(crate) mod widget;
mod widgets;

pub use renderer::UiRenderer;
pub use widget::Event;
#[cfg(all(feature = "pango", feature = "cosmic-text"))]
pub use text::toggle_backend;
pub use renderer::mark_ui_dirty;

use std::sync::atomic::{AtomicU32, Ordering};
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

static SCREEN_W: AtomicU32 = AtomicU32::new(1);
static SCREEN_H: AtomicU32 = AtomicU32::new(1);

pub fn set_screen_size(w: u32, h: u32) {
    SCREEN_W.store(w, Ordering::Relaxed);
    SCREEN_H.store(h, Ordering::Relaxed);
}
