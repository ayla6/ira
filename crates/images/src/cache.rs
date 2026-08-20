use gdk4::Texture;
use gtk4::prelude::TextureExt;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use tracing::info_span;

pub(crate) type PendingCallback = Box<dyn FnOnce(Option<Texture>)>;
pub(crate) type PendingMap = HashMap<String, Vec<PendingCallback>>;
pub(crate) type DecodeResult = (String, Option<(Vec<u8>, u32, u32)>);
pub(crate) type PendingPixbufCallback = Box<dyn FnOnce(Option<gtk4::gdk_pixbuf::Pixbuf>)>;
pub(crate) type PendingPixbufMap = HashMap<String, Vec<PendingPixbufCallback>>;

pub(crate) const PIXBUF_CACHE_MAX: usize = 10;
pub(crate) const DECODE_POOL_SIZE: usize = 4;

/// Textures at or below this size are treated as icons (sidebar rows,
/// achievement icons) and live in the high-capacity icon cache.
pub(crate) const ICON_MAX_DIM: u32 = 128;
/// The icon cache is allowed to grow far beyond the large-image cache — a
/// library's sidebar icons add up, but each stays tiny.
pub(crate) const ICON_CACHE_MAX_ENTRIES: usize = 1500;
pub(crate) const ICON_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
/// Large textures (heroes, covers, headers) are the real memory hogs; one
/// game screen only ever shows a handful, so keep the cap tight.
pub(crate) const LARGE_CACHE_MAX_ENTRIES: usize = 16;
pub(crate) const LARGE_CACHE_MAX_BYTES: usize = 48 * 1024 * 1024;

thread_local! {
    pub(crate) static TEXTURE_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new(
        LARGE_CACHE_MAX_ENTRIES,
        LARGE_CACHE_MAX_BYTES,
    ));
    pub(crate) static ICON_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new(
        ICON_CACHE_MAX_ENTRIES,
        ICON_CACHE_MAX_BYTES,
    ));
    pub(crate) static PENDING_LOADS: RefCell<PendingMap> = RefCell::new(HashMap::new());
    pub(crate) static PIXBUF_CACHE: RefCell<HashMap<String, gtk4::gdk_pixbuf::Pixbuf>> =
        RefCell::new(HashMap::new());
    pub(crate) static PENDING_PIXBUFS: RefCell<PendingPixbufMap> = RefCell::new(HashMap::new());
}

pub(crate) struct TextureCache {
    map: HashMap<String, Texture>,
    order: HashMap<String, u64>,
    counter: u64,
    total_bytes: usize,
    max_bytes: usize,
    max_entries: usize,
}

impl TextureCache {
    pub(crate) fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: HashMap::new(),
            counter: 0,
            total_bytes: 0,
            max_bytes,
            max_entries,
        }
    }

    fn texture_bytes(t: &Texture) -> usize {
        (t.width() as usize) * (t.height() as usize) * 4
    }

    pub(crate) fn get(&mut self, path: &str) -> Option<Texture> {
        let hit = self.map.contains_key(path);
        let _s = info_span!(
            "cache_get",
            path,
            hit,
            entries = self.map.len(),
            total_bytes = self.total_bytes
        )
        .entered();
        if let Some(t) = self.map.get(path) {
            self.counter += 1;
            self.order.insert(path.to_string(), self.counter);
            return Some(t.clone());
        }
        None
    }

    pub(crate) fn insert(&mut self, path: &str, texture: Texture) {
        let bytes = Self::texture_bytes(&texture);
        let _s = info_span!(
            "cache_insert",
            path,
            bytes,
            entries_before = self.map.len(),
            total_bytes_before = self.total_bytes
        )
        .entered();
        while (self.total_bytes + bytes > self.max_bytes || self.map.len() >= self.max_entries)
            && !self.map.is_empty()
        {
            let lru_key = self
                .order
                .iter()
                .min_by_key(|(_, &time)| time)
                .map(|(k, _)| k.clone());
            if let Some(key) = lru_key {
                if let Some(old_texture) = self.map.remove(&key) {
                    self.total_bytes -= Self::texture_bytes(&old_texture);
                }
                self.order.remove(&key);
            }
        }
        self.total_bytes += bytes;
        self.counter += 1;
        self.map.insert(path.to_string(), texture);
        self.order.insert(path.to_string(), self.counter);
    }

    pub(crate) fn remove(&mut self, path: &str) {
        let hit = self.map.contains_key(path);
        let _s = info_span!(
            "cache_remove",
            path,
            hit,
            entries_before = self.map.len(),
            total_bytes_before = self.total_bytes
        )
        .entered();
        if let Some(texture) = self.map.remove(path) {
            self.total_bytes -= Self::texture_bytes(&texture);
        }
        self.order.remove(path);
    }

    pub(crate) fn retain(&mut self, keep: &HashSet<String>) {
        let removed: Vec<(String, usize)> = self
            .map
            .iter()
            .filter(|(path, _)| !keep.contains(*path))
            .map(|(path, texture)| (path.clone(), Self::texture_bytes(texture)))
            .collect();
        for (path, bytes) in removed {
            self.map.remove(&path);
            self.order.remove(&path);
            self.total_bytes = self.total_bytes.saturating_sub(bytes);
        }
    }

    pub(crate) fn clear(&mut self) {
        let _s = info_span!(
            "cache_clear",
            entries_before = self.map.len(),
            total_bytes_before = self.total_bytes
        )
        .entered();
        self.map.clear();
        self.order.clear();
        self.total_bytes = 0;
    }
}

