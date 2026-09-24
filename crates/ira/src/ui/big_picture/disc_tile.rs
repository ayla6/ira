//! The big-picture disc art tile: a fixed square that paints its texture
//! GPU-scaled to fit, instead of a `Picture` whose texture natural size
//! would grow the tile past its request. The size is set once from the
//! viewport; swapping the texture repaints without ever relayouting,
//! so the ring parks once and the row never reflows.

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};

/// Fit a `tw`×`th` texture inside a `vw`×`vh` tile, preserving aspect:
/// the draw rect, centered.
fn contain_rect(vw: f32, vh: f32, tw: f32, th: f32) -> (f32, f32, f32, f32) {
    let scale = (vw / tw).min(vh / th);
    let (dw, dh) = (tw * scale, th * scale);
    ((vw - dw) / 2.0, (vh - dh) / 2.0, dw, dh)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DiscArtTile {
        pub size: Cell<i32>,
        pub texture: RefCell<Option<gtk4::gdk::Texture>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DiscArtTile {
        const NAME: &'static str = "IraBpDiscArtTile";
        type Type = super::DiscArtTile;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for DiscArtTile {}

    impl WidgetImpl for DiscArtTile {
        // Fixed both ways: the tile never measures its texture, so art
        // swaps and resolution changes cannot move the row.
        fn measure(
            &self,
            _orientation: gtk4::Orientation,
            _for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let size = self.size.get().max(1);
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let Some(texture) = self.texture.borrow().clone() else {
                return;
            };
            let obj = self.obj();
            let (x, y, w, h) = super::contain_rect(
                obj.width() as f32,
                obj.height() as f32,
                texture.width() as f32,
                texture.height() as f32,
            );
            snapshot.append_texture(
                &texture,
                &gtk4::graphene::Rect::new(x, y, w, h),
            );
        }
    }
}

glib::wrapper! {
    pub struct DiscArtTile(ObjectSubclass<imp::DiscArtTile>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl DiscArtTile {
    pub fn with_size(size: i32) -> Self {
        let this: Self = glib::Object::new();
        this.imp().size.set(size);
        this
    }

    /// Swap the painted texture. A repaint, never a relayout.
    pub fn set_texture(&self, texture: Option<gtk4::gdk::Texture>) {
        self.imp().texture.replace(texture);
        self.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::contain_rect;

    #[test]
    fn test_contain_rect_fits_wide_texture() {
        assert_eq!(contain_rect(300.0, 300.0, 600.0, 300.0), (0.0, 75.0, 300.0, 150.0));
    }

    #[test]
    fn test_contain_rect_centers_square_texture() {
        assert_eq!(contain_rect(300.0, 300.0, 100.0, 100.0), (0.0, 0.0, 300.0, 300.0));
    }

    #[test]
    fn test_contain_rect_fits_tall_texture() {
        assert_eq!(contain_rect(300.0, 300.0, 150.0, 600.0), (112.5, 0.0, 75.0, 300.0));
    }
}
