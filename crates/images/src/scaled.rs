use gdk4::subclass::paintable::PaintableImpl;
use gdk4::{Paintable, PaintableFlags, Snapshot, Texture};
use glib::prelude::*;
use glib::subclass::prelude::*;
use gtk4::prelude::SnapshotExt;
use std::cell::{Cell, RefCell};

mod paintable_imp {
    use super::*;

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
            if h > 0.0 { w / h } else { 0.0 }
        }

        fn snapshot(&self, snapshot: &Snapshot, width: f64, height: f64) {
            if let Some(texture) = self.texture.borrow().as_ref() {
                if let Some(snap) = snapshot.downcast_ref::<gtk4::Snapshot>() {
                    let rect = gtk4::graphene::Rect::new(
                        0.0,
                        0.0,
                        width as f32,
                        height as f32,
                    );
                    snap.append_texture(texture, &rect);
                }
            }
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

impl ScaledPaintable {
    pub fn new(texture: &Texture, width: i32, height: i32) -> Self {
        let obj = glib::Object::new::<Self>();
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
