use gdk4::{MemoryTexture, MemoryFormat, Texture};
use glib::object::Cast;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static TEXTURE_CACHE: RefCell<HashMap<String, Texture>> = RefCell::new(HashMap::new());
    static SCALED_CACHE: RefCell<HashMap<(String, i32), Texture>> = RefCell::new(HashMap::new());
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

/// Get a texture scaled to the target size, creating it from the original
/// image file and caching the result. The returned texture has an intrinsic
/// size equal to (width, height), so gtk4::Picture will report that as its
/// natural size — fixing FlowBox layout.
pub fn scaled_texture(path: &str, width: i32, height: i32) -> Option<Texture> {
    if path.is_empty() {
        return None;
    }
    let key = (path.to_string(), width);
    SCALED_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some(t) = cache.get(&key) {
            return Some(t.clone());
        }
        let img = image::open(path).ok()?;
        let resized = img.resize_exact(width as u32, height as u32, image::imageops::FilterType::Triangle);
        let rgba = resized.to_rgba8();
        let (w, h) = rgba.dimensions();
        let row_stride = w as usize * 4;
        let bytes = glib::Bytes::from_owned(rgba.into_raw());
        let mem_tex = MemoryTexture::new(
            w as i32,
            h as i32,
            MemoryFormat::R8g8b8a8,
            &bytes,
            row_stride,
        );
        let tex: Texture = mem_tex.upcast();
        cache.insert(key, tex.clone());
        Some(tex)
    })
}

/// Set a Picture's paintable to a pre-scaled texture so the widget's natural
/// size matches the target dimensions.
pub fn set_picture_scaled(pic: &gtk4::Picture, path: &str, width: i32, height: i32) {
    if let Some(t) = scaled_texture(path, width, height) {
        pic.set_paintable(Some(&t));
    }
}

pub fn clear_texture_cache() {
    TEXTURE_CACHE.with(|cell| {
        cell.borrow_mut().clear();
    });
    SCALED_CACHE.with(|cell| {
        cell.borrow_mut().clear();
    });
}
