use crate::api_emulators_shared::{
    api_emulators_dir, detect_arch, detect_folder_bitness, dir_has_any_dll, find_game_dll_folder,
    has_emulator_backups, is_windows, prepare_original_file, restore_backup,
};
use ira_models::AppDetails;
use std::path::{Path, PathBuf};

pub fn find_steam_settings(
    game_exe: &str,
    game_folder: &str,
    save_dir: &str,
    app_id: &str,
) -> Option<PathBuf> {
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
    crate::api_emulators_shared::find_settings_from_exe(
        Path::new(game_exe),
        Path::new(game_folder),
        "steam_settings",
    )
}

/// Find `steam_settings/` in the game folder. Move a discovered directory to
/// the game root, link its old location back to the root, and point every
/// Steam-DLL directory at the same canonical settings directory.
///
/// This ensures GBE/GSE finds its config regardless of which subdirectory the
/// Steam DLL lives in, without duplicating or moving config files.
pub fn centralize_steam_settings(
    game_folder: &str,
    game_exe: &str,
) -> Result<Option<PathBuf>, String> {
    crate::api_emulators_shared::centralize_settings(
        game_folder,
        "steam_settings",
        STEAM_DLL_NAMES,
        Some(Path::new(game_exe)),
    )
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
        // A failed read must not fall through to the rewrite below — that
        // would replace the whole file with just the language line.
        content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
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
    "libsteam_api.so",
    "libsteam_api64.so",
    "steamclient.so",
    "steamclient64.so",
    "steam_api.dll",
    "steam_api64.dll",
    "steamclient.dll",
    "steamclient64.dll",
];

/// Files in a game folder indicating a 64-bit Steam API setup.
const STEAM_64_FILES: &[&str] = &[
    "steam_api64.dll",
    "steamclient64.dll",
    "libsteam_api64.so",
    "steamclient64.so",
];

