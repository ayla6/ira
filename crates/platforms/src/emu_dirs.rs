//! Shared base-directory helpers for emulator integrations.

use std::path::PathBuf;

pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// The flatpak app root (`~/.var/app/<id>`) when `executable` carries the
/// `flatpak:<app id>` marker used by the console settings.
pub fn flatpak_app_dir(executable: &str) -> Option<PathBuf> {
    let id = executable.strip_prefix("flatpak:")?;
    if id.is_empty() {
        return None;
    }
    Some(home_dir().join(".var").join("app").join(id))
}

pub fn cache_home() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_cache_home()
        .unwrap_or_else(|| home_dir().join(".cache"))
}

pub fn config_home() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_config_home()
        .unwrap_or_else(|| home_dir().join(".config"))
}

pub fn data_home() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_data_home()
        .unwrap_or_else(|| home_dir().join(".local/share"))
}