/// Route a texture into the cache tier that fits its size.
pub(crate) fn insert_texture(path: &str, texture: Texture) {
    let is_icon = texture.width() <= ICON_MAX_DIM as i32
        && texture.height() <= ICON_MAX_DIM as i32;
    if is_icon {
        ICON_CACHE.with(|cell| cell.borrow_mut().insert(path, texture));
    } else {
        TEXTURE_CACHE.with(|cell| cell.borrow_mut().insert(path, texture));
    }
}

pub(crate) fn get_texture(path: &str) -> Option<Texture> {
    ICON_CACHE
        .with(|cell| cell.borrow_mut().get(path))
        .or_else(|| TEXTURE_CACHE.with(|cell| cell.borrow_mut().get(path)))
}

pub fn clear_texture_cache() {
    let _s = info_span!("clear_texture_cache").entered();
    TEXTURE_CACHE.with(|cell| cell.borrow_mut().clear());
    ICON_CACHE.with(|cell| cell.borrow_mut().clear());
    PIXBUF_CACHE.with(|cell| cell.borrow_mut().clear());
    PENDING_LOADS.with(|cell| cell.borrow_mut().clear());
    PENDING_PIXBUFS.with(|cell| cell.borrow_mut().clear());
}

/// Drop every cached image except the protected paths. Used on game start to
/// release heroes/covers from previously viewed games while keeping the
/// sidebar icons and the current screen's images warm.
pub fn trim_image_caches(protected: &HashSet<String>) {
    let _s = info_span!("trim_image_caches", protected = protected.len()).entered();
    TEXTURE_CACHE.with(|cell| cell.borrow_mut().retain(protected));
    ICON_CACHE.with(|cell| cell.borrow_mut().retain(protected));
    PIXBUF_CACHE.with(|cell| {
        cell.borrow_mut().retain(|path, _| protected.contains(path))
    });
}

pub fn invalidate_texture(path: &str) {
    let _s = info_span!("invalidate_texture", path).entered();
    TEXTURE_CACHE.with(|cell| cell.borrow_mut().remove(path));
    ICON_CACHE.with(|cell| cell.borrow_mut().remove(path));
    PIXBUF_CACHE.with(|cell| cell.borrow_mut().remove(path));
    PENDING_LOADS.with(|cell| cell.borrow_mut().remove(path));
    PENDING_PIXBUFS.with(|cell| cell.borrow_mut().remove(path));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdk4::{MemoryFormat, MemoryTexture};
    use glib::Bytes;
    use glib::object::Cast;

    fn texture(w: i32, h: i32) -> Texture {
        let bytes = Bytes::from_owned(vec![0u8; (w * h * 4) as usize]);
        MemoryTexture::new(w, h, MemoryFormat::R8g8b8a8, &bytes, (w * 4) as usize).upcast()
    }

    fn icon_texture() -> Texture {
        texture(32, 32)
    }

    fn large_texture() -> Texture {
        texture(300, 450)
    }

    fn clear_all() {
        ICON_CACHE.with(|c| c.borrow_mut().clear());
        TEXTURE_CACHE.with(|c| c.borrow_mut().clear());
        PIXBUF_CACHE.with(|c| c.borrow_mut().clear());
    }

    #[test]
    fn test_insert_texture_routes_small_into_icon_cache() {
        clear_all();
        insert_texture("icon.png", icon_texture());
        assert!(get_texture("icon.png").is_some());
        assert!(ICON_CACHE.with(|c| c.borrow().map.contains_key("icon.png")));
        assert!(!TEXTURE_CACHE.with(|c| c.borrow().map.contains_key("icon.png")));
    }

    #[test]
    fn test_insert_texture_routes_large_into_texture_cache() {
        clear_all();
        insert_texture("hero.jpg", large_texture());
        assert!(get_texture("hero.jpg").is_some());
        assert!(TEXTURE_CACHE.with(|c| c.borrow().map.contains_key("hero.jpg")));
        assert!(!ICON_CACHE.with(|c| c.borrow().map.contains_key("hero.jpg")));
    }

    #[test]
    fn test_large_cache_evicts_lru_when_full() {
        let cache = &mut TextureCache::new(2, 4 * 1024 * 1024);
        cache.insert("a", large_texture());
        cache.insert("b", large_texture());
        cache.insert("c", large_texture());
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn test_retain_keeps_only_protected_paths() {
        let cache = &mut TextureCache::new(100, 4 * 1024 * 1024);
        cache.insert("a", icon_texture());
        cache.insert("b", icon_texture());
        let keep: HashSet<String> = ["a".to_string()].into();
        cache.retain(&keep);
        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
    }

    #[test]
    fn test_trim_image_caches_preserves_protected() {
        clear_all();
        insert_texture("keep-icon.png", icon_texture());
        insert_texture("drop-icon.png", icon_texture());
        insert_texture("keep-hero.jpg", large_texture());
        insert_texture("drop-hero.jpg", large_texture());
        let protected: HashSet<String> =
            ["keep-icon.png".to_string(), "keep-hero.jpg".to_string()].into();
        trim_image_caches(&protected);
        assert!(get_texture("keep-icon.png").is_some());
        assert!(get_texture("keep-hero.jpg").is_some());
        assert!(get_texture("drop-icon.png").is_none());
        assert!(get_texture("drop-hero.jpg").is_none());
    }
}
