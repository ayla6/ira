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
