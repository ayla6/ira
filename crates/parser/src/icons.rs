use std::path::{Path, PathBuf};

use image::ImageEncoder;

/// Encodes little-endian RGB565 pixel data as a PNG file.
pub fn save_rgb565_png(path: &Path, width: u32, height: u32, rgb565: &[u8]) -> Result<(), String> {
    let expected = (width as usize) * (height as usize) * 2;
    if rgb565.len() < expected {
        return Err(format!(
            "RGB565 buffer too small: {} < {expected}",
            rgb565.len()
        ));
    }
    let expand = |v: u16| ((v as u32 * 255 + 15) / 31) as u8;
    let img = image::ImageBuffer::from_fn(width, height, |x, y| {
        let i = (y as usize * width as usize + x as usize) * 2;
        let pixel = u16::from_le_bytes([rgb565[i], rgb565[i + 1]]);
        image::Rgb([
            expand(pixel >> 11),
            expand((pixel >> 5) & 0x1F),
            expand(pixel & 0x1F),
        ])
    });
    img.save(path).map_err(|e| e.to_string())
}

/// Encodes tightly packed RGBA8 pixel data as a PNG file.
pub fn save_rgba_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() < expected {
        return Err(format!(
            "RGBA buffer too small: {} < {expected}",
            rgba.len()
        ));
    }
    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let i = (y as usize * width as usize + x as usize) * 4;
        image::Rgba([rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]])
    });
    img.save(path).map_err(|e| e.to_string())
}

pub fn convert_ico_to_png(ico_path: &Path) -> Result<PathBuf, String> {
    let _s = tracing::info_span!("convert_ico_to_png", path = %ico_path.display()).entered();
    let png_path = ico_path.with_extension("png");
    if png_path.exists() {
        let _ = std::fs::remove_file(ico_path);
        return Ok(png_path);
    }
    let img = image::open(ico_path).map_err(|e| e.to_string())?;
    img.save(&png_path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(ico_path);
    Ok(png_path)
}

fn try_convert_ico(path: &Path) -> PathBuf {
    if path.extension().and_then(|e| e.to_str()) == Some("ico") {
        if let Ok(png) = convert_ico_to_png(path) {
            return png;
        }
    }
    path.to_path_buf()
}

/// Copies an image file into `dest_dir` as `{base}` and re-encodes it to
/// lossless WebP. Returns the converted path, or None when the source is
/// missing or undecodable. Uses `load_image_bytes`, so extensionless or
/// magic-less formats like TGA decode too.
pub fn import_image_as_webp(src: &Path, dest_dir: &Path, base: &str) -> Option<PathBuf> {
    use super::paths::{find_image_file, load_image_bytes};
    if !src.is_file() {
        return None;
    }
    std::fs::create_dir_all(dest_dir).ok()?;
    let data = std::fs::read(src).ok()?;
    let img = load_image_bytes(&data)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let encoded = webp::Encoder::from_rgba(rgba.as_raw(), w, h).encode_lossless();
    let dest = dest_dir.join(format!("{base}.webp"));
    std::fs::write(&dest, &*encoded).ok()?;
    find_image_file(dest_dir, base)
}

pub fn find_icon_path(ach_dir: &Path, icon_field: &str) -> String {
    if icon_field.is_empty() {
        return String::new();
    }
    if Path::new(icon_field).extension().is_none() {
        return String::new();
    }
    let path = ach_dir.join(icon_field);
    if path.is_file() {
        let converted = try_convert_ico(&path);
        return converted.to_string_lossy().into_owned();
    }

    let base = Path::new(icon_field).file_name().unwrap_or_default();
    let candidates = [
        ach_dir.join(base),
        ach_dir.join("achievement_images").join(base),
        ach_dir.join("img").join(base),
    ];
    candidates
        .iter()
        .find(|cand| cand.is_file())
        .map(|cand| try_convert_ico(cand).to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_rgb565_png_decodes_first_pixel() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("icon.png");
        let mut pixels = vec![0u8; 48 * 48 * 2];
        pixels[0..2].copy_from_slice(&0xF800u16.to_le_bytes()); // red
        pixels[2..4].copy_from_slice(&0x07E0u16.to_le_bytes()); // green

        save_rgb565_png(&path, 48, 48, &pixels).unwrap();

        let img = image::open(&path).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (48, 48));
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 255, 0]);
    }

    #[test]
    fn test_save_rgb565_png_rejects_short_buffer() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(save_rgb565_png(&tmp.path().join("i.png"), 48, 48, &[0, 0]).is_err());
    }
}

/// Downscale an image to preview size for tiles that show it at a fixed
/// few hundred pixels: decoding full-res photos on the main loop stalls
/// a picker open for seconds in debug builds, and a size request is a
/// minimum — a bigger texture's natural size would grow the tile past
/// it. Images already within `max_px` pass through byte-identical;
/// anything undecodable passes through untouched. Callers persist the
/// original separately.
pub fn preview_bytes(png: &[u8], max_px: u32) -> Vec<u8> {
    let img = match image::load_from_memory(png) {
        Ok(img) => img,
        Err(_) => return png.to_vec(),
    };
    if img.width() <= max_px && img.height() <= max_px {
        return png.to_vec();
    }
    let small = img.thumbnail(max_px, max_px).to_rgba8();
    let mut out = Vec::new();
    if image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            small.as_raw(),
            small.width(),
            small.height(),
            image::ExtendedColorType::Rgba8,
        )
        .is_err()
    {
        return png.to_vec();
    }
    out
}

