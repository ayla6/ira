mod cache;
mod scaled;

pub use scaled::ScaledPaintable;

use cache::TextureCache;
use gdk4::{MemoryFormat, MemoryTexture, Texture};
use gtk4::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use tracing::info_span;

type PendingCallback = Box<dyn FnOnce(Option<Texture>)>;
type PendingMap = HashMap<String, Vec<PendingCallback>>;
type DecodeJob = (String, mpsc::Sender<Option<(Vec<u8>, u32, u32)>>);

thread_local! {
    static TEXTURE_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new());
    static PENDING_LOADS: RefCell<PendingMap> = RefCell::new(HashMap::new());
    static PIXBUF_CACHE: RefCell<HashMap<String, gtk4::gdk_pixbuf::Pixbuf>> =
        RefCell::new(HashMap::new());
}

const PIXBUF_CACHE_MAX: usize = 15;
const DECODE_POOL_SIZE: usize = 3;

fn decode_queue() -> &'static mpsc::Sender<DecodeJob> {
    static QUEUE: OnceLock<mpsc::Sender<DecodeJob>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<DecodeJob>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..DECODE_POOL_SIZE {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("ira-decode-{i}"))
                .spawn(move || {
                    while let Ok((path, result_tx)) = rx.lock().unwrap().recv() {
                        let _s = info_span!("bg_decode", path = %path).entered();
                        let result = ira_parser::decode_to_rgba(std::path::Path::new(&path));
                        let _ = result_tx.send(result);
                    }
                })
                .expect("Failed to spawn decode thread");
        }
        tx
    })
}

// -----------------------------------------------------------------------
// Synchronous API — cache hits are instant, cache misses block main thread
// -----------------------------------------------------------------------

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

// -----------------------------------------------------------------------
// Async API — cache hits are instant, cache misses decode on a background
// thread and call back on the main thread via idle sources.
// -----------------------------------------------------------------------

/// Load a texture asynchronously. Cache hits call `callback` immediately.
/// Cache misses spawn a background thread that reads the file and decodes it
/// to RGBA8 via the `image` crate. When decoding finishes, the raw pixels are
/// wrapped in a `gdk::MemoryTexture` on the main thread and cached.
///
/// Multiple requests for the same path are deduplicated — only one
/// background decode runs per path, and all pending callbacks are called
/// when it completes.
pub fn load_texture_async<F>(path: &str, callback: F)
where
    F: FnOnce(Option<Texture>) + 'static,
{
    load_texture_async_with_priority(path, glib::Priority::LOW, callback);
}

