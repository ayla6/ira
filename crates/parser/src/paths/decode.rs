use std::path::Path;

/// Encode tightly packed RGBA8 pixels as lossless WebP bytes.
pub fn encode_lossless_webp(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    webp::Encoder::from_rgba(pixels, width, height)
        .encode_lossless()
        .to_vec()
}

pub fn is_ico_data(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == [0x00, 0x00, 0x01, 0x00]
}

/// Decode arbitrary image bytes. TGA carries no magic bytes, so after
/// format guessing fails, try the TGA decoder explicitly (Cemu game
/// icons are uncompressed TGA).
pub fn load_image_bytes(data: &[u8]) -> Option<image::DynamicImage> {
    if let Ok(img) = image::load_from_memory(data) {
        return Some(img);
    }
    if is_ico_data(data) {
        if let Some(img) = decode_largest_png_ico_frame(data) {
            return Some(img);
        }
    }
    let cursor = std::io::Cursor::new(data);
    let decoder = image::codecs::tga::TgaDecoder::new(cursor).ok()?;
    image::DynamicImage::from_decoder(decoder).ok()
}

/// Extracts the largest PNG-compressed frame from an ICO container and
/// decodes it. Needed because the image crate's ICO decoder refuses
/// PNG frames that are not RGBA8 — exactly what Steam's uploaded icons
/// contain — and rejects the whole file over one such frame.
fn decode_largest_png_ico_frame(data: &[u8]) -> Option<image::DynamicImage> {
    const PNG_MAGIC: [u8; 4] = [0x89, b'P', b'N', b'G'];
    if data.len() < 6 {
        return None;
    }
    let count = u16::from_le_bytes([data[4], data[5]]) as usize;
    let mut best: Option<(u64, image::DynamicImage)> = None;
    for index in 0..count {
        // A malformed entry loses only itself.
        let Some(dir_end) = (6 + index * 16).checked_add(16).filter(|end| *end <= data.len())
        else {
            continue;
        };
        let dir = &data[6 + index * 16..dir_end];
        let frame_size = u32::from_le_bytes([dir[8], dir[9], dir[10], dir[11]]) as usize;
        let frame_offset = u32::from_le_bytes([dir[12], dir[13], dir[14], dir[15]]) as usize;
        let Some(frame_end) = frame_offset.checked_add(frame_size).filter(|end| *end <= data.len())
        else {
            continue;
        };
        let frame = &data[frame_offset..frame_end];
        if frame.len() < PNG_MAGIC.len() || frame[..4] != PNG_MAGIC {
            continue;
        }
        let Ok(img) = image::load_from_memory(frame) else {
            continue;
        };
        let actual = u64::from(img.width()) * u64::from(img.height());
        if best.as_ref().is_none_or(|(best_area, _)| actual > *best_area) {
            best = Some((actual, img));
        }
    }
    best.map(|(_, img)| img)
}

/// Convert raw image bytes to lossless WebP if the source is PNG or ICO.
/// Returns `None` if the format is not convertible or decoding/encoding fails.
pub fn convert_bytes_to_lossless_webp(data: &[u8]) -> Option<Vec<u8>> {
    let _s = tracing::info_span!("convert_bytes_to_lossless_webp").entered();
    if let Ok(format) = image::guess_format(data) {
        if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Ico) {
            return Some(data.to_vec());
        }
    }
    encode_bytes_to_lossless_webp(data)
}

/// Always decodes raw image bytes and re-encodes them as lossless WebP,
/// whatever the source format — a JPEG input yields real WebP bytes.
/// Returns `None` when the bytes do not decode as an image.
pub fn encode_bytes_to_lossless_webp(data: &[u8]) -> Option<Vec<u8>> {
    let img = load_image_bytes(data)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(encode_lossless_webp(rgba.as_raw(), w, h))
}

/// Check that raw bytes fully decode to an image. This decodes the whole
/// payload, so callers that persist files should run this first and keep
/// corrupt bytes off disk.
pub fn is_decodable_image(data: &[u8]) -> bool {
    load_image_bytes(data).is_some()
}

/// SGDB serves a takedown notice card in place of removed art in fixed
/// per-aspect layouts; detect it by exact 8x8 average-hash match.
/// New layouts are added by appending their hash here.
const DMCA_CARD_HASHES: &[u64] = &[
    0x000000ff7e000000,
    0x0000007eff000000,
    0x000000ff7f000000,
    0x00007e7eff7e0000,
];

/// 8x8 average hash: downscale to 8x8 luma, set a bit per cell brighter
/// than the mean.
pub fn image_average_hash(data: &[u8]) -> Option<u64> {
    let img = load_image_bytes(data)?;
    let thumb = img.resize_exact(8, 8, image::imageops::FilterType::Triangle);
    let gray = thumb.to_luma8();
    let px = gray.as_raw();
    let mean = px.iter().map(|&v| u32::from(v)).sum::<u32>() / px.len() as u32;
    let mut hash = 0u64;
    for (i, &v) in px.iter().enumerate() {
        if u32::from(v) > mean {
            hash |= 1 << i;
        }
    }
    Some(hash)
}

