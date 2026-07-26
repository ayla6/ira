use std::sync::{Mutex, OnceLock};

use ash::vk;

use crate::types::DeviceFns;

pub const ATLAS_WIDTH: u32 = 2048;
pub const ATLAS_HEIGHT: u32 = 2048;

#[derive(Clone, Copy)]
pub struct AtlasSlot {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub struct PendingUpload {
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub(crate) struct AtlasCache {
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    pub(crate) pending: Vec<PendingUpload>,
}

impl AtlasCache {
    fn new() -> Self {
        Self {
            cursor_x: 1,
            cursor_y: 1,
            row_height: 1,
            pending: Vec::new(),
        }
    }

    fn reset(&mut self) {
        self.cursor_x = 1;
        self.cursor_y = 1;
        self.row_height = 1;
        self.pending.clear();
    }
}

static ATLAS_CACHE: OnceLock<Mutex<AtlasCache>> = OnceLock::new();

pub(crate) fn lock_cache() -> std::sync::MutexGuard<'static, AtlasCache> {
    ATLAS_CACHE.get_or_init(|| Mutex::new(AtlasCache::new())).lock().unwrap()
}

pub fn clear_cache() {
    lock_cache().reset();
}

/// Allocates space in the atlas texture for an image of the given size.
pub fn pack(cache: &mut AtlasCache, w: u32, h: u32) -> AtlasSlot {
    if w == 0 || h == 0 {
        return AtlasSlot { x: 0, y: 0, w: 0, h: 0 };
    }
    let padded_w = w + 1;
    let padded_h = h + 1;
    if cache.cursor_x + padded_w > ATLAS_WIDTH {
        cache.cursor_y += cache.row_height;
        cache.cursor_x = 1;
        cache.row_height = 1;
    }
    if cache.cursor_y + padded_h > ATLAS_HEIGHT {
        eprintln!("ira-overlay: texture atlas full");
        return AtlasSlot { x: 0, y: 0, w: 0, h: 0 };
    }
    let slot = AtlasSlot {
        x: cache.cursor_x,
        y: cache.cursor_y,
        w,
        h,
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
    fence: vk::Fence,
}

static OLD_STAGING: Mutex<Option<StagingCleanup>> = Mutex::new(None);

pub fn cleanup_old_staging() {
    let mut guard = OLD_STAGING.lock().unwrap();
    if let Some(s) = guard.take() {
        unsafe {
            let _ = (s.fns.wait_for_fences)(s.device, 1, &s.fence, vk::TRUE, 5_000_000_000);
            (s.fns.unmap_memory)(s.device, s.memory);
            (s.fns.destroy_buffer)(s.device, s.buffer, std::ptr::null());
            (s.fns.free_memory)(s.device, s.memory, std::ptr::null());
        }
    }
}

pub fn queue_staging_cleanup(fns: DeviceFns, device: vk::Device, buffer: vk::Buffer, memory: vk::DeviceMemory, fence: vk::Fence) {
    *OLD_STAGING.lock().unwrap() = Some(StagingCleanup { fns, device, buffer, memory, fence });
}
