use std::path::PathBuf;

/// Find the Steam installation directory on Linux.
/// Checks common locations: ~/.local/share/Steam, ~/.steam/steam, ~/.steam/root
pub fn steam_install_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let candidates = [
        PathBuf::from(&home).join(".local/share/Steam"),
        PathBuf::from(&home).join(".steam/steam"),
        PathBuf::from(&home).join(".steam/root"),
    ];
    for c in &candidates {
        if c.join("steamapps").is_dir() {
            return Some(c.clone());
        }
    }
    candidates.into_iter().find(|c| c.is_dir())
}

pub fn steamapps_dir() -> Option<PathBuf> {
    steam_install_dir().map(|d| d.join("steamapps"))
}

pub fn library_folders_path() -> Option<PathBuf> {
    steamapps_dir().map(|d| d.join("libraryfolders.vdf"))
}

pub fn loginusers_path() -> Option<PathBuf> {
    steam_install_dir().map(|d| d.join("config").join("loginusers.vdf"))
}

pub fn userdata_dir() -> Option<PathBuf> {
    steam_install_dir().map(|d| d.join("userdata"))
}

/// Returns all Steam user IDs found in the userdata directory.
/// Each subdirectory of userdata/ is a Steam ID.
pub fn get_steam_user_ids() -> Vec<String> {
    let Some(dir) = userdata_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if name.parse::<u64>().is_ok() {
                ids.push(name.to_string());
            }
        }
    }
    ids
}

/// Returns the most recently logged-in Steam user ID, parsed from loginusers.vdf.
/// Falls back to the first user ID found in userdata/.
pub fn get_most_recent_user_id() -> Option<String> {
    if let Some(path) = loginusers_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(super::vdf::VdfValue::Obj(users)) = super::vdf::parse_vdf(&text) {
                let mut best: Option<(i64, String)> = None;
                for (id, value) in &users {
                    if matches!(value, super::vdf::VdfValue::Obj(_)) {
                        let most_recent = super::vdf::get_str(value, "MostRecent")
                            .map(|s| s == "1")
                            .unwrap_or(false);
                        let timestamp = super::vdf::get_str(value, "Timestamp")
                            .and_then(|s: &str| s.parse::<i64>().ok())
                            .unwrap_or(0);
                        if most_recent || timestamp > best.as_ref().map(|(t, _)| *t).unwrap_or(0) {
                            best = Some((timestamp, id.clone()));
                        }
                    }
                }
                if let Some((_, id)) = best {
                    return Some(id);
                }
            }
        }
    }
    get_steam_user_ids().into_iter().next()
}

/// Path to the librarycache directory for a given Steam user ID.
pub fn librarycache_dir(steam_id: &str) -> Option<PathBuf> {
    steam_install_dir()
        .map(|d| d.join("userdata").join(steam_id).join("config").join("librarycache"))
}

/// Path to the achievement cache JSON for a specific app.
pub fn librarycache_path(steam_id: &str, app_id: &str) -> Option<PathBuf> {
    librarycache_dir(steam_id).map(|d| d.join(format!("{}.json", app_id)))
}

/// Read playtime (hours) and last played (unix timestamp) from localconfig.vdf.
pub fn read_playtime(app_id: &str) -> Option<(f64, i64)> {
    let steam_id = get_most_recent_user_id()?;
    let path = steam_install_dir()?.join("userdata").join(&steam_id).join("config").join("localconfig.vdf");
    let text = std::fs::read_to_string(&path).ok()?;
    let parsed = super::vdf::parse_vdf(&text)?;
    let app = super::vdf::get_value(&parsed, "Software")
        .and_then(|s| super::vdf::get_value(s, "Valve"))
        .and_then(|v| super::vdf::get_value(v, "Steam"))
        .and_then(|s| super::vdf::get_value(s, "apps"))
        .and_then(|v| super::vdf::get_value(v, app_id))?;
    let playtime_min: f64 = super::vdf::get_str(app, "Playtime").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let last_played: i64 = super::vdf::get_str(app, "LastPlayed").and_then(|s| s.parse().ok()).unwrap_or(0);
    Some((playtime_min / 60.0, last_played))
}
