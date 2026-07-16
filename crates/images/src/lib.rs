mod cache;
mod scaled;

pub use scaled::ScaledPaintable;

use cache::TextureCache;
use gdk4::Texture;
use std::cell::RefCell;
thread_local! {
    static TEXTURE_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new());
}

/// Returns a cached texture without any file I/O.
/// Use this to check if a texture is already loaded before queuing async.
pub fn cached_texture(path: &str) -> Option<Texture> {
    if path.is_empty() {
        return None;
    }
    TEXTURE_CACHE.with(|cell| cell.borrow_mut().get(path))
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

pub fn set_picture_natural(pic: &gtk4::Picture, path: &str, w: i32, h: i32) {
    if w <= 0 || h <= 0 || path.is_empty() {
        return;
    }
    if let Some(t) = texture_for(path) {
        let paintable = ScaledPaintable::new(&t, w, h);
        pic.set_paintable(Some(&paintable));
    }
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

pub fn invalidate_texture(path: &str) {
    TEXTURE_CACHE.with(|cell| {
        cell.borrow_mut().remove(path);
    });
}
