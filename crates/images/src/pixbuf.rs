use crate::async_load::{ensure_drain, submit_decode};
use crate::cache::{PENDING_LOADS, PENDING_PIXBUFS, PIXBUF_CACHE, PIXBUF_CACHE_MAX};
use gtk4::gdk_pixbuf::Pixbuf;
use tracing::info_span;

pub(crate) fn cache_pixbuf(path: &str, pb: &Pixbuf) {
    PIXBUF_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.len() >= PIXBUF_CACHE_MAX {
            if let Some(key) = cache.keys().next().cloned() {
                cache.remove(&key);
            }
        }
        cache.insert(path.to_string(), pb.clone());
    });
}

pub fn pixbuf_for(path: &str) -> Option<Pixbuf> {
    let _s = info_span!("pixbuf_for", path).entered();
    if path.is_empty() {
        return None;
    }
    if let Some(pb) = PIXBUF_CACHE.with(|cell| cell.borrow().get(path).cloned()) {
        return Some(pb);
    }
    match Pixbuf::from_file(path) {
        Ok(pb) => {
            let cloned = pb.clone();
            cache_pixbuf(path, &cloned);
            Some(cloned)
        }
        Err(e) => {
            eprintln!("pixbuf_for: failed to decode {}: {}", path, e);
            None
        }
    }
}

pub fn pixbuf_for_async<F>(path: String, callback: F)
where
    F: FnOnce(Option<Pixbuf>) + 'static,
{
    let _s = info_span!("pixbuf_for_async", path = %path).entered();
    if path.is_empty() {
        callback(None);
        return;
    }
    if let Some(pb) = PIXBUF_CACHE.with(|cell| cell.borrow().get(&path).cloned()) {
        callback(Some(pb));
        return;
    }

    let already_pending = PENDING_PIXBUFS.with(|cell| {
        let mut loads = cell.borrow_mut();
        let was_pending = loads.contains_key(&path);
        loads
            .entry(path.clone())
            .or_default()
            .push(Box::new(callback));
        was_pending
    });
    if already_pending {
        return;
    }

    // If a texture decode for the same path is already in flight, the shared
    // drain loop serves both callback maps — don't submit a second decode job.
    let texture_pending = PENDING_LOADS.with(|cell| cell.borrow().contains_key(&path));
    if !texture_pending {
        submit_decode(path, glib::Priority::LOW);
    }
    ensure_drain();
}