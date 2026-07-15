use gdk4::Texture;
use gtk4::prelude::TextureExt;
use std::collections::{HashMap, VecDeque};

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
            max_bytes: 50 * 1024 * 1024,
            max_entries: 150,
        }
    }

    fn texture_bytes(t: &Texture) -> usize {
        (t.width() as usize) * (t.height() as usize) * 4
    }

    pub(super) fn get(&mut self, path: &str) -> Option<Texture> {
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

    pub(super) fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.total_bytes = 0;
    }
}
