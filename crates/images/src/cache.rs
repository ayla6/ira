use gdk4::Texture;
use gtk4::prelude::TextureExt;
use std::cell::RefCell;
use std::collections::HashMap;
use tracing::info_span;

pub(crate) type PendingCallback = Box<dyn FnOnce(Option<Texture>)>;
pub(crate) type PendingMap = HashMap<String, Vec<PendingCallback>>;
pub(crate) type DecodeResult = (String, Option<(Vec<u8>, u32, u32)>);

pub(crate) const PIXBUF_CACHE_MAX: usize = 10;
pub(crate) const DECODE_POOL_SIZE: usize = 2;

thread_local! {
    pub(crate) static TEXTURE_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new());
    pub(crate) static PENDING_LOADS: RefCell<PendingMap> = RefCell::new(HashMap::new());
    pub(crate) static PIXBUF_CACHE: RefCell<HashMap<String, gtk4::gdk_pixbuf::Pixbuf>> =
        RefCell::new(HashMap::new());
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
    pub(crate) fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: HashMap::new(),
            counter: 0,
            total_bytes: 0,
            max_bytes: 150 * 1024 * 1024,
            max_entries: 200,
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
