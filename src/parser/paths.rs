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

pub fn find_image_file(dir: &Path, base_name: &str) -> Option<PathBuf> {
    for ext in &["png", "jpg", "jpeg", "webp"] {
        let p = dir.join(format!("{}.{}", base_name, ext));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn url_extension(url: &str) -> &str {
    let path = Path::new(url);
    path.extension().and_then(|e| e.to_str()).unwrap_or("png")
}

pub fn achievements_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir(save_dir, app_id).join("achievements")
}

pub fn unlock_status_path(save_dir: &str, kind: &str, app_id: &str, platform_id: &str) -> PathBuf {
    match kind {
        "gog" => Path::new(save_dir).join("gog").join(GALAXY_ID).join(platform_id).join("achievements.json"),
        _ => Path::new(save_dir).join("steam").join(app_id).join("achievements.json"),
    }
}