/// Files in a game folder indicating a 32-bit Steam API setup.
const STEAM_32_FILES: &[&str] = &[
    "steam_api.dll",
    "steamclient.dll",
    "libsteam_api.so",
    "steamclient.so",
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

/// Resolve the directory holding the Steam API DLLs for a game install.
/// Falls back from a shallow exe-relative scan to a recursive scan of the
/// full game folder (nested installs like Unreal Engine games).
pub fn find_steam_dll_folder(game_exe: &str, game_folder: &str) -> Option<PathBuf> {
    find_game_dll_folder(game_exe, game_folder, GSE_VERSION_FILES)
}

/// Check if `dll_folder` contains original Steam API DLLs, either in place or
/// renamed under a backup-style name (`.bak`, `.owo`, `_o`) by another tool.
pub fn has_original_steam_dlls(dll_folder: &Path) -> bool {
    const ORIGINAL_API_FILES: &[&str] = &[
        "libsteam_api.so",
        "libsteam_api64.so",
        "steam_api.dll",
        "steam_api64.dll",
    ];
    dir_has_any_dll(dll_folder, ORIGINAL_API_FILES)
        || has_emulator_backups(dll_folder, ORIGINAL_API_FILES)
}

/// All Steam API DLL/SO filenames (original and emulator share these names).
const STEAM_DLL_NAMES: &[&str] = &[
    "steam_api.dll",
    "steam_api64.dll",
    "libsteam_api.so",
    "libsteam_api64.so",
    "steamclient.dll",
    "steamclient64.dll",
    "steamclient.so",
    "steamclient64.so",
];

/// Returns every directory that directly contains at least one Steam API file.
pub fn find_steam_dlls_recursive(game_folder: &str) -> Vec<PathBuf> {
    crate::api_emulators_shared::find_dll_dirs_recursive(Path::new(game_folder), STEAM_DLL_NAMES)
}

pub fn has_steam_emulator_backups(dir: &Path) -> bool {
    crate::api_emulators_shared::has_emulator_backups(dir, STEAM_DLL_NAMES)
}

pub fn list_gse_versions(save_dir: &str) -> Vec<String> {
    let root = api_emulators_dir(save_dir).join("steam");
    crate::api_emulators_shared::list_version_dirs(&root, &["generate_interfaces"])
}

fn resolve_gse_version(save_dir: &str, version: &str) -> Result<PathBuf, String> {
    crate::api_emulators_shared::resolve_emu_version(save_dir, "steam", version, "GSE")
}

pub fn install_gse(
    save_dir: &str,
    game_exe: &str,
    game_folder: &str,
    app_id: &str,
    languages: &[String],
    version: &str,
) -> Result<PathBuf, String> {
    let dll_folder = find_game_dll_folder(game_exe, game_folder, GSE_VERSION_FILES)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    if !has_original_steam_dlls(&dll_folder) {
        return Err(
            "No original Steam DLL found in game folder. Cannot install API emulator.".to_string(),
        );
    }

    let is_win = is_windows(game_exe);
    // The DLLs in the folder are the ground truth for which GSE build to use —
    // exe name/size heuristics misclassify small launcher stubs as 32-bit.
    let is64 = detect_folder_bitness(&dll_folder, STEAM_64_FILES, STEAM_32_FILES)
        .unwrap_or_else(|| detect_arch(game_exe) == "x64");

    install_gse_into_folder(
        save_dir,
        &dll_folder,
        is_win,
        is64,
        app_id,
        languages,
        version,
    )?;
    Ok(dll_folder)
}

/// Recursively search `game_folder` for the directory containing Steam DLLs and
/// install the Goldberg emulator there. Used by the auto-add flow which only
/// knows the install folder, not the exe. Skips directories that already have
/// emulator backups (already patched, including `_o`-renamed originals left
/// by cracks). `steam_settings` is not treated as evidence of an install — a
/// centralized root `steam_settings` with symlinks exists for every game.
pub fn install_gse_from_folder(
    save_dir: &str,
    game_folder: &str,
    app_id: &str,
    languages: &[String],
    version: &str,
) -> Result<(), String> {
    let dirs = find_steam_dlls_recursive(game_folder);
    let dll_folder = dirs
        .iter()
        .find(|d| !has_steam_emulator_backups(d))
        .ok_or_else(|| "Steam DLLs already have an emulator (backups found)".to_string())?;

    let is_win = dir_has_any_dll(dll_folder, &["steam_api.dll", "steam_api64.dll"]);
    let is64 = detect_folder_bitness(dll_folder, STEAM_64_FILES, STEAM_32_FILES).unwrap_or(false);

    install_gse_into_folder(
        save_dir, dll_folder, is_win, is64, app_id, languages, version,
    )
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
        if *prefix == "SteamClient"
            && found.len() > 1
            && found.contains(&"SteamClient017".to_string())
        {
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
    std::fs::write(output_path, content).map_err(|e| format!("write steam_interfaces.txt: {e}"))?;
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
    let mut found = Vec::new();

    let mut i = 0;
    while i + prefix_bytes.len() <= data.len() {
        if &data[i..i + prefix_bytes.len()] == prefix_bytes {
            let mut j = i + prefix_bytes.len();
            while j < data.len() && data[j].is_ascii_digit() {
                j += 1;
            }
            let full = &data[i..j];
            if let Ok(s) = std::str::from_utf8(full) {
                // Must have at least one digit, unless the prefix itself is the full match
                // (STEAMCONTROLLER_INTERFACE_VERSION has no digits in some SDKs)
                if (s.len() > prefix.len() || prefix.ends_with("VERSION"))
                    && !found.contains(&s.to_string())
                {
                    found.push(s.to_string());
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    found
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
    let (primary_src, _) = gse_file_map(is64, is_win)
        .first()
        .ok_or_else(|| "No GSE API files configured".to_string())?;
    if !version_dir.join(primary_src).is_file() {
        return Err(format!(
            "GSE version is missing required file {}",
            version_dir.join(primary_src).display()
        ));
    }

    let settings_dir = dll_folder.join("steam_settings");
    std::fs::create_dir_all(&settings_dir).map_err(|e| format!("create steam_settings: {}", e))?;

    // Scan the original DLL before it is swapped below; the GSE
    // generate_interfaces.exe can't run on Linux.
    let interfaces_path = settings_dir.join("steam_interfaces.txt");
    if !interfaces_path.exists() {
        let dll_name = if is64 {
            "steam_api64.dll"
        } else {
            "steam_api.dll"
        };
        let dll_path = crate::api_emulators_shared::find_file_case_insensitive(dll_folder, dll_name)
            .unwrap_or_else(|| dll_folder.join(dll_name));
        if dll_path.is_file() {
            if let Err(e) = generate_steam_interfaces(&dll_path, &interfaces_path) {
                eprintln!("Failed to generate steam_interfaces.txt: {}", e);
            }
        }
    }

    // Every DLL swap below is journaled: any failure rolls the folder
    // back to its pre-install state instead of leaving a partial one.
    let mut swapped: Vec<crate::api_emulators_shared::SwappedFile> = Vec::new();
    let install = (|| {
        for (src_name, dst_name) in gse_file_map(is64, is_win) {
            let src = version_dir.join(src_name);
            if !src.is_file() {
                continue;
            }
            let original = prepare_original_file(dll_folder, src_name, dst_name)?;
            let dst = dll_folder.join(dst_name);
            // Preserve the game's API surface: optional files such as
            // `steamclient` are patched only when the game already ships them.
            if original.is_some() || has_emulator_backups(dll_folder, &[dst_name, src_name])
            {
                swapped.push(crate::api_emulators_shared::swap_in_with_backup(
                    &src,
                    &dst,
                    original.as_deref(),
                )?);
            }
        }

        let appid_path = settings_dir.join("steam_appid.txt");
        if !appid_path.exists() {
            std::fs::write(&appid_path, app_id)
                .map_err(|e| format!("write steam_appid.txt: {}", e))?;
        }
        Ok::<(), String>(())
    })();
    if let Err(e) = install {
        crate::api_emulators_shared::rollback_swapped(&swapped);
        return Err(e);
    }

    let lang_path = settings_dir.join("supported_languages.txt");
    if !lang_path.exists() && !languages.is_empty() {
        let content = languages.join("\n") + "\n";
        if let Err(e) = std::fs::write(&lang_path, content) {
            eprintln!("Failed to write supported_languages.txt: {}", e);
        }
    }

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

pub fn uninstall_gse(game_exe: &str, game_folder: &str) -> Result<(), String> {
    let dll_folder = find_game_dll_folder(game_exe, game_folder, GSE_VERSION_FILES)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    let is_win = is_windows(game_exe);
    let is64 = has_emulator_backups(&dll_folder, STEAM_64_FILES)
        || detect_folder_bitness(&dll_folder, STEAM_64_FILES, STEAM_32_FILES)
            .unwrap_or_else(|| detect_arch(game_exe) == "x64");
    let generated_targets: Vec<PathBuf> = if is_win {
        Vec::new()
    } else {
        gse_file_map(is64, false)
            .iter()
            .filter(|(source, destination)| {
                source != destination && has_emulator_backups(&dll_folder, &[*source])
            })
            .map(|(_, destination)| dll_folder.join(destination))
            .collect()
    };

    for file in GSE_VERSION_FILES {
        let path = dll_folder.join(file);
        restore_backup(&path)?;
    }
    for path in generated_targets {
        if path.is_file() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("remove generated Steam DLL {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Whether the GSE emulator is currently installed in `dll_folder`, i.e. the
/// original API DLLs have been backed up and replaced with emulator builds.
/// The `steam_settings` dir is not used as evidence — a centralized root
/// `steam_settings` with symlinks exists for any game.
pub fn is_gse_installed(dll_folder: &Path) -> bool {
    has_steam_emulator_backups(dll_folder)
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

    #[test]
    fn test_is_gse_installed_requires_backups() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("steam_settings")).unwrap();
        assert!(!is_gse_installed(dir));
        std::fs::write(dir.join("steam_api64.dll.bak"), b"x").unwrap();
        assert!(is_gse_installed(dir));
    }

    #[test]
    fn test_gse_preserves_missing_steamclient_and_restores_api() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().join("save");
        let version_dir = save_dir.join("api_emulators/steam/v1");
        let game_dir = tmp.path().join("game");
        let game_exe = game_dir.join("game.exe");
        std::fs::create_dir_all(&version_dir).unwrap();
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(&game_exe, b"exe").unwrap();
        std::fs::write(game_dir.join("steam_api64.dll"), b"original api").unwrap();
        std::fs::write(version_dir.join("steam_api64.dll"), b"emulator api").unwrap();
        std::fs::write(version_dir.join("steamclient64.dll"), b"emulator client").unwrap();

        install_gse(
            &save_dir.to_string_lossy(),
            &game_exe.to_string_lossy(),
            &game_dir.to_string_lossy(),
            "123",
            &[],
            "v1",
        )
        .unwrap();

        assert_eq!(
            std::fs::read(game_dir.join("steam_api64.dll")).unwrap(),
            b"emulator api"
        );
        assert!(!game_dir.join("steamclient64.dll").exists());

        uninstall_gse(
            &game_exe.to_string_lossy(),
            &game_dir.to_string_lossy(),
        )
        .unwrap();

        assert_eq!(
            std::fs::read(game_dir.join("steam_api64.dll")).unwrap(),
            b"original api"
        );
        assert!(!game_dir.join("steamclient64.dll").exists());
    }

    #[test]
    fn test_gse_normalizes_mixed_case_linux_api_before_installing() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().join("save");
        let version_dir = save_dir.join("api_emulators/steam/v1");
        let game_dir = tmp.path().join("game");
        let game_exe = game_dir.join("game.x86_64");
        std::fs::create_dir_all(&version_dir).unwrap();
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(&game_exe, b"elf").unwrap();
        std::fs::write(game_dir.join("LibSteam_Api64.So"), b"original api").unwrap();
        std::fs::write(version_dir.join("libsteam_api64.so"), b"emulator api").unwrap();

        install_gse(
            &save_dir.to_string_lossy(),
            &game_exe.to_string_lossy(),
            &game_dir.to_string_lossy(),
            "123",
            &[],
            "v1",
        )
        .unwrap();

        assert_eq!(
            std::fs::read(game_dir.join("libsteam_api.so")).unwrap(),
            b"emulator api"
        );
        assert_eq!(
            std::fs::read(game_dir.join("libsteam_api64.so.bak")).unwrap(),
            b"original api"
        );
        assert!(!game_dir.join("LibSteam_Api64.So").exists());

        uninstall_gse(
            &game_exe.to_string_lossy(),
            &game_dir.to_string_lossy(),
        )
        .unwrap();
        assert!(!game_dir.join("libsteam_api.so").exists());
        assert_eq!(
            std::fs::read(game_dir.join("libsteam_api64.so")).unwrap(),
            b"original api"
        );
    }

    #[test]
    fn test_gse_missing_api_does_not_create_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().join("save");
        let version_dir = save_dir.join("api_emulators/steam/v1");
        let game_dir = tmp.path().join("game");
        let game_exe = game_dir.join("game.exe");
        std::fs::create_dir_all(&version_dir).unwrap();
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(&game_exe, b"exe").unwrap();
        std::fs::write(game_dir.join("steam_api64.dll"), b"original api").unwrap();

        assert!(install_gse(
            &save_dir.to_string_lossy(),
            &game_exe.to_string_lossy(),
            &game_dir.to_string_lossy(),
            "123",
            &[],
            "v1",
        )
        .is_err());
        assert!(!game_dir.join("steam_settings").exists());
        assert!(!ira_parser::achievements_dir(&save_dir.to_string_lossy(), "123").exists());
    }

    #[test]
    fn test_find_steam_settings_searches_nested_game_platforms() {
        let tmp = tempfile::tempdir().unwrap();
        let game_root = tmp.path().join("RenpyGame");
        let linux_dir = game_root.join("Linux");
        let linux_settings = linux_dir.join("steam_settings");
        let windows_settings = game_root.join("Windows").join("steam_settings");
        std::fs::create_dir_all(&linux_settings).unwrap();
        std::fs::create_dir_all(&windows_settings).unwrap();
        std::fs::write(linux_dir.join("money_for_girls.sh"), b"x").unwrap();
        std::fs::write(linux_settings.join("steam_appid.txt"), b"123").unwrap();
        std::fs::write(windows_settings.join("steam_appid.txt"), b"123").unwrap();

        let found = find_steam_settings(
            &linux_dir.join("money_for_girls.sh").to_string_lossy(),
            &game_root.to_string_lossy(),
            &tmp.path().join("save").to_string_lossy(),
            "123",
        );

        assert_eq!(found, Some(linux_settings.clone()));

        std::fs::remove_dir_all(&linux_settings).unwrap();
        let found = find_steam_settings(
            &linux_dir.join("money_for_girls.sh").to_string_lossy(),
            &game_root.to_string_lossy(),
            &tmp.path().join("save").to_string_lossy(),
            "123",
        );
        assert_eq!(found, Some(windows_settings));
    }

    #[test]
    fn test_find_steam_settings_stays_inside_game_root() {
        let tmp = tempfile::tempdir().unwrap();
        let game_root = tmp.path().join("CurrentGame");
        let other_settings = tmp.path().join("OtherGame").join("steam_settings");
        let exe = game_root.join("bin").join("game.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&other_settings).unwrap();
        std::fs::write(&exe, b"x").unwrap();
        std::fs::write(other_settings.join("steam_appid.txt"), b"123").unwrap();

        let found = find_steam_settings(
            &exe.to_string_lossy(),
            &game_root.to_string_lossy(),
            &tmp.path().join("save").to_string_lossy(),
            "123",
        );

        assert_eq!(found, None);
    }

    #[test]
    fn test_has_original_steam_dlls_accepts_o_renamed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // Cracked game: original renamed to _o, wrapper sits at the plain name.
        std::fs::write(dir.join("steam_api64_o.dll"), b"x").unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"wrapper").unwrap();
        assert!(has_original_steam_dlls(dir));

        // Even without the wrapper the stashed original counts.
        std::fs::remove_file(dir.join("steam_api64.dll")).unwrap();
        assert!(has_original_steam_dlls(dir));
    }

    #[test]
    fn test_has_original_steam_dlls_empty_folder() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!has_original_steam_dlls(tmp.path()));
    }

    #[test]
    fn test_has_original_steam_dlls_accepts_mixed_case_linux_64_api() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("LibSteam_Api64.So"), b"x").unwrap();
        assert!(has_original_steam_dlls(tmp.path()));
    }
}
