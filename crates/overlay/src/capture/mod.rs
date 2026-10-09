//! Frame capture — staging buffers, lifecycle, and public API.
//!
//! Split by responsibility (one job per file, no "and"):
//! - `record.rs`: present-thread record path (`capture`, readback polling,
//!   GPU resource construction).
//! - `threads.rs`: readback/encode threads, ffmpeg pipes, recording and
//!   replay-buffer control.
//! - `convert.rs`: staged-bytes → RGBA8 conversion + screenshot encoding.
//!
//! This file owns the shared state, the lifecycle (`init`/`destroy`), and
//! re-exports the public API so `ira_overlay::capture::*` paths don't move.

mod convert;
mod record;
mod resources;
mod threads;

pub use record::{capture, check_and_readback, drain_pending};
pub use threads::toggle_recording;

use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ash::vk;
use ira_overlay_ipc::MappedShm;

use crate::types::DeviceFns;

use record::{check_blit_support, src_bpp};
use resources::{create_intermediate_image, create_staging_buffers};
use threads::{encode_thread, readback_thread, start_replay_buffer, stop_replay_buffer};

struct StagingBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    ptr: *mut u8,
    size: u64,
}

unsafe impl Send for StagingBuffer {}

struct IntermediateImage {
    image: vk::Image,
    memory: vk::DeviceMemory,
}

unsafe impl Send for IntermediateImage {}

/// A staged frame waiting for its fence: the staging allocation travels
/// with the frame (no field-by-field rebuilds when recycling).
struct PendingFrame {
    staging: StagingBuffer,
    width: u32,
    height: u32,
    use_blit: bool,
    src_format: vk::Format,
    fence: vk::Fence,
    is_screenshot: bool,
}

unsafe impl Send for PendingFrame {}

struct EncodeFrame {
    rgba: Vec<u8>,
    is_screenshot: bool,
    width: u32,
    height: u32,
}

struct State {
    fns: DeviceFns,
    device: vk::Device,
    extent: vk::Extent2D,
    format: vk::Format,
    use_blit: bool,
    staging_size: u64,
    free: Vec<StagingBuffer>,
    pending: Vec<PendingFrame>,
    intermediate: Option<IntermediateImage>,
    ready_tx: Sender<PendingFrame>,
    encode_tx: SyncSender<EncodeFrame>,
    free_rx: Receiver<StagingBuffer>,
    screenshot_requested: bool,
    readback_handle: std::thread::JoinHandle<()>,
    encode_handle: std::thread::JoinHandle<()>,
    shutdown: Arc<AtomicBool>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static DEFERRED_BUFFERS: Mutex<Vec<StagingBuffer>> = Mutex::new(Vec::new());
static DEFERRED_IMAGES: Mutex<Vec<IntermediateImage>> = Mutex::new(Vec::new());
static RECORDING: AtomicBool = AtomicBool::new(false);
static FFMPEG_PIPE: Mutex<Option<ChildStdin>> = Mutex::new(None);
static REPLAY_FFMPEG_PIPE: Mutex<Option<ChildStdin>> = Mutex::new(None);
static FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
static RECORD_START: Mutex<Option<Instant>> = Mutex::new(None);

pub fn request_screenshot() {
    if let Ok(mut s) = STATE.lock() {
        if let Some(state) = s.as_mut() {
            state.screenshot_requested = true;
        }
    }
}

pub fn is_screenshot_requested() -> bool {
    STATE
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|s| s.screenshot_requested)
}

pub fn is_recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

pub fn is_capture_active() -> bool {
    is_screenshot_requested() || is_recording() || threads::replay_buffer_active()
}

fn mark_direct_capture_ready() {
    let Ok(path) = std::env::var("IRA_OVERLAY_SHM") else {
        return;
    };
    match MappedShm::open_rw(&path) {
        Ok(shm) => shm.header().direct_capture_ready.store(1, Ordering::SeqCst),
        Err(error) => eprintln!("ira-overlay: failed to mark direct capture ready: {error}"),
    }
}

/// Stops an active recording: drops the pipe and logs the tally.
/// Callers gate on the RECORDING flag; `reason` disambiguates the log
/// ("", or " (resize)").
fn stop_recording(reason: &str) {
    *FFMPEG_PIPE.lock().unwrap() = None;
    let frames = FRAME_COUNT.swap(0, Ordering::Relaxed);
    if let Some(start) = RECORD_START.lock().unwrap().take() {
        let dur = start.elapsed();
        eprintln!(
            "ira-overlay: recording stopped{reason}, {} frames in {:.1}s ({:.0} fps avg)",
            frames,
            dur.as_secs_f64(),
            frames as f64 / dur.as_secs_f64().max(0.001)
        );
    }
}

/// Defers every live allocation (free list, in-flight receives, and the
/// intermediate image) for destruction once the GPU is done with it.
/// Takes field pieces (not `&mut State`) so `destroy` can reuse it after
/// the thread handles have been moved out by `join`.
fn defer_buffers(
    free_rx: &Receiver<StagingBuffer>,
    free: &mut Vec<StagingBuffer>,
    intermediate: &mut Option<IntermediateImage>,
) {
    while let Ok(staging) = free_rx.try_recv() {
        DEFERRED_BUFFERS.lock().unwrap().push(staging);
    }
    for staging in free.drain(..) {
        DEFERRED_BUFFERS.lock().unwrap().push(staging);
    }
    if let Some(img) = intermediate.take() {
        DEFERRED_IMAGES.lock().unwrap().push(img);
    }
}