fn is_dmca_hash(hash: u64) -> bool {
    // The card is byte-identical per layout; match exactly.
    DMCA_CARD_HASHES.contains(&hash)
}

pub fn is_dmca_placeholder(data: &[u8]) -> bool {
    let Some(hash) = image_average_hash(data) else {
        return false;
    };
    is_dmca_hash(hash)
}

/// Decode an image file to raw RGBA8 pixels suitable for `gdk::MemoryTexture`.
/// Returns `None` if the file can't be read or the format is unsupported.
///
/// Designed to be called on a background thread — the returned `(Vec<u8>, w, h)`
/// is `Send` and can be passed back to the main thread to create a `MemoryTexture`.
pub fn decode_to_rgba(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let _s = tracing::info_span!("decode_to_rgba", path = %path.display()).entered();
    let data = std::fs::read(path).ok()?;
    let img = load_image_bytes(&data)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((rgba.into_raw(), w, h))
}

#[cfg(test)]
fn png_bytes(img: image::DynamicImage) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_lossless_webp_round_trips() {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 200, 30, 255]));
        let encoded = encode_lossless_webp(img.as_raw(), 4, 4);
        assert_eq!(image::guess_format(&encoded).unwrap(), image::ImageFormat::WebP);
        let back = image::load_from_memory(&encoded).unwrap().into_rgba8();
        assert_eq!(back.dimensions(), (4, 4));
        assert!(back.pixels().all(|pixel| pixel.0 == [10, 200, 30, 255]));
    }

    #[test]
    fn test_is_decodable_image_accepts_valid_rejects_garbage() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
        let png = png_bytes(image::DynamicImage::ImageRgba8(img));
        assert!(is_decodable_image(&png), "valid PNG must decode");

        // PNG signature with garbage payload: magic bytes alone would pass a
        // guess-format check, but a full decode must reject it.
        let truncated_png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0xFF];
        assert!(!is_decodable_image(&truncated_png));
        assert!(!is_decodable_image(b"not an image"));
        assert!(!is_decodable_image(&[]));
    }

    #[test]
    fn test_is_ico_data_real_ico() {
        let ico_bytes = [0x00, 0x00, 0x01, 0x00, 0x01, 0x00];
        assert!(is_ico_data(&ico_bytes));
    }

    #[test]
    fn test_is_ico_data_png() {
        let png_bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A];
        assert!(!is_ico_data(&png_bytes));
    }

    #[test]
    fn test_is_ico_data_empty() {
        assert!(!is_ico_data(&[]));
    }

    #[test]
    fn test_convert_bytes_to_lossless_webp_converts_png() {
        let img = image::RgbaImage::new(4, 4);
        let png_buf = png_bytes(image::DynamicImage::ImageRgba8(img));
        let result = convert_bytes_to_lossless_webp(&png_buf);
        let result = result.expect("PNG should convert to WebP");
        let format = image::guess_format(&result).unwrap();
        assert_eq!(format, image::ImageFormat::WebP);
    }

    #[test]
    fn test_convert_bytes_to_lossless_webp_skips_jpeg() {
        let img = image::DynamicImage::new_rgb8(4, 4);
        let mut jpg_buf = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut jpg_buf),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
        let original = jpg_buf.clone();
        let result = convert_bytes_to_lossless_webp(&jpg_buf);
        assert_eq!(
            result,
            Some(original),
            "JPEG bytes should be returned unchanged"
        );
    }

    #[test]
    fn test_convert_bytes_to_lossless_webp_skips_webp() {
        let raw = image::RgbaImage::new(2, 2);
        let webp_bytes = encode_lossless_webp(raw.as_raw(), 2, 2);
        let original = webp_bytes.clone();
        let result = convert_bytes_to_lossless_webp(&original);
        assert_eq!(
            result,
            Some(original),
            "WebP bytes should be returned unchanged"
        );
    }

    #[test]
    fn test_dmca_card_hashes_match_exactly() {
        for reference in DMCA_CARD_HASHES {
            assert!(is_dmca_hash(*reference));
            // Near misses are just as unwanted as the card itself: real
            // cover art must never trip the filter.
            assert!(!is_dmca_hash(reference ^ 1));
            assert!(!is_dmca_hash(reference ^ (1 << 63)));
        }
        assert!(!is_dmca_hash(0));
        assert!(!is_dmca_hash(u64::MAX));
    }

    #[test]
    fn test_image_average_hash_is_stable_across_sizes() {
        // A notice-shaped card at two sizes hashes to nearly the same
        // pattern: black background, bright centered band.
        let draw = |width: u32, height: u32| {
            let mut card = image::RgbaImage::new(width, height);
            for (_, _, px) in card.enumerate_pixels_mut() {
                *px = image::Rgba([8, 8, 8, 255]);
            }
            let band_top = (height as f64 * 0.42) as i32;
            let band_h = (height as f64 * 0.16) as i32;
            let band_x = (width as f64 * 0.12) as i32;
            let band_w = (width as f64 * 0.76) as i32;
            for y in band_top..band_top + band_h {
                for x in band_x..band_x + band_w {
                    if y < 0 || x < 0 || y >= height as i32 || x >= width as i32 {
                        continue;
                    }
                    card.put_pixel(x as u32, y as u32, image::Rgba([240, 25, 127, 255]));
                }
            }
            png_bytes(image::DynamicImage::ImageRgba8(card))
        };
        let tall = image_average_hash(&draw(600, 900)).unwrap();
        let tall_again = image_average_hash(&draw(600, 900)).unwrap();
        let square = image_average_hash(&draw(512, 512)).unwrap();
        // Deterministic render: identical bytes, identical hash.
        assert_eq!(tall, tall_again);
        // The layout hash itself is size-independent.
        assert_eq!(tall, square);
    }
}

