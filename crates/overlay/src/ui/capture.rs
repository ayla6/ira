use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ash::vk;

use crate::types::DeviceFns;

const NUM_STAGING: usize = 4;

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

struct PendingFrame {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    ptr: *mut u8,
    size: u64,
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
    _readback_thread: std::thread::JoinHandle<()>,
    _encode_thread: std::thread::JoinHandle<()>,
    shutdown: Arc<AtomicBool>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static DEFERRED_BUFFERS: Mutex<Vec<StagingBuffer>> = Mutex::new(Vec::new());
static DEFERRED_IMAGES: Mutex<Vec<IntermediateImage>> = Mutex::new(Vec::new());
static RECORDING: AtomicBool = AtomicBool::new(false);
static FFMPEG_PIPE: Mutex<Option<std::process::ChildStdin>> = Mutex::new(None);
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

pub fn toggle_recording() {
    if RECORDING.fetch_xor(true, Ordering::Relaxed) {
        let mut pipe = FFMPEG_PIPE.lock().unwrap();
        *pipe = None;
        let frames = FRAME_COUNT.swap(0, Ordering::Relaxed);
        if let Some(start) = RECORD_START.lock().unwrap().take() {
            let dur = start.elapsed();
            eprintln!(
                "ira-overlay: recording stopped, {} frames in {:.1}s ({:.0} fps avg)",
                frames,
                dur.as_secs_f64(),
                frames as f64 / dur.as_secs_f64().max(0.001)
            );
        }
    } else {
        let (extent, has_state) = {
            let s = STATE.lock().unwrap();
            (s.as_ref().map(|s| s.extent), s.is_some())
        };
        if !has_state {
            RECORDING.store(false, Ordering::Relaxed);
            return;
        }
        let Some(extent) = extent else {
            RECORDING.store(false, Ordering::Relaxed);
            return;
        };

        let path = video_path();
        let size = format!("{}x{}", extent.width, extent.height);
        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-y",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            &size,
            "-use_wallclock_as_timestamps",
            "1",
            "-i",
            "-",
            "-c:v",
            "libx264",
            "-crf",
            "18",
            "-preset",
            "fast",
            "-pix_fmt",
            "yuv420p",
            "-r",
            "60",
            "-loglevel",
            "error",
        ]);
        cmd.arg(&path);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        match cmd.spawn() {
            Ok(mut child) => {
                if let Some(stdin) = child.stdin.take() {
                    *FFMPEG_PIPE.lock().unwrap() = Some(stdin);
                    *RECORD_START.lock().unwrap() = Some(Instant::now());
                    FRAME_COUNT.store(0, Ordering::Relaxed);
                    eprintln!("ira-overlay: recording to {}", path.display());
                }
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => {
                RECORDING.store(false, Ordering::Relaxed);
                eprintln!("ira-overlay: failed to start ffmpeg: {e}");
            }
        }
    }
}

fn src_bpp(format: vk::Format) -> u64 {
    match format {
        vk::Format::R16G16B16A16_SFLOAT | vk::Format::R16G16B16A16_UNORM => 8,
        _ => 4,
    }
}

fn check_blit_support(
    fns: DeviceFns,
    physical_device: vk::PhysicalDevice,
    format: vk::Format,
) -> bool {
    let mut props = vk::FormatProperties::default();
    unsafe {
        (fns.get_format_props)(physical_device, format, &mut props);
    }
    props
        .optimal_tiling_features
        .contains(vk::FormatFeatureFlags::BLIT_SRC)
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
            return;
        }
        let mut deferred = DEFERRED_BUFFERS.lock().unwrap();
        while let Ok(s) = state.free_rx.try_recv() {
            deferred.push(s);
        }
        for s in state.free.drain(..) {
            deferred.push(s);
        }
        drop(deferred);

        if let Some(img) = state.intermediate.take() {
            DEFERRED_IMAGES.lock().unwrap().push(img);
        }

        if RECORDING.swap(false, Ordering::Relaxed) {
            *FFMPEG_PIPE.lock().unwrap() = None;
            let frames = FRAME_COUNT.swap(0, Ordering::Relaxed);
            if let Some(start) = RECORD_START.lock().unwrap().take() {
                let dur = start.elapsed();
                eprintln!(
                    "ira-overlay: recording stopped (resize), {} frames in {:.1}s ({:.0} fps avg)",
                    frames,
                    dur.as_secs_f64(),
                    frames as f64 / dur.as_secs_f64().max(0.001)
                );
            }
        }

        let staging = create_staging_buffers(fns, device, physical_device, buf_size);
        if staging.is_empty() {
            return;
        }
        let intermediate = if use_blit {
            unsafe { create_intermediate_image(fns, device, physical_device, extent) }
        } else {
            None
        };

        state.extent = extent;
        state.format = format;
        state.use_blit = use_blit;
        state.staging_size = buf_size;
        state.free = staging;
        state.intermediate = intermediate;
        return;
    }
    drop(guard);

    let staging = create_staging_buffers(fns, device, physical_device, buf_size);
    if staging.is_empty() {
        return;
    }
    let intermediate = if use_blit {
        unsafe { create_intermediate_image(fns, device, physical_device, extent) }
    } else {
        None
    };

    let (ready_tx, ready_rx) = channel::<PendingFrame>();
    let (free_tx, free_rx) = channel::<StagingBuffer>();
    let (encode_tx, encode_rx) = sync_channel::<EncodeFrame>(3);

    let shutdown = Arc::new(AtomicBool::new(false));
    let rb_shutdown = shutdown.clone();
    let rb_encode_tx = encode_tx.clone();
    let readback_thread = std::thread::spawn(move || {
        readback_thread(ready_rx, free_tx, rb_encode_tx, rb_shutdown);
    });

    let enc_shutdown = shutdown.clone();
    let encode_thread = std::thread::spawn(move || {
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
        _readback_thread: readback_thread,
        _encode_thread: encode_thread,
        shutdown,
    });
}

