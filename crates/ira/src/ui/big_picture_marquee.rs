//! A clipping rail for the floating couch title. The widget spans the full
//! row and positions its label internally, so moving the title never
//! changes the rail's own size — a `Fixed` would grow with the label and
//! shove the rest of the screen around. Short text sits centered under the
//! selected tile; long text glides left to reveal its end, pauses, glides
//! back, and loops — calm enough to fit the rest of the couch UI.

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::time::Duration;

/// Width the scrolling text is clipped to.
const MAX_WIDTH: f64 = 480.0;
/// Hold at each end before scrolling back, and scroll speed.
const HOLD_MS: f64 = 1_400.0;
const SPEED: f64 = 42.0;
const TICK_MS: u64 = 16;

/// One side of the scroll sweep: the label starts left-aligned in the clip
/// window (readable), glides right to reveal its end, holds, glides back.
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
        Self { x: 0.0, min_x, max_x, hold_ms: HOLD_MS }
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

mod imp {
    use super::*;

    pub struct Marquee {
        pub label: RefCell<Option<gtk4::Label>>,
        /// Where the 480px clip window sits inside the full-width rail.
        pub window_x: Cell<f64>,
        pub(super) sweep: RefCell<Option<Sweep>>,
        pub rail_height: Cell<i32>,
        pub tick: RefCell<Option<glib::SourceId>>,
    }

    impl Default for Marquee {
        fn default() -> Self {
            Self {
                label: RefCell::new(None),
                window_x: Cell::new(0.0),
                sweep: RefCell::new(None),
                rail_height: Cell::new(56),
                tick: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Marquee {
        const NAME: &'static str = "IraBpMarquee";
        type Type = super::Marquee;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for Marquee {
        fn dispose(&self) {
            if let Some(label) = self.label.borrow().as_ref() {
                label.unparent();
            }
        }
    }

    impl WidgetImpl for Marquee {
        /// Width never reports the label, so a traveling title can't push
        /// the rest of the screen around; height is the fixed rail strip.
        fn measure(&self, orientation: gtk4::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk4::Orientation::Horizontal {
                (0, MAX_WIDTH as i32, -1, -1)
            } else {
                let h = self.rail_height.get();
                (h, h, -1, -1)
            }
        }

        fn size_allocate(&self, _width: i32, height: i32, _baseline: i32) {
            let label_guard = self.label.borrow();
            let Some(label) = label_guard.as_ref() else {
                return;
            };
            let (_, natural_w, _, _) = label.measure(gtk4::Orientation::Horizontal, -1);
            let (_, natural_h, _, _) = label.measure(gtk4::Orientation::Vertical, -1);
            let sweep = self.sweep.borrow();
            // Text wider than the clip window rides within it at the sweep
            // offset; fitting text centers in the window.
            let x = match sweep.as_ref() {
                Some(s) => self.window_x.get() + s.x,
                None => {
                    self.window_x.get() + ((MAX_WIDTH - natural_w as f64) / 2.0).max(0.0)
                }
            };
            let tx = gtk4::gsk::Transform::new().translate(&gtk4::graphene::Point::new(
                x as f32,
                ((height - natural_h) / 2) as f32,
            ));
            label.allocate(natural_w.max(1), natural_h.max(1), -1, Some(tx));
        }
    }
}

glib::wrapper! {
    pub struct Marquee(ObjectSubclass<imp::Marquee>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl Marquee {
    pub(super) fn new(rail_height: i32) -> Self {
        let this: Self = glib::Object::new();
        this.set_overflow(gtk4::Overflow::Hidden);
        this.imp().rail_height.set(rail_height);

        let label = gtk4::Label::new(None);
        label.add_css_class(super::css::CSS_BP_TITLE);
        label.set_parent(&this);
        *this.imp().label.borrow_mut() = Some(label);
        this
    }

    pub(super) fn widget(&self) -> &gtk4::Widget {
        self.upcast_ref()
    }

    /// Slide the clip window so its center rests on `center` (a tile's
    /// center in this rail's coordinates), kept on screen within `viewport`.
    pub(super) fn set_position(&self, center: f64, viewport: f64) {
        let x = (center - MAX_WIDTH / 2.0).clamp(0.0, (viewport - MAX_WIDTH).max(0.0));
        if self.imp().window_x.get() != x {
            self.imp().window_x.set(x);
            self.queue_allocate();
        }
    }

    /// Swap the text and restart the sweep from a readable position.
    pub(super) fn set_text(&self, text: &str) {
        if let Some(id) = self.imp().tick.borrow_mut().take() {
            id.remove();
        }
        let label_guard = self.imp().label.borrow();
        let Some(label) = label_guard.as_ref() else {
            return;
        };
        label.set_text(text);
        let (_, natural, _, _) = label.measure(gtk4::Orientation::Horizontal, -1);
        drop(label_guard);
        let natural = natural.max(1) as f64;
        let sweep = (natural > MAX_WIDTH).then(|| Sweep::new(MAX_WIDTH - natural, 0.0));
        *self.imp().sweep.borrow_mut() = sweep;
        self.queue_allocate();
        if sweep.is_some() {
            self.start_ticker();
        }
    }

    fn start_ticker(&self) {
        let obj = self.downgrade();
        let id = glib::timeout_add_local(Duration::from_millis(TICK_MS), move || {
            let Some(marquee) = obj.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let imp = marquee.imp();
            let moved = imp
                .sweep
                .borrow_mut()
                .as_mut()
                .is_some_and(|s| s.advance(TICK_MS as f64));
            if moved {
                marquee.queue_allocate();
            }
            glib::ControlFlow::Continue
        });
        *self.imp().tick.borrow_mut() = Some(id);
    }
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
