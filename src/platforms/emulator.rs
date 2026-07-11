use std::path::{Path, PathBuf};
use crate::api::types::AppDetails;

pub fn find_steam_settings(game_exe: &str, save_dir: &str, app_id: &str) -> Option<PathBuf> {
    let ach_dir = crate::parser::achievements_dir(save_dir, app_id);
    if ach_dir.exists() {
        if let Ok(meta) = std::fs::symlink_metadata(&ach_dir) {
            if meta.file_type().is_symlink() {
                if let Ok(target) = std::fs::read_link(&ach_dir) {
                    return Some(target);
                }
            }
        }
        if ach_dir.join("configs.app.ini").exists() || ach_dir.join("steam_appid.txt").exists() {
            return Some(ach_dir);
        }
    }
    let mut current = Path::new(game_exe).parent()?.to_path_buf();
    loop {
        let candidate = current.join("steam_settings");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

pub fn write_gse_dlc_config(settings_dir: &Path, details: &AppDetails) -> Result<(), String> {
    let mut content = String::from("[app::dlcs]\n");
    let any_disabled = details.dlcs.values().any(|d| !d.enabled);
    if any_disabled {
        content.push_str("unlock_all=0\n");
        for (_, dlc) in &details.dlcs {
            if dlc.enabled {
                content.push_str(&format!("{}={}\n", dlc.app_id, dlc.name));
            }
        }
    } else {
        content.push_str("unlock_all=1\n");
    }
    std::fs::write(settings_dir.join("configs.app.ini"), content)
        .map_err(|e| format!("Failed to write configs.app.ini: {}", e))
}

pub fn find_galaxy_settings(game_exe: &str) -> Option<PathBuf> {
    let mut current = Path::new(game_exe).parent()?.to_path_buf();
    loop {
        let candidate = current.join("ngalaxye_settings");
        if candidate.is_dir() {
            return Some(candidate);
        }
        for dll in &["galaxy.dll", "galaxy64.dll", "Galaxy.dll", "Galaxy64.dll"] {
            if current.join(dll).is_file() {
                let settings = current.join("ngalaxye_settings");
                return Some(settings);
            }
        }
        if !current.pop() {
            break;
        }
    }
    None
}

pub fn write_nge_dlc_config(settings_dir: &Path, details: &AppDetails) -> Result<(), String> {
    let config_path = settings_dir.join("NemirtingasGalaxyEmu.json");
    let data = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read NGE config: {}", e))?;
    let mut json: serde_json::Value = serde_json::from_str(&data)
        .map_err(|e| format!("Failed to parse NGE config: {}", e))?;

    let any_disabled = details.dlcs.values().any(|d| !d.enabled);
    let dlcs_map: serde_json::Map<String, serde_json::Value> = details
        .dlcs
        .iter()
        .filter(|(_, d)| d.enabled)
        .map(|(_, d)| (d.app_id.to_string(), serde_json::Value::String(d.name.clone())))
        .collect();

    if let Some(galaxy) = json.get_mut("GalaxyEmu") {
        if let Some(apps) = galaxy.get_mut("Apps") {
            apps["UnlockDlcs"] = serde_json::Value::Bool(!any_disabled);
            apps["DlcList"] = serde_json::Value::Object(dlcs_map);
        }
    } else {
        if any_disabled {
            json["unlock_dlcs"] = serde_json::Value::Bool(false);
        } else {
            json["unlock_dlcs"] = serde_json::Value::Bool(true);
        }
    }

    let out = serde_json::to_string_pretty(&json)
        .map_err(|e| format!("Failed to serialize NGE config: {}", e))?;
    std::fs::write(&config_path, out)
        .map_err(|e| format!("Failed to write NGE config: {}", e))
}

pub fn write_dlc_configs(
    trophy_source: &str,
    game_exe: &str,
    save_dir: &str,
    app_id: &str,
    details: &AppDetails,
) {
    if details.dlcs.is_empty() {
        return;
    }
    match trophy_source {
        crate::models::GSE => {
            if let Some(settings_dir) = find_steam_settings(game_exe, save_dir, app_id) {
                if let Err(e) = write_gse_dlc_config(&settings_dir, details) {
                    eprintln!("DLC config write failed: {}", e);
                }
            }
        }
        crate::models::NGE => {
            if let Some(settings_dir) = find_galaxy_settings(game_exe) {
                if let Err(e) = write_nge_dlc_config(&settings_dir, details) {
                    eprintln!("DLC config write failed: {}", e);
                }
            }
        }
        _ => {}
    }
}
