//! A floating tooltip bubble for the big-picture screens, drawn as one shape in
//! a single `snapshot()` pass: a rounded pill joined with a tail that
//! reaches the selected tile (Cairo), with the title repeating after a
//! gap — "name    name" — always drifting right and wrapping around when
//! it overflows. The bubble and its tail are one Cairo sub-path union
//! (same winding, single fill), so they cannot come apart or
//! double-composite. The widget spans its rail and positions the bubble
//! internally; moving it only queues a redraw.

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::time::Duration;

/// Drift speed of the wrapping title, and how often it moves.
const SPEED: f64 = 42.0;
const TICK_MS: u64 = 16;
/// Silence between one copy of the name and the next.
const GAP: f64 = 56.0;
/// The bubble's padding, corner rounding and tail shape. The tail is two
/// pixels taller than the gap the callers keep (10px), so its tip lands
/// ON the tile instead of floating short of it.
const PAD_X: i32 = 14;
const PAD_Y: i32 = 6;
const BUBBLE_RADIUS: f64 = 9.0;
/// The tail IS the distance: its tip is pinned to the tile edge and the
/// bubble hangs tail-length away, so the pill-to-tile distance is this
/// constant on every page, at every font size. Reference (1080p) pixels.
const TAIL_HEIGHT: f64 = 9.0;

/// [`TAIL_HEIGHT`] at the current viewport scale, for callers that float
/// the pill relative to the ring.
pub(super) fn scaled_tail_height() -> f64 {
    s(TAIL_HEIGHT)
}
const TAIL_HALF: f64 = 16.0;
/// Elevated popover surface: clearly lighter than the page background so
/// the bubble reads as its own object, dark enough for the accent title.
const BUBBLE_RGBA: (f64, f64, f64, f64) = (0.23, 0.23, 0.25, 0.97);
/// How deep the tail's base sits inside the pill, so the two shapes
/// overlap well past their junction and antialiasing cannot shave a line
/// where they meet.
const TAIL_BASE_OVERLAP: f64 = 4.0;
/// Keep the bubble this far inside the viewport's edges.
const EDGE_MARGIN: f64 = 16.0;

/// Reference (1080p) pixels at the current viewport scale. The pill's
/// chrome keeps proportion with the fonts it names, whose sizes the
/// stylesheet scales the same way.
fn s(px: f64) -> f64 {
    px * crate::ui::css::bp_scale()
}

/// Advances the drift phase by one tick, wrapping at `period` (one name
/// plus the gap) so the repeat loops seamlessly.
fn advance_slide(slide: f64, period: f64, dt_ms: f64) -> f64 {
    if period <= 0.0 {
        return 0.0;
    }
    (slide + s(SPEED) * dt_ms / 1000.0) % period
}

/// A clipping window for the drifting title. Two copies of the label ride
/// inside it, one period apart. A plain `GtkBox` cannot serve: its own
/// layout re-allocates children on every allocation pass, overwriting the
/// slide — this subclass allocates its children exactly as told, and its
/// overflow clips them to the bubble's inner width.
mod clip {
    use super::*;

