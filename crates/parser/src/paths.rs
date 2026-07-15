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

pub fn ra_icon_path(save_dir: &str, game_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("ra").join(format!("game_{}_icon.png", game_id))
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