#[cfg(test)]
mod tga_tests {
    use super::{decode_to_rgba, is_decodable_image, load_image_bytes};

    /// Minimal uncompressed 32-bit top-down TGA, the shape Cemu icons use.
    fn sample_tga() -> Vec<u8> {
        let mut data = vec![
            0, 0, // no id, no palette
            2, // uncompressed truecolor
            0, 0, 0, 0, 0, // palette fields
            0, 0, 0, 0, // no origin
            2, 0, // width 2
            2, 0,  // height 2
            32, // bpp
            8,  // top-down
        ];
        for _ in 0..4 {
            data.extend_from_slice(&[0x11, 0x22, 0x33, 0xff]); // BGRA
        }
        data
    }

    #[test]
    fn test_tga_decodes_through_explicit_fallback() {
        let tga = sample_tga();
        // Sanity: guessing alone cannot identify TGA (no magic bytes).
        assert!(image::guess_format(&tga).is_err());
        assert!(is_decodable_image(&tga));
        let img = load_image_bytes(&tga).expect("TGA must decode");
        assert_eq!(img.to_rgba8().dimensions(), (2, 2));
    }

    #[test]
    fn test_decode_to_rgba_reads_tga_file() {
        let tmp = tempfile::NamedTempFile::with_suffix(".tga").unwrap();
        std::fs::write(tmp.path(), sample_tga()).unwrap();
        let (pixels, w, h) = decode_to_rgba(tmp.path()).expect("file decode");
        assert_eq!((w, h), (2, 2));
        assert_eq!(pixels.len(), 2 * 2 * 4);
    }
}

#[cfg(test)]
mod ico_frame_tests {
    use super::*;

    /// Wraps `png_bytes` in a minimal ICO container whose directory entry
    /// declares `width`/`height` (0 means 256, like real containers).
    fn ico_container(png_bytes: &[u8], width: u8, height: u8) -> Vec<u8> {
        let mut container = vec![0x00, 0x00, 0x01, 0x00, 0x01, 0x00];
        container.extend_from_slice(&[width, height, 0, 0, 1, 0, 32, 0]);
        container.extend_from_slice(&(png_bytes.len() as u32).to_le_bytes());
        container.extend_from_slice(&22u32.to_le_bytes());
        container.extend_from_slice(png_bytes);
        container
    }

    fn png_frame(w: u32, h: u32) -> Vec<u8> {
        let image = image::RgbImage::new(w, h);
        super::png_bytes(image::DynamicImage::ImageRgb8(image))
    }

    #[test]
    fn test_load_image_bytes_decodes_rgb_png_ico_frames() {
        // RGB (not RGBA8) PNG frames inside an ICO are what the image
        // crate's own decoder refuses — Steam's uploaded icons are exactly
        // this, and a real icon payload must still decode.
        let container = ico_container(&png_frame(32, 32), 32, 32);
        let img = load_image_bytes(&container).expect("RGB PNG frame must decode");
        assert_eq!((img.width(), img.height()), (32, 32));
    }

    #[test]
    fn test_load_image_bytes_picks_largest_ico_png_frame() {
        // Arrange: two PNG frames with correct data offsets.
        let small = png_frame(16, 16);
        let large = png_frame(64, 64);
        let mut container = vec![0x00, 0x00, 0x01, 0x00, 0x02, 0x00];
        let mut offset = (6 + 2 * 16) as u32;
        for (bytes, side) in [&small, &large].into_iter().zip([16u8, 64u8]) {
            container.extend_from_slice(&[side, side, 0, 0, 1, 0, 32, 0]);
            container.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            container.extend_from_slice(&offset.to_le_bytes());
            offset += bytes.len() as u32;
        }
        for bytes in [&small, &large] {
            container.extend_from_slice(bytes);
        }
        let img = load_image_bytes(&container).expect("largest PNG frame must decode");
        assert_eq!((img.width(), img.height()), (64, 64));
    }
}