    #[derive(Default)]
    pub struct Clip {
        pub a: RefCell<Option<gtk4::Widget>>,
        pub b: RefCell<Option<gtk4::Widget>>,
        /// Where copy A sits; copy B trails one `period` to the left.
        pub base_x: Cell<i32>,
        pub period: Cell<i32>,
        /// How far the copies have drifted left; applied at paint time, so
        /// a tick costs a redraw — never a relayout that re-measures the
        /// text through Pango 60 times a second.
        pub drift: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Clip {
        const NAME: &'static str = "IraBpMarqueeClip";
        type Type = super::Clip;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for Clip {
        fn dispose(&self) {
            for child in [self.a.borrow().as_ref(), self.b.borrow().as_ref()]
                .into_iter()
                .flatten()
            {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Clip {
        // The clip only bounds the marquee's drawing; picking must fall
        // through so the covers underneath get the mouse.
        fn contains(&self, _x: f64, _y: f64) -> bool {
            false
        }

        fn measure(
            &self,
            orientation: gtk4::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            match self.a.borrow().as_ref() {
                Some(child) => child.measure(orientation, for_size),
                None => (0, 0, -1, -1),
            }
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let drift = self.drift.get();
            if drift != 0 {
                // Whole pixels only: fractional offsets re-phase glyph
                // rasterization and text randomly draws a pixel short as
                // it moves.
                snapshot.save();
                snapshot.translate(&gtk4::graphene::Point::new(-(drift as f32), 0.0));
            }
            for child in [self.a.borrow().clone(), self.b.borrow().clone()]
                .into_iter()
                .flatten()
            {
                self.obj().snapshot_child(&child, snapshot);
            }
            if drift != 0 {
                snapshot.restore();
            }
        }

        fn size_allocate(&self, _width: i32, height: i32, _baseline: i32) {
            let guard = self.a.borrow();
            let Some(child) = guard.as_ref() else {
                return;
            };
            let (_, natural_w, _, _) = child.measure(gtk4::Orientation::Horizontal, -1);
            let (_, natural_h, _, _) = child.measure(gtk4::Orientation::Vertical, -1);
            let y = ((height - natural_h) / 2).max(0) as f32;
            let x = self.base_x.get() as f32;
            let transform = |px: f32| {
                gtk4::gsk::Transform::new().translate(&gtk4::graphene::Point::new(px, y))
            };
            child.allocate(natural_w.max(1), natural_h.max(1), height, Some(transform(x)));
            drop(guard);
            if let Some(copy) = self.b.borrow().as_ref() {
                copy.allocate(
                    natural_w.max(1),
                    natural_h.max(1),
                    height,
                    Some(transform(x + self.period.get() as f32)),
                );
            }
        }
    }
}

glib::wrapper! {
    pub struct Clip(ObjectSubclass<clip::Clip>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

/// The bubble and its tail, in the widget's coordinate space.
struct Bubble {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    apex_x: f64,
    /// Tail on the top edge pointing up (bubble below its tile) or on the
    /// bottom edge pointing down (bubble above its tile).
    tail_up: bool,
}

impl Bubble {
    /// The tail triangle: base well inside the bubble (so the single-fill
    /// union has no shaved junction) and its apex reaching the tile. The
    /// points are ordered so the triangle winds the SAME circular
    /// direction as the pill's arcs in both tail orientations — Cairo
    /// fills with the non-zero rule, so opposite winding makes the
    /// overlap net to zero and punches it fully transparent (an
    /// "inversion" where the tail meets the pill).
    fn tail(&self, tail_half: f64, tail_height: f64) -> [(f64, f64); 3] {
        let (base_y, apex_y) = if self.tail_up {
            (self.y + s(TAIL_BASE_OVERLAP), self.y - tail_height)
        } else {
            (self.y + self.h - s(TAIL_BASE_OVERLAP), self.y + self.h + tail_height)
        };
        if self.tail_up {
            // Apex first: clockwise, like the pill.
            [
                (self.apex_x, apex_y),
                (self.apex_x + tail_half, base_y),
                (self.apex_x - tail_half, base_y),
            ]
        } else {
            // Base-left, base-right, apex: clockwise, like the pill.
            [
                (self.apex_x - tail_half, base_y),
                (self.apex_x + tail_half, base_y),
                (self.apex_x, apex_y),
            ]
        }
    }
}

/// Traces the bubble into `cr`: a rounded pill joined with its tail, as
/// one sub-path union filled in a single pass — no seams, no
/// double-composited alpha.
fn bubble_path(
    cr: &gtk4::cairo::Context,
    bubble: &Bubble,
    radius: f64,
    tail_half: f64,
    tail_height: f64,
) {
    let (x, y, w, h) = (bubble.x, bubble.y, bubble.w, bubble.h);
    let r = radius.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, -std::f64::consts::FRAC_PI_2);
    cr.close_path();
    let [(ax, ay), (bx, by), (cx, cy)] = bubble.tail(tail_half, tail_height);
    cr.move_to(ax, ay);
    cr.line_to(bx, by);
    cr.line_to(cx, cy);
    cr.close_path();
}

mod imp {
    use super::*;

    pub struct Marquee {
        /// The pill: a transparent clip window the two title copies
        /// drift inside.
        pub clip: RefCell<Option<Clip>>,
        pub label: RefCell<Option<gtk4::Label>>,
        pub label2: RefCell<Option<gtk4::Label>>,
        /// The bubble's geometry from the last allocation, for painting:
        /// (x, y, width, height) plus the tail's apex x.
        pub bubble: Cell<(f64, f64, f64, f64)>,
        pub tail_apex: Cell<f64>,
        /// Where the bubble centers (the selected tile), and the width it
        /// must stay inside.
        pub center_x: Cell<f64>,
        pub viewport: Cell<f64>,
        /// The tail tip's absolute y (pinned to the tile edge) and which
        /// edge carries the tail. The bubble hangs off the tip — its own
        /// height never moves the touch point.
        pub tip_y: Cell<f64>,
        pub tail_up: Cell<bool>,
        /// Slide phase of copy A within [0, period); 0 keeps the name
        /// readable at the bubble's left padding.
        pub(super) slide: Cell<f64>,
        /// One name plus the gap — how far a copy travels per loop.
        pub(super) period: Cell<f64>,
        /// True while the text overflows and the drift ticker runs.
        pub(super) sliding: Cell<bool>,
        pub rail_height: Cell<i32>,
        pub max_width: Cell<f64>,
        pub tick: RefCell<Option<glib::SourceId>>,
    }

    impl Default for Marquee {
        fn default() -> Self {
            Self {
                clip: RefCell::new(None),
                label: RefCell::new(None),
                label2: RefCell::new(None),
                bubble: Cell::new((0.0, 0.0, 0.0, 0.0)),
                tail_apex: Cell::new(0.0),
                center_x: Cell::new(0.0),
                viewport: Cell::new(0.0),
                tip_y: Cell::new(0.0),
                tail_up: Cell::new(false),
                slide: Cell::new(0.0),
                period: Cell::new(0.0),
                sliding: Cell::new(false),
                rail_height: Cell::new(56),
                max_width: Cell::new(400.0),
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
            if let Some(clip) = self.clip.borrow().as_ref() {
                clip.unparent();
            }
        }
    }

    impl WidgetImpl for Marquee {
        // The marquee spans the whole page as a drawing surface but is
        // never interactive: picking must fall through to the covers, or
        // the float eats every click, motion, and scroll event.
        fn contains(&self, _x: f64, _y: f64) -> bool {
            false
        }

        /// Width never reports the bubble, so a traveling title can't push
        /// the rest of the screen around; height is the rail strip.
        fn measure(&self, orientation: gtk4::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk4::Orientation::Horizontal {
                (0, self.max_width.get() as i32, -1, -1)
            } else {
                let h = self.rail_height.get();
                (h, h, -1, -1)
            }
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            // No text, no pill: before the library loads the marquee would
            // otherwise spawn as an empty bubble floating over nothing.
            let has_text = self
                .label
                .borrow()
                .as_ref()
                .is_some_and(|label| !label.text().is_empty());
            if !has_text {
                return;
            }
            let (bx, by, bw, bh) = self.bubble.get();
            if bw > 1.0 && bh > 1.0 {
                let bounds = gtk4::graphene::Rect::new(
                    0.0,
                    0.0,
                    self.obj().width() as f32,
                    self.obj().height() as f32,
                );
                let ctx = snapshot.append_cairo(&bounds);
                bubble_path(
                    &ctx,
                    &Bubble {
                        x: bx,
                        y: by,
                        w: bw,
                        h: bh,
                        apex_x: self.tail_apex.get(),
                        tail_up: self.tail_up.get(),
                    },
                    s(BUBBLE_RADIUS),
                    s(TAIL_HALF),
                    s(TAIL_HEIGHT),
                );
                // Adwaita's elevated popover surface.
                let (r, g, b, a) = BUBBLE_RGBA;
                ctx.set_source_rgba(r, g, b, a);
                ctx.fill().expect("bubble fill");
            }
            if let Some(clip) = self.clip.borrow().as_ref() {
                self.obj().snapshot_child(clip.upcast_ref::<gtk4::Widget>(), snapshot);
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let label_guard = self.label.borrow();
            let Some(label) = label_guard.as_ref() else {
                return;
            };
            let (_, natural_w, _, _) = label.measure(gtk4::Orientation::Horizontal, -1);
            let natural_w = natural_w as f64;
            let max_width = self.max_width.get();
            // A caller may position the bubble before passing a real
            // viewport (first open of a page); this allocation is the
            // truth at paint time — the widget's own width() here would
            // still report the previous pass.
            let viewport = {
                let stored = self.viewport.get();
                if stored > 1.0 { stored } else { width as f64 }
            };
            let center = self.center_x.get();
            let margin = s(EDGE_MARGIN);

            // The drift can only engage once the label carries its final
            // style — measuring at set_text time races the CSS font size.
            let overflowing = natural_w + 2.0 * s(PAD_X as f64) > max_width;
            if overflowing && !self.sliding.get() {
                self.sliding.set(true);
                self.slide.set(0.0);
                self.start_ticker();
            } else if !overflowing {
                self.sliding.set(false);
            }
            self.period.set(natural_w + s(GAP));

            // The bubble hugs its text and only caps at max_width when the
            // text overflows (that cap is the drift's clip window). It
            // centers on the tile, clamped to stay on screen.
            let bubble_w = (natural_w + 2.0 * s(PAD_X as f64)).min(max_width);
            let bubble_x = (center - bubble_w / 2.0)
                .clamp(margin, (viewport - bubble_w - margin).max(margin));
            let bubble_h = label.measure(gtk4::Orientation::Vertical, -1).1 as f64
                + 2.0 * s(PAD_Y as f64)
                + s(4.0);
            // The TIP is the anchor: it is pinned to the tile edge by the
            // caller, and the bubble hangs off it — its own measured
            // height can drift with fonts without ever moving the touch
            // point. Tail up: the bubble sits below the tip; tail down:
            // above it.
            let bubble_h = bubble_h.round() as i32;
            let bubble_y = if self.tail_up.get() {
                (self.tip_y.get() + s(TAIL_HEIGHT)) as i32
            } else {
                (self.tip_y.get() - s(TAIL_HEIGHT)) as i32 - bubble_h
            };
            self.bubble
                .set((bubble_x, bubble_y as f64, bubble_w, bubble_h as f64));
            // The tail points at the tile, kept within the bubble, drawn
            // in the widget's ABSOLUTE coordinates (snapshot paints the
            // bubble at its stored position, so a local apex here would
            // land near the screen's left edge — visible only on the
            // first tile, whose bubble happens to sit at x≈margin). A
            // bubble narrower than the tail's bounds clamps to its center
            // instead of an inverted range (clamp aborts on min > max).
            let tail_lo = s(TAIL_HALF + 6.0);
            let tail_hi = (bubble_w - s(TAIL_HALF + 6.0)).max(tail_lo);
            self.tail_apex
                .set(bubble_x + (center - bubble_x).clamp(tail_lo, tail_hi));

            if let Some(clip) = self.clip.borrow().as_ref() {
                // Drifting text starts at the readable left padding and
                // slides left at paint time; the next copy follows from
                // the right. Fitting text centers.
                let text_x = if self.sliding.get() {
                    s(PAD_X as f64) as i32
                } else {
                    ((bubble_w - natural_w) / 2.0).round() as i32
                };
                clip.imp().base_x.set(text_x);
                if !self.sliding.get() {
                    clip.imp().drift.set(0);
                }
                // Copy B follows one period behind, entering from the
                // right as copy A exits left — the classic ticker.
                clip.imp().period.set(self.period.get().round() as i32);
                let tx = gtk4::gsk::Transform::new()
                    .translate(&gtk4::graphene::Point::new(
                        bubble_x.round() as f32,
                        bubble_y as f32,
                    ));
                clip.allocate(bubble_w as i32, bubble_h, -1, Some(tx));
            }
        }
    }

    impl Marquee {
        /// Drive the drift while it runs. A tick only advances the phase —
        /// moving the text is a redraw of the clip, not a relayout of the
        /// marquee.
        pub(super) fn start_ticker(&self) {
            if self.tick.borrow().is_some() {
                return;
            }
            let obj = self.obj().downgrade();
            let id = glib::timeout_add_local(Duration::from_millis(TICK_MS), move || {
                let Some(marquee) = obj.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let imp = marquee.imp();
                if !imp.sliding.get() {
                    *imp.tick.borrow_mut() = None;
                    return glib::ControlFlow::Break;
                }
                let period = imp.period.get();
                if period > 1.0 {
                    imp.slide.set(advance_slide(imp.slide.get(), period, TICK_MS as f64));
                    if let Some(clip) = imp.clip.borrow().as_ref() {
                        // Only the copies drift: push the new offset into
                        // the clip and repaint it — a relayout here would
                        // re-measure the text through Pango every frame.
                        clip.imp().drift.set(imp.slide.get().round() as i32);
                        clip.queue_draw();
                    } else {
                        marquee.queue_allocate();
                    }
                }
                glib::ControlFlow::Continue
            });
            *self.tick.borrow_mut() = Some(id);
        }
    }
}

glib::wrapper! {
    pub struct Marquee(ObjectSubclass<imp::Marquee>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl Marquee {
    /// `rail_height` is the widget's own height when it rides a layout
    /// (the home title rail); a free-floating overlay fills its parent and
    /// positions the bubble at explicit coordinates. Text beyond
    /// `max_width` repeats with a gap and drifts right, wrapping around.
    pub(super) fn new(rail_height: i32, max_width: f64) -> Self {
        let this: Self = glib::Object::new();
        let imp = this.imp();
        imp.rail_height.set(rail_height);
        imp.max_width.set(max_width);

        let clip: Clip = glib::Object::new();
        clip.set_overflow(gtk4::Overflow::Hidden);
        for slot in [&imp.label, &imp.label2] {
            let label = gtk4::Label::new(None);
            label.add_css_class(crate::ui::css::CSS_BP_TITLE);
            crate::ui::helpers::crisp_label(&label);
            label.set_parent(&clip);
            *slot.borrow_mut() = Some(label);
        }
        let label = imp.label.borrow().as_ref().unwrap().clone();
        *clip.imp().a.borrow_mut() = Some(label.upcast::<gtk4::Widget>());
        let label2 = imp.label2.borrow().as_ref().unwrap().clone();
        *clip.imp().b.borrow_mut() = Some(label2.upcast::<gtk4::Widget>());
        clip.set_parent(&this);
        *imp.clip.borrow_mut() = Some(clip);
        this
    }

    pub(super) fn widget(&self) -> &gtk4::Widget {
        self.upcast_ref()
    }

    /// Change the marquee's clip width (e.g. the grid following its item
    /// size); the drift re-sizes itself on the next pass.
    pub(super) fn set_max_width(&self, max_width: f64) {
        let imp = self.imp();
        if (imp.max_width.get() - max_width).abs() > 0.5 {
            imp.max_width.set(max_width);
            imp.slide.set(0.0);
            self.queue_allocate();
        }
    }

    /// Center the bubble on `center_x`, kept fully on screen within
    /// `viewport`. `tip_y` is where the tail's apex lands (pinned to the
    /// tile edge — the bubble hangs off it, so text/font changes never
    /// move the touch point), and `tail_up` picks which edge carries the
    /// tail: true hangs the bubble below the tip, false above it.
    pub(super) fn set_position(&self, center_x: f64, viewport: f64, tip_y: f64, tail_up: bool) {
        let imp = self.imp();
        if (imp.center_x.get() - center_x).abs() > 0.5
            || (imp.viewport.get() - viewport).abs() > 0.5
            || (imp.tip_y.get() - tip_y).abs() > 0.5
            || imp.tail_up.get() != tail_up
        {
            imp.center_x.set(center_x);
            imp.viewport.set(viewport);
            imp.tip_y.set(tip_y);
            imp.tail_up.set(tail_up);
            self.queue_allocate();
        }
    }

    /// The bubble's rendered height, for callers that float it above a tile.
    pub(super) fn pill_height(&self) -> i32 {
        self.imp()
            .label
            .borrow()
            .as_ref()
            .map(|label| {
                (label.measure(gtk4::Orientation::Vertical, -1).1 as f64
                    + 2.0 * s(PAD_Y as f64)
                    + s(4.0)) as i32
            })
            .unwrap_or(0)
    }

    /// Swap the text on both copies; the drift re-sizes itself on the next
    /// allocation, when the label carries its final style. Unchanged text
    /// is a no-op — callers re-run this on every scroll tick.
    pub(super) fn set_text(&self, text: &str) {
        let imp = self.imp();
        let label = imp.label.borrow();
        let label2 = imp.label2.borrow();
        let unchanged = label.as_ref().is_some_and(|l| l.text() == text)
            && label2.as_ref().is_some_and(|l| l.text() == text);
        if unchanged {
            return;
        }
        if let (Some(a), Some(b)) = (label.as_ref(), label2.as_ref()) {
            a.set_text(text);
            b.set_text(text);
        }
        drop(label);
        drop(label2);
        imp.slide.set(0.0);
        self.queue_allocate();
        // One post-layout nudge: the first allocation can land before the
        // label's style settles, measuring the text too narrow and never
        // engaging the drift. After this re-run the measurement is final.
        let obj = self.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(marquee) = obj.upgrade() {
                marquee.queue_allocate();
            }
        });
    }

    /// Re-create the label layouts from the current style. The big-picture CSS
    /// reloads with new font sizes on a viewport resize, but a label
    /// whose text was set while its page sat hidden keeps measuring its
    /// stale Pango layout — and `set_text`'s unchanged-guard skips
    /// exactly that refresh. Cycling the text forces a fresh layout; it
    /// runs one layout cycle later, because a label that becomes visible
    /// this cycle has not settled its new style yet.
    pub(super) fn revalidate_text(&self) {
        let text = self
            .imp()
            .label
            .borrow()
            .as_ref()
            .map(|label| label.text())
            .unwrap_or_default();
        if text.is_empty() {
            return;
        }
        let obj = self.downgrade();
        glib::idle_add_local_once(move || {
            let Some(marquee) = obj.upgrade() else {
                return;
            };
            // Toggling the class re-matches the label's style node against
            // the provider's CURRENT contents: a reload that happened while
            // this page sat hidden never invalidated the node, so its
            // computed font stayed at whatever era it was last resolved.
            let imp = marquee.imp();
            for slot in [&imp.label, &imp.label2] {
                let label = slot.borrow();
                if let Some(label) = label.as_ref() {
                    label.remove_css_class(crate::ui::css::CSS_BP_TITLE);
                    label.add_css_class(crate::ui::css::CSS_BP_TITLE);
                }
            }
            marquee.set_text("");
            marquee.set_text(&text);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{advance_slide, Bubble, SPEED};

    #[test]
    fn test_advance_slide_advances_and_wraps() {
        let period = 100.0;
        let one_sec = SPEED; // px travelled per 1000 ms
        let s = advance_slide(0.0, period, 1000.0);
        assert!((s - one_sec % period).abs() < 1e-9, "drifts right by SPEED");
        let wrapped = advance_slide(99.0, period, 1000.0);
        assert!(wrapped >= 0.0 && wrapped < period, "wraps at the period");
        assert!((wrapped - (99.0 + one_sec) % period).abs() < 1e-9);
    }

    #[test]
    fn test_advance_slide_zero_period_never_moves() {
        assert_eq!(advance_slide(10.0, 0.0, 1000.0), 0.0);
        assert_eq!(advance_slide(10.0, -5.0, 1000.0), 0.0);
    }

    #[test]
    fn test_tail_points_at_the_tile_from_either_side() {
        let mut bubble = Bubble {
            x: 100.0,
            y: 40.0,
            w: 300.0,
            h: 32.0,
            apex_x: 150.0,
            tail_up: false,
        };
        // Above the tile: base on the pill's bottom edge, apex further
        // down, pointing at the tile beneath. Base-left, base-right,
        // apex — clockwise, same as the pill's arcs.
        let [(lx, ly), (rx, ry), (ax, ay)] = bubble.tail(11.0, 9.0);
        assert_eq!(ly, 68.0);
        assert_eq!(ry, 68.0);
        assert_eq!(ax, 150.0);
        assert_eq!(ay, 81.0);
        assert!(lx < 150.0 && rx > 150.0);

        // Below the tile: base on the top edge, apex above. Apex first —
        // the ordering that keeps the winding clockwise here; the
        // base-first order would reverse it and punch the overlap
        // transparent.
        bubble.tail_up = true;
        let [(ax, ay), (rx, ry), (lx, ly)] = bubble.tail(11.0, 9.0);
        assert_eq!(ay, 31.0);
        assert_eq!(ax, 150.0);
        assert_eq!(ry, 44.0);
        assert_eq!(ly, 44.0);
        assert!(lx < 150.0 && rx > 150.0);
    }
}
