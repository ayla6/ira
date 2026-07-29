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

    let version_dir = resolve_gse_version(save_dir, version)?;

    let dll_folder = find_api_emu_dll_folder(game_exe, GSE_VERSION_FILES)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    let is_win = is_windows(game_exe);
    let is64 = detect_arch(game_exe) == "x64";

    // Step 1: Create steam_settings dir before running generate_interfaces
    let settings_dir = dll_folder.join("steam_settings");
    std::fs::create_dir_all(&settings_dir)
        .map_err(|e| format!("create steam_settings: {}", e))?;

    // Step 2: Run generate_interfaces BEFORE swapping DLLs
    let gen_interfaces = api_emulators_dir(save_dir).join("steam").join("generate_interfaces");
    if gen_interfaces.is_file() {
        let dst = settings_dir.join("generate_interfaces");
        if !dst.exists() {
            if let Err(e) = copy_file(&gen_interfaces, &dst) {
                eprintln!("Failed to copy generate_interfaces: {}", e);
            }
        }
        // Run it from the steam_settings directory
        if let Err(e) = std::process::Command::new(&dst)
            .current_dir(&settings_dir)
            .output()
        {
            eprintln!("Failed to run generate_interfaces: {}", e);
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
