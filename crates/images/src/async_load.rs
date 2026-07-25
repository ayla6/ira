use crate::cache::{DecodeResult, DECODE_POOL_SIZE, PENDING_LOADS, TEXTURE_CACHE};
use crate::scaled::ScaledPaintable;
use crate::texture::{cached_texture, texture_for};
use gdk4::{MemoryFormat, MemoryTexture, Texture};
use gtk4::prelude::*;
use std::cell::Cell;
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use tracing::info_span;

struct DecodeInfra {
    job_tx: mpsc::Sender<String>,
    result_rx: Mutex<mpsc::Receiver<DecodeResult>>,
}

fn decode_infra() -> &'static DecodeInfra {
    static INFRA: OnceLock<DecodeInfra> = OnceLock::new();
    INFRA.get_or_init(|| {
        let (job_tx, job_rx) = mpsc::channel::<String>();
        let (result_tx, result_rx) = mpsc::channel::<DecodeResult>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        for i in 0..DECODE_POOL_SIZE {
            let job_rx = job_rx.clone();
            let result_tx = result_tx.clone();
            std::thread::Builder::new()
                .name(format!("ira-decode-{i}"))
                .spawn(move || {
                    while let Ok(path) = job_rx.lock().unwrap().recv() {
                        let _s = info_span!("bg_decode", path = %path).entered();
                        let result = ira_parser::decode_to_rgba(std::path::Path::new(&path));
                        let _ = result_tx.send((path, result));
                    }
                })
                .expect("Failed to spawn decode thread");
        }
        DecodeInfra { job_tx, result_rx: Mutex::new(result_rx) }
    })
}

thread_local! {
    static DRAIN_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

fn drain_results() -> glib::ControlFlow {
    let infra = decode_infra();
    loop {
        match infra.result_rx.lock().unwrap().try_recv() {
            Ok((path, result)) => {
                let callbacks = PENDING_LOADS.with(|cell| cell.borrow_mut().remove(&path));
                let texture = result.map(|(pixels, w, h)| {
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
                    TEXTURE_CACHE.with(|cell| cell.borrow_mut().insert(&path, t.clone()));
                }
                if let Some(callbacks) = callbacks {
                    for cb in callbacks {
                        cb(texture.clone());
                    }
                }
            }
            Err(TryRecvError::Empty) => {
                let has_pending = PENDING_LOADS.with(|cell| !cell.borrow().is_empty());
                if has_pending {
                    return glib::ControlFlow::Continue;
                }
                DRAIN_ACTIVE.with(|a| a.set(false));
                return glib::ControlFlow::Break;
            }
            Err(TryRecvError::Disconnected) => {
                let all: Vec<_> = PENDING_LOADS.with(|cell| cell.borrow_mut().drain().collect());
                for (_, cbs) in all {
                    for cb in cbs {
                        cb(None);
                    }
                }
                DRAIN_ACTIVE.with(|a| a.set(false));
                return glib::ControlFlow::Break;
            }
        }
    }
}

fn ensure_drain() {
    if !DRAIN_ACTIVE.with(|a| a.replace(true)) {
        glib::source::idle_add_local_full(glib::Priority::LOW, drain_results);
    }
}

pub fn load_texture_async<F>(path: &str, callback: F)
where
    F: FnOnce(Option<Texture>) + 'static,
{
    load_texture_async_with_priority(path, glib::Priority::LOW, callback);
}

pub fn load_texture_async_with_priority<F>(path: &str, _priority: glib::Priority, callback: F)
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

    if decode_infra().job_tx.send(path_str).is_err() {
        let callbacks = PENDING_LOADS.with(|cell| cell.borrow_mut().remove(path));
        if let Some(callbacks) = callbacks {
            for cb in callbacks {
                cb(None);
            }
        }
        return;
    }

    ensure_drain();
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
