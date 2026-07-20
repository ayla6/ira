mod cache;
mod scaled;

pub use scaled::ScaledPaintable;

use cache::TextureCache;
use gdk4::Texture;
use std::cell::RefCell;
use tracing::info_span;
thread_local! {
    static TEXTURE_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new());
}

/// Returns a cached texture without any file I/O.
/// Use this to check if a texture is already loaded before queuing async.
pub fn cached_texture(path: &str) -> Option<Texture> {
    let _s = info_span!("cached_texture", path).entered();
    if path.is_empty() {
        return None;
    }
    TEXTURE_CACHE.with(|cell| cell.borrow_mut().get(path))
}

pub fn texture_for(path: &str) -> Option<Texture> {
    let _s = info_span!("texture_for", path).entered();
    if path.is_empty() {
        return None;
    }
    TEXTURE_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some(t) = cache.get(path) {
            return Some(t);
        }
        let decoded = {
            let _s = info_span!("Texture::from_filename", path).entered();
            Texture::from_filename(path)
        };
        match decoded {
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
    let _s = info_span!("set_image", path).entered();
    if let Some(t) = texture_for(path) {
        img.set_paintable(Some(&t));
    }
}

pub fn set_picture_natural(pic: &gtk4::Picture, path: &str, w: i32, h: i32) {
    let _s = info_span!("set_picture_natural", path, w, h).entered();
    if w <= 0 || h <= 0 || path.is_empty() {
        return;
    }
    if let Some(t) = texture_for(path) {
        let paintable = ScaledPaintable::new(&t, w, h);
        pic.set_paintable(Some(&paintable));
    }
}

pub fn new_image_from_file(path: &str) -> gtk4::Image {
    let _s = info_span!("new_image_from_file", path).entered();
    if let Some(t) = texture_for(path) {
        gtk4::Image::from_paintable(Some(&t))
    } else {
        gtk4::Image::from_icon_name("application-x-executable")
    }
}

pub fn clear_texture_cache() {
    let _s = info_span!("clear_texture_cache").entered();
    TEXTURE_CACHE.with(|cell| {
        cell.borrow_mut().clear();
    });
}

pub fn invalidate_texture(path: &str) {
    let _s = info_span!("invalidate_texture", path).entered();
    TEXTURE_CACHE.with(|cell| {
        cell.borrow_mut().remove(path);
    });
}