/// Same as `load_texture_async` but allows specifying the idle priority for
/// the result-processing callback. Use a higher priority (e.g. `DEFAULT`)
/// for images that should appear before lower-priority ones.
pub fn load_texture_async_with_priority<F>(path: &str, priority: glib::Priority, callback: F)
where
    F: FnOnce(Option<Texture>) + 'static,
{
    let _s = info_span!("load_texture_async", path).entered();
    if path.is_empty() {
        callback(None);
        return;
    }
    if let Some(t) = cached_texture(path) {
        callback(Some(t));
        return;
    }

    let path_str = path.to_string();

    let already_pending = PENDING_LOADS.with(|cell| {
        let mut loads = cell.borrow_mut();
        let was_pending = loads.contains_key(&path_str);
        loads
            .entry(path_str.clone())
            .or_default()
            .push(Box::new(callback));
        was_pending
    });

    if already_pending {
        return;
    }

    let (tx, rx) = mpsc::channel::<Option<(Vec<u8>, u32, u32)>>();
    let rx = RefCell::new(rx);

    let path_for_decode = path_str.clone();
    if decode_queue().send((path_for_decode, tx)).is_err() {
        let callbacks = PENDING_LOADS.with(|cell| cell.borrow_mut().remove(&path_str));
        if let Some(callbacks) = callbacks {
            for cb in callbacks {
                cb(None);
            }
        }
        return;
    }

    let path_for_idle = path_str;
    glib::source::idle_add_local_full(priority, move || {
        match rx.borrow_mut().try_recv() {
            Ok(result) => {
                let callbacks = PENDING_LOADS.with(|cell| cell.borrow_mut().remove(&path_for_idle));

                if let Some(callbacks) = callbacks {
                    let texture: Option<Texture> = result.map(|(pixels, w, h)| {
                        let _s = info_span!("MemoryTexture_new", path = %path_for_idle, w, h).entered();
                        let bytes = glib::Bytes::from_owned(pixels);
                        MemoryTexture::new(
                            w as i32,
                            h as i32,
                            MemoryFormat::R8g8b8a8,
                            &bytes,
                            (w * 4) as usize,
                        )
                        .upcast::<Texture>()
                    });

                    if let Some(ref t) = texture {
                        TEXTURE_CACHE.with(|cell| cell.borrow_mut().insert(&path_for_idle, t.clone()));
                    }

                    for cb in callbacks {
                        cb(texture.clone());
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let callbacks = PENDING_LOADS.with(|cell| cell.borrow_mut().remove(&path_for_idle));
                if let Some(callbacks) = callbacks {
                    for cb in callbacks {
                        cb(None);
                    }
                }
                glib::ControlFlow::Break
            }
        }
    });
}

/// Async version of `set_picture_natural`. Sets the paintable when the
/// texture arrives. Uses a weak ref so it's safe if the widget is destroyed.
pub fn set_picture_natural_async(pic: &gtk4::Picture, path: &str, w: i32, h: i32) {
    let _s = info_span!("set_picture_natural_async", path, w, h).entered();
    if w <= 0 || h <= 0 || path.is_empty() {
        return;
    }
    if let Some(t) = texture_for(path) {
        let paintable = ScaledPaintable::new(&t, w, h);
        pic.set_paintable(Some(&paintable));
        return;
    }
    let pic_weak = pic.downgrade();
    load_texture_async(path, move |texture| {
        if let Some(pic) = pic_weak.upgrade() {
            if let Some(t) = texture {
                let paintable = ScaledPaintable::new(&t, w, h);
                pic.set_paintable(Some(&paintable));
            }
        }
    });
}

/// Async version of `set_image`. Sets the paintable when the texture arrives.
pub fn set_image_async(img: &gtk4::Image, path: &str) {
    let _s = info_span!("set_image_async", path).entered();
    if let Some(t) = texture_for(path) {
        img.set_paintable(Some(&t));
        return;
    }
    let img_weak = img.downgrade();
    load_texture_async(path, move |texture| {
        if let Some(img) = img_weak.upgrade() {
            if let Some(t) = texture {
                img.set_paintable(Some(&t));
            }
        }
    });
}

/// Async version of `set_picture_contain`. Sets up the picture immediately
/// (content fit, alignment, height) and loads the paintable asynchronously.
pub fn set_picture_contain_async(pic: &gtk4::Picture, path: &str, max_h: i32) {
    let _s = info_span!("set_picture_contain_async", path, max_h).entered();
    if path.is_empty() {
        return;
    }
    if let Some(t) = texture_for(path) {
        pic.set_paintable(Some(&t));
    } else {
        let pic_weak = pic.downgrade();
        load_texture_async(path, move |texture| {
            if let Some(pic) = pic_weak.upgrade() {
                if let Some(t) = texture {
                    pic.set_paintable(Some(&t));
                }
            }
        });
    }
    pic.set_content_fit(gtk4::ContentFit::Contain);
    pic.set_halign(gtk4::Align::Start);
    pic.set_valign(gtk4::Align::Center);
    if max_h > 0 {
        pic.set_height_request(max_h);
    }
}

// -----------------------------------------------------------------------
// Pixbuf API — for Cairo-based rendering (logo overlays) that needs Pixbuf
// instead of Texture. Cached to avoid re-reading the file on every rebuild.
// -----------------------------------------------------------------------

pub fn pixbuf_for(path: &str) -> Option<gtk4::gdk_pixbuf::Pixbuf> {
    let _s = info_span!("pixbuf_for", path).entered();
    if path.is_empty() {
        return None;
    }
    PIXBUF_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.len() >= PIXBUF_CACHE_MAX {
            if let Some(key) = cache.keys().next().cloned() {
                cache.remove(&key);
            }
        }
        if let Some(pb) = cache.get(path) {
            return Some(pb.clone());
        }
        match gtk4::gdk_pixbuf::Pixbuf::from_file(path) {
            Ok(pb) => {
                let cloned = pb.clone();
                cache.insert(path.to_string(), pb);
                Some(cloned)
            }
            Err(_) => None,
        }
    })
}

// -----------------------------------------------------------------------
// Cache management
// -----------------------------------------------------------------------

pub fn clear_texture_cache() {
    let _s = info_span!("clear_texture_cache").entered();
    TEXTURE_CACHE.with(|cell| cell.borrow_mut().clear());
    PIXBUF_CACHE.with(|cell| cell.borrow_mut().clear());
    PENDING_LOADS.with(|cell| cell.borrow_mut().clear());
}

pub fn invalidate_texture(path: &str) {
    let _s = info_span!("invalidate_texture", path).entered();
    TEXTURE_CACHE.with(|cell| cell.borrow_mut().remove(path));
    PIXBUF_CACHE.with(|cell| cell.borrow_mut().remove(path));
    PENDING_LOADS.with(|cell| cell.borrow_mut().remove(path));
}
