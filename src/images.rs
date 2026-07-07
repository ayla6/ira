use gdk4::Texture;
use std::cell::RefCell;
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