fn defer_all(state: &mut State) {
    defer_buffers(&state.free_rx, &mut state.free, &mut state.intermediate);
}

/// The caller rebuilds after.
fn teardown_for_resize(state: &mut State) {
    defer_all(state);
    if RECORDING.swap(false, Ordering::Relaxed) {
        stop_recording(" (resize)");
    }
    stop_replay_buffer();
}

/// (Re)builds staging + intermediate for an extent. None when allocation
/// fails — the game presents without capture.
fn build_resources(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    extent: vk::Extent2D,
    use_blit: bool,
    buf_size: u64,
) -> Option<(Vec<StagingBuffer>, Option<IntermediateImage>)> {
    let staging = create_staging_buffers(fns, device, physical_device, buf_size);
    if staging.is_empty() {
        return None;
    }
    let intermediate = if use_blit {
        unsafe { create_intermediate_image(fns, device, physical_device, extent) }
    } else {
        None
    };
    Some((staging, intermediate))
}

pub fn init(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    extent: vk::Extent2D,
    format: vk::Format,
) {
    free_deferred(fns, device);

    let use_blit = check_blit_support(fns, physical_device, format);
    let bpp = if use_blit { 4 } else { src_bpp(format) };
    let buf_size = (extent.width as u64) * (extent.height as u64) * bpp;

    let mut guard = STATE.lock().unwrap();
    if let Some(state) = guard.as_mut() {
        if state.extent.width == extent.width
            && state.extent.height == extent.height
            && state.use_blit == use_blit
        {
            mark_direct_capture_ready();
            return;
        }
        teardown_for_resize(state);

        let Some((staging, intermediate)) =
            build_resources(fns, device, physical_device, extent, use_blit, buf_size)
        else {
            return;
        };

        state.extent = extent;
        state.format = format;
        state.use_blit = use_blit;
        state.staging_size = buf_size;
        state.free = staging;
        state.intermediate = intermediate;
        drop(guard);
        mark_direct_capture_ready();
        start_replay_buffer(extent);
        return;
    }
    drop(guard);

    let Some((staging, intermediate)) =
        build_resources(fns, device, physical_device, extent, use_blit, buf_size)
    else {
        return;
    };

    let (ready_tx, ready_rx) = channel::<PendingFrame>();
    let (free_tx, free_rx) = channel::<StagingBuffer>();
    let (encode_tx, encode_rx) = sync_channel::<EncodeFrame>(3);

    let shutdown = Arc::new(AtomicBool::new(false));
    let rb_shutdown = shutdown.clone();
    let rb_encode_tx = encode_tx.clone();
    let readback_handle = std::thread::spawn(move || {
        readback_thread(ready_rx, free_tx, rb_encode_tx, rb_shutdown);
    });

    let enc_shutdown = shutdown.clone();
    let encode_handle = std::thread::spawn(move || {
        encode_thread(encode_rx, enc_shutdown);
    });

    *STATE.lock().unwrap() = Some(State {
        fns,
        device,
        extent,
        format,
        use_blit,
        staging_size: buf_size,
        free: staging,
        pending: Vec::new(),
        intermediate,
        ready_tx,
        encode_tx,
        free_rx,
        screenshot_requested: false,
        readback_handle,
        encode_handle,
        shutdown,
    });
    mark_direct_capture_ready();
    start_replay_buffer(extent);
}

pub fn destroy(fns: DeviceFns, device: vk::Device) {
    if RECORDING.swap(false, Ordering::Relaxed) {
        stop_recording("");
    }
    stop_replay_buffer();

    let state = STATE.lock().unwrap().take();
    let Some(mut state) = state else { return };

    state.shutdown.store(true, Ordering::Relaxed);
    drop(state.ready_tx);
    drop(state.encode_tx);
    let _ = state.readback_handle.join();
    let _ = state.encode_handle.join();

    // Collect every allocation into deferred via the shared helper, then
    // drain in-flight frames (teardown-only: the device is going away).
    defer_buffers(&state.free_rx, &mut state.free, &mut state.intermediate);
    let mut deferred = DEFERRED_BUFFERS.lock().unwrap();
    for frame in state.pending.drain(..) {
        deferred.push(frame.staging);
    }
    drop(deferred);

    free_deferred(fns, device);
}

pub fn free_deferred(fns: DeviceFns, device: vk::Device) {
    let buffers = std::mem::take(&mut *DEFERRED_BUFFERS.lock().unwrap());
    for s in &buffers {
        unsafe {
            (fns.unmap_memory)(device, s.memory);
            (fns.destroy_buffer)(device, s.buffer, std::ptr::null());
            (fns.free_memory)(device, s.memory, std::ptr::null());
        }
    }
    let images = std::mem::take(&mut *DEFERRED_IMAGES.lock().unwrap());
    for img in &images {
        unsafe {
            (fns.destroy_image)(device, img.image, std::ptr::null());
            (fns.free_memory)(device, img.memory, std::ptr::null());
        }
    }
}
