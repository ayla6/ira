use std::path::{Path, PathBuf};

pub const GALAXY_ID: &str = "100000000000000000";

pub fn data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("steam").join(app_id)
}

pub fn ps4_data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("ps4").join(app_id)
}

pub fn ps3_data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("ps3").join(app_id)
}

pub fn sgdb_data_dir(save_dir: &str, sgdb_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("steamgriddb").join(sgdb_id)
}

pub fn retro_data_dir(save_dir: &str, db_id: i64) -> PathBuf {
    Path::new(save_dir).join("data").join("retro").join(db_id.to_string())
}

/// Returns the data directory for a game based on its kind, trophy source,
/// and SGDB ID. Centralizes the branching logic that was duplicated across
/// game_loader, edit_game_dialog, image_manager, and context_menu.
pub fn game_data_dir(save_dir: &str, game: &ira_models::Game) -> PathBuf {
    if game.kind == ira_models::GameKind::Retro {
        retro_data_dir(save_dir, game.db_id)
    } else if game.kind == ira_models::GameKind::Ps4 {
        ps4_data_dir(save_dir, &game.app_id)
    } else if game.kind == ira_models::GameKind::Ps3 {
        ps3_data_dir(save_dir, &game.app_id)
    } else if game.trophy_source.has_steam_enrichment() {
        data_dir(save_dir, &game.app_id)
    } else if !game.sgdb_id.is_empty() {
        sgdb_data_dir(save_dir, &game.sgdb_id)
    } else {
        data_dir(save_dir, &game.app_id)
    }
}

/// Same as `game_data_dir` but takes a `GameEntry` (DB row) instead of a `Game`.
pub fn entry_data_dir(save_dir: &str, entry: &ira_models::GameEntry) -> PathBuf {
    let app_id = if !entry.steam_id.is_empty() { &entry.steam_id } else { &entry.game_id };
    let sgdb_id = entry.sgdb_id.as_deref().unwrap_or("");
    if entry.kind == ira_models::GameKind::Retro {
        retro_data_dir(save_dir, entry.id)
    } else if entry.kind == ira_models::GameKind::Ps4 {
        ps4_data_dir(save_dir, app_id)
    } else if entry.kind == ira_models::GameKind::Ps3 {
        ps3_data_dir(save_dir, app_id)
    } else if entry.trophy_source.has_steam_enrichment() {
        data_dir(save_dir, app_id)
    } else if !sgdb_id.is_empty() {
        sgdb_data_dir(save_dir, sgdb_id)
    } else {
        data_dir(save_dir, app_id)
    }
}

