use gdk4::Texture;
use gtk4::prelude::TextureExt;
use std::collections::HashMap;
use tracing::info_span;

pub(super) struct TextureCache {
    map: HashMap<String, Texture>,
    /// Access-order timestamps: higher = more recently used.
    /// Replaces the previous `VecDeque` + `position()` approach which was
    /// O(n) on every cache hit (string comparisons + element shifting).
    order: HashMap<String, u64>,
    counter: u64,
    total_bytes: usize,
    max_bytes: usize,
    max_entries: usize,
}

impl TextureCache {
    pub(super) fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: HashMap::new(),
            counter: 0,
            total_bytes: 0,
            max_bytes: 400 * 1024 * 1024,
            max_entries: 500,
        }
    }

    fn texture_bytes(t: &Texture) -> usize {
        (t.width() as usize) * (t.height() as usize) * 4
    }

    /// O(1) — just a HashMap lookup + counter increment.
    pub(super) fn get(&mut self, path: &str) -> Option<Texture> {
        let hit = self.map.contains_key(path);
        let _s = info_span!("cache_get", path, hit, entries = self.map.len(), total_bytes = self.total_bytes).entered();
        if let Some(t) = self.map.get(path) {
            self.counter += 1;
            self.order.insert(path.to_string(), self.counter);
            return Some(t.clone());
        }
        None
    }

    pub(super) fn insert(&mut self, path: &str, texture: Texture) {
        let bytes = Self::texture_bytes(&texture);
        let _s = info_span!("cache_insert", path, bytes, entries_before = self.map.len(), total_bytes_before = self.total_bytes).entered();
        while (self.total_bytes + bytes > self.max_bytes || self.map.len() >= self.max_entries)
            && !self.map.is_empty()
        {
            // Find the LRU entry (minimum timestamp). O(n) but only on eviction,
            // which is rare — not on every access like the old VecDeque approach.
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

    pub(super) fn remove(&mut self, path: &str) {
        let hit = self.map.contains_key(path);
        let _s = info_span!("cache_remove", path, hit, entries_before = self.map.len(), total_bytes_before = self.total_bytes).entered();
        if let Some(texture) = self.map.remove(path) {
            self.total_bytes -= Self::texture_bytes(&texture);
        }
        self.order.remove(path);
    }

    pub(super) fn clear(&mut self) {
        let _s = info_span!("cache_clear", entries_before = self.map.len(), total_bytes_before = self.total_bytes).entered();
        self.map.clear();
        self.order.clear();
        self.total_bytes = 0;
    }
}
