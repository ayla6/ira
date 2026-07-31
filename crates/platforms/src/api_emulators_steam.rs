use std::path::{Path, PathBuf};
use ira_models::AppDetails;
use crate::api_emulators_shared::{
    api_emulators_dir, backup_file, copy_file, detect_arch, find_api_emu_dll_folder, is_windows,
    restore_backup,
};

pub fn find_steam_settings(game_exe: &str, save_dir: &str, app_id: &str) -> Option<PathBuf> {
    let ach_dir = ira_parser::achievements_dir(save_dir, app_id);
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

/// Find `steam_settings/` in the game folder. If it exists somewhere other
/// than the root, leave it in place and create a symlink at the game root
/// pointing to it. If it doesn't exist, create it at the root. Then create
/// symlinks from every directory containing Steam DLLs to the root
/// `steam_settings/`.
///
/// This ensures GBE/GSE finds its config regardless of which subdirectory the
/// Steam DLL lives in, without duplicating or moving config files.
pub fn centralize_steam_settings(game_folder: &str) -> Result<Option<PathBuf>, String> {
    let root = Path::new(game_folder);
    let root_settings = root.join("steam_settings");

    // If steam_settings/ is already at the root (real dir, not symlink), just symlink DLL dirs.
    if root_settings.is_dir() && !std::fs::symlink_metadata(&root_settings).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        symlink_dll_dirs_to_settings(root, &root_settings);
        return Ok(Some(root_settings));
    }

    // Search recursively for steam_settings/ in subdirectories
    let found = find_steam_settings_recursive(root);

    if let Some(found_dir) = found {
        // steam_settings exists somewhere — symlink root to it (don't move)
        #[cfg(unix)]
        {
            if !root_settings.exists() {
                let rel = compute_relative(&root_settings, &found_dir);
                let _ = std::os::unix::fs::symlink(&rel, &root_settings);
            }
        }
        symlink_dll_dirs_to_settings(root, &root_settings);
        return Ok(Some(root_settings));
    }

    // No steam_settings found — create at root and symlink DLL dirs
    std::fs::create_dir_all(&root_settings)
        .map_err(|e| format!("create steam_settings at root: {e}"))?;
    symlink_dll_dirs_to_settings(root, &root_settings);
    Ok(Some(root_settings))
}

/// Recursively search for a `steam_settings/` directory under `root`.
fn find_steam_settings_recursive(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().and_then(|n| n.to_str()) == Some("steam_settings") {
                        return Some(path);
                    }
                    stack.push(path);
                }
            }
        }
    }
    None
}