pub fn find_image_file(dir: &Path, base_name: &str) -> Option<PathBuf> {
    for ext in &["webp", "jpg"] {
        let p = dir.join(format!("{}.{}", base_name, ext));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Generate a `_small` thumbnail variant of an image if one doesn't already exist.
///
/// Format conversion matrix:
///
/// | Source  | Already ≤ max dims? | Action                          |
/// |---------|---------------------|---------------------------------|
/// | JPEG    | Yes                 | Copy to `_small.jpg`            |
/// | JPEG    | No                  | Resize → lossy WebP at 90%      |
/// | PNG/ICO | Yes                 | Lossless WebP (no resize)       |
/// | PNG/ICO | No                  | Resize → lossless WebP          |
/// | Icon    | Any                 | Always lossless WebP            |
///
/// Icons are always lossless regardless of source format.
/// JPEGs that are already the right size are copied as-is (no re-encode).
/// JPEGs that need resizing are re-encoded as lossy WebP (resize requires re-encode anyway).
/// PNGs/ICOs are always converted to lossless WebP (good compression, no loss).
pub fn ensure_small_image(dir: &Path, base_name: &str, max_w: u32, max_h: u32) {
    let _s = tracing::info_span!("ensure_small_image", base_name, max_w, max_h).entered();
    if find_image_file(dir, &format!("{}_small", base_name)).is_some() {
        return;
    }
    let small_name = format!("{}_small", base_name);
    for ext in &["png", "jpeg", "ico"] {
        let _ = std::fs::remove_file(dir.join(format!("{}.{}", small_name, ext)));
    }
    let Some(source) = find_image_file(dir, base_name) else { return };

    let is_jpeg = source.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
    let is_icon = base_name == "icon";

    let img = {
        let _s = tracing::info_span!("ensure_small_decode", base_name).entered();
        match image::ImageReader::open(&source)
            .map_err(|_| ())
            .and_then(|r| r.with_guessed_format().map_err(|_| ()))
            .and_then(|r| r.decode().map_err(|_| ()))
        {
            Ok(i) => i,
            Err(_) => return,
        }
    };
    let (w, h) = (img.width(), img.height());

    if w <= max_w && h <= max_h {
        if is_jpeg {
            let dest = dir.join(format!("{}.jpg", small_name));
            let _ = std::fs::copy(&source, &dest);
            return;
        }
        let data = img.to_rgba8();
        let (fw, fh) = data.dimensions();
        let dest = dir.join(format!("{}.webp", small_name));
        let encoded = {
            let _s = tracing::info_span!("ensure_small_encode", base_name, w, h, mode = "lossless_no_resize").entered();
            webp::Encoder::from_rgba(data.as_raw(), fw, fh).encode_lossless()
        };
        let _ = std::fs::write(&dest, &*encoded);
        return;
    }

    let ratio = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    let new_w = (w as f64 * ratio).ceil() as u32;
    let new_h = (h as f64 * ratio).ceil() as u32;
    let new_w = new_w.max(1);
    let new_h = new_h.max(1);
    let resized = {
        let _s = tracing::info_span!("ensure_small_resize", base_name, src_w = w, src_h = h, dst_w = new_w, dst_h = new_h, filter = "Lanczos3").entered();
        img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
    };

    let data = resized.to_rgba8();
    let (fw, fh) = data.dimensions();
    let dest = dir.join(format!("{}.webp", small_name));

    let is_lossless = is_icon || !is_jpeg;

    let encoded = {
        let _s = tracing::info_span!("ensure_small_encode", base_name, w = fw, h = fh, mode = if is_lossless { "lossless" } else { "lossy" }).entered();
        if is_lossless {
            webp::Encoder::from_rgba(data.as_raw(), fw, fh).encode_lossless()
        } else {
            webp::Encoder::from_rgba(data.as_raw(), fw, fh).encode(90.0)
        }
    };
    let _ = std::fs::write(&dest, &*encoded);
}

/// Remove all image files with the given base_name (e.g. "icon", "hero", "vertical")
/// in the directory, across all known image extensions. Call before saving a new
/// image to avoid stale files with different extensions.
pub fn remove_image_variants(dir: &Path, base_name: &str) {
    let _s = tracing::info_span!("remove_image_variants", base_name).entered();
    for ext in &["png", "jpg", "jpeg", "webp", "ico", "tmp"] {
        let p = dir.join(format!("{}.{}", base_name, ext));
        let _ = std::fs::remove_file(&p);
    }
}

/// Given a path that may point to a `_small` image, return the full-size path.
pub fn full_image_path(path: &str) -> String {
    if path.is_empty() || !path.contains("_small") {
        return path.to_string();
    }
    let p = Path::new(path);
    let parent = p.parent().unwrap_or(Path::new(""));
    let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let stem = fname.replace("_small", "");
    let stem = stem.split('.').next().unwrap_or(&stem);
    if let Some(full) = find_image_file(parent, stem) {
        return full.to_string_lossy().into_owned();
    }
    path.to_string()
}

/// Check if raw bytes are an ICO file by reading the magic header.
pub fn is_ico_data(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == [0x00, 0x00, 0x01, 0x00]
}

/// Convert raw image bytes to lossless WebP if the source is PNG or ICO.
/// Returns `None` if the format is not convertible or decoding/encoding fails.
pub fn convert_bytes_to_lossless_webp(data: &[u8]) -> Option<Vec<u8>> {
    let _s = tracing::info_span!("convert_bytes_to_lossless_webp").entered();
    let format = image::guess_format(data).ok()?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Ico) {
        return Some(data.to_vec());
    }
    let img = image::load_from_memory(data).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(webp::Encoder::from_rgba(rgba.as_raw(), w, h).encode_lossless().to_vec())
}

/// Open an image file (PNG, ICO, etc.) and re-save as lossless WebP,
/// removing the original. Does nothing if the file is already WebP or is
/// a JPEG (JPEGs are kept as-is to avoid generation loss).
pub fn convert_to_lossless_webp(path: &Path) {
    let _s = tracing::info_span!("convert_to_lossless_webp", path = %path.display()).entered();
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("webp") || e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")) {
        return;
    }
    let base = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent = path.parent().unwrap_or(Path::new("."));
    let webp = parent.join(format!("{}.webp", base));

    let img = match image::ImageReader::open(path)
        .map_err(|_| ())
        .and_then(|r| r.with_guessed_format().map_err(|_| ()))
        .and_then(|r| r.decode().map_err(|_| ()))
    {
        Ok(i) => i,
        Err(_) => return,
    };

    let data = img.to_rgba8();
    let (w, h) = data.dimensions();
    let encoded = webp::Encoder::from_rgba(data.as_raw(), w, h).encode_lossless();
    if std::fs::write(&webp, &*encoded).is_ok() {
        let _ = std::fs::remove_file(path);
    }
}

pub fn url_extension(url: &str) -> &str {
    let path = Path::new(url);
    path.extension().and_then(|e| e.to_str()).unwrap_or("png")
}

/// Decode an image file to raw RGBA8 pixels suitable for `gdk::MemoryTexture`.
/// Reads the file and decodes it using the `image` crate. Returns `None` if
/// the file can't be read or the format is unsupported.
///
/// Designed to be called on a background thread — the returned `(Vec<u8>, w, h)`
/// is `Send` and can be passed back to the main thread to create a `MemoryTexture`.
pub fn decode_to_rgba(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let _s = tracing::info_span!("decode_to_rgba", path = %path.display()).entered();
    let data = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&data).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((rgba.into_raw(), w, h))
}

