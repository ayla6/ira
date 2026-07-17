use std::path::{Path, PathBuf};

pub const GALAXY_ID: &str = "100000000000000000";

pub fn data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("steam").join(app_id)
}

pub fn ps4_data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("ps4").join(app_id)
}

pub fn sgdb_data_dir(save_dir: &str, sgdb_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("steamgriddb").join(sgdb_id)
}

pub fn retro_data_dir(save_dir: &str, db_id: i64) -> PathBuf {
    Path::new(save_dir).join("data").join("retro").join(db_id.to_string())
}

pub fn ra_icon_path(save_dir: &str, game_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("ra").join(game_id).join("icon.webp")
}

pub fn find_image_file(dir: &Path, base_name: &str) -> Option<PathBuf> {
    for ext in &["png", "jpg", "jpeg", "webp", "ico"] {
        let p = dir.join(format!("{}.{}", base_name, ext));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn ensure_small_image(dir: &Path, base_name: &str, max_w: u32, max_h: u32) {
    if dir.join(format!("{}_small.webp", base_name)).is_file() {
        return;
    }
    let small_name = format!("{}_small", base_name);
    for ext in &["png", "jpg", "jpeg", "ico"] {
        let _ = std::fs::remove_file(dir.join(format!("{}.{}", small_name, ext)));
    }
    let Some(source) = find_image_file(dir, base_name) else { return };

    let img = match image::ImageReader::open(&source)
        .map_err(|_| ())
        .and_then(|r| r.with_guessed_format().map_err(|_| ()))
        .and_then(|r| r.decode().map_err(|_| ()))
    {
        Ok(i) => i,
        Err(_) => return,
    };
    let (w, h) = (img.width(), img.height());

    let is_lossless = matches!(base_name, "icon")
        || (base_name != "vertical" && base_name != "hero" && base_name != "header"
            && source.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("png")));

    let dest = dir.join(format!("{}.webp", small_name));

    if w <= max_w && h <= max_h {
        let data = img.to_rgba8();
        let (fw, fh) = data.dimensions();
        let encoded = if is_lossless {
            webp::Encoder::from_rgba(data.as_raw(), fw, fh).encode_lossless()
        } else {
            webp::Encoder::from_rgba(data.as_raw(), fw, fh).encode(85.0)
        };
        let _ = std::fs::write(&dest, &*encoded);
        return;
    }

    let ratio = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    let new_w = (w as f64 * ratio).ceil() as u32;
    let new_h = (h as f64 * ratio).ceil() as u32;
    let new_w = new_w.max(1);
    let new_h = new_h.max(1);
    let resized = img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3);

    let data = resized.to_rgba8();
    let (fw, fh) = data.dimensions();

    let encoded = if is_lossless {
        webp::Encoder::from_rgba(data.as_raw(), fw, fh).encode_lossless()
    } else {
        webp::Encoder::from_rgba(data.as_raw(), fw, fh).encode(85.0)
    };
    let _ = std::fs::write(&dest, &*encoded);
}

/// Remove all image files with the given base_name (e.g. "icon", "hero", "vertical")
/// in the directory, across all known image extensions. Call before saving a new
/// image to avoid stale files with different extensions.
pub fn remove_image_variants(dir: &Path, base_name: &str) {
    for ext in &["png", "jpg", "jpeg", "webp", "ico"] {
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
    let full_name = fname.replacen("_small.", ".", 1);
    let full = parent.join(&full_name);
    if full.is_file() { full.to_string_lossy().into_owned() } else { path.to_string() }
}

/// Open an image file (PNG, JPEG, ICO, etc.) and re-save as lossless WebP,
/// removing the original. Does nothing if the file can't be decoded.
pub fn convert_to_lossless_webp(path: &Path) {
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

pub fn achievements_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir(save_dir, app_id).join("achievements")
}

pub fn unlock_status_path(save_dir: &str, trophy_source: ira_models::TrophySource, app_id: &str, platform_id: &str) -> PathBuf {
    match trophy_source {
        ira_models::TrophySource::Nge => Path::new(save_dir).join("gog").join(GALAXY_ID).join(platform_id).join("achievements.json"),
        _ => Path::new(save_dir).join("steam").join(app_id).join("achievements.json"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
    fn test_find_image_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test_icon.png");
        fs::write(&file_path, "fake png content").unwrap();
        let result = find_image_file(dir.path(), "test_icon");
        assert_eq!(result, Some(file_path));
    }

    #[test]
    fn test_find_image_file_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let result = find_image_file(dir.path(), "nonexistent");
        assert_eq!(result, None);
    }
}
