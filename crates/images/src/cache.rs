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

pub(crate) const DECODE_POOL_SIZE: usize = 4;

/// The header/editor logos draw through a pixbuf (cairo), so the decoded logo
/// is also reachable as a pixbuf. Both the texture and the pixbuf wrap the
/// same decoded byte buffer, so the pixbuf cache adds no pixel data of its own
/// — it only keeps the wrapper reachable so logos stay warm and instant.
pub(crate) const PIXBUF_CACHE_MAX_BYTES: usize = 128 * 1024 * 1024;

/// Textures at or below this size are treated as icons (sidebar rows,
/// achievement icons) and live in the high-capacity icon cache.
pub(crate) const ICON_MAX_DIM: u32 = 128;
/// The icon cache is bounded by bytes only — a sidebar's worth of tiny icons
/// (≤64KB each) stays warm under the cap without an entry count to thrash on.
pub(crate) const ICON_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
/// Textures above this many pixels are "game page" images (heroes) — the real
/// memory hogs. The entry cap is generous enough that browsing through game
/// pages keeps the last ~dozen heroes warm (mmap returns evicted buffers to
/// the OS, so the byte cap is the real memory guard); the cap only prevents
/// pathological growth from a huge hero-dense library.
pub(crate) const HERO_MIN_PIXELS: u64 = 1_000_000;
pub(crate) const HERO_CACHE_MAX_ENTRIES: usize = 16;
pub(crate) const HERO_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
/// General tier for grid covers, headers, logos. Byte cap only, so a full
/// grid viewport stays warm while heroes stay bounded by their own tier.
pub(crate) const TEXTURE_CACHE_MAX_BYTES: usize = 256 * 1024 * 1024;

thread_local! {
    pub(crate) static TEXTURE_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new(
        None,
        TEXTURE_CACHE_MAX_BYTES,
    ));
    pub(crate) static HERO_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new(
        Some(HERO_CACHE_MAX_ENTRIES),
        HERO_CACHE_MAX_BYTES,
    ));
    pub(crate) static ICON_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new(
        None,
        ICON_CACHE_MAX_BYTES,
    ));
    pub(crate) static PENDING_LOADS: RefCell<PendingMap> = RefCell::new(HashMap::new());
    pub(crate) static PIXBUF_CACHE: RefCell<PixbufCache> =
        RefCell::new(PixbufCache::new(PIXBUF_CACHE_MAX_BYTES));
    pub(crate) static PENDING_PIXBUFS: RefCell<PendingPixbufMap> = RefCell::new(HashMap::new());
}

pub(crate) struct TextureCache {
    map: HashMap<String, Texture>,
    order: HashMap<String, u64>,
    counter: u64,
    total_bytes: usize,
    max_bytes: usize,
    max_entries: Option<usize>,
}

