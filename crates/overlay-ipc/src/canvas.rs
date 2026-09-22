//! Canvas shared memory — pixel frames from the out-of-process GTK host.
//!
//! Layout (fixed size, no back-compat shims per house rules):
//! ```text
//! [0 .. H]              CanvasHeader (rings embedded)
//! [H .. H + MAX_BYTES]  packed RGBA8 pixels, stride = width * 4
//! ```
//!
//! The GTK host renders on change (or ~30fps while visible), copies the
//! frame, then bumps `write_seq`. The Vulkan layer keeps a local `last_seq`
//! and re-uploads only when `write_seq` advances; it writes `ack_seq` after
//! compositing so the host can apply backpressure. Command/action rings are
//! SPSC with a single atomic write index each — readers track their own
//! read index locally, matching the notification ring in `protocol.rs`.

use std::ffi::CString;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::commands::{CanvasAction, CanvasCommand, MAX_CANVAS_ACTIONS, MAX_CANVAS_COMMANDS};

pub const CANVAS_MAGIC: u32 = 0x49524143;
/// Bumped when the header layout changes (cursor block added in v2,
/// screen size + bigger pixel buffer in v3).
/// Both ends check it and refuse to speak across versions — silent
/// garbage is worse than no overlay.
pub const CANVAS_VERSION: u32 = 3;

/// Panel-sized canvas. 768×1152 RGBA is ~3.5 MB — re-rendered on change or
/// while visible, SHM bandwidth stays in the noise.
pub const CANVAS_MAX_W: usize = 768;
pub const CANVAS_MAX_H: usize = 1152;
pub const CANVAS_BPP: usize = 4;
pub const CANVAS_MAX_BYTES: usize = CANVAS_MAX_W * CANVAS_MAX_H * CANVAS_BPP;

/// Cursor side-channel: the host publishes its theme arrow once; the layer
/// draws it per-present at native size. 128×128 RGBA premultiplied.
pub const CURSOR_MAX: usize = 128;
pub const CURSOR_MAX_BYTES: usize = CURSOR_MAX * CURSOR_MAX * CANVAS_BPP;

/// Total mapping size: header + worst-case pixel buffer.
pub const CANVAS_SHM_SIZE: usize =
    std::mem::size_of::<CanvasHeader>() + CANVAS_MAX_BYTES;

/// Fixed-size header at the start of the canvas region.
#[repr(C)]
pub struct CanvasHeader {
    pub magic: u32,
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// 1 = host has content the layer should composite, 0 = show nothing.
    /// The layer renders nothing when 0 — a dead host shows no stale frame
    /// once the launcher clears this on teardown (best effort).
    pub visible: AtomicU32,
    /// Bumped by the host after each pixel write (SeqCst fence before store).
    pub write_seq: AtomicU32,
    /// Written by the layer after compositing seq N (backpressure signal).
    pub ack_seq: AtomicU32,
    /// Layer → host command ring write index.
    pub cmd_write: AtomicU32,
    /// Host → layer action ring write index.
    pub act_write: AtomicU32,
    /// Swapchain extent, written by the layer each present (0 = unknown).
    /// The host sizes its window from this so the panel grows and shrinks
    /// with the game instead of staying a fixed postage stamp.
    pub screen_w: AtomicU32,
    pub screen_h: AtomicU32,
    pub _pad: [u8; 4],
    pub cmds: [CanvasCommand; MAX_CANVAS_COMMANDS],
    pub acts: [CanvasAction; MAX_CANVAS_ACTIONS],
    /// Cursor image size (0 = unpublished).
    pub cursor_w: u32,
    pub cursor_h: u32,
    /// Cursor hotspot offset in pixels.
    pub cursor_xhot: i32,
    pub cursor_yhot: i32,
    /// Bumped by the host after publishing the cursor image.
    pub cursor_seq: AtomicU32,
    pub _pad2: [u8; 8],
    /// Top-left packed RGBA premultiplied cursor pixels (`cursor_w×h` used).
    pub cursor_pixels: [u8; CURSOR_MAX_BYTES],
}

/// POSIX SHM name for a game's canvas region.
pub fn canvas_shm_path(db_id: i64) -> String {
    format!("/ira_canvas_{db_id}")
}

