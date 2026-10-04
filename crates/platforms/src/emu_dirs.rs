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

/// `<flatpak app root>/<suffix>` for `flatpak:<id>` executables, else just
/// `suffix` (relative to the executable's own directory).
pub(crate) fn app_path_for(executable: &str, suffix: &str) -> PathBuf {
    match flatpak_app_dir(executable) {
        Some(base) => base.join(suffix),
        None => PathBuf::from(suffix),
    }
}

pub fn cache_home() -> PathBuf {
    base_dirs().get_cache_home().unwrap_or_else(|| home_dir().join(".cache"))
}

pub fn config_home() -> PathBuf {
    base_dirs()
        .get_config_home()
        .unwrap_or_else(|| home_dir().join(".config"))
}

pub fn data_home() -> PathBuf {
    base_dirs()
        .get_data_home()
        .unwrap_or_else(|| home_dir().join(".local/share"))
}

fn base_dirs() -> xdg::BaseDirectories {
    xdg::BaseDirectories::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flatpak_app_dir_parses_marker() {
        let dir = flatpak_app_dir("flatpak:org.libretro.RetroArch").unwrap();
        assert!(dir.ends_with(".var/app/org.libretro.RetroArch"));
        assert_eq!(flatpak_app_dir("flatpak:"), None);
        assert_eq!(flatpak_app_dir("/usr/bin/retroarch"), None);
    }

    #[test]
    fn test_app_path_for_uses_flatpak_root_or_relative() {
        let flat = app_path_for("flatpak:org.example.App", "config/data");
        assert!(flat.ends_with(".var/app/org.example.App/config/data"));
        assert_eq!(
            app_path_for("/usr/bin/emu", "config/data"),
            PathBuf::from("config/data")
        );
    }
}
