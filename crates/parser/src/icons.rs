use std::path::{Path, PathBuf};

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
