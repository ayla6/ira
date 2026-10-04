//! Pixel readback + format conversion (CPU side, readback thread).
//!
//! The GPU always stages whole frames; this module turns staged bytes into
//! packed RGBA8 for the encoder. Blit frames are already RGBA8 (memcpy);
//! direct-copy frames convert from the swapchain format. Strides are derived
//! from the width here — every caller stages tightly packed rows.

use ash::vk;

use crate::capture_ffmpeg::screenshot_path;

use super::PendingFrame;

pub(super) unsafe fn read_pixels(frame: &PendingFrame) -> Vec<u8> {
    if frame.use_blit {
        let total = (frame.width as usize) * (frame.height as usize) * 4;
        let mut rgba = vec![0u8; total];
        unsafe { std::ptr::copy_nonoverlapping(frame.staging.ptr, rgba.as_mut_ptr(), total) };
        return rgba;
    }
    read_pixels_fallback(frame)
}

unsafe fn read_pixels_fallback(frame: &PendingFrame) -> Vec<u8> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let bpp = super::src_bpp(frame.src_format) as usize;
    let src_size = w * h * bpp;

    // The fence already signaled (readback thread), so the GPU is done;
    // HOST_COHERENT staging needs no flush — convert in place without an
    // intermediate copy.
    let src = unsafe { std::slice::from_raw_parts(frame.staging.ptr, src_size) };

    match frame.src_format {
        vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB => convert_8bpp(src, w, h, true),
        vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB => convert_8bpp(src, w, h, false),
        vk::Format::R16G16B16A16_SFLOAT | vk::Format::R16G16B16A16_UNORM => convert_f16(src, w, h),
        vk::Format::A2B10G10R10_UNORM_PACK32 | vk::Format::A2R10G10B10_UNORM_PACK32 => {
            convert_10bpp(src, w, h, frame.src_format)
        }
        // Unknown formats stage nothing usable; the empty frame is dropped
        // by the readback thread before it can reach an encoder.
        _ => Vec::new(),
    }
}

fn convert_8bpp(src: &[u8], w: usize, h: usize, swap_bgr: bool) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &src[y * w * 4..y * w * 4 + w * 4];
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

fn convert_f16(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &src[y * w * 8..y * w * 8 + w * 8];
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

fn convert_10bpp(src: &[u8], w: usize, h: usize, format: vk::Format) -> Vec<u8> {
    let swap = format == vk::Format::A2B10G10R10_UNORM_PACK32;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &src[y * w * 4..y * w * 4 + w * 4];
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
        // Subnormals flush to signed zero (no gradual underflow in 8-bit output).
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

pub(super) fn encode_webp(mut rgba: Vec<u8>, width: u32, height: u32) {
    let (chunks, _) = rgba.as_chunks_mut::<4>();
    // Screenshots composite over transparency elsewhere; flatten alpha so
    // the saved file never carries a stray translucent frame.
    for chunk in chunks {
        chunk[3] = 255;
    }
    let encoder = webp::Encoder::from_rgba(&rgba, width, height);
    let webp_data = encoder.encode_lossless();
    let path = screenshot_path();
    eprintln!("ira-overlay: saving screenshot to {:?}", path);
    if let Err(e) = std::fs::write(&path, &*webp_data) {
        eprintln!("ira-overlay: failed to save screenshot: {e}");
    }
}