/// Computes the on-screen panel rect (x, y, w, h) for a canvas frame.
/// Uniformly fit-scales the frame into the screen minus a 16px margin and
/// anchors it per `position` (`OverlayPosition` as u32; unknown = top-left).
pub fn canvas_panel_rect(
    screen_w: u32,
    screen_h: u32,
    frame_w: u32,
    frame_h: u32,
    position: u32,
) -> (f32, f32, f32, f32) {
    const MARGIN: f32 = 16.0;
    let fw = frame_w.max(1) as f32;
    let fh = frame_h.max(1) as f32;
    let avail_w = (screen_w as f32 - 2.0 * MARGIN).max(1.0);
    let avail_h = (screen_h as f32 - 2.0 * MARGIN).max(1.0);
    let scale = (avail_w / fw).min(avail_h / fh).min(1.0);
    let w = fw * scale;
    let h = fh * scale;
    let (x, y) = match position {
        1 => (screen_w as f32 - MARGIN - w, MARGIN),
        2 => (MARGIN, screen_h as f32 - MARGIN - h),
        3 => (screen_w as f32 - MARGIN - w, screen_h as f32 - MARGIN - h),
        4 => ((screen_w as f32 - w) / 2.0, (screen_h as f32 - h) / 2.0),
        _ => (MARGIN, MARGIN),
    };
    (x.max(0.0), y.max(0.0), w, h)
}

/// Window size for a screen, keeping the 512:800 panel aspect: fills the
/// screen minus a margin, clamped to the canvas maximum. The host applies
/// this to its window so the panel grows and shrinks with the game.
pub fn fit_panel_size(screen_w: u32, screen_h: u32) -> (u32, u32) {
    const BASE_W: f64 = 512.0;
    const BASE_H: f64 = 800.0;
    const MARGIN: f64 = 32.0;
    let avail_w = (screen_w as f64 - MARGIN).max(1.0);
    let avail_h = (screen_h as f64 - MARGIN).max(1.0);
    let scale = (avail_w / BASE_W).min(avail_h / BASE_H);
    let w = ((BASE_W * scale) as u32).clamp(1, CANVAS_MAX_W as u32);
    let h = ((BASE_H * scale) as u32).clamp(1, CANVAS_MAX_H as u32);
    (w, h)
}

/// Filesystem path for debugging (`/dev/shm/<name>`).
pub fn canvas_shm_file_path(name: &str) -> String {
    format!("/dev/shm/{name}")
}

/// RAII wrapper around the mapped canvas region. Unmaps on drop.
pub struct CanvasShm {
    ptr: *mut u8,
    size: usize,
    owned: bool,
}

unsafe impl Send for CanvasShm {}