pub fn destroy(_fns: DeviceFns, _device: vk::Device) {
    if RECORDING.swap(false, Ordering::Relaxed) {
        *FFMPEG_PIPE.lock().unwrap() = None;
        let frames = FRAME_COUNT.swap(0, Ordering::Relaxed);
        if let Some(start) = RECORD_START.lock().unwrap().take() {
            let dur = start.elapsed();
            eprintln!(
                "ira-overlay: recording stopped, {} frames in {:.1}s ({:.0} fps avg)",
                frames,
                dur.as_secs_f64(),
                frames as f64 / dur.as_secs_f64().max(0.001)
            );
        }
    }

    let state = STATE.lock().unwrap().take();
    let Some(state) = state else { return };

    state.shutdown.store(true, Ordering::Relaxed);
    drop(state.ready_tx);
    drop(state.encode_tx);
    let _ = state._readback_thread.join();
    let _ = state._encode_thread.join();

    let mut deferred = DEFERRED_BUFFERS.lock().unwrap();
    while let Ok(s) = state.free_rx.try_recv() {
        deferred.push(s);
    }
    for s in &state.free {
        deferred.push(StagingBuffer {
            buffer: s.buffer,
            memory: s.memory,
            ptr: s.ptr,
            size: s.size,
        });
    }
    for p in &state.pending {
        deferred.push(StagingBuffer {
            buffer: p.buffer,
            memory: p.memory,
            ptr: p.ptr,
            size: p.size,
        });
    }
    drop(deferred);

    if let Some(img) = state.intermediate {
        DEFERRED_IMAGES.lock().unwrap().push(img);
    }
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

pub fn drain_pending() {
    let mut guard = STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else { return };
    let expected = state.staging_size;
    while let Ok(s) = state.free_rx.try_recv() {
        if s.size == expected {
            state.free.push(s);
        } else {
            DEFERRED_BUFFERS.lock().unwrap().push(s);
        }
    }
    for frame in state.pending.drain(..) {
        let _ = state.ready_tx.send(frame);
    }
}

/// # Safety
/// `cmd` must be in the recording state. `src_image` must be a valid swapchain
/// image with `TRANSFER_SRC` usage. `fence` must be unsignaled.
pub unsafe fn capture(
    cmd: vk::CommandBuffer,
    src_image: vk::Image,
    fence: vk::Fence,
    image_extent: vk::Extent2D,
) -> bool {
    let mut guard = STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else {
        return false;
    };

    if image_extent.width != state.extent.width || image_extent.height != state.extent.height {
        return false;
    }

    let is_screenshot = state.screenshot_requested;
    if !is_screenshot && !RECORDING.load(Ordering::Relaxed) {
        return false;
    }

    let Some(staging) = state.free.pop() else {
        return false;
    };

    if is_screenshot {
        state.screenshot_requested = false;
    }

    let fns = state.fns;
    let extent = state.extent;
    let use_blit = state.use_blit;
    let src_format = state.format;

    let color_sub = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };

    let src_barrier = vk::ImageMemoryBarrier::default()
        .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .src_access_mask(vk::AccessFlags::NONE)
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(src_image)
        .subresource_range(color_sub);

    if use_blit {
        let intermediate = state
            .intermediate
            .as_ref()
            .expect("blit enabled but no intermediate");
        let dst_barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_access_mask(vk::AccessFlags::NONE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(intermediate.image)
            .subresource_range(color_sub);

        let barriers = [src_barrier, dst_barrier];
        (fns.cmd_pipeline_barrier)(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            2,
            barriers.as_ptr(),
        );

        let blit_region = vk::ImageBlit::default()
            .src_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_offsets([
                vk::Offset3D { x: 0, y: 0, z: 0 },
                vk::Offset3D {
                    x: extent.width as i32,
                    y: extent.height as i32,
                    z: 1,
                },
            ])
            .dst_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .dst_offsets([
                vk::Offset3D { x: 0, y: 0, z: 0 },
                vk::Offset3D {
                    x: extent.width as i32,
                    y: extent.height as i32,
                    z: 1,
                },
            ]);

        (fns.cmd_blit_image)(
            cmd,
            src_image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            intermediate.image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            1,
            &blit_region,
            vk::Filter::LINEAR,
        );

        let post_blit = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(intermediate.image)
            .subresource_range(color_sub);

        (fns.cmd_pipeline_barrier)(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &post_blit,
        );

        let copy_region = vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(extent.width)
            .buffer_image_height(extent.height)
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            });

        (fns.cmd_copy_image_to_buffer)(
            cmd,
            intermediate.image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            staging.buffer,
            1,
            &copy_region,
        );
    } else {
        (fns.cmd_pipeline_barrier)(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &src_barrier,
        );

        let copy_region = vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(extent.width)
            .buffer_image_height(extent.height)
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            });

        (fns.cmd_copy_image_to_buffer)(
            cmd,
            src_image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            staging.buffer,
            1,
            &copy_region,
        );
    }

    state.pending.push(PendingFrame {
        buffer: staging.buffer,
        memory: staging.memory,
        ptr: staging.ptr,
        size: staging.size,
        width: extent.width,
        height: extent.height,
        use_blit,
        src_format,
        fence,
        is_screenshot,
    });

    true
}