impl TextureCache {
    /// `max_entries` is `None` for byte-cap-only tiers (icons, general
    /// textures): those never evict by count, so scrolling a long grid or
    /// sidebar doesn't thrash. `Some(n)` is only used for the hero tier,
    /// where even a few full-screen textures would blow the memory budget.
    pub(crate) fn new(max_entries: Option<usize>, max_bytes: usize) -> Self {
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
        // Re-inserting an existing path replaces the old entry, so subtract
        // its bytes first — otherwise total_bytes drifts upward and the caps
        // start evicting entries that still fit.
        if let Some(old) = self.map.get(path) {
            self.total_bytes -= Self::texture_bytes(old);
        }
        let is_new = !self.map.contains_key(path);
        while (self.total_bytes + bytes > self.max_bytes
            || (is_new && self.max_entries.is_some_and(|n| self.map.len() >= n)))
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

/// Byte-capped cache for the pixbuf wrappers (header/editor logos). No entry
/// cap: a library's logos must all stay warm so switching games is instant.
/// Wrappers share the decoded byte buffer with the texture cache.
pub(crate) struct PixbufCache {
    map: HashMap<String, gtk4::gdk_pixbuf::Pixbuf>,
    total_bytes: usize,
    max_bytes: usize,
}

impl PixbufCache {
    fn pixbuf_bytes(pb: &gtk4::gdk_pixbuf::Pixbuf) -> usize {
        (pb.height() as usize).saturating_mul(pb.rowstride() as usize)
    }

    pub(crate) fn new(max_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            total_bytes: 0,
            max_bytes,
        }
    }

    pub(crate) fn get(&self, path: &str) -> Option<gtk4::gdk_pixbuf::Pixbuf> {
        self.map.get(path).cloned()
    }

    pub(crate) fn insert(&mut self, path: &str, pb: gtk4::gdk_pixbuf::Pixbuf) {
        let bytes = Self::pixbuf_bytes(&pb);
        if let Some(old) = self.map.get(path) {
            self.total_bytes = self.total_bytes.saturating_sub(Self::pixbuf_bytes(old));
        }
        while self.total_bytes + bytes > self.max_bytes && !self.map.is_empty() {
            if let Some(key) = self.map.keys().next().cloned() {
                if let Some(old) = self.map.remove(&key) {
                    self.total_bytes = self.total_bytes.saturating_sub(Self::pixbuf_bytes(&old));
                }
            }
        }
        self.total_bytes += bytes;
        self.map.insert(path.to_string(), pb);
    }

    pub(crate) fn remove(&mut self, path: &str) {
        if let Some(old) = self.map.remove(path) {
            self.total_bytes = self.total_bytes.saturating_sub(Self::pixbuf_bytes(&old));
        }
    }

    pub(crate) fn retain(&mut self, keep: &HashSet<String>) {
        self.map.retain(|path, _| keep.contains(path));
        self.total_bytes = self.map.values().map(Self::pixbuf_bytes).sum();
    }

    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.total_bytes = 0;
    }
}

/// Route a texture into the tier that fits its size: icons (≤128px), heroes
/// (game-page sized, entry-capped) and everything else (byte-capped only).
pub(crate) fn insert_texture(path: &str, texture: Texture) {
    let w = texture.width();
    let h = texture.height();
    if w <= ICON_MAX_DIM as i32 && h <= ICON_MAX_DIM as i32 {
        ICON_CACHE.with(|cell| cell.borrow_mut().insert(path, texture));
    } else if (w as u64) * (h as u64) > HERO_MIN_PIXELS {
        HERO_CACHE.with(|cell| cell.borrow_mut().insert(path, texture));
    } else {
        TEXTURE_CACHE.with(|cell| cell.borrow_mut().insert(path, texture));
    }
}

pub(crate) fn get_texture(path: &str) -> Option<Texture> {
    ICON_CACHE
        .with(|cell| cell.borrow_mut().get(path))
        .or_else(|| TEXTURE_CACHE.with(|cell| cell.borrow_mut().get(path)))
        .or_else(|| HERO_CACHE.with(|cell| cell.borrow_mut().get(path)))
}

pub fn clear_texture_cache() {
    let _s = info_span!("clear_texture_cache").entered();
    TEXTURE_CACHE.with(|cell| cell.borrow_mut().clear());
    HERO_CACHE.with(|cell| cell.borrow_mut().clear());
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
    HERO_CACHE.with(|cell| cell.borrow_mut().retain(protected));
    ICON_CACHE.with(|cell| cell.borrow_mut().retain(protected));
    PIXBUF_CACHE.with(|cell| cell.borrow_mut().retain(protected));
}

