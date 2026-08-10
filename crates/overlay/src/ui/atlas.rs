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
    /// Glyph left bearing (placement.left).
    pub offset_x: i32,
    /// Glyph ascent (placement.top).
    pub offset_y: i32,
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
    ATLAS_CACHE
        .get_or_init(|| Mutex::new(AtlasCache::new()))
        .lock()
        .unwrap()
}

pub fn clear_cache() {
    lock_cache().reset();
}

/// Packs a glyph of the given dimensions into the atlas.
/// Stores the glyph's bearing offsets for later positioning.
pub fn pack_glyph(
    cache: &mut AtlasCache,
    w: u32,
    h: u32,
    offset_x: i32,
    offset_y: i32,
) -> AtlasSlot {
    if w == 0 || h == 0 {
        return AtlasSlot {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            offset_x,
            offset_y,
        };
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
        return AtlasSlot {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            offset_x,
            offset_y,
        };
    }
    let slot = AtlasSlot {
        x: cache.cursor_x,
        y: cache.cursor_y,
        w,
        h,
        offset_x,
        offset_y,
    };
    cache.cursor_x += padded_w;
    cache.row_height = cache.row_height.max(padded_h);
    slot
}

pub fn take_pending_uploads() -> Vec<PendingUpload> {
    let mut cache = lock_cache();
    std::mem::take(&mut cache.pending)
}

// --- Persistent staging buffer ---

struct PersistentStaging {
    fns: DeviceFns,
    device: vk::Device,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    ptr: *mut u8,
    size: u64,
    fence: vk::Fence,
}

unsafe impl Send for PersistentStaging {}

static STAGING: Mutex<Option<PersistentStaging>> = Mutex::new(None);

pub(crate) fn prepare_staging(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    needed_size: u64,
) -> Option<(vk::Buffer, *mut u8, u64)> {
    let mut guard = STAGING.lock().unwrap();

    if let Some(ref s) = *guard {
        if s.fence != vk::Fence::null() {
            unsafe {
                let _ = (s.fns.wait_for_fences)(s.device, 1, &s.fence, vk::TRUE, 5_000_000_000);
            }
        }
    }

    let need_recreate = guard.as_ref().is_none_or(|s| s.size < needed_size);
    if need_recreate {
        if let Some(ref s) = *guard {
            unsafe {
                (s.fns.unmap_memory)(s.device, s.memory);
                (s.fns.destroy_buffer)(s.device, s.buffer, std::ptr::null());
                (s.fns.free_memory)(s.device, s.memory, std::ptr::null());
            }
        }
        let alloc_size = needed_size.max(1_048_576);
        let (buffer, memory, ptr) = unsafe {
            super::resources::create_buffer(
                fns,
                device,
                physical_device,
                alloc_size,
                vk::BufferUsageFlags::TRANSFER_SRC,
            )?
        };
        *guard = Some(PersistentStaging {
            fns,
            device,
            buffer,
            memory,
            ptr: ptr as *mut u8,
            size: alloc_size,
            fence: vk::Fence::null(),
        });
    }

    guard.as_ref().map(|s| (s.buffer, s.ptr, s.size))
}

pub(crate) fn set_staging_fence(fence: vk::Fence) {
    let mut guard = STAGING.lock().unwrap();
    if let Some(ref mut s) = *guard {
        s.fence = fence;
    }
}

pub fn destroy_staging(_fns: DeviceFns, _device: vk::Device) {
    let mut guard = STAGING.lock().unwrap();
    if let Some(s) = guard.take() {
        unsafe {
            (s.fns.unmap_memory)(s.device, s.memory);
            (s.fns.destroy_buffer)(s.device, s.buffer, std::ptr::null());
            (s.fns.free_memory)(s.device, s.memory, std::ptr::null());
        }
    }
}