pub fn check_and_readback() {
    let mut guard = STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else { return };

    let expected = state.staging_size;
    while let Ok(staging) = state.free_rx.try_recv() {
        if staging.size == expected {
            state.free.push(staging);
        } else {
            DEFERRED_BUFFERS.lock().unwrap().push(staging);
        }
    }

    let fns = state.fns;
    let device = state.device;

    let mut still_pending = Vec::new();
    for frame in state.pending.drain(..) {
        let status = unsafe { (fns.get_fence_status)(device, frame.fence) };
        if status == vk::Result::SUCCESS {
            let _ = state.ready_tx.send(frame);
        } else {
            still_pending.push(frame);
        }
    }
    state.pending = still_pending;
}

fn readback_thread(
    ready_rx: Receiver<PendingFrame>,
    free_tx: Sender<StagingBuffer>,
    encode_tx: SyncSender<EncodeFrame>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Relaxed) {
        match ready_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(frame) => {
                let rgba = unsafe { read_pixels(&frame) };
                let _ = free_tx.send(StagingBuffer {
                    buffer: frame.buffer,
                    memory: frame.memory,
                    ptr: frame.ptr,
                    size: frame.size,
                });
                match encode_tx.try_send(EncodeFrame {
                    rgba,
                    is_screenshot: frame.is_screenshot,
                    width: frame.width,
                    height: frame.height,
                }) {
                    Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
                }
            }
            Err(_) => continue,
        }
    }
}

fn encode_thread(encode_rx: Receiver<EncodeFrame>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Relaxed) {
        match encode_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(frame) => {
                if frame.is_screenshot {
                    encode_webp(frame.rgba, frame.width, frame.height);
                } else if RECORDING.load(Ordering::Relaxed) {
                    let pipe = FFMPEG_PIPE.lock().unwrap().take();
                    if let Some(mut pipe) = pipe {
                        let _ = pipe.write_all(&frame.rgba);
                        FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
                        if RECORDING.load(Ordering::Relaxed) {
                            *FFMPEG_PIPE.lock().unwrap() = Some(pipe);
                        }
                    }
                }
            }
            Err(_) => continue,
        }
    }
}

unsafe fn read_pixels(frame: &PendingFrame) -> Vec<u8> {
    if frame.use_blit {
        let total = (frame.width as usize) * (frame.height as usize) * 4;
        let mut rgba = vec![0u8; total];
        std::ptr::copy_nonoverlapping(frame.ptr, rgba.as_mut_ptr(), total);
        return rgba;
    }
    read_pixels_fallback(frame)
}

