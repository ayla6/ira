use gdk4::subclass::paintable::PaintableImpl;
use gdk4::{Paintable, PaintableFlags, Snapshot, Texture};
use glib::prelude::*;
use glib::subclass::prelude::*;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};

struct TextureCache {
    map: HashMap<String, Texture>,
    order: VecDeque<String>,
    total_bytes: usize,
    max_bytes: usize,
    max_entries: usize,
}

impl TextureCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            total_bytes: 0,
            max_bytes: 50 * 1024 * 1024, // 50 MB
            max_entries: 150,
        }
    }

    fn texture_bytes(t: &Texture) -> usize {
        (t.width() as usize) * (t.height() as usize) * 4
    }

    fn get(&mut self, path: &str) -> Option<Texture> {
        if let Some(t) = self.map.get(path) {
            if let Some(pos) = self.order.iter().position(|k| k == path) {
                self.order.remove(pos);
                self.order.push_back(path.to_string());
            }
            return Some(t.clone());
        }
        None
    }

    fn insert(&mut self, path: &str, texture: Texture) {
        let bytes = Self::texture_bytes(&texture);
        // Evict until both limits are satisfied
        while (self.total_bytes + bytes > self.max_bytes || self.map.len() >= self.max_entries)
            && !self.order.is_empty()
        {
            if let Some(old_key) = self.order.pop_front() {
                if let Some(old_texture) = self.map.remove(&old_key) {
                    self.total_bytes -= Self::texture_bytes(&old_texture);
                }
            }
        }
        self.total_bytes += bytes;
        self.map.insert(path.to_string(), texture);
        self.order.push_back(path.to_string());
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.total_bytes = 0;
    }
}

thread_local! {
    static TEXTURE_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new());
}

pub fn texture_for(path: &str) -> Option<Texture> {
    if path.is_empty() {
        return None;
    }
    TEXTURE_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some(t) = cache.get(path) {
            return Some(t);
        }
        match Texture::from_filename(path) {
            Ok(t) => {
                let cloned = t.clone();
                cache.insert(path, t);
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
