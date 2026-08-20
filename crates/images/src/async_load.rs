use crate::cache::{
    insert_texture, DecodeResult, DECODE_POOL_SIZE, PENDING_LOADS, PENDING_PIXBUFS,
};
use crate::texture::cached_texture;
use gdk4::{MemoryFormat, MemoryTexture, Texture};
use gtk4::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk4::prelude::*;
use std::cell::Cell;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use tracing::info_span;

struct DecodeJob {
    priority: glib::Priority,
    seq: u64,
    path: String,
}

impl Ord for DecodeJob {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Max-heap: glib priority is inverted (High=-100 < Low=300), so higher
        // priority must compare as greater. Within a priority, older seq first.
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for DecodeJob {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for DecodeJob {}

impl PartialEq for DecodeJob {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq && self.path == other.path
    }
}

struct JobQueue {
    heap: Mutex<BinaryHeap<DecodeJob>>,
    cv: Condvar,
}

impl JobQueue {
    fn push(&self, job: DecodeJob) {
        let mut heap = self.heap.lock().unwrap();
        heap.push(job);
        self.cv.notify_one();
    }

    fn pop(&self) -> DecodeJob {
        let mut heap = self.heap.lock().unwrap();
        loop {
            if let Some(job) = heap.pop() {
                return job;
            }
            heap = self.cv.wait(heap).unwrap();
        }
    }
}

struct DecodeInfra {
    queue: Arc<JobQueue>,
    result_rx: Mutex<mpsc::Receiver<DecodeResult>>,
}

fn decode_infra() -> &'static DecodeInfra {
    static INFRA: OnceLock<DecodeInfra> = OnceLock::new();
    INFRA.get_or_init(|| {
        let (result_tx, result_rx) = mpsc::channel::<DecodeResult>();
        let queue = Arc::new(JobQueue {
            heap: Mutex::new(BinaryHeap::new()),
            cv: Condvar::new(),
        });
        for i in 0..DECODE_POOL_SIZE {
            let queue = queue.clone();
            let result_tx = result_tx.clone();
            std::thread::Builder::new()
                .name(format!("ira-decode-{i}"))
                .spawn(move || loop {
                    let path = queue.pop().path;
                    let _s = info_span!("bg_decode", path = %path).entered();
                    let result = match ira_parser::decode_to_rgba(std::path::Path::new(&path)) {
                        Some(r) => Some(r),
                        None => {
                            eprintln!("ira-decode: failed to decode image: {}", path);
                            None
                        }
                    };
                    let _ = result_tx.send((path, result));
                })
                .expect("Failed to spawn decode thread");
        }
        DecodeInfra {
            queue,
            result_rx: Mutex::new(result_rx),
        }
    })
}

pub(crate) fn submit_decode(path: String, priority: glib::Priority) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    decode_infra().queue.push(DecodeJob {
        priority,
        seq,
        path,
    });
}

thread_local! {
    static DRAIN_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

fn drain_results() -> glib::ControlFlow {
    let infra = decode_infra();
    loop {
        match infra.result_rx.lock().unwrap().try_recv() {
            Ok((path, result)) => {
                let texture_callbacks =
                    PENDING_LOADS.with(|cell| cell.borrow_mut().remove(&path));
                let pixbuf_callbacks =
                    PENDING_PIXBUFS.with(|cell| cell.borrow_mut().remove(&path));
                let texture = result.as_ref().map(|(pixels, w, h)| {
                    let bytes = glib::Bytes::from_owned(pixels.clone());
                    MemoryTexture::new(
                        *w as i32,
                        *h as i32,
                        MemoryFormat::R8g8b8a8,
                        &bytes,
                        (*w * 4) as usize,
                    )
                    .upcast::<Texture>()
                });
                let pixbuf = result.as_ref().map(|(pixels, w, h)| {
                    let bytes = glib::Bytes::from_owned(pixels.clone());
                    Pixbuf::from_bytes(
                        &bytes,
                        Colorspace::Rgb,
                        true,
                        8,
                        *w as i32,
                        *h as i32,
                        (*w * 4) as i32,
                    )
                });
                if let Some(ref t) = texture {
                    insert_texture(&path, t.clone());
                }
                if let Some(ref pb) = pixbuf {
                    crate::pixbuf::cache_pixbuf(&path, pb);
                }
                if let Some(callbacks) = texture_callbacks {
                    for cb in callbacks {
                        cb(texture.clone());
                    }
                }
                if let Some(callbacks) = pixbuf_callbacks {
                    for cb in callbacks {
                        cb(pixbuf.clone());
                    }
                }
            }
            Err(TryRecvError::Empty) => {
                let has_pending = PENDING_LOADS.with(|cell| !cell.borrow().is_empty())
                    || PENDING_PIXBUFS.with(|cell| !cell.borrow().is_empty());
                if has_pending {
                    return glib::ControlFlow::Continue;
                }
                DRAIN_ACTIVE.with(|a| a.set(false));
                return glib::ControlFlow::Break;
            }
            Err(TryRecvError::Disconnected) => {
                let all_textures: Vec<_> =
                    PENDING_LOADS.with(|cell| cell.borrow_mut().drain().collect());
                for (_, cbs) in all_textures {
                    for cb in cbs {
                        cb(None);
                    }
                }
                let all_pixbufs: Vec<_> =
                    PENDING_PIXBUFS.with(|cell| cell.borrow_mut().drain().collect());
                for (_, cbs) in all_pixbufs {
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

pub(crate) fn ensure_drain() {
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

    // If a pixbuf decode for the same path is in flight, the shared drain loop
    // also services this texture callback — no second decode job needed.
    let pixbuf_pending = PENDING_PIXBUFS.with(|cell| cell.borrow().contains_key(&path_str));
    if !pixbuf_pending {
        submit_decode(path_str, priority);
    }
    ensure_drain();
}

pub fn set_image_async(img: &gtk4::Image, path: &str) {
    let _s = info_span!("set_image_async", path).entered();
    if let Some(t) = cached_texture(path) {
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
    if let Some(t) = cached_texture(path) {
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