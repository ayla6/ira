use crate::api_emulators_shared::{
    api_emulators_dir, backup_file, copy_file, detect_arch, find_game_dll_folder, is_windows,
    restore_backup,
};
use ira_models::AppDetails;
use std::path::{Path, PathBuf};

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
    let mut json: serde_json::Value =
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse NGE config: {}", e))?;

    let any_disabled = details.dlcs.values().any(|d| !d.enabled);
    let dlcs_map: serde_json::Map<String, serde_json::Value> = details
        .dlcs
        .iter()
        .filter(|(_, d)| d.enabled)
        .map(|(_, d)| {
            (
                d.app_id.to_string(),
                serde_json::Value::String(d.name.clone()),
            )
        })
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
    std::fs::write(&config_path, out).map_err(|e| format!("Failed to write NGE config: {}", e))
}

pub fn read_nge_language(settings_dir: &Path) -> Option<String> {
    let config_path = settings_dir.join("NemirtingasGalaxyEmu.json");
    let data = std::fs::read_to_string(&config_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&data).ok()?;
    if let Some(galaxy) = json.get("GalaxyEmu") {
        if let Some(user) = galaxy.get("User") {
            return user
                .get("Language")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }
    json.get("language")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn write_nge_language(settings_dir: &Path, language: &str) -> Result<(), String> {
    let config_path = settings_dir.join("NemirtingasGalaxyEmu.json");
    let data = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read NGE config: {}", e))?;
    let mut json: serde_json::Value =
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse NGE config: {}", e))?;

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
    std::fs::write(&config_path, out).map_err(|e| format!("Failed to write NGE config: {}", e))
}

const NGE_VERSION_FILES: &[&str] = &["Galaxy.dll", "Galaxy64.dll"];

fn nge_file_map(is_64: bool) -> &'static [(&'static str, &'static str)] {
    if is_64 {
        &[("Galaxy64.dll", "Galaxy64.dll")]
    } else {
        &[("Galaxy.dll", "Galaxy.dll")]
    }
}

/// Resolve the directory holding the GOG Galaxy DLLs for a game install.
/// Falls back from a shallow exe-relative scan to a recursive scan of the
/// full game folder (nested installs like Unreal Engine games).
pub fn find_gog_dll_folder(game_exe: &str, game_folder: &str) -> Option<PathBuf> {
    find_game_dll_folder(game_exe, game_folder, NGE_VERSION_FILES)
}

/// Check if `dll_folder` contains original GOG Galaxy DLLs.
pub fn has_original_gog_dlls(dll_folder: &Path) -> bool {
    ["galaxy.dll", "galaxy64.dll", "Galaxy.dll", "Galaxy64.dll"]
        .iter()
        .any(|d| dll_folder.join(d).exists())
}

/// Find `ngalaxye_settings/` in the game folder. If it exists somewhere other
/// than the root, leave it in place and create a symlink at the game root
/// pointing to it. If it doesn't exist, create it at the root. Then create
/// symlinks from every directory containing Galaxy DLLs to the root settings.
pub fn centralize_galaxy_settings(game_folder: &str) -> Result<Option<PathBuf>, String> {
    let root = Path::new(game_folder);
    let root_settings = root.join("ngalaxye_settings");

    // If ngalaxye_settings/ is already at the root (real dir, not symlink), just symlink DLL dirs.
    if root_settings.is_dir()
        && !std::fs::symlink_metadata(&root_settings)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    {
        symlink_gog_dll_dirs_to_settings(root, &root_settings);
        return Ok(Some(root_settings));
    }

    let found = find_galaxy_settings_recursive(root);

    if let Some(found_dir) = found {
        // ngalaxye_settings exists somewhere — symlink root to it (don't move)
        #[cfg(unix)]
        {
            if !root_settings.exists() {
                let rel = compute_relative(&root_settings, &found_dir);
                let _ = std::os::unix::fs::symlink(&rel, &root_settings);
            }
        }
        symlink_gog_dll_dirs_to_settings(root, &root_settings);
        return Ok(Some(root_settings));
    }

    // No ngalaxye_settings found — create at root and symlink DLL dirs
    std::fs::create_dir_all(&root_settings)
        .map_err(|e| format!("create ngalaxye_settings at root: {e}"))?;
    symlink_gog_dll_dirs_to_settings(root, &root_settings);
    Ok(Some(root_settings))
}

fn find_galaxy_settings_recursive(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().and_then(|n| n.to_str()) == Some("ngalaxye_settings") {
                        return Some(path);
                    }
                    stack.push(path);
                }
            }
        }
    }
    None
}