/// For every directory containing Steam DLLs under `root`, create a relative
/// symlink to `settings_dir` if the directory doesn't already have one.
fn symlink_dll_dirs_to_settings(root: &Path, settings_dir: &Path) {
    let dll_dirs = super::api_emulators_shared::find_dll_dirs_recursive(
        root,
        &["steam_api.dll", "steam_api64.dll", "libsteam_api.so"],
    );
    for dir in dll_dirs {
        let link = dir.join("steam_settings");
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

/// Compute a relative path from `from_dir` to `to_path`.
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

pub fn write_gse_dlc_config(settings_dir: &Path, details: &AppDetails) -> Result<(), String> {
    let mut content = String::from("[app::dlcs]\n");
    let any_disabled = details.dlcs.values().any(|d| !d.enabled);
    if any_disabled {
        content.push_str("unlock_all=0\n");
        for dlc in details.dlcs.values() {
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

pub fn read_gse_language(settings_dir: &Path) -> Option<String> {
    let path = settings_dir.join("configs.user.ini");
    let content = std::fs::read_to_string(&path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("language=") {
            return Some(val.trim().to_string());
        }
    }
    None
}

pub fn write_gse_language(settings_dir: &Path, language: &str) -> Result<(), String> {
    let path = settings_dir.join("configs.user.ini");
    let mut content = String::new();
    if path.exists() {
        content = std::fs::read_to_string(&path).unwrap_or_default();
    }
    if !content.contains("[user::general]") {
        content.push_str("[user::general]\n");
    }
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut found = false;
    for line in lines.iter_mut() {
        let trimmed = line.trim();
        if trimmed.starts_with("language=") {
            *line = format!("language={}", language);
            found = true;
            break;
        }
    }
    if !found {
        let general_idx = lines.iter().position(|l| l.trim() == "[user::general]");
        if let Some(idx) = general_idx {
            lines.insert(idx + 1, format!("language={}", language));
        } else {
            lines.push(format!("[user::general]\nlanguage={}", language));
        }
    }
    lines.push(String::new());
    std::fs::write(&path, lines.join("\n"))
        .map_err(|e| format!("Failed to write configs.user.ini: {}", e))
}

const GSE_VERSION_FILES: &[&str] = &[
    "libsteam_api.so", "libsteam_api64.so",
    "steamclient.so", "steamclient64.so",
    "steam_api.dll", "steam_api64.dll",
    "steamclient.dll", "steamclient64.dll",
];

fn gse_file_map(is_64: bool, is_win: bool) -> &'static [(&'static str, &'static str)] {
    if is_win {
        if is_64 {
            &[
                ("steam_api64.dll", "steam_api64.dll"),
                ("steamclient64.dll", "steamclient64.dll"),
            ]
        } else {
            &[
                ("steam_api.dll", "steam_api.dll"),
                ("steamclient.dll", "steamclient.dll"),
            ]
        }
    } else if is_64 {
        &[
            ("libsteam_api64.so", "libsteam_api.so"),
            ("steamclient64.so", "steamclient.so"),
        ]
    } else {
        &[
            ("libsteam_api.so", "libsteam_api.so"),
            ("steamclient.so", "steamclient.so"),
        ]
    }
}

/// Check if the game folder contains original Steam API DLLs
pub fn has_original_steam_dlls(game_exe: &str) -> bool {
    let dlls = &["libsteam_api.so", "steam_api.dll", "steam_api64.dll"];
    let result = find_api_emu_dll_folder(game_exe, dlls);
    if result.is_none() {
        let search_dir = Path::new(game_exe).parent().map(|p| p.to_path_buf()).unwrap_or_default();
        eprintln!("has_original_steam_dlls: no Steam DLL found in {} (exe={})", search_dir.display(), game_exe);
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

/// All Steam API DLL/SO filenames (original and emulator share these names).
const STEAM_DLL_NAMES: &[&str] = &[
    "steam_api.dll", "steam_api64.dll",
    "libsteam_api.so", "libsteam_api64.so",
    "steamclient.dll", "steamclient64.dll",
    "steamclient.so", "steamclient64.so",
];

/// Recursively search `game_folder` for directories containing Steam DLLs.
/// Returns every directory that directly contains at least one Steam API file.
pub fn find_steam_dlls_recursive(game_folder: &str) -> Vec<PathBuf> {
    crate::api_emulators_shared::find_dll_dirs_recursive(Path::new(game_folder), STEAM_DLL_NAMES)
}

/// Check whether `dir` already has emulator backups (`.dll.bak`/`.bak.dll`/`.owo.dll`).
pub fn has_steam_emulator_backups(dir: &Path) -> bool {
    crate::api_emulators_shared::has_emulator_backups(dir, STEAM_DLL_NAMES)
}

/// List available GSE versions under api_emulators/steam/
pub fn list_gse_versions(save_dir: &str) -> Vec<String> {
    let root = api_emulators_dir(save_dir).join("steam");
    let mut versions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    if name != "generate_interfaces" {
                        versions.push(name.to_string());
                    }
                }
            }
        }
    }
    versions.sort();
    versions
}

fn resolve_gse_version(save_dir: &str, version: &str) -> Result<PathBuf, String> {
    let root = api_emulators_dir(save_dir).join("steam");
    if version.is_empty() {
        let versions = list_gse_versions(save_dir);
        let v = versions.first().ok_or_else(|| {
            format!("No GSE versions found at {:?}", root)
        })?;
        Ok(root.join(v))
    } else {
        let dir = root.join(version);
        if !dir.is_dir() {
            return Err(format!("GSE version '{}' not found at {:?}", version, dir));
        }
        Ok(dir)
    }
}

pub fn install_gse(
    save_dir: &str,
    game_exe: &str,
    app_id: &str,
    languages: &[String],
    version: &str,
) -> Result<(), String> {
    if !has_original_steam_dlls(game_exe) {
        return Err("No original Steam DLL found in game folder. Cannot install API emulator.".to_string());
    }

    let dll_folder = find_api_emu_dll_folder(game_exe, GSE_VERSION_FILES)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    let is_win = is_windows(game_exe);
    let is64 = detect_arch(game_exe) == "x64";

    install_gse_into_folder(save_dir, &dll_folder, is_win, is64, app_id, languages, version)
}

/// Recursively search `game_folder` for the directory containing Steam DLLs and
/// install the Goldberg emulator there. Used by the auto-add flow which only
/// knows the install folder, not the exe. Skips directories that already have
/// emulator backups (already patched).
pub fn install_gse_from_folder(
    save_dir: &str,
    game_folder: &str,
    app_id: &str,
    languages: &[String],
    version: &str,
) -> Result<(), String> {
    let dirs = find_steam_dlls_recursive(game_folder);
    let dll_folder = dirs.iter()
        .find(|d| !has_steam_emulator_backups(d) && !d.join("steam_settings").is_dir())
        .ok_or_else(|| "Steam DLLs already have an emulator (backups or steam_settings found)".to_string())?;

    let is_win = dll_folder.join("steam_api.dll").exists()
        || dll_folder.join("steam_api64.dll").exists();
    let is64 = dll_folder.join("steam_api64.dll").exists()
        || dll_folder.join("steamclient64.dll").exists()
        || dll_folder.join("libsteam_api64.so").exists()
        || dll_folder.join("steamclient64.so").exists();

    install_gse_into_folder(save_dir, dll_folder, is_win, is64, app_id, languages, version)
}

/// Scan a Steam API DLL for exported interface version strings and write
/// them to `steam_interfaces.txt`. This replaces the GSE `generate_interfaces`
/// tool, which is a Windows executable that can't run on Linux.
///
/// Ported from `references/gbe_fork/tools/generate_interfaces/generate_interfaces.cpp`.
fn generate_steam_interfaces(dll_path: &Path, output_path: &Path) -> Result<(), String> {
    let data = std::fs::read(dll_path).map_err(|e| format!("read DLL: {e}"))?;

    let mut all_matches: Vec<String> = Vec::new();

    for prefix in INTERFACE_PREFIXES {
        let found = find_prefix_matches(&data, prefix);
        // Special case: if SteamClient has multiple matches and 017 is among them,
        // keep only SteamClient017 (legacy compatibility from GSE source).
        if *prefix == "SteamClient" && found.len() > 1 && found.contains(&"SteamClient017".to_string()) {
            all_matches.push("SteamClient017".to_string());
        } else {
            all_matches.extend(found);
        }
    }

    if all_matches.is_empty() {
        return Err("no Steam interfaces found in DLL".to_string());
    }

    all_matches.sort();
    all_matches.dedup();
    let content = all_matches.join("\n") + "\n";
    std::fs::write(output_path, content)
        .map_err(|e| format!("write steam_interfaces.txt: {e}"))?;
    Ok(())
}

/// Interface name prefixes from the GSE generate_interfaces.cpp source.
/// Each is followed by 0+ digits in the binary.
const INTERFACE_PREFIXES: &[&str] = &[
    "STEAMAPPS_INTERFACE_VERSION",
    "SteamApps",
    "STEAMAPPLIST_INTERFACE_VERSION",
    "STEAMAPPTICKET_INTERFACE_VERSION",
    "SteamClient",
    "STEAMCONTROLLER_INTERFACE_VERSION",
    "SteamController",
    "SteamFriends",
    "SteamGameServerStats",
    "SteamGameCoordinator",
    "SteamGameServer",
    "STEAMHTMLSURFACE_INTERFACE_VERSION_",
    "STEAMHTTP_INTERFACE_VERSION",
    "SteamInput",
    "STEAMINVENTORY_INTERFACE_V",
    "SteamMatchMakingServers",
    "SteamMatchMaking",
    "SteamMatchGameSearch",
    "SteamParties",
    "STEAMMUSIC_INTERFACE_VERSION",
    "STEAMMUSICREMOTE_INTERFACE_VERSION",
    "SteamNetworkingMessages",
    "SteamNetworkingSockets",
    "SteamNetworkingUtils",
    "SteamNetworking",
    "STEAMPARENTALSETTINGS_INTERFACE_VERSION",
    "STEAMREMOTEPLAY_INTERFACE_VERSION",
    "STEAMREMOTESTORAGE_INTERFACE_VERSION",
    "STEAMSCREENSHOTS_INTERFACE_VERSION",
    "STEAMTIMELINE_INTERFACE_V",
    "STEAMUGC_INTERFACE_VERSION",
    "SteamUser",
    "STEAMUSERSTATS_INTERFACE_VERSION",
    "SteamUtils",
    "STEAMVIDEO_INTERFACE_V",
    "STEAMUNIFIEDMESSAGES_INTERFACE_VERSION",
    "SteamMasterServerUpdater",
];

/// Find all occurrences of `prefix` in the binary data, followed by 0+ digits.
/// Returns full matches (prefix + trailing digits), deduplicated.
fn find_prefix_matches(data: &[u8], prefix: &str) -> Vec<String> {
    let prefix_bytes = prefix.as_bytes();
    let mut matches = Vec::new();

    let mut i = 0;
    while i + prefix_bytes.len() <= data.len() {
        if &data[i..i + prefix_bytes.len()] == prefix_bytes {
            // Collect trailing digits
            let mut j = i + prefix_bytes.len();
            while j < data.len() && data[j].is_ascii_digit() {
                j += 1;
            }
            let full = &data[i..j];
            if let Ok(s) = std::str::from_utf8(full) {
                // Must have at least one digit, unless the prefix itself is the full match
                // (STEAMCONTROLLER_INTERFACE_VERSION has no digits in some SDKs)
                if (s.len() > prefix.len() || prefix.ends_with("VERSION"))
                    && !matches.contains(&s.to_string())
                {
                    matches.push(s.to_string());
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    matches
}

fn install_gse_into_folder(
    save_dir: &str,
    dll_folder: &Path,
    is_win: bool,
    is64: bool,
    app_id: &str,
    languages: &[String],
    version: &str,
) -> Result<(), String> {
    let version_dir = resolve_gse_version(save_dir, version)?;

    // Step 1: Create steam_settings dir before running generate_interfaces
    let settings_dir = dll_folder.join("steam_settings");
    std::fs::create_dir_all(&settings_dir)
        .map_err(|e| format!("create steam_settings: {}", e))?;

    // Step 2: Generate steam_interfaces.txt by scanning the original DLL
    // (before it's swapped in Step 3). We do this natively in Rust instead
    // of running the GSE generate_interfaces.exe, which doesn't work on Linux.
    let interfaces_path = settings_dir.join("steam_interfaces.txt");
    if !interfaces_path.exists() {
        let dll_name = if is64 { "steam_api64.dll" } else { "steam_api.dll" };
        let dll_path = dll_folder.join(dll_name);
        if dll_path.is_file() {
            if let Err(e) = generate_steam_interfaces(&dll_path, &interfaces_path) {
                eprintln!("Failed to generate steam_interfaces.txt: {}", e);
            }
        }
    }

    // Step 3: Backup original DLLs and copy emulator files
    for (src_name, dst_name) in gse_file_map(is64, is_win) {
        let src = version_dir.join(src_name);
        let dst = dll_folder.join(dst_name);
        if src.is_file() {
            backup_file(&dst)?;
            copy_file(&src, &dst)?;
        }
    }

    // Step 4: Write steam_appid.txt
    let appid_path = settings_dir.join("steam_appid.txt");
    if !appid_path.exists() {
        std::fs::write(&appid_path, app_id)
            .map_err(|e| format!("write steam_appid.txt: {}", e))?;
    }

    // Step 5: Write supported_languages.txt if languages available
    let lang_path = settings_dir.join("supported_languages.txt");
    if !lang_path.exists() && !languages.is_empty() {
        let content = languages.join("\n") + "\n";
        if let Err(e) = std::fs::write(&lang_path, content) {
            eprintln!("Failed to write supported_languages.txt: {}", e);
        }
    }

    // Step 6: Create achievement symlink
    let ach_dir = ira_parser::achievements_dir(save_dir, app_id);
    if !ach_dir.exists() {
        if let Some(parent) = ach_dir.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("Failed to create achievement directory: {}", e);
            }
        }
        #[cfg(unix)]
        if let Err(e) = std::os::unix::fs::symlink(&settings_dir, &ach_dir) {
            eprintln!("Failed to create achievement symlink: {}", e);
        }
    }

    Ok(())
}

pub fn uninstall_gse(game_exe: &str) -> Result<(), String> {
    let all_files: Vec<&str> = GSE_VERSION_FILES.to_vec();
    let dll_folder = find_api_emu_dll_folder(game_exe, &all_files)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    for file in &all_files {
        let path = dll_folder.join(file);
        restore_backup(&path)?;
    }
    Ok(())
}

pub fn is_gse_installed(game_exe: &str) -> bool {
    let all_files: Vec<&str> = GSE_VERSION_FILES.to_vec();
    if let Some(folder) = find_api_emu_dll_folder(game_exe, &all_files) {
        folder.join("steam_settings").is_dir()
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_prefix_matches_camelcase() {
        let data = b"\x00SteamClient017\x00SteamUser019\x00junk\x00";
        let matches = find_prefix_matches(data, "SteamClient");
        assert_eq!(matches, vec!["SteamClient017"]);
        let matches = find_prefix_matches(data, "SteamUser");
        assert_eq!(matches, vec!["SteamUser019"]);
    }

    #[test]
    fn test_find_prefix_matches_all_caps() {
        let data = b"\x00STEAMAPPS_INTERFACE_VERSION001\x00STEAMHTTP_INTERFACE_VERSION003\x00";
        let matches = find_prefix_matches(data, "STEAMAPPS_INTERFACE_VERSION");
        assert_eq!(matches, vec!["STEAMAPPS_INTERFACE_VERSION001"]);
        let matches = find_prefix_matches(data, "STEAMHTTP_INTERFACE_VERSION");
        assert_eq!(matches, vec!["STEAMHTTP_INTERFACE_VERSION003"]);
    }

    #[test]
    fn test_find_prefix_matches_no_digits() {
        // STEAMCONTROLLER_INTERFACE_VERSION may have no trailing digits
        let data = b"\x00STEAMCONTROLLER_INTERFACE_VERSION\x00";
        let matches = find_prefix_matches(data, "STEAMCONTROLLER_INTERFACE_VERSION");
        assert_eq!(matches, vec!["STEAMCONTROLLER_INTERFACE_VERSION"]);
    }

    #[test]
    fn test_find_prefix_matches_dedup() {
        let data = b"SteamClient017\x00SteamClient017\x00SteamClient020\x00";
        let matches = find_prefix_matches(data, "SteamClient");
        assert_eq!(matches.len(), 2);
        assert!(matches.contains(&"SteamClient017".to_string()));
        assert!(matches.contains(&"SteamClient020".to_string()));
    }

    #[test]
    fn test_generate_steam_interfaces_extracts_both_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        let dll_data = b"\x00SteamClient017\x00SteamUser019\x00STEAMAPPS_INTERFACE_VERSION001\x00STEAMHTTP_INTERFACE_VERSION003\x00junk\x00";
        let dll_path = tmp.path().join("steam_api64.dll");
        std::fs::write(&dll_path, dll_data).unwrap();

        let output_path = tmp.path().join("steam_interfaces.txt");
        generate_steam_interfaces(&dll_path, &output_path).unwrap();

        let content = std::fs::read_to_string(&output_path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert!(lines.contains(&"SteamClient017"));
        assert!(lines.contains(&"SteamUser019"));
        assert!(lines.contains(&"STEAMAPPS_INTERFACE_VERSION001"));
        assert!(lines.contains(&"STEAMHTTP_INTERFACE_VERSION003"));
        assert!(!lines.contains(&"junk"));
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn test_generate_steam_interfaces_steamclient_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        // Multiple SteamClient versions, 017 among them → keep only 017
        let dll_data = b"SteamClient017\x00SteamClient020\x00SteamUser019\x00";
        let dll_path = tmp.path().join("steam_api64.dll");
        std::fs::write(&dll_path, dll_data).unwrap();

        let output_path = tmp.path().join("steam_interfaces.txt");
        generate_steam_interfaces(&dll_path, &output_path).unwrap();

        let content = std::fs::read_to_string(&output_path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert!(lines.contains(&"SteamClient017"));
        assert!(!lines.contains(&"SteamClient020"));
        assert!(lines.contains(&"SteamUser019"));
    }

    #[test]
    fn test_generate_steam_interfaces_no_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let dll_data = b"random\x00data\x00without\x00interfaces\x00";
        let dll_path = tmp.path().join("steam_api64.dll");
        std::fs::write(&dll_path, dll_data).unwrap();

        let output_path = tmp.path().join("steam_interfaces.txt");
        assert!(generate_steam_interfaces(&dll_path, &output_path).is_err());
    }
}
