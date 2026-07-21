use std::path::PathBuf;

/// RPCS3 config directory.
/// Linux: ~/.config/rpcs3/
pub fn rpcs3_config_dir() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_config_home()
        .map(|p| p.join("rpcs3"))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config").join("rpcs3")
        })
}

/// dev_hdd0 — the PS3 virtual hard drive.
pub fn dev_hdd0_dir() -> PathBuf {
    rpcs3_config_dir().join("dev_hdd0")
}

/// Installed PSN games directory (each subfolder is a game named by TITLE_ID, e.g. NPUB30698).
pub fn games_dir() -> PathBuf {
    dev_hdd0_dir().join("game")
}

/// Disc games directory (mounted disc images).
pub fn disc_dir() -> PathBuf {
    dev_hdd0_dir().join("disc")
}

/// The first user home folder under dev_hdd0/home/ (default: 00000001).
/// Returns None if no user folder exists.
pub fn first_user_id() -> Option<String> {
    let home_dir = dev_hdd0_dir().join("home");
    let entries = std::fs::read_dir(&home_dir).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.chars().all(|c| c.is_ascii_digit()) && name_str.len() == 8 {
            return Some(name_str.into_owned());
        }
    }
    None
}

/// Path to the active user's home directory (dev_hdd0/home/<user>/).
/// Falls back to 00000001 if no user folder is found.
pub fn user_home_dir() -> PathBuf {
    let user_id = first_user_id().unwrap_or_else(|| "00000001".to_string());
    dev_hdd0_dir().join("home").join(&user_id)
}

/// Path to the trophy directory for a given NPCommId (e.g. NPWR00906_00).
/// dev_hdd0/home/<user>/trophy/<npwr_id>/
pub fn trophy_dir(npwr_id: &str) -> PathBuf {
    user_home_dir().join("trophy").join(npwr_id)
}

/// Path to TROPCONF.SFM — the trophy definitions XML (extracted from TROPHY.TRP by RPCS3).
pub fn trophy_conf_path(npwr_id: &str) -> PathBuf {
    trophy_dir(npwr_id).join("TROPCONF.SFM")
}

/// Path to TROPUSR.DAT — the binary user unlock-state file.
pub fn tropusr_path(npwr_id: &str) -> PathBuf {
    trophy_dir(npwr_id).join("TROPUSR.DAT")
}

/// Path to a trophy icon (TROP000.PNG, TROP001.PNG, ...).
pub fn trophy_icon_path(npwr_id: &str, trophy_id: u32) -> PathBuf {
    trophy_dir(npwr_id).join(format!("TROP{:03}.PNG", trophy_id))
}

/// Path to persistent_settings.dat — INI file with [Playtime] and [LastPlayed] sections.
pub fn persistent_settings_path() -> PathBuf {
    rpcs3_config_dir().join("GuiConfigs").join("persistent_settings.dat")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trophy_dir_format() {
        let p = trophy_dir("NPWR00906_00");
        assert!(p.to_string_lossy().ends_with("dev_hdd0/home/00000001/trophy/NPWR00906_00"));
    }

    #[test]
    fn test_trophy_icon_path_zero_padded() {
        let p = trophy_icon_path("NPWR00906_00", 5);
        assert!(p.to_string_lossy().ends_with("TROP005.PNG"));
    }

    #[test]
    fn test_tropusr_path() {
        let p = tropusr_path("NPWR00906_00");
        assert!(p.to_string_lossy().ends_with("TROPUSR.DAT"));
    }

    #[test]
    fn test_persistent_settings_path() {
        let p = persistent_settings_path();
        assert!(p.to_string_lossy().ends_with("GuiConfigs/persistent_settings.dat"));
    }
}
