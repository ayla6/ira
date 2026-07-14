use std::collections::HashMap;
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

/// Read playtime (hours) and last played (unix timestamp) for ALL apps from
/// localconfig.vdf in a single pass. Returns a map keyed by app_id string.
pub fn read_all_playtimes() -> HashMap<String, (f64, i64)> {
    let mut result = HashMap::new();

    let Some(steam_id) = get_most_recent_user_id() else {
        eprintln!("[steam] read_all_playtimes: no Steam user ID found");
        return result;
    };
    let Some(install) = steam_install_dir() else {
        eprintln!("[steam] read_all_playtimes: Steam install dir not found");
        return result;
    };
    let path = install.join("userdata").join(&steam_id).join("config").join("localconfig.vdf");

    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("[steam] read_all_playtimes: cannot read {}", path.display());
        return result;
    };
    let Some(parsed) = super::vdf::parse_vdf(&text) else {
        eprintln!("[steam] read_all_playtimes: VDF parse failed for {}", path.display());
        return result;
    };

    let Some(apps) = super::vdf::get_value(&parsed, "Software")
        .and_then(|s| super::vdf::get_value(s, "Valve"))
        .and_then(|v| super::vdf::get_value(v, "Steam"))
        .and_then(|s| super::vdf::get_value(s, "apps"))
    else {
        eprintln!("[steam] read_all_playtimes: Software/Valve/Steam/apps not found in localconfig.vdf");
        return result;
    };

    if let super::vdf::VdfValue::Obj(app_map) = apps {
        for (app_id, value) in app_map {
            if let super::vdf::VdfValue::Obj(_) = value {
                let pt = super::vdf::get_str(value, "Playtime")
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let lp = super::vdf::get_str(value, "LastPlayed")
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0);
                if pt > 0.0 || lp > 0 {
                    result.insert(app_id.clone(), (pt / 60.0, lp));
                }
            }
        }
    }

    result
}

/// Read playtime for a single app. Prefer `read_all_playtimes` when loading
/// multiple games — it reads localconfig.vdf once instead of per-app.
pub fn read_playtime(app_id: &str) -> Option<(f64, i64)> {
    read_all_playtimes().get(app_id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_playtimes_from_localconfig() {
        let vdf = r#"
"UserLocalConfigStore"
{
    "Software"
    {
        "Valve"
        {
            "Steam"
            {
                "apps"
                {
                    "250900"
                    {
                        "Playtime" "300"
                        "LastPlayed" "1700000000"
                    }
                    "440"
                    {
                        "Playtime" "1200"
                        "LastPlayed" "1700000001"
                    }
                    "0"
                    {
                        "Playtime" "0"
                        "LastPlayed" "0"
                    }
                }
            }
        }
    }
}
"#;
        let parsed = super::super::vdf::parse_vdf(vdf).unwrap();
        let apps = super::super::vdf::get_value(&parsed, "Software")
            .and_then(|s| super::super::vdf::get_value(s, "Valve"))
            .and_then(|v| super::super::vdf::get_value(v, "Steam"))
            .and_then(|s| super::super::vdf::get_value(s, "apps"))
            .unwrap();

        let mut playtimes = HashMap::new();
        if let super::super::vdf::VdfValue::Obj(app_map) = apps {
            for (app_id, value) in app_map {
                let pt = super::super::vdf::get_str(value, "Playtime")
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let lp = super::super::vdf::get_str(value, "LastPlayed")
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0);
                if pt > 0.0 || lp > 0 {
                    playtimes.insert(app_id.clone(), (pt / 60.0, lp));
                }
            }
        }

        assert_eq!(playtimes.len(), 2);
        assert_eq!(playtimes["250900"], (5.0, 1700000000));
        assert_eq!(playtimes["440"], (20.0, 1700000001));
        assert!(!playtimes.contains_key("0"));
    }
}