impl CanvasShm {
    pub fn create(db_id: i64) -> Result<Self, String> {
        let path = canvas_shm_path(db_id);
        let c_path = CString::new(path.as_str()).map_err(|e| e.to_string())?;
        unsafe { libc::shm_unlink(c_path.as_ptr()) };
        let fd = unsafe { libc::shm_open(c_path.as_ptr(), libc::O_RDWR | libc::O_CREAT, 0o600) };
        if fd < 0 {
            return Err(format!(
                "canvas shm_open failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Self::map_fd(fd, true)
    }

    pub fn open(path: &str) -> Result<Self, String> {
        Self::open_with(path, libc::O_RDONLY, false)
    }

    pub fn open_rw(path: &str) -> Result<Self, String> {
        Self::open_with(path, libc::O_RDWR, true)
    }

    fn open_with(path: &str, flags: i32, writable: bool) -> Result<Self, String> {
        let c_path = CString::new(path).map_err(|e| e.to_string())?;
        let fd = unsafe { libc::shm_open(c_path.as_ptr(), flags, 0o600) };
        if fd < 0 {
            return Err(format!(
                "canvas shm_open failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Self::map_fd(fd, writable)
    }

    fn map_fd(fd: RawFd, writable: bool) -> Result<Self, String> {
        if writable && unsafe { libc::ftruncate(fd, CANVAS_SHM_SIZE as libc::off_t) } < 0 {
            unsafe { libc::close(fd) };
            return Err(format!(
                "canvas ftruncate failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let prot = if writable {
            libc::PROT_READ | libc::PROT_WRITE
        } else {
            libc::PROT_READ
        };
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                CANVAS_SHM_SIZE,
                prot,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        unsafe { libc::close(fd) };
        if ptr == libc::MAP_FAILED {
            return Err(format!(
                "canvas mmap failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self {
            ptr: ptr as *mut u8,
            size: CANVAS_SHM_SIZE,
            owned: writable,
        })
    }

    pub fn header(&self) -> &CanvasHeader {
        unsafe { &*(self.ptr as *const CanvasHeader) }
    }

    pub fn header_mut(&mut self) -> &mut CanvasHeader {
        assert!(self.owned, "cannot write to read-only canvas mapping");
        unsafe { &mut *(self.ptr as *mut CanvasHeader) }
    }

    /// Packed pixel buffer (full `CANVAS_MAX_BYTES`; live frame is the
    /// `width × height × 4` prefix, `stride` bytes per row).
    pub fn pixels(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.ptr.add(std::mem::size_of::<CanvasHeader>()),
                CANVAS_MAX_BYTES,
            )
        }
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        assert!(self.owned, "cannot write to read-only canvas mapping");
        unsafe {
            std::slice::from_raw_parts_mut(
                self.ptr.add(std::mem::size_of::<CanvasHeader>()),
                CANVAS_MAX_BYTES,
            )
        }
    }

    /// Zeroes the region and stamps magic/version. Call after `create()`.
    pub fn init_header(&mut self) {
        let bytes = unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) };
        bytes.fill(0);
        let hdr = self.header_mut();
        hdr.magic = CANVAS_MAGIC;
        hdr.version = CANVAS_VERSION;
    }

    /// Host side: publish a packed RGBA8 frame. Copies pixels, sets the
    /// extent, then bumps `write_seq` (fence first so the copy is visible).
    pub fn write_frame(&mut self, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
        let w = width as usize;
        let h = height as usize;
        if w == 0 || h == 0 || w > CANVAS_MAX_W || h > CANVAS_MAX_H {
            return Err(format!("bad canvas extent {w}x{h}"));
        }
        if pixels.len() != w * h * CANVAS_BPP {
            return Err(format!(
                "pixel len {} != {w}x{h}x4",
                pixels.len()
            ));
        }
        self.pixels_mut()[..pixels.len()].copy_from_slice(pixels);
        let hdr = self.header_mut();
        hdr.width = width;
        hdr.height = height;
        hdr.stride = width * CANVAS_BPP as u32;
        hdr.visible.store(1, Ordering::SeqCst);
        std::sync::atomic::fence(Ordering::SeqCst);
        hdr.write_seq.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Layer side: current frame metadata without copying pixels.
    pub fn frame_meta(&self) -> (u32, u32, u32, u32) {
        let h = self.header();
        (
            h.width,
            h.height,
            h.write_seq.load(Ordering::SeqCst),
            h.visible.load(Ordering::SeqCst),
        )
    }

    /// Host side: publish the theme cursor once. Packed premultiplied RGBA.
    pub fn publish_cursor(
        &mut self,
        width: u32,
        height: u32,
        xhot: i32,
        yhot: i32,
        pixels: &[u8],
    ) -> Result<(), String> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 || w > CURSOR_MAX || h > CURSOR_MAX {
            return Err(format!("bad cursor extent {w}x{h}"));
        }
        if pixels.len() != w * h * CANVAS_BPP {
            return Err(format!("cursor len {} != {w}x{h}x4", pixels.len()));
        }
        let hdr = self.header_mut();
        hdr.cursor_pixels[..pixels.len()].copy_from_slice(pixels);
        hdr.cursor_w = width;
        hdr.cursor_h = height;
        hdr.cursor_xhot = xhot;
        hdr.cursor_yhot = yhot;
        std::sync::atomic::fence(Ordering::SeqCst);
        hdr.cursor_seq.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Layer side: cursor metadata (w, h, xhot, yhot, seq) + pixel view.
    /// Empty when unpublished (`w == 0`).
    pub fn cursor_frame(&self) -> (u32, u32, i32, i32, u32, &[u8]) {
        let h = self.header();
        let len = h.cursor_w as usize * h.cursor_h as usize * CANVAS_BPP;
        let len = len.min(h.cursor_pixels.len());
        (
            h.cursor_w,
            h.cursor_h,
            h.cursor_xhot,
            h.cursor_yhot,
            h.cursor_seq.load(Ordering::SeqCst),
            &h.cursor_pixels[..len],
        )
    }

    /// Layer side: push one input command (producer). Wraps the 64-slot ring.
    pub fn push_command(&mut self, cmd: CanvasCommand) {
        let hdr = self.header_mut();
        let idx = hdr.cmd_write.load(Ordering::SeqCst);
        hdr.cmds[idx as usize % MAX_CANVAS_COMMANDS] = cmd;
        std::sync::atomic::fence(Ordering::SeqCst);
        hdr.cmd_write.store(idx + 1, Ordering::SeqCst);
    }

    /// Host side: drain commands written since `from_idx`. Returns the new
    /// read index alongside the commands.
    pub fn drain_commands(&self, from_idx: u32) -> (u32, Vec<CanvasCommand>) {
        let hdr = self.header();
        let write_idx = hdr.cmd_write.load(Ordering::SeqCst);
        let mut out = Vec::new();
        let mut idx = from_idx;
        while idx != write_idx {
            out.push(hdr.cmds[idx as usize % MAX_CANVAS_COMMANDS]);
            idx = idx.wrapping_add(1);
        }
        (write_idx, out)
    }

    /// Host side: push one action (producer).
    pub fn push_action(&mut self, act: CanvasAction) {
        let hdr = self.header_mut();
        let idx = hdr.act_write.load(Ordering::SeqCst);
        hdr.acts[idx as usize % MAX_CANVAS_ACTIONS] = act;
        std::sync::atomic::fence(Ordering::SeqCst);
        hdr.act_write.store(idx + 1, Ordering::SeqCst);
    }

    /// Layer side: drain actions written since `from_idx`.
    pub fn drain_actions(&self, from_idx: u32) -> (u32, Vec<CanvasAction>) {
        let hdr = self.header();
        let write_idx = hdr.act_write.load(Ordering::SeqCst);
        let mut out = Vec::new();
        let mut idx = from_idx;
        while idx != write_idx {
            out.push(hdr.acts[idx as usize % MAX_CANVAS_ACTIONS]);
            idx = idx.wrapping_add(1);
        }
        (write_idx, out)
    }
}

impl Drop for CanvasShm {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.size) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{ACT_SCREENSHOT, CMD_NAV_UP};

    const _: () = {
        assert!(CANVAS_SHM_SIZE < 4 * 1024 * 1024);
        assert!(CANVAS_MAX_BYTES == CANVAS_MAX_W * CANVAS_MAX_H * CANVAS_BPP);
    };

    fn cleanup(db_id: i64) {
        let c_path = CString::new(canvas_shm_path(db_id)).unwrap();
        unsafe { libc::shm_unlink(c_path.as_ptr()) };
    }

    #[test]
    fn test_create_open_roundtrip() {
        let db_id = 111_222_333;
        let mut shm = CanvasShm::create(db_id).unwrap();
        shm.init_header();
        shm.write_frame(2, 2, &[255; 2 * 2 * 4]).unwrap();
        let reader = CanvasShm::open(&canvas_shm_path(db_id)).unwrap();
        assert_eq!(reader.header().magic, CANVAS_MAGIC);
        assert_eq!(reader.header().version, CANVAS_VERSION);
        let (w, h, seq, vis) = reader.frame_meta();
        assert_eq!((w, h, seq, vis), (2, 2, 1, 1));
        assert_eq!(&reader.pixels()[..16], &[255; 16]);
        cleanup(db_id);
    }

    #[test]
    fn test_write_frame_rejects_bad_extent() {
        let db_id = 444_555_666;
        let mut shm = CanvasShm::create(db_id).unwrap();
        shm.init_header();
        assert!(shm.write_frame(0, 10, &[]).is_err());
        assert!(shm.write_frame(9999, 10, &[]).is_err());
        assert!(shm.write_frame(2, 2, &[0; 4]).is_err());
        cleanup(db_id);
    }

    #[test]
    fn test_command_ring_wraps() {
        let db_id = 777_888_999;
        let mut shm = CanvasShm::create(db_id).unwrap();
        shm.init_header();
        for _ in 0..(MAX_CANVAS_COMMANDS as u32 + 5) {
            shm.push_command(CanvasCommand::nav(CMD_NAV_UP));
        }
        let reader = CanvasShm::open(&canvas_shm_path(db_id)).unwrap();
        let (next, cmds) = reader.drain_commands(0);
        assert_eq!(next, MAX_CANVAS_COMMANDS as u32 + 5);
        assert_eq!(cmds.len(), MAX_CANVAS_COMMANDS + 5);
        assert!(cmds.iter().all(|c| c.kind == CMD_NAV_UP));
        cleanup(db_id);
    }

    #[test]
    fn test_action_ring_drain() {
        let db_id = 123_123_123;
        let mut shm = CanvasShm::create(db_id).unwrap();
        shm.init_header();
        shm.push_action(CanvasAction::of(ACT_SCREENSHOT));
        let reader = CanvasShm::open(&canvas_shm_path(db_id)).unwrap();
        let (next, acts) = reader.drain_actions(0);
        assert_eq!((next, acts.len()), (1, 1));
        assert_eq!(acts[0].kind, ACT_SCREENSHOT);
        let (next2, acts2) = reader.drain_actions(next);
        assert_eq!((next2, acts2.len()), (1, 0));
        cleanup(db_id);
    }

    #[test]
    fn test_panel_rect_anchors() {
        // 512×800 panel on 1920×1080: fits unscaled.
        assert_eq!(
            canvas_panel_rect(1920, 1080, 512, 800, 0),
            (16.0, 16.0, 512.0, 800.0)
        );
        assert_eq!(
            canvas_panel_rect(1920, 1080, 512, 800, 1),
            (1920.0 - 16.0 - 512.0, 16.0, 512.0, 800.0)
        );
        assert_eq!(
            canvas_panel_rect(1920, 1080, 512, 800, 3),
            (1920.0 - 16.0 - 512.0, 1080.0 - 16.0 - 800.0, 512.0, 800.0)
        );
        assert_eq!(
            canvas_panel_rect(1920, 1080, 512, 800, 4),
            ((1920.0 - 512.0) / 2.0, (1080.0 - 800.0) / 2.0, 512.0, 800.0)
        );
    }

    #[test]
    fn test_panel_rect_fit_scales_small_screen() {
        // 512×800 panel on 1280×720: scale = min(1248/512, 688/800) = 0.86.
        let (x, y, w, h) = canvas_panel_rect(1280, 720, 512, 800, 0);
        assert!((w - 512.0 * 0.86).abs() < 0.01);
        assert!((h - 800.0 * 0.86).abs() < 0.01);
        assert_eq!((x, y), (16.0, 16.0));
        assert!(x + w <= 1280.0 && y + h <= 720.0);
    }

    #[test]
    fn test_fit_panel_size_1080p() {
        // 1920×1080: scale = min(1888/512, 1048/800) = 1.31 → 670×1048.
        assert_eq!(fit_panel_size(1920, 1080), (670, 1048));
    }

    #[test]
    fn test_fit_panel_size_shrinks_small_screen() {
        // 800×600: scale = min(768/512, 568/800) = 0.71 → 363×568.
        assert_eq!(fit_panel_size(800, 600), (363, 568));
    }

    #[test]
    fn test_fit_panel_size_clamps_to_max() {
        // 4K would ask for 1362×2128: clamped to the canvas maximum.
        assert_eq!(
            fit_panel_size(3840, 2160),
            (CANVAS_MAX_W as u32, CANVAS_MAX_H as u32)
        );
    }

    #[test]
    fn test_fit_panel_size_keeps_aspect() {
        let (w, h) = fit_panel_size(1920, 1080);
        let ratio = w as f64 / h as f64;
        assert!((ratio - 512.0 / 800.0).abs() < 0.01);
    }

    #[test]
    fn test_publish_cursor_roundtrip() {
        let db_id = 313131313;
        let mut shm = CanvasShm::create(db_id).unwrap();
        shm.init_header();
        let reader = CanvasShm::open(&canvas_shm_path(db_id)).unwrap();
        assert_eq!(reader.cursor_frame().4, 0);
        assert_eq!(reader.cursor_frame().0, 0);
        shm.publish_cursor(2, 2, 1, 1, &[255; 2 * 2 * 4]).unwrap();
        let (w, h, xhot, yhot, seq, px) = reader.cursor_frame();
        assert_eq!((w, h, xhot, yhot, seq), (2, 2, 1, 1, 1));
        assert_eq!(px, &[255; 16]);
        assert!(shm.publish_cursor(0, 2, 0, 0, &[]).is_err());
        assert!(shm.publish_cursor(999, 2, 0, 0, &[]).is_err());
        cleanup(db_id);
    }
}