/// Crops transparent margins off an image: the bounding box of pixels
/// with alpha above [`TRIM_ALPHA_CUTOFF`], re-encoded as PNG. Anything
/// at or below the cutoff counts as transparent because service art
/// carries near-transparent noise at the canvas edge, and a single such
/// pixel spanning the frame defeats exact-zero logic (the crop then
/// returns the whole image untouched). Returns `None` for images with
/// no transparent pixels, fully transparent images, or anything that
/// fails to decode — callers keep their original bytes. Disc photos
/// carry wildly different transparent canvases; trimming them at save
/// time is what keeps the picker tiles a regular size.
pub const TRIM_ALPHA_CUTOFF: u8 = 8;

pub fn trim_transparent_margins(png: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(png).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    let mut left = w;
    let mut top = h;
    let mut right = 0u32;
    let mut bottom = 0u32;
    for (x, y, pixel) in img.enumerate_pixels() {
        if pixel.0[3] > TRIM_ALPHA_CUTOFF {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    if right <= left || bottom <= top || (left == 0 && top == 0 && right == w && bottom == h) {
        return None;
    }
    let cropped =
        image::imageops::crop_imm(&img, left, top, right - left, bottom - top).to_image();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            cropped.as_raw(),
            cropped.width(),
            cropped.height(),
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod trim_tests {
    use super::*;

    fn rgba_fixture() -> Vec<u8> {
        // 6x4 with a 2x2 opaque block centered in transparency.
        let mut img = image::RgbaImage::new(6, 4);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            if (2..4).contains(&x) && (1..3).contains(&y) {
                *pixel = image::Rgba([200, 100, 50, 255]);
            }
        }
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), 6, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    #[test]
    fn test_trim_transparent_margins_crops_to_content() {
        let trimmed = trim_transparent_margins(&rgba_fixture()).expect("trims");
        let back = image::load_from_memory(&trimmed).unwrap().into_rgba8();
        assert_eq!(back.dimensions(), (2, 2));
        assert!(back.pixels().all(|pixel| pixel.0[3] != 0));
    }

    #[test]
    fn test_trim_transparent_margins_ignores_opaque_images() {
        let mut img = image::RgbaImage::new(4, 4);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgba([10, 20, 30, 255]);
        }
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), 4, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert!(trim_transparent_margins(&out).is_none());
    }

    #[test]
    fn test_trim_transparent_margins_rejects_garbage() {
        assert!(trim_transparent_margins(b"not an image").is_none());
    }

    #[test]
    fn test_preview_bytes_passes_small_images_through() {
        let mut img = image::RgbaImage::new(64, 64);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgba([10, 200, 30, 255]);
        }
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), 64, 64, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert_eq!(preview_bytes(&out, 512), out);
    }

    #[test]
    fn test_preview_bytes_caps_large_images() {
        let img = image::RgbaImage::from_pixel(1024, 768, image::Rgba([10, 200, 30, 255]));
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(
                img.as_raw(),
                1024,
                768,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let small = preview_bytes(&out, 256);
        assert!(small.len() < out.len());
        let back = image::load_from_memory(&small).unwrap();
        assert!(back.width() <= 256 && back.height() <= 256);
    }

    #[test]
    fn test_trim_transparent_margins_ignores_near_transparent_noise() {
        // 8x8 with an opaque 4x4 center, fully transparent margins, and
        // a 1px ring of near-transparent (alpha 5) noise at the canvas
        // edge like lossy service art carries. The noise spans the full
        // canvas, so exact-zero logic keeps the whole image; the crop
        // must still land on the opaque content.
        let mut img = image::RgbaImage::new(8, 8);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            if (2..6).contains(&x) && (2..6).contains(&y) {
                *pixel = image::Rgba([200, 100, 50, 255]);
            } else if x == 0 || y == 0 || x == 7 || y == 7 {
                *pixel = image::Rgba([10, 10, 10, 5]);
            }
        }
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), 8, 8, image::ExtendedColorType::Rgba8)
            .unwrap();
        let trimmed = trim_transparent_margins(&out).expect("trims");
        let back = image::load_from_memory(&trimmed).unwrap().into_rgba8();
        assert_eq!(back.dimensions(), (4, 4));
    }
}

#[cfg(test)]
mod import_tests {
    use super::*;

    #[test]
    fn test_import_image_as_webp_converts_tga() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("iconTex.tga");
        let img = image::DynamicImage::new_rgb8(4, 4);
        let mut tga = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut tga), image::ImageFormat::Tga)
            .unwrap();
        std::fs::write(&src, &tga).unwrap();

        let dest = import_image_as_webp(&src, &tmp.path().join("data"), "icon").unwrap();

        assert!(dest.ends_with("icon.webp"));
        assert!(dest.is_file());
    }

    #[test]
    fn test_import_image_as_webp_rejects_missing_source() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            import_image_as_webp(&tmp.path().join("nope.tga"), &tmp.path().join("d"), "icon")
                .is_none()
        );
    }
}
