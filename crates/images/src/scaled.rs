use gdk4::subclass::paintable::PaintableImpl;
use gdk4::{Paintable, PaintableFlags, Snapshot, Texture};
use glib::prelude::*;
use glib::subclass::prelude::*;
use gtk4::prelude::SnapshotExt;
use std::cell::{Cell, RefCell};

mod paintable_imp {
    use super::*;
    use gdk4::prelude::TextureExt as _;

    pub struct ScaledPaintable {
        pub texture: RefCell<Option<Texture>>,
        pub width: Cell<i32>,
        pub height: Cell<i32>,
    }

    impl Default for ScaledPaintable {
        fn default() -> Self {
            Self {
                texture: RefCell::new(None),
                width: Cell::new(0),
                height: Cell::new(0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ScaledPaintable {
        const NAME: &'static str = "GseScaledPaintable";
        type Type = super::ScaledPaintable;
        type ParentType = glib::Object;
        type Interfaces = (Paintable,);
    }

    impl ObjectImpl for ScaledPaintable {}

    impl PaintableImpl for ScaledPaintable {
        fn flags(&self) -> PaintableFlags {
            PaintableFlags::STATIC_SIZE
        }

        fn intrinsic_width(&self) -> i32 {
            self.width.get()
        }

        fn intrinsic_height(&self) -> i32 {
            self.height.get()
        }

        fn intrinsic_aspect_ratio(&self) -> f64 {
            let w = self.width.get() as f64;
            let h = self.height.get() as f64;
            if h > 0.0 {
                w / h
            } else {
                0.0
            }
        }

        fn snapshot(&self, snapshot: &Snapshot, width: f64, height: f64) {
            let guard = self.texture.borrow();
            let Some(texture) = guard.as_ref() else {
                return;
            };
            let Some(snap) = snapshot.downcast_ref::<gtk4::Snapshot>() else {
                return;
            };
            // Cover, never stretch: scale the texture to fill the slot,
            // then center-crop the overflow (square art in a vertical
            // slot loses its left/right edges, never its proportions).
            let (dx, dy, dw, dh) = cover_rect(
                texture.width() as f64,
                texture.height() as f64,
                width,
                height,
            );
            snap.save();
            snap.push_clip(&gtk4::graphene::Rect::new(
                0.0,
                0.0,
                width as f32,
                height as f32,
            ));
            snap.translate(&gtk4::graphene::Point::new(dx as f32, dy as f32));
            snap.append_texture(
                texture,
                &gtk4::graphene::Rect::new(0.0, 0.0, dw as f32, dh as f32),
            );
            snap.pop();
            snap.restore();
        }

        fn current_image(&self) -> Paintable {
            let texture = self.texture.borrow().clone();
            let w = self.width.get();
            let h = self.height.get();
            match texture {
                Some(t) => super::ScaledPaintable::new(&t, w, h).upcast::<Paintable>(),
                None => glib::Object::new::<super::ScaledPaintable>().upcast::<Paintable>(),
            }
        }
    }
}

glib::wrapper! {
    pub struct ScaledPaintable(ObjectSubclass<paintable_imp::ScaledPaintable>)
        @implements gdk4::Paintable;
}

/// Cover-fit destination for a `tex_w`×`tex_h` texture in a `w`×`h`
/// slot: `(dx, dy, dw, dh)` to draw at, centered, aspect preserved.
/// Degenerate inputs fall back to the full slot so callers never divide
/// by zero.
fn cover_rect(tex_w: f64, tex_h: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
    if tex_w <= 0.0 || tex_h <= 0.0 || w <= 0.0 || h <= 0.0 {
        return (0.0, 0.0, w.max(0.0), h.max(0.0));
    }
    let scale = (w / tex_w).max(h / tex_h);
    let dw = tex_w * scale;
    let dh = tex_h * scale;
    ((w - dw) / 2.0, (h - dh) / 2.0, dw, dh)
}

impl ScaledPaintable {
    pub fn new(texture: &Texture, width: i32, height: i32) -> Self {        let obj = glib::Object::new::<Self>();
        obj.imp().texture.replace(Some(texture.clone()));
        obj.imp().width.set(width);
        obj.imp().height.set(height);
        obj
    }

    pub fn new_empty(width: i32, height: i32) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp().texture.replace(None);
        obj.imp().width.set(width);
        obj.imp().height.set(height);
        obj
    }
}

#[cfg(test)]
mod tests {
    use super::cover_rect;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn test_cover_rect_exact_fit_is_identity() {
        assert_eq!(cover_rect(100.0, 200.0, 100.0, 200.0), (0.0, 0.0, 100.0, 200.0));
    }

    #[test]
    fn test_cover_rect_crops_square_into_vertical() {
        // 1:1 art in a 2:3 slot scales by height, spills left/right.
        let (dx, dy, dw, dh) = cover_rect(100.0, 100.0, 200.0, 300.0);
        assert!(close(dw, 300.0));
        assert!(close(dh, 300.0));
        assert!(close(dx, -50.0));
        assert!(close(dy, 0.0));
    }

    #[test]
    fn test_cover_rect_crops_wide_into_narrow() {
        // 2:1 art in a 1:1 slot scales by width, spills top/bottom.
        let (dx, dy, dw, dh) = cover_rect(200.0, 100.0, 100.0, 100.0);
        assert!(close(dw, 200.0));
        assert!(close(dh, 100.0));
        assert!(close(dx, -50.0));
        assert!(close(dy, 0.0));
    }

    #[test]
    fn test_cover_rect_degenerate_falls_back_to_slot() {
        assert_eq!(cover_rect(0.0, 100.0, 50.0, 60.0), (0.0, 0.0, 50.0, 60.0));
        assert_eq!(cover_rect(100.0, 100.0, 0.0, 0.0), (0.0, 0.0, 0.0, 0.0));
    }
}