unsafe fn read_pixels_fallback(frame: &PendingFrame) -> Vec<u8> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let bpp = src_bpp(frame.src_format) as usize;
    let src_size = w * h * bpp;
    let row_pitch = w * bpp;

    let mut local = vec![0u8; src_size];
    std::ptr::copy_nonoverlapping(frame.ptr, local.as_mut_ptr(), src_size);

    match frame.src_format {
        vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB => {
            convert_8bpp(&local, w, h, row_pitch, true)
        }
        vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB => {
            convert_8bpp(&local, w, h, row_pitch, false)
        }
        vk::Format::R16G16B16A16_SFLOAT | vk::Format::R16G16B16A16_UNORM => {
            convert_f16(&local, w, h, row_pitch)
        }
        vk::Format::A2B10G10R10_UNORM_PACK32 | vk::Format::A2R10G10B10_UNORM_PACK32 => {
            convert_10bpp(&local, w, h, row_pitch, frame.src_format)
        }
        _ => Vec::new(),
    }
}

fn convert_8bpp(src: &[u8], w: usize, h: usize, row_pitch: usize, swap_bgr: bool) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &src[y * row_pitch..y * row_pitch + w * 4];
        if swap_bgr {
            let (chunks, _) = row.as_chunks::<4>();
            for chunk in chunks {
                rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
            }
        } else {
            rgba.extend_from_slice(row);
        }
    }
    rgba
}

fn convert_f16(src: &[u8], w: usize, h: usize, row_pitch: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &src[y * row_pitch..y * row_pitch + w * 8];
        let (chunks, _) = row.as_chunks::<8>();
        for px in chunks {
            let r = u16::from_le_bytes([px[0], px[1]]);
            let g = u16::from_le_bytes([px[2], px[3]]);
            let b = u16::from_le_bytes([px[4], px[5]]);
            rgba.extend_from_slice(&[f16_to_u8(r), f16_to_u8(g), f16_to_u8(b), 255]);
        }
    }
    rgba
}

fn convert_10bpp(src: &[u8], w: usize, h: usize, row_pitch: usize, format: vk::Format) -> Vec<u8> {
    let swap = format == vk::Format::A2B10G10R10_UNORM_PACK32;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &src[y * row_pitch..y * row_pitch + w * 4];
        let (chunks, _) = row.as_chunks::<4>();
        for px in chunks {
            let packed = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
            let r10 = packed & 0x3ff;
            let g10 = (packed >> 10) & 0x3ff;
            let b10 = (packed >> 20) & 0x3ff;
            let a2 = (packed >> 30) & 0x3;
            let (r, b) = if swap { (b10, r10) } else { (r10, b10) };
            rgba.extend_from_slice(&[
                (r >> 2) as u8,
                (g10 >> 2) as u8,
                (b >> 2) as u8,
                (a2 << 6) as u8,
            ]);
        }
    }
    rgba
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1f) as u32;
    let mant = (bits & 0x3ff) as u32;
    if exp == 0 {
        return f32::from_bits(sign << 31);
    }
    if exp == 31 {
        return f32::from_bits((sign << 31) | (0xff << 23) | (mant << 13));
    }
    f32::from_bits((sign << 31) | ((exp + 112) << 23) | (mant << 13))
}

fn f16_to_u8(bits: u16) -> u8 {
    (f16_to_f32(bits).clamp(0.0, 1.0) * 255.0).round() as u8
}

fn create_staging_buffers(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    buf_size: u64,
) -> Vec<StagingBuffer> {
    let mut buffers = Vec::with_capacity(NUM_STAGING);
    for _ in 0..NUM_STAGING {
        let Some(staging) =
            (unsafe { create_one_staging_buffer(fns, device, physical_device, buf_size) })
        else {
            break;
        };
        buffers.push(staging);
    }
    buffers
}

