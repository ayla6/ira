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

use std::sync::atomic::{AtomicU32, Ordering};

use crate::commands::{CanvasAction, CanvasCommand, MAX_CANVAS_ACTIONS, MAX_CANVAS_COMMANDS};
use crate::map::{map_fd, open_fd, shm_file_path};

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

/// Filesystem path for debugging (`/dev/shm/<name>`).
pub fn canvas_shm_file_path(name: &str) -> String {
    shm_file_path(name)
}

/// RAII wrapper around the mapped canvas region. Unmaps on drop.
pub struct CanvasShm {
    ptr: *mut u8,
    size: usize,
    writable: bool,
}

unsafe impl Send for CanvasShm {}

impl CanvasShm {
    pub fn create(db_id: i64) -> Result<Self, String> {
        let path = canvas_shm_path(db_id);
        let c_path =
            std::ffi::CString::new(path.as_str()).map_err(|e| e.to_string())?;
        unsafe { libc::shm_unlink(c_path.as_ptr()) };
        let fd = open_fd(&path, libc::O_RDWR | libc::O_CREAT, "canvas")?;
        Self::map_shm_fd(fd, true)
    }

    pub fn open(path: &str) -> Result<Self, String> {
        Self::open_with(path, libc::O_RDONLY, false)
    }

    pub fn open_rw(path: &str) -> Result<Self, String> {
        Self::open_with(path, libc::O_RDWR, true)
    }

    fn open_with(path: &str, flags: i32, writable: bool) -> Result<Self, String> {
        let fd = open_fd(path, flags, "canvas")?;
        Self::map_shm_fd(fd, writable)
    }

    fn map_shm_fd(fd: std::os::unix::io::RawFd, writable: bool) -> Result<Self, String> {
        let ptr = map_fd(fd, CANVAS_SHM_SIZE, writable, "canvas")?;
        Ok(Self {
            ptr,
            size: CANVAS_SHM_SIZE,
            writable,
        })
    }

    fn assert_writable(&self) {
        assert!(self.writable, "cannot write to read-only canvas mapping");
    }

    pub fn header(&self) -> &CanvasHeader {
        unsafe { &*(self.ptr as *const CanvasHeader) }
    }

    pub fn header_mut(&mut self) -> &mut CanvasHeader {
        self.assert_writable();
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
        self.assert_writable();
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
    /// Empty when unpublished (`w == 0`). Corrupt extents clamp to the
    /// published buffer instead of overflowing the length math.
    pub fn cursor_frame(&self) -> (u32, u32, i32, i32, u32, &[u8]) {
        let h = self.header();
        let len = (h.cursor_w as usize)
            .checked_mul(h.cursor_h as usize)
            .and_then(|v| v.checked_mul(CANVAS_BPP))
            .unwrap_or(usize::MAX)
            .min(h.cursor_pixels.len());
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
        push_ring(&mut hdr.cmds, &hdr.cmd_write, cmd);
    }

    /// Host side: drain commands written since `from_idx`. Returns the new
    /// read index alongside the commands.
    pub fn drain_commands(&self, from_idx: u32) -> (u32, Vec<CanvasCommand>) {
        let hdr = self.header();
        drain_ring(&hdr.cmds, &hdr.cmd_write, from_idx)
    }

    /// Host side: push one action (producer).
    pub fn push_action(&mut self, act: CanvasAction) {
        let hdr = self.header_mut();
        push_ring(&mut hdr.acts, &hdr.act_write, act);
    }

    /// Layer side: drain actions written since `from_idx`.
    pub fn drain_actions(&self, from_idx: u32) -> (u32, Vec<CanvasAction>) {
        let hdr = self.header();
        drain_ring(&hdr.acts, &hdr.act_write, from_idx)
    }
}

fn push_ring<T: Copy, const N: usize>(
    slots: &mut [T; N],
    write_idx: &AtomicU32,
    val: T,
) {
    let idx = write_idx.load(Ordering::SeqCst);
    slots[idx as usize % N] = val;
    std::sync::atomic::fence(Ordering::SeqCst);
    write_idx.store(idx + 1, Ordering::SeqCst);
}

fn drain_ring<T: Copy, const N: usize>(
    slots: &[T; N],
    write_idx: &AtomicU32,
    from_idx: u32,
) -> (u32, Vec<T>) {
    let write = write_idx.load(Ordering::SeqCst);
    let mut out = Vec::new();
    let mut idx = from_idx;
    while idx != write {
        out.push(slots[idx as usize % N]);
        idx = idx.wrapping_add(1);
    }
    (write, out)
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
    use std::ffi::CString;

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

    #[test]
    fn test_cursor_frame_clamps_corrupt_extent() {
        let db_id = 414141414;
        let mut shm = CanvasShm::create(db_id).unwrap();
        shm.init_header();
        shm.header_mut().cursor_w = u32::MAX;
        shm.header_mut().cursor_h = u32::MAX;
        let reader = CanvasShm::open(&canvas_shm_path(db_id)).unwrap();
        let (_, _, _, _, _, px) = reader.cursor_frame();
        assert_eq!(px.len(), CURSOR_MAX_BYTES);
        cleanup(db_id);
    }
}
