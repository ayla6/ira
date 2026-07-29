use std::path::{Path, PathBuf};
use ira_models::AppDetails;
use crate::api_emulators_shared::{
    api_emulators_dir, backup_file, copy_file, detect_arch, find_api_emu_dll_folder, is_windows,
    restore_backup,
};

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

pub fn read_nge_language(settings_dir: &Path) -> Option<String> {
    let config_path = settings_dir.join("NemirtingasGalaxyEmu.json");
    let data = std::fs::read_to_string(&config_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&data).ok()?;
    if let Some(galaxy) = json.get("GalaxyEmu") {
        if let Some(user) = galaxy.get("User") {
            return user.get("Language").and_then(|v| v.as_str()).map(|s| s.to_string());
        }
    }
    json.get("language").and_then(|v| v.as_str()).map(|s| s.to_string())
}

pub fn write_nge_language(settings_dir: &Path, language: &str) -> Result<(), String> {
    let config_path = settings_dir.join("NemirtingasGalaxyEmu.json");
    let data = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read NGE config: {}", e))?;
    let mut json: serde_json::Value = serde_json::from_str(&data)
        .map_err(|e| format!("Failed to parse NGE config: {}", e))?;

    if let Some(galaxy) = json.get_mut("GalaxyEmu") {
        if let Some(user) = galaxy.get_mut("User") {
            user["Language"] = serde_json::Value::String(language.to_string());
            if let Some(langs) = user.get_mut("Languages") {
                if let Some(arr) = langs.as_array_mut() {
                    arr.clear();
                    arr.push(serde_json::Value::String(language.to_string()));
                }
            }
        }
    } else {
        json["language"] = serde_json::Value::String(language.to_string());
    }

    let out = serde_json::to_string_pretty(&json)
        .map_err(|e| format!("Failed to serialize NGE config: {}", e))?;
    std::fs::write(&config_path, out)
        .map_err(|e| format!("Failed to write NGE config: {}", e))
}

const NGE_VERSION_FILES: &[&str] = &["Galaxy.dll", "Galaxy64.dll"];

fn nge_file_map(is_64: bool) -> &'static [(&'static str, &'static str)] {
    if is_64 {
        &[("Galaxy64.dll", "Galaxy64.dll")]
    } else {
        &[("Galaxy.dll", "Galaxy.dll")]
    }
}

/// Check if the game folder contains original GOG Galaxy DLLs
pub fn has_original_gog_dlls(game_exe: &str) -> bool {
    let dlls = &["galaxy.dll", "galaxy64.dll"];
    let result = find_api_emu_dll_folder(game_exe, dlls);
    if result.is_none() {
        let search_dir = Path::new(game_exe).parent().map(|p| p.to_path_buf()).unwrap_or_default();
        eprintln!("has_original_gog_dlls: no Galaxy DLL found in {} (exe={})", search_dir.display(), game_exe);
        if let Ok(entries) = std::fs::read_dir(&search_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    eprintln!("  file: {}", name);
                }
            }
        }
    }
    result.is_some()
}

/// All GOG Galaxy DLL filenames (case-insensitive match).
const GOG_DLL_NAMES: &[&str] = &["galaxy.dll", "galaxy64.dll"];

/// Recursively search `game_folder` for directories containing Galaxy DLLs.
pub fn find_gog_dlls_recursive(game_folder: &str) -> Vec<PathBuf> {
    crate::api_emulators_shared::find_dll_dirs_recursive(Path::new(game_folder), GOG_DLL_NAMES)
}

/// Check whether `dir` already has NGE backups (`.dll.bak`/`.bak.dll`/`.owo.dll`).
pub fn has_gog_emulator_backups(dir: &Path) -> bool {
    crate::api_emulators_shared::has_emulator_backups(dir, GOG_DLL_NAMES)
}

/// List available NGE versions under api_emulators/gog/
pub fn list_gog_versions(save_dir: &str) -> Vec<String> {
    let root = api_emulators_dir(save_dir).join("gog");
    let mut versions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    versions.push(name.to_string());
                }
            }
        }
    }
    versions.sort();
    versions
}

fn resolve_gog_version(save_dir: &str, version: &str) -> Result<PathBuf, String> {
    let root = api_emulators_dir(save_dir).join("gog");
    if version.is_empty() {
        let versions = list_gog_versions(save_dir);
        let v = versions.first().ok_or_else(|| {
            format!("No NGE versions found at {:?}", root)
        })?;
        Ok(root.join(v))
    } else {
        let dir = root.join(version);
        if !dir.is_dir() {
            return Err(format!("NGE version '{}' not found at {:?}", version, dir));
        }
        Ok(dir)
    }
}

pub fn install_nge(
    save_dir: &str,
    game_exe: &str,
    product_id: &str,
    version: &str,
) -> Result<(), String> {
    if !is_windows(game_exe) {
        return Err("Nemirtingas API emulator is Windows-only (no Linux .so)".to_string());
    }

    if !has_original_gog_dlls(game_exe) {
        return Err("No original GOG Galaxy DLL found in game folder. Cannot install API emulator.".to_string());
    }

    let version_dir = resolve_gog_version(save_dir, version)?;

    let is64 = detect_arch(game_exe) == "x64";

    let dll_folder = find_api_emu_dll_folder(game_exe, NGE_VERSION_FILES)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    // Backup original DLL and copy emulator file
    for (src_name, dst_name) in nge_file_map(is64) {
        let src = version_dir.join(src_name);
        let dst = dll_folder.join(dst_name);
        if src.is_file() {
            backup_file(&dst)?;
            copy_file(&src, &dst)?;
        }
    }

    let settings_dir = dll_folder.join("ngalaxye_settings");
    std::fs::create_dir_all(&settings_dir)
        .map_err(|e| format!("create ngalaxye_settings: {}", e))?;

    crate::gog::generate_galaxy_emu_config(
        &dll_folder.to_string_lossy(),
        product_id,
    )?;

    Ok(())
}

pub fn uninstall_nge(game_exe: &str) -> Result<(), String> {
    let all_files: Vec<&str> = NGE_VERSION_FILES.to_vec();
    let dll_folder = find_api_emu_dll_folder(game_exe, &all_files)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    for file in &all_files {
        let path = dll_folder.join(file);
        restore_backup(&path)?;
    }
    Ok(())
}

pub fn is_nge_installed(game_exe: &str) -> bool {
    let all_files: Vec<&str> = NGE_VERSION_FILES.to_vec();
    if let Some(folder) = find_api_emu_dll_folder(game_exe, &all_files) {
        folder.join("ngalaxye_settings").is_dir()
    } else {
        false
    }
}