unsafe fn create_one_staging_buffer(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    size: u64,
) -> Option<StagingBuffer> {
    let info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(vk::BufferUsageFlags::TRANSFER_DST)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut buffer = vk::Buffer::null();
    if (fns.create_buffer)(device, &info, std::ptr::null(), &mut buffer) != vk::Result::SUCCESS {
        return None;
    }

    let mut reqs = vk::MemoryRequirements::default();
    (fns.get_buffer_memory_requirements)(device, buffer, &mut reqs);

    let mut mem_props = vk::PhysicalDeviceMemoryProperties::default();
    (fns.get_mem_props)(physical_device, &mut mem_props);

    let find_mem =
        |require: vk::MemoryPropertyFlags, avoid: vk::MemoryPropertyFlags| -> Option<u32> {
            for i in 0..mem_props.memory_type_count as usize {
                if (reqs.memory_type_bits & (1 << i)) != 0 {
                    let flags = mem_props.memory_types[i].property_flags;
                    if flags.contains(require) && !flags.intersects(avoid) {
                        return Some(i as u32);
                    }
                }
            }
            None
        };

    let mem_type = find_mem(
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .or_else(|| {
        find_mem(
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            vk::MemoryPropertyFlags::empty(),
        )
    });
    let mem_type = mem_type?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .memory_type_index(mem_type);
    let mut memory = vk::DeviceMemory::null();
    if (fns.allocate_memory)(device, &alloc_info, std::ptr::null(), &mut memory)
        != vk::Result::SUCCESS
    {
        (fns.destroy_buffer)(device, buffer, std::ptr::null());
        return None;
    }
    if (fns.bind_buffer_memory)(device, buffer, memory, 0) != vk::Result::SUCCESS {
        (fns.free_memory)(device, memory, std::ptr::null());
        (fns.destroy_buffer)(device, buffer, std::ptr::null());
        return None;
    }

    let mut ptr = std::ptr::null_mut();
    if (fns.map_memory)(
        device,
        memory,
        0,
        reqs.size,
        vk::MemoryMapFlags::empty(),
        &mut ptr,
    ) != vk::Result::SUCCESS
    {
        (fns.free_memory)(device, memory, std::ptr::null());
        (fns.destroy_buffer)(device, buffer, std::ptr::null());
        return None;
    }

    Some(StagingBuffer {
        buffer,
        memory,
        ptr: ptr as *mut u8,
        size: reqs.size,
    })
}

unsafe fn create_intermediate_image(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    extent: vk::Extent2D,
) -> Option<IntermediateImage> {
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    let mut image = vk::Image::null();
    if (fns.create_image)(device, &info, std::ptr::null(), &mut image) != vk::Result::SUCCESS {
        return None;
    }

    let mut reqs = vk::MemoryRequirements::default();
    (fns.get_image_memory_requirements)(device, image, &mut reqs);

    let mut mem_props = vk::PhysicalDeviceMemoryProperties::default();
    (fns.get_mem_props)(physical_device, &mut mem_props);

    let mem_type = (0..mem_props.memory_type_count as usize)
        .find(|&i| {
            (reqs.memory_type_bits & (1 << i)) != 0
                && mem_props.memory_types[i]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .map(|i| i as u32)
        .or_else(|| {
            (0..mem_props.memory_type_count as usize)
                .find(|&i| (reqs.memory_type_bits & (1 << i)) != 0)
                .map(|i| i as u32)
        });
    let mem_type = mem_type?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .memory_type_index(mem_type);
    let mut memory = vk::DeviceMemory::null();
    if (fns.allocate_memory)(device, &alloc_info, std::ptr::null(), &mut memory)
        != vk::Result::SUCCESS
    {
        (fns.destroy_image)(device, image, std::ptr::null());
        return None;
    }
    if (fns.bind_image_memory)(device, image, memory, 0) != vk::Result::SUCCESS {
        (fns.free_memory)(device, memory, std::ptr::null());
        (fns.destroy_image)(device, image, std::ptr::null());
        return None;
    }

    Some(IntermediateImage { image, memory })
}

fn encode_webp(mut rgba: Vec<u8>, width: u32, height: u32) {
    let (chunks, _) = rgba.as_chunks_mut::<4>();
    for chunk in chunks {
        chunk[3] = 255;
    }
    let encoder = webp::Encoder::from_rgba(&rgba, width, height);
    let webp_data = encoder.encode_lossless();
    let path = screenshot_path();
    eprintln!("ira-overlay: saving screenshot to {:?}", path);
    let _ = std::fs::write(&path, &*webp_data);
}

fn screenshot_path() -> std::path::PathBuf {
    let base = data_dir();
    let dir = base.join("ira").join("screenshots");
    let _ = std::fs::create_dir_all(&dir);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    dir.join(format!("screenshot_{secs}.webp"))
}

fn video_path() -> std::path::PathBuf {
    let base = data_dir();
    let dir = base.join("ira").join("videos");
    let _ = std::fs::create_dir_all(&dir);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    dir.join(format!("video_{secs}.mp4"))
}

fn data_dir() -> std::path::PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home).join(".local/share")
        })
}
