use std::path::{Path, PathBuf};

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
    for cand in &candidates {
        if cand.is_file() {
            let converted = try_convert_ico(cand);
            return converted.to_string_lossy().into_owned();
        }
    }
    String::new()
}