fn symlink_gog_dll_dirs_to_settings(root: &Path, settings_dir: &Path) {
    let dll_dirs = find_gog_dlls_recursive(&root.to_string_lossy());
    for dir in dll_dirs {
        let link = dir.join("ngalaxye_settings");
        if link.exists() {
            continue;
        }
        #[cfg(unix)]
        {
            let rel = compute_relative(&dir, settings_dir);
            let _ = std::os::unix::fs::symlink(&rel, &link);
        }
    }
}

#[cfg(unix)]
fn compute_relative(from_dir: &Path, to_path: &Path) -> PathBuf {
    use std::path::Component;
    let from_components: Vec<_> = from_dir.components().collect();
    let to_components: Vec<_> = to_path.components().collect();
    let mut common = 0;
    while common < from_components.len()
        && common < to_components.len()
        && from_components[common] == to_components[common]
    {
        common += 1;
    }
    let up = from_components.len() - common;
    let mut result = PathBuf::new();
    for _ in 0..up {
        result.push("..");
    }
    for comp in &to_components[common..] {
        if let Component::Normal(s) = comp {
            result.push(s);
        }
    }
    result
}

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
        let v = versions
            .first()
            .ok_or_else(|| format!("No NGE versions found at {:?}", root))?;
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
    game_folder: &str,
    product_id: &str,
    version: &str,
) -> Result<PathBuf, String> {
    if !is_windows(game_exe) {
        return Err("Nemirtingas API emulator is Windows-only (no Linux .so)".to_string());
    }

    let dll_folder = find_game_dll_folder(game_exe, game_folder, NGE_VERSION_FILES)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    if !has_original_gog_dlls(&dll_folder) {
        return Err(
            "No original GOG Galaxy DLL found in game folder. Cannot install API emulator."
                .to_string(),
        );
    }

    let is64 = detect_arch(game_exe) == "x64";
    install_nge_into_folder(save_dir, &dll_folder, is64, product_id, version)?;
    Ok(dll_folder)
}

/// Recursively search `game_folder` for the directory containing Galaxy DLLs and
/// install the Nemirtingas Galaxy Emulator there. Skips directories that already
/// have emulator backups. Used by the auto-add flow which only knows the install
/// folder, not the exe. NGE is Windows-only.
pub fn install_nge_from_folder(
    save_dir: &str,
    game_folder: &str,
    product_id: &str,
    version: &str,
) -> Result<(), String> {
    let dirs = find_gog_dlls_recursive(game_folder);
    let dll_folder = dirs
        .iter()
        .find(|d| !has_gog_emulator_backups(d))
        .ok_or_else(|| "No unpatched GOG Galaxy DLLs found in game folder".to_string())?;

    let is64 = dll_folder.join("Galaxy64.dll").exists() || dll_folder.join("galaxy64.dll").exists();
    install_nge_into_folder(save_dir, dll_folder, is64, product_id, version)
}

fn install_nge_into_folder(
    save_dir: &str,
    dll_folder: &Path,
    is64: bool,
    product_id: &str,
    version: &str,
) -> Result<(), String> {
    let version_dir = resolve_gog_version(save_dir, version)?;

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

    crate::gog::generate_galaxy_emu_config(&dll_folder.to_string_lossy(), product_id)?;

    Ok(())
}

pub fn uninstall_nge(game_exe: &str, game_folder: &str) -> Result<(), String> {
    let all_files: Vec<&str> = NGE_VERSION_FILES.to_vec();
    let dll_folder = find_game_dll_folder(game_exe, game_folder, &all_files)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    for file in &all_files {
        let path = dll_folder.join(file);
        restore_backup(&path)?;
    }
    Ok(())
}

pub fn is_nge_installed(dll_folder: &Path) -> bool {
    dll_folder.join("ngalaxye_settings").is_dir()
}
