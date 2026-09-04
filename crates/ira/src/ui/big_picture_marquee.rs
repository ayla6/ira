//! A clipped one-line title that scrolls like a marquee when its text is
//! wider than the space it gets. Short text sits centered and static; long
//! text starts left-aligned, glides to reveal its end, pauses, glides back,
//! and loops — calm enough to fit the rest of the couch UI.

use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// Width the scrolling text is clipped to.
const MAX_WIDTH: f64 = 480.0;
/// Hold at each end before scrolling back, and scroll speed.
const HOLD_MS: f64 = 1_400.0;
const SPEED: f64 = 42.0;
const TICK_MS: u64 = 16;

/// One side of the scroll sweep: `min_x` is the left-aligned position where
/// the text's start is readable, `max_x` the right-aligned one where its end
/// is. Starts left-aligned (readable), glides right, holds, glides back.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Sweep {
    x: f64,
    min_x: f64,
    max_x: f64,
    /// Milliseconds of hold left at the current end before moving on.
    hold_ms: f64,
}

impl Sweep {
    fn new(min_x: f64, max_x: f64) -> Self {
        Self { x: max_x, min_x, max_x, hold_ms: HOLD_MS }
    }

    /// Advance by `dt_ms`; true when the position changed.
    fn advance(&mut self, dt_ms: f64) -> bool {
        if self.min_x >= self.max_x {
            return false;
        }
        if self.hold_ms > 0.0 {
            self.hold_ms = (self.hold_ms - dt_ms).max(0.0);
            if self.hold_ms > 0.0 {
                return false;
            }
        }
        let before = self.x;
        if self.x < self.max_x {
            self.x = (self.x + SPEED * dt_ms / 1000.0).min(self.max_x);
            if self.x >= self.max_x {
                self.hold_ms = HOLD_MS;
            }
        } else {
            self.x = (self.x - SPEED * dt_ms / 1000.0).max(self.min_x);
            if self.x <= self.min_x {
                self.hold_ms = HOLD_MS;
            }
        }
        (self.x - before).abs() > f64::EPSILON
    }
}

/// The widget: a `Fixed` clipping one label, so the text can stick out
/// horizontally without growing the layout.
pub(super) struct Marquee {
    fixed: gtk4::Fixed,
    label: gtk4::Label,
    sweep: Rc<Cell<Option<Sweep>>>,
    tick: RefCell<Option<glib::SourceId>>,
}

impl Marquee {
    pub(super) fn widget(&self) -> &gtk4::Fixed {
        &self.fixed
    }

    pub(super) fn new() -> Self {
        let fixed = gtk4::Fixed::new();
        fixed.set_overflow(gtk4::Overflow::Hidden);
        fixed.set_halign(gtk4::Align::Start);

        let label = gtk4::Label::new(None);
        label.add_css_class(super::css::CSS_BP_TITLE);
        fixed.put(&label, 0.0, 0.0);

        Self { fixed, label, sweep: Rc::new(Cell::new(None)), tick: RefCell::new(None) }
    }

    /// Swap the text and restart the sweep from a readable position. The
    /// widget requests its clipped width and the label's height.
    pub(super) fn set_text(&self, text: &str) {
        if let Some(id) = self.tick.borrow_mut().take() {
            id.remove();
        }
        self.label.set_text(text);
        let (_, natural, _, _) = self.label.measure(gtk4::Orientation::Horizontal, -1);
        let (_, height, _, _) = self.label.measure(gtk4::Orientation::Vertical, -1);
        let natural = natural.max(1) as f64;
        self.fixed.set_size_request(MAX_WIDTH as i32, height.max(1));

        let sweep = Sweep::new(MAX_WIDTH - natural, 0.0);
        self.sweep.set(Some(sweep));
        self.apply();
        if sweep.min_x < sweep.max_x {
            self.start_ticker();
        }
    }

    pub(super) fn set_visible(&self, visible: bool) {
        self.fixed.set_visible(visible);
    }

    fn start_ticker(&self) {
        let sweep = Rc::clone(&self.sweep);
        let fixed = self.fixed.downgrade();
        let id = glib::timeout_add_local(Duration::from_millis(TICK_MS), move || {
            let mut current = sweep.take();
            let mut moved = false;
            if let Some(s) = current.as_mut() {
                moved = s.advance(TICK_MS as f64);
            }
            sweep.set(current);
            match fixed.upgrade() {
                Some(fixed) => {
                    if moved {
                        position_label(&fixed, sweep.get());
                    }
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            }
        });
        *self.tick.borrow_mut() = Some(id);
    }

    fn apply(&self) {
        position_label(&self.fixed, self.sweep.get());
    }
}

fn position_label(fixed: &gtk4::Fixed, sweep: Option<Sweep>) {
    let Some(sweep) = sweep else {
        return;
    };
    fixed.move_(&fixed.first_child().unwrap(), sweep.x, 0.0);
}

#[cfg(test)]
mod tests {
    use super::Sweep;

    #[test]
    fn test_sweep_starts_readable_then_glides() {
        let mut s = Sweep::new(-120.0, 0.0);
        assert_eq!(s.x, 0.0, "starts left-aligned where the title is readable");
        assert!(!s.advance(100.0), "holds before the first glide");
        assert!(s.advance(10_000.0), "glides after the hold");
        assert_eq!(s.x, -120.0);
        assert!(!s.advance(10.0), "holds at the far end");
        assert!(s.advance(10_000.0));
        assert_eq!(s.x, 0.0, "returns to the readable start");
    }

    #[test]
    fn test_sweep_ignores_text_that_fits() {
        let mut s = Sweep::new(0.0, 0.0);
        assert!(!s.advance(10_000.0), "a fitting title never scrolls");
    }
}
