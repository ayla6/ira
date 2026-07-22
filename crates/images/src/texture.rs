use crate::cache::TEXTURE_CACHE;
use crate::scaled::ScaledPaintable;
use gdk4::Texture;
use gtk4::prelude::*;
use tracing::info_span;

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

pub fn set_picture_contain(pic: &gtk4::Picture, path: &str, max_h: i32) {
    let _s = info_span!("set_picture_contain", path, max_h).entered();
    if path.is_empty() {
        return;
    }
    if let Some(t) = texture_for(path) {
        pic.set_paintable(Some(&t));
    }
    pic.set_content_fit(gtk4::ContentFit::Contain);
    pic.set_halign(gtk4::Align::Start);
    pic.set_valign(gtk4::Align::Center);
    if max_h > 0 {
        pic.set_height_request(max_h);
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
