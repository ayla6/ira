use std::path::{Path, PathBuf};

use crate::ps4::{parse_psf, psf_get_title, psf_get_title_id};

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn vita_fs_path() -> PathBuf {
    vita_fs_path_for("")
}

pub fn vita_fs_path_for(executable: &str) -> PathBuf {
    if !executable.is_empty() && !executable.starts_with("flatpak:") {
        let path = Path::new(executable);
        for root in [path.parent(), Some(path)].into_iter().flatten() {
            let portable = root.join("portable").join("fs");
            if portable.is_dir() {
                return portable;
            }
        }
    }

    let shared = xdg::BaseDirectories::new()
        .get_data_home()
        .unwrap_or_else(|| home_dir().join(".local").join("share"));
    shared.join("Vita3K").join("Vita3K")
}

#[derive(Debug, Clone)]
pub struct Vita3KGame {
    pub title_id: String,
    pub title: String,
    pub game_path: PathBuf,
    pub icon_path: PathBuf,
}

fn scan_app(path: &Path) -> Option<Vita3KGame> {
    let sfo_path = path.join("sce_sys").join("param.sfo");
    if !sfo_path.is_file() {
        return None;
    }
    let psf = parse_psf(&sfo_path).ok()?;
    let title_id = psf_get_title_id(&psf);
    if title_id.is_empty() {
        return None;
    }
    Some(Vita3KGame {
        title_id,
        title: psf_get_title(&psf),
        game_path: path.to_path_buf(),
        icon_path: path.join("sce_sys").join("icon0.png"),
    })
}

pub fn discover_games() -> Vec<Vita3KGame> {
    discover_games_for_executable("")
}

pub fn discover_games_for_executable(executable: &str) -> Vec<Vita3KGame> {
    let app_dir = vita_fs_path_for(executable).join("ux0").join("app");
    let Ok(entries) = std::fs::read_dir(app_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|entry| scan_app(&entry.path()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vita_fs_path_uses_portable_root() {
        let tmp = tempfile::tempdir().unwrap();
        let executable = tmp.path().join("Vita3K");
        std::fs::create_dir_all(executable.parent().unwrap().join("portable/fs")).unwrap();
        let path = vita_fs_path_for(&executable.to_string_lossy());
        assert!(path.ends_with("portable/fs"));
    }

    #[test]
    fn test_discover_vita_games_missing_root() {
        assert!(discover_games_for_executable("/nonexistent/Vita3K").is_empty());
    }
}
