use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ash::vk;
use cosmic_text::{CacheKey, SwashContent, SwashImage};

use crate::types::DeviceFns;

pub const ATLAS_WIDTH: u32 = 2048;
pub const ATLAS_HEIGHT: u32 = 2048;

#[derive(Clone, Copy)]
pub struct AtlasSlot {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub offset_x: i32,
    pub offset_y: i32,
}

pub struct PendingUpload {
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub(crate) struct GlyphCache {
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    pub(crate) entries: HashMap<CacheKey, AtlasSlot>,
    pub(crate) pending: Vec<PendingUpload>,
}

impl GlyphCache {
    fn new() -> Self {
        Self {
            cursor_x: 1,
            cursor_y: 1,
            row_height: 1,
            entries: HashMap::new(),
            pending: Vec::new(),
        }
    }

    fn reset(&mut self) {
        self.cursor_x = 1;
        self.cursor_y = 1;
        self.row_height = 1;
        self.entries.clear();
        self.pending.clear();
    }
}

static GLYPH_CACHE: OnceLock<Mutex<GlyphCache>> = OnceLock::new();

pub(crate) fn lock_cache() -> std::sync::MutexGuard<'static, GlyphCache> {
    GLYPH_CACHE.get_or_init(|| Mutex::new(GlyphCache::new())).lock().unwrap()
}

pub fn clear_cache() {
    lock_cache().reset();
}

pub fn convert_pixels(image: &SwashImage) -> Vec<u8> {
    match image.content {
        SwashContent::Mask => {
            image.data.iter().flat_map(|&m| [m, m, m, m]).collect()
        }
        SwashContent::Color => {
            image.data.chunks_exact(4)
                .flat_map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]])
                .collect()
        }
        SwashContent::SubpixelMask => {
            image.data.chunks_exact(4)
                .flat_map(|rgba| [rgba[3], rgba[3], rgba[3], rgba[3]])
                .collect()
        }
    }
}

pub fn pack_glyph(cache: &mut GlyphCache, w: u32, h: u32, offset_x: i32, offset_y: i32) -> AtlasSlot {
    if w == 0 || h == 0 {
        return AtlasSlot { x: 0, y: 0, w: 0, h: 0, offset_x, offset_y };
    }
    let padded_w = w + 1;
    let padded_h = h + 1;
    if cache.cursor_x + padded_w > ATLAS_WIDTH {
        cache.cursor_y += cache.row_height;
        cache.cursor_x = 1;
        cache.row_height = 1;
    }
    if cache.cursor_y + padded_h > ATLAS_HEIGHT {
        eprintln!("ira-overlay: glyph atlas full");
        return AtlasSlot { x: 0, y: 0, w: 0, h: 0, offset_x, offset_y };
    }
    let slot = AtlasSlot {
        x: cache.cursor_x,
        y: cache.cursor_y,
        w, h, offset_x, offset_y,
    };
    cache.cursor_x += padded_w;
    cache.row_height = cache.row_height.max(padded_h);
    slot
}

pub fn take_pending_uploads() -> Vec<PendingUpload> {
    let mut cache = lock_cache();
    std::mem::take(&mut cache.pending)
}

struct StagingCleanup {
    fns: DeviceFns,
    device: vk::Device,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
}

static OLD_STAGING: Mutex<Option<StagingCleanup>> = Mutex::new(None);

pub fn cleanup_old_staging() {
    let mut guard = OLD_STAGING.lock().unwrap();
    if let Some(s) = guard.take() {
        unsafe {
            (s.fns.unmap_memory)(s.device, s.memory);
            (s.fns.destroy_buffer)(s.device, s.buffer, std::ptr::null());
            (s.fns.free_memory)(s.device, s.memory, std::ptr::null());
        }
    }
}

pub fn queue_staging_cleanup(fns: DeviceFns, device: vk::Device, buffer: vk::Buffer, memory: vk::DeviceMemory) {
    *OLD_STAGING.lock().unwrap() = Some(StagingCleanup { fns, device, buffer, memory });
}
