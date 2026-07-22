use crate::cache::{PIXBUF_CACHE, PIXBUF_CACHE_MAX};
use tracing::info_span;

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
