use crate::api_emulators_shared::{
    api_emulators_dir, backup_file, copy_file, detect_arch, detect_folder_bitness,
    find_game_dll_folder, is_windows, restore_backup,
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
        json["unlock_dlcs"] = serde_json::Value::Bool(!any_disabled);
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

/// Files in a game folder indicating a 64-bit Galaxy setup.
const GALAXY_64_FILES: &[&str] = &["Galaxy64.dll", "galaxy64.dll"];

/// Files in a game folder indicating a 32-bit Galaxy setup.
const GALAXY_32_FILES: &[&str] = &["Galaxy.dll", "galaxy.dll"];

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
    crate::api_emulators_shared::centralize_settings(
        game_folder,
        "ngalaxye_settings",
        GOG_DLL_NAMES,
    )
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
    crate::api_emulators_shared::list_version_dirs(&root, &[])
}

fn resolve_gog_version(save_dir: &str, version: &str) -> Result<PathBuf, String> {
    crate::api_emulators_shared::resolve_emu_version(save_dir, "gog", version, "NGE")
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

    let is64 = detect_folder_bitness(&dll_folder, GALAXY_64_FILES, GALAXY_32_FILES)
        .unwrap_or_else(|| detect_arch(game_exe) == "x64");
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
        .find(|d| !is_nge_installed(d))
        .ok_or_else(|| "No unpatched GOG Galaxy DLLs found in game folder".to_string())?;

    let is64 =
        detect_folder_bitness(dll_folder, GALAXY_64_FILES, GALAXY_32_FILES).unwrap_or(false);
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
    let dll_folder = find_game_dll_folder(game_exe, game_folder, NGE_VERSION_FILES)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    for file in NGE_VERSION_FILES {
        let path = dll_folder.join(file);
        restore_backup(&path)?;
    }
    Ok(())
}

/// Whether the NGE emulator is currently installed in `dll_folder`, i.e. the
/// original Galaxy DLLs have been backed up and replaced with emulator builds.
/// The `ngalaxye_settings` dir is not used as evidence — a centralized root
/// `ngalaxye_settings` with symlinks exists for any game.
pub fn is_nge_installed(dll_folder: &Path) -> bool {
    crate::api_emulators_shared::has_emulator_backups(dll_folder, NGE_VERSION_FILES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_nge_installed_requires_backups() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("ngalaxye_settings")).unwrap();
        assert!(!is_nge_installed(dir));
        std::fs::write(dir.join("Galaxy64.dll.bak"), b"x").unwrap();
        assert!(is_nge_installed(dir));
    }

    fn nge_config(dir: &std::path::Path, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("NemirtingasGalaxyEmu.json"), body).unwrap();
    }

    #[test]
    fn test_read_nge_language_both_shapes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("s");
        nge_config(&dir, r#"{"GalaxyEmu":{"User":{"Language":"german"}}}"#);
        assert_eq!(read_nge_language(&dir).as_deref(), Some("german"));
        nge_config(&dir, r#"{"language":"french"}"#);
        assert_eq!(read_nge_language(&dir).as_deref(), Some("french"));
        nge_config(&dir, r#"{}"#);
        assert_eq!(read_nge_language(&dir), None);
    }

    #[test]
    fn test_write_nge_language_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("s");
        nge_config(&dir, r#"{"GalaxyEmu":{"User":{"Language":"english"}}}"#);
        write_nge_language(&dir, "german").unwrap();
        assert_eq!(read_nge_language(&dir).as_deref(), Some("german"));
    }

    #[test]
    fn test_write_nge_dlc_config_unlock_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("s");
        nge_config(&dir, r#"{"GalaxyEmu":{"Apps":{"UnlockDlcs":true,"DlcList":{}}}}"#);
        let mut details = AppDetails {
            name: String::new(),
            languages: Vec::new(),
            dlcs: std::collections::HashMap::new(),
            ufs_savefiles: Vec::new(),
            ufs_rootoverrides: Vec::new(),
        };
        details.dlcs.insert(
            "1".to_string(),
            ira_models::DlcInfo {
                app_id: 10,
                name: "Pack".to_string(),
                image_url: String::new(),
                enabled: false,
            },
        );
        write_nge_dlc_config(&dir, &details).unwrap();
        let data = std::fs::read_to_string(dir.join("NemirtingasGalaxyEmu.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(json["GalaxyEmu"]["Apps"]["UnlockDlcs"], false);
        assert!(json["GalaxyEmu"]["Apps"]["DlcList"].as_object().unwrap().is_empty());
    }

    #[test]
    fn test_find_galaxy_settings_walks_up() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("game");
        let sub = game.join("bin");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(game.join("ngalaxye_settings")).unwrap();
        let exe = sub.join("game.exe");
        std::fs::write(&exe, b"x").unwrap();
        let found = find_galaxy_settings(&exe.to_string_lossy()).unwrap();
        assert_eq!(found, game.join("ngalaxye_settings"));
    }
}
