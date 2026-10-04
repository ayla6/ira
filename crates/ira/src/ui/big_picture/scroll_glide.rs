//! One scroll-glide mechanism for every big-picture surface: the game
//! grid, the groups grid and the home carousel all ease toward their
//! scroll goal on frame-clock ticks instead of jumping. A discrete press
//! glides; input that arrives mid-glide (held repeats) snaps straight to
//! the new goal instead — the camera always shows the selection, never
//! trails it with a perpetually restarting ease.

use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

/// Glide length, shared so a tab slide never mixes scroll speeds.
const SCROLL_MILLIS: u64 = 90;

/// Smallest glide worth animating; anything tinier snaps directly.
const SNAP_PIXELS: f64 = 0.5;

/// The in-flight glide toward a scroll goal. Cloneable so frame
/// callbacks can hold it past the borrowing widget's lifetime.
#[derive(Clone)]
pub(super) struct ScrollGlide {
    goal: Rc<Cell<f64>>,
    anim: Rc<RefCell<Option<gtk4::TickCallbackId>>>,
}

impl ScrollGlide {
    pub(super) fn new() -> Self {
        Self {
            goal: Rc::new(Cell::new(0.0)),
            anim: Rc::new(RefCell::new(None)),
        }
    }

    /// Glide `adj` toward `target` on frame ticks with an ease-out, in
    /// sync with vsync instead of drifting timers. When a glide is
    /// already running the new goal wins immediately: holding a
    /// direction tracks the selection row by row instead of scrolling
    /// blind behind a chase camera.
    pub(super) fn animate_to(&self, scrolled: &gtk4::ScrolledWindow, adj: &gtk4::Adjustment, target: f64) {
        self.goal.set(target);
        if self.anim.borrow().is_some() {
            self.finish();
            adj.set_value(target);
            return;
        }
        if (target - adj.value()).abs() < SNAP_PIXELS {
            return;
        }
        let adj = adj.clone();
        let start = adj.value();
        let started = Instant::now();
        let goal = Rc::clone(&self.goal);
        let anim = Rc::clone(&self.anim);
        let id = scrolled.add_tick_callback(move |_, _| {
            let t = (started.elapsed().as_millis() as f64 / SCROLL_MILLIS as f64).min(1.0);
            let eased = 1.0 - (1.0 - t) * (1.0 - t);
            let g = goal.get();
            adj.set_value(start + (g - start) * eased);
            if t >= 1.0 {
                adj.set_value(g);
                *anim.borrow_mut() = None;
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        *self.anim.borrow_mut() = Some(id);
    }

    /// End any in-flight glide, leaving the value where it is. Tab
    /// switches snap to their own target afterwards.
    pub(super) fn finish(&self) {
        if let Some(id) = self.anim.borrow_mut().take() {
            id.remove();
        }
    }
}
