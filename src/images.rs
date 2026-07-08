use gdk4::subclass::paintable::PaintableImpl;
use gdk4::{Paintable, PaintableFlags, Snapshot, Texture};
use glib::prelude::*;
use glib::subclass::prelude::*;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

thread_local! {
    static TEXTURE_CACHE: RefCell<HashMap<String, Texture>> = RefCell::new(HashMap::new());
}

pub fn texture_for(path: &str) -> Option<Texture> {
    if path.is_empty() {
        return None;
    }
    TEXTURE_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some(t) = cache.get(path) {
            return Some(t.clone());
        }
        match Texture::from_filename(path) {
            Ok(t) => {
                let cloned = t.clone();
                cache.insert(path.to_string(), t);
                Some(cloned)
            }
            Err(_) => None,
        }
    })
}

pub fn set_image(img: &gtk4::Image, path: &str) {
    if let Some(t) = texture_for(path) {
        img.set_paintable(Some(&t));
    }
}

pub fn set_picture(pic: &gtk4::Picture, path: &str) {
    if let Some(t) = texture_for(path) {
        pic.set_paintable(Some(&t));
    }
}

/// Set a Picture's paintable from file, reporting a custom intrinsic size
/// so that GridView (which uses natural size for row-height calculation)
/// gets the correct dimensions without pre-scaling the image data.
pub fn set_picture_natural(pic: &gtk4::Picture, path: &str, w: i32, h: i32) {
    if w <= 0 || h <= 0 || path.is_empty() {
        return;
    }
    if let Some(t) = texture_for(path) {
        let paintable = ScaledPaintable::new(&t, w, h);
        pic.set_paintable(Some(&paintable));
    }
}

/// Load an image, scale it to exactly `w × h` (cover-style, centre crop), and
/// set it on the Picture.  This makes the Picture's natural size equal the
/// target dimensions, so containers like FlowBox respect the desired size
/// instead of the source image's intrinsic resolution.
pub fn set_picture_scaled(pic: &gtk4::Picture, path: &str, w: i32, h: i32) {
    if w <= 0 || h <= 0 || path.is_empty() {
        return;
    }
    let Ok(pixbuf) = gtk4::gdk_pixbuf::Pixbuf::from_file(path) else {
        return;
    };
    let src_w = pixbuf.width();
    let src_h = pixbuf.height();
    if src_w <= 0 || src_h <= 0 {
        return;
    }
    // Scale to fill the target area (cover behaviour), then crop.
    let scale = (w as f64 / src_w as f64).max(h as f64 / src_h as f64);
    let scaled_w = (src_w as f64 * scale).round() as i32;
    let scaled_h = (src_h as f64 * scale).round() as i32;
    let scaled_w = scaled_w.max(1);
    let scaled_h = scaled_h.max(1);
    let Some(scaled) = pixbuf.scale_simple(scaled_w, scaled_h, gtk4::gdk_pixbuf::InterpType::Bilinear) else {
        return;
    };
    let x = ((scaled_w - w) / 2).max(0);
    let y = ((scaled_h - h) / 2).max(0);
    let cw = w.min(scaled_w).max(1);
    let ch = h.min(scaled_h).max(1);
    let cropped = scaled.new_subpixbuf(x, y, cw, ch);
    pic.set_paintable(Some(&gdk4::Texture::for_pixbuf(&cropped)));
}

pub fn new_image_from_file(path: &str) -> gtk4::Image {
    if let Some(t) = texture_for(path) {
        gtk4::Image::from_paintable(Some(&t))
    } else {
        gtk4::Image::from_icon_name("application-x-executable")
    }
}

pub fn clear_texture_cache() {
    TEXTURE_CACHE.with(|cell| {
        cell.borrow_mut().clear();
    });
}

// === ScaledPaintable: wraps a Texture, reports a custom intrinsic size ===

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
