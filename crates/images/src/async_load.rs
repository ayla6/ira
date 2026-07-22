use crate::cache::{DecodeJob, DECODE_POOL_SIZE, PENDING_LOADS, TEXTURE_CACHE};
use crate::scaled::ScaledPaintable;
use crate::texture::{cached_texture, texture_for};
use gdk4::{MemoryFormat, MemoryTexture, Texture};
use gtk4::prelude::*;
use std::cell::RefCell;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use tracing::info_span;

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

pub fn load_texture_async<F>(path: &str, callback: F)
where
    F: FnOnce(Option<Texture>) + 'static,
{
    load_texture_async_with_priority(path, glib::Priority::LOW, callback);
}

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