pub fn invalidate_texture(path: &str) {
    let _s = info_span!("invalidate_texture", path).entered();
    TEXTURE_CACHE.with(|cell| cell.borrow_mut().remove(path));
    HERO_CACHE.with(|cell| cell.borrow_mut().remove(path));
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

    fn hero_texture() -> Texture {
        texture(1920, 1080)
    }

    fn clear_all() {
        ICON_CACHE.with(|c| c.borrow_mut().clear());
        TEXTURE_CACHE.with(|c| c.borrow_mut().clear());
        HERO_CACHE.with(|c| c.borrow_mut().clear());
        PIXBUF_CACHE.with(|c| c.borrow_mut().clear());
    }

    #[test]
    fn test_insert_texture_routes_small_into_icon_cache() {
        clear_all();
        insert_texture("icon.png", icon_texture());
        assert!(get_texture("icon.png").is_some());
        assert!(ICON_CACHE.with(|c| c.borrow().map.contains_key("icon.png")));
        assert!(!TEXTURE_CACHE.with(|c| c.borrow().map.contains_key("icon.png")));
        assert!(!HERO_CACHE.with(|c| c.borrow().map.contains_key("icon.png")));
    }

    #[test]
    fn test_insert_texture_routes_large_into_texture_cache() {
        clear_all();
        insert_texture("cover.jpg", large_texture());
        assert!(get_texture("cover.jpg").is_some());
        assert!(TEXTURE_CACHE.with(|c| c.borrow().map.contains_key("cover.jpg")));
        assert!(!ICON_CACHE.with(|c| c.borrow().map.contains_key("cover.jpg")));
        assert!(!HERO_CACHE.with(|c| c.borrow().map.contains_key("cover.jpg")));
    }

    #[test]
    fn test_insert_texture_routes_hero_into_hero_cache() {
        clear_all();
        insert_texture("hero.jpg", hero_texture());
        assert!(get_texture("hero.jpg").is_some());
        assert!(HERO_CACHE.with(|c| c.borrow().map.contains_key("hero.jpg")));
        assert!(!TEXTURE_CACHE.with(|c| c.borrow().map.contains_key("hero.jpg")));
        assert!(!ICON_CACHE.with(|c| c.borrow().map.contains_key("hero.jpg")));
    }

    #[test]
    fn test_byte_cap_only_cache_never_evicts_by_count() {
        // A grid-sized tier has no entry cap: inserting more entries than a
        // viewport holds must not evict anything, as long as the byte cap
        // isn't exceeded (40 × 300×450×4 ≈ 21.6MB).
        let cache = &mut TextureCache::new(None, 32 * 1024 * 1024);
        for i in 0..40 {
            cache.insert(&format!("cover{i}.jpg"), large_texture());
        }
        for i in 0..40 {
            assert!(cache.get(&format!("cover{i}.jpg")).is_some());
        }
    }

    #[test]
    fn test_entry_capped_cache_evicts_lru_when_full() {
        let cache = &mut TextureCache::new(Some(2), 4 * 1024 * 1024);
        cache.insert("a", large_texture());
        cache.insert("b", large_texture());
        cache.insert("c", large_texture());
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn test_reinsert_does_not_inflate_total_bytes() {
        let cache = &mut TextureCache::new(None, 4 * 1024 * 1024);
        cache.insert("a", large_texture());
        let bytes_after_first = cache.total_bytes;
        cache.insert("a", large_texture());
        assert_eq!(cache.total_bytes, bytes_after_first);
        assert_eq!(cache.map.len(), 1);
        assert!(cache.get("a").is_some());
    }

    #[test]
    fn test_retain_keeps_only_protected_paths() {
        let cache = &mut TextureCache::new(Some(100), 4 * 1024 * 1024);
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
        insert_texture("keep-cover.jpg", large_texture());
        insert_texture("drop-cover.jpg", large_texture());
        insert_texture("keep-hero.jpg", hero_texture());
        insert_texture("drop-hero.jpg", hero_texture());
        let protected: HashSet<String> = [
            "keep-icon.png".to_string(),
            "keep-cover.jpg".to_string(),
            "keep-hero.jpg".to_string(),
        ]
        .into();
        trim_image_caches(&protected);
        assert!(get_texture("keep-icon.png").is_some());
        assert!(get_texture("keep-cover.jpg").is_some());
        assert!(get_texture("keep-hero.jpg").is_some());
        assert!(get_texture("drop-icon.png").is_none());
        assert!(get_texture("drop-cover.jpg").is_none());
        assert!(get_texture("drop-hero.jpg").is_none());
    }
}