pub fn achievements_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir(save_dir, app_id).join("achievements")
}

pub fn unlock_status_path(save_dir: &str, trophy_source: ira_models::TrophySource, app_id: &str, platform_id: &str) -> PathBuf {
    match trophy_source {
        ira_models::TrophySource::Nge => Path::new(save_dir).join("emulator_saves").join("nge").join(GALAXY_ID).join(platform_id).join("achievements.json"),
        _ => Path::new(save_dir).join("emulator_saves").join("gbe").join(app_id).join("achievements.json"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_extension_png() {
        assert_eq!(url_extension("image.png"), "png");
    }

    #[test]
    fn test_url_extension_jpg() {
        assert_eq!(url_extension("photo.jpg"), "jpg");
    }

    #[test]
    fn test_url_extension_no_extension() {
        assert_eq!(url_extension("image"), "png");
    }

    #[test]
    fn test_url_extension_complex_url() {
        let url = "https://example.com/path/icon.png?w=200&h=200";
        let ext = url_extension(url);
        assert!(ext.contains("png"));
    }

    #[test]
    fn test_convert_to_lossless_webp_skips_webp() {
        let tmp = tempfile::tempdir().unwrap();
        let webp_path = tmp.path().join("vertical.webp");
        let img = image::RgbaImage::new(2, 2);
        let encoded = webp::Encoder::from_rgba(img.as_raw(), 2, 2).encode_lossless();
        std::fs::write(&webp_path, &*encoded).unwrap();
        convert_to_lossless_webp(&webp_path);
        assert!(webp_path.is_file(), "webp file should not be deleted when input is already webp");
    }

    #[test]
    fn test_convert_to_lossless_webp_converts_png() {
        let tmp = tempfile::tempdir().unwrap();
        let png_path = tmp.path().join("icon.png");
        let img = image::RgbaImage::new(2, 2);
        img.save(&png_path).unwrap();
        convert_to_lossless_webp(&png_path);
        assert!(!png_path.is_file(), "png file should be removed after conversion");
        let webp_out = tmp.path().join("icon.webp");
        assert!(webp_out.is_file(), "webp file should exist after conversion");
    }

    #[test]
    fn test_convert_to_lossless_webp_skips_jpg() {
        let tmp = tempfile::tempdir().unwrap();
        let jpg_path = tmp.path().join("hero.jpg");
        let img = image::DynamicImage::new_rgb8(4, 4);
        img.save_with_format(&jpg_path, image::ImageFormat::Jpeg).unwrap();
        convert_to_lossless_webp(&jpg_path);
        assert!(jpg_path.is_file(), "jpg file should not be deleted or converted");
        let webp_out = tmp.path().join("hero.webp");
        assert!(!webp_out.is_file(), "webp file should not be created from jpg");
    }

    #[test]
    fn test_full_image_path_finds_jpg_from_small_webp() {
        let tmp = tempfile::tempdir().unwrap();
        let jpg = tmp.path().join("header.jpg");
        std::fs::write(&jpg, [0u8]).unwrap();
        let small_webp = tmp.path().join("header_small.webp");
        std::fs::write(&small_webp, [0u8]).unwrap();
        let result = full_image_path(small_webp.to_str().unwrap());
        assert!(result.ends_with("header.jpg"), "should find header.jpg, got: {result}");
    }

    #[test]
    fn test_full_image_path_no_small_suffix() {
        assert_eq!(full_image_path("/foo/bar/hero.jpg"), "/foo/bar/hero.jpg");
        assert_eq!(full_image_path(""), "");
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
        let mut png_buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png_buf), image::ImageFormat::Png).unwrap();
        let result = convert_bytes_to_lossless_webp(&png_buf);
        let result = result.expect("PNG should convert to WebP");
        let format = image::guess_format(&result).unwrap();
        assert_eq!(format, image::ImageFormat::WebP);
    }

    #[test]
    fn test_convert_bytes_to_lossless_webp_skips_jpeg() {
        let img = image::DynamicImage::new_rgb8(4, 4);
        let mut jpg_buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut jpg_buf), image::ImageFormat::Jpeg).unwrap();
        let original = jpg_buf.clone();
        let result = convert_bytes_to_lossless_webp(&jpg_buf);
        assert_eq!(result, Some(original), "JPEG bytes should be returned unchanged");
    }

    #[test]
    fn test_convert_bytes_to_lossless_webp_skips_webp() {
        let raw = image::RgbaImage::new(2, 2);
        let webp_bytes = webp::Encoder::from_rgba(raw.as_raw(), 2, 2).encode_lossless();
        let original = webp_bytes.to_vec();
        let result = convert_bytes_to_lossless_webp(&original);
        assert_eq!(result, Some(original), "WebP bytes should be returned unchanged");
    }
}
