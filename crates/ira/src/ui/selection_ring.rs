//! The couch selection ring: a drawn frame that floats 4px outside the
//! selected tile. Square frames keep their INNER edge straight and round
//! only the outside corners a touch; the round variant (the All Software
//! circle) is a full circle. Drawn as the fill between an outer shape
//! and an inner cutout (not a stroke, not CSS) so the two edges can
//! differ and it never affects layout.

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::Cell;

/// Frame thickness, gap to the tile, and the outside corner rounding,
/// in reference (1080p) pixels; the caller scales them with the viewport.
const LINE: f64 = 4.0;
const OFFSET: f64 = 4.0;
const OUTER_RADIUS: f64 = 2.0;

/// Frame metrics scaled from the reference and kept whole-pixel, so the
/// frame's straight edges land exactly on the pixel grid.
fn frame_metrics(scale: f64) -> (f64, f64, f64) {
    (
        (LINE * scale).round().max(2.0),
        (OFFSET * scale).round().max(1.0),
        (OUTER_RADIUS * scale).round(),
    )
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SelectionRing {
        /// The tile's rect in the parent's coordinates.
        pub x: Cell<f64>,
        pub y: Cell<f64>,
        pub w: Cell<f64>,
        pub h: Cell<f64>,
        /// Viewport scale (1.0 = 1080p reference) for line and radius.
        pub scale: Cell<f64>,
        /// Draw a circle instead of a square frame.
        pub round: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SelectionRing {
        const NAME: &'static str = "IraBpSelectionRing";
        type Type = super::SelectionRing;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for SelectionRing {
        fn dispose(&self) {}
    }

    impl WidgetImpl for SelectionRing {
        fn measure(&self, _o: gtk4::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            // Scale 0 means the caller has no viewport yet (page still
            // hidden): painting then would draw a hairline frame in a
            // stale spot — the thing this widget exists to avoid.
            let scale = self.scale.get();
            if scale <= 0.0 {
                return;
            }
            let (line, gap, radius) = frame_metrics(scale);
            let grow = gap + line;
            let x = self.x.get();
            let y = self.y.get();
            let w = self.w.get();
            let h = self.h.get();
            let outer = (
                (x - grow).round(),
                (y - grow).round(),
                (w + 2.0 * grow).round(),
                (h + 2.0 * grow).round(),
            );
            if outer.2 < line * 2.0 || outer.3 < line * 2.0 {
                return;
            }
            let (ox, oy, ow, oh) = outer;
            let bounds =
                gtk4::graphene::Rect::new(ox as f32, oy as f32, ow as f32, oh as f32);
            let ctx = snapshot.append_cairo(&bounds);
            if self.round.get() {
                let cx = ox + ow / 2.0;
                let cy = oy + oh / 2.0;
                let r_out = ow.min(oh) / 2.0;
                let r_in = (w.min(h) / 2.0 + gap).min(r_out - line);
                ctx.arc(cx, cy, r_out, 0.0, std::f64::consts::TAU);
                ctx.close_path();
                ctx.arc(cx, cy, r_in, 0.0, std::f64::consts::TAU);
                ctx.close_path();
            } else {
                // The outside corners carry the only rounding; the inner
                // cutout is straight-edged.
                rounded_rect(&ctx, ox, oy, ow, oh, radius);
                ctx.rectangle(
                    (x - gap).round(),
                    (y - gap).round(),
                    (w + 2.0 * gap).round(),
                    (h + 2.0 * gap).round(),
                );
            }
            // The accent comes from the theme through the widget's color
            // (the .bp-ring class sets it), so theme variants work.
            let color = self.obj().color();
            ctx.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                1.0,
            );
            // Even-odd turns the inner shape into a hole regardless of
            // the two paths' winding, so the fill cannot double-cover.
            ctx.set_fill_rule(gtk4::cairo::FillRule::EvenOdd);
            ctx.fill().expect("ring fill");
        }
    }
}

/// Traces a rounded rectangle into `cr`.
fn rounded_rect(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, -std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

glib::wrapper! {
    pub struct SelectionRing(ObjectSubclass<imp::SelectionRing>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl SelectionRing {
    pub fn new() -> Self {
        let this: Self = glib::Object::new();
        // Halign/valign Fill would stretch us; the rect is explicit.
        this.set_halign(gtk4::Align::Fill);
        this.set_valign(gtk4::Align::Fill);
        this.set_overflow(gtk4::Overflow::Visible);
        this.add_css_class("bp-ring");
        this
    }

    /// Place the frame over a tile: `x`/`y` is the tile's top-left in
    /// the parent's coordinates, `w`/`h` its size. The frame grows
    /// outwards; `round` draws a circle instead of a square frame.
    pub fn place(&self, x: f64, y: f64, w: f64, h: f64, scale: f64, round: bool) {
        let imp = self.imp();
        let moved = (imp.x.get() - x).abs() > 0.5
            || (imp.y.get() - y).abs() > 0.5
            || (imp.w.get() - w).abs() > 0.5
            || (imp.h.get() - h).abs() > 0.5
            || (imp.scale.get() - scale).abs() > 0.01
            || imp.round.get() != round;
        if moved {
            imp.x.set(x);
            imp.y.set(y);
            imp.w.set(w);
            imp.h.set(h);
            imp.scale.set(scale);
            imp.round.set(round);
            self.queue_allocate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::frame_metrics;

    #[test]
    fn test_frame_metrics_reference() {
        assert_eq!(frame_metrics(1.0), (4.0, 4.0, 2.0));
    }

    #[test]
    fn test_frame_metrics_scales_whole_pixel() {
        // 720p: 4 × 0.661 = 2.65 rounds to 3, radius 1.32 to 1.
        assert_eq!(frame_metrics(0.6614583333333334), (3.0, 3.0, 1.0));
    }

    #[test]
    fn test_frame_metrics_floors_survive_tiny_scales() {
        let (line, gap, radius) = frame_metrics(0.05);
        assert_eq!((line, gap, radius), (2.0, 1.0, 0.0));
    }
}
