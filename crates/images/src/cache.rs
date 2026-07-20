use gdk4::Texture;
use gtk4::prelude::TextureExt;
use std::collections::{HashMap, VecDeque};
use tracing::info_span;

pub(super) struct TextureCache {
    map: HashMap<String, Texture>,
    order: VecDeque<String>,
    total_bytes: usize,
    max_bytes: usize,
    max_entries: usize,
}

impl TextureCache {
    pub(super) fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            total_bytes: 0,
            max_bytes: 400 * 1024 * 1024,
            max_entries: 500,
        }
    }

    fn texture_bytes(t: &Texture) -> usize {
        (t.width() as usize) * (t.height() as usize) * 4
    }

    pub(super) fn get(&mut self, path: &str) -> Option<Texture> {
        let hit = self.map.contains_key(path);
        let _s = info_span!("cache_get", path, hit, entries = self.map.len(), total_bytes = self.total_bytes).entered();
        if let Some(t) = self.map.get(path) {
            if let Some(pos) = self.order.iter().position(|k| k == path) {
                self.order.remove(pos);
                self.order.push_back(path.to_string());
            }
            return Some(t.clone());
        }
        None
    }

    pub(super) fn insert(&mut self, path: &str, texture: Texture) {
        let bytes = Self::texture_bytes(&texture);
        let _s = info_span!("cache_insert", path, bytes, entries_before = self.map.len(), total_bytes_before = self.total_bytes).entered();
        while (self.total_bytes + bytes > self.max_bytes || self.map.len() >= self.max_entries)
            && !self.order.is_empty()
        {
            if let Some(old_key) = self.order.pop_front() {
                if let Some(old_texture) = self.map.remove(&old_key) {
                    self.total_bytes -= Self::texture_bytes(&old_texture);
                }
            }
        }
        self.total_bytes += bytes;
        self.map.insert(path.to_string(), texture);
        self.order.push_back(path.to_string());
    }

    pub(super) fn remove(&mut self, path: &str) {
        let hit = self.map.contains_key(path);
        let _s = info_span!("cache_remove", path, hit, entries_before = self.map.len(), total_bytes_before = self.total_bytes).entered();
        if let Some(texture) = self.map.remove(path) {
            self.total_bytes -= Self::texture_bytes(&texture);
        }
        if let Some(pos) = self.order.iter().position(|k| k == path) {
            self.order.remove(pos);
        }
    }

    pub(super) fn clear(&mut self) {
        let _s = info_span!("cache_clear", entries_before = self.map.len(), total_bytes_before = self.total_bytes).entered();
        self.map.clear();
        self.order.clear();
        self.total_bytes = 0;
    }
}
