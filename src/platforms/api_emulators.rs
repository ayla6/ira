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

pub fn read_current_language(
    trophy_source: &str,
    game_exe: &str,
    save_dir: &str,
    app_id: &str,
) -> Option<String> {
    match trophy_source {
        crate::models::GSE => {
            find_steam_settings(game_exe, save_dir, app_id)
                .and_then(|dir| read_gse_language(&dir))
        }
        crate::models::NGE => {
            find_galaxy_settings(game_exe)
                .and_then(|dir| read_nge_language(&dir))
        }
        _ => None,
    }
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

pub fn write_language_configs(
    trophy_source: &str,
    game_exe: &str,
    save_dir: &str,
    app_id: &str,
    language: &str,
) {
    if language.is_empty() {
        return;
    }
    match trophy_source {
        crate::models::GSE => {
            if let Some(settings_dir) = find_steam_settings(game_exe, save_dir, app_id) {
                if let Err(e) = write_gse_language(&settings_dir, language) {
                    eprintln!("Language config write failed: {}", e);
                }
            }
        }
        crate::models::NGE => {
            if let Some(settings_dir) = find_galaxy_settings(game_exe) {
                if let Err(e) = write_nge_language(&settings_dir, language) {
                    eprintln!("Language config write failed: {}", e);
                }
            }
        }
        _ => {}
    }
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

fn backup_file(path: &Path) -> Result<(), String> {
    let bak = path.with_extension(format!(
        "{}.bak",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    if !bak.exists() && path.exists() {
        std::fs::rename(path, &bak).map_err(|e| format!("backup failed: {}", e))?;
    }
    Ok(())
}

fn restore_backup(path: &Path) -> Result<(), String> {
    let bak = path.with_extension(format!(
        "{}.bak",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    if bak.exists() {
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| format!("remove emu file: {}", e))?;
        }
        std::fs::rename(&bak, path).map_err(|e| format!("restore backup: {}", e))?;
    }
    Ok(())
}

fn copy_file(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::copy(src, dst).map_err(|e| format!("copy {:?} \u{2192} {:?}: {}", src, dst, e))?;
    Ok(())
}

pub fn api_emulators_dir(save_dir: &str) -> PathBuf {
    Path::new(save_dir).join("api_emulators")
}

fn detect_arch(game_exe: &str) -> &'static str {
    if game_exe.ends_with(".exe") || game_exe.ends_with(".bat") {
        let is64 = game_exe.contains("64") || std::fs::metadata(game_exe)
            .map(|m| m.len() > 1_500_000).unwrap_or(false);
        if is64 { "x64" } else { "x86" }
    } else {
        if std::env::consts::ARCH == "x86_64" { "x64" } else { "x86" }
    }
}

fn is_windows(game_exe: &str) -> bool {
    game_exe.ends_with(".exe") || game_exe.ends_with(".bat")
}

// ========= Version-grouped directory layout =========
// api_emulators/
//   steam/
//     generate_interfaces           ← not versioned
//     <version>/
//       libsteam_api.so             ← Linux x86
//       libsteam_api64.so           ← Linux x64
//       steamclient.so              ← Linux x86
//       steamclient64.so            ← Linux x64
//       steam_api.dll               ← Windows x86
//       steamapi64.dll              ← Windows x64
//       steamclient.dll             ← Windows x86
//       steamclient64.dll           ← Windows x64
//   gog/
//     <version>/
//       Galaxy.dll                  ← Windows x86
//       Galaxy64.dll                ← Windows x64

const GSE_VERSION_FILES: &[&str] = &[
    "libsteam_api.so", "libsteam_api64.so",
    "steamclient.so", "steamclient64.so",
    "steam_api.dll", "steamapi64.dll",
    "steamclient.dll", "steamclient64.dll",
];

const NGE_VERSION_FILES: &[&str] = &["Galaxy.dll", "Galaxy64.dll"];

fn gse_file_map(is_64: bool, is_win: bool) -> &'static [(&'static str, &'static str)] {
    if is_win {
        if is_64 {
            &[
                ("steamapi64.dll", "steamapi64.dll"),
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

fn nge_file_map(is_64: bool) -> &'static [(&'static str, &'static str)] {
    if is_64 {
        &[("Galaxy64.dll", "Galaxy64.dll")]
    } else {
        &[("Galaxy.dll", "Galaxy.dll")]
    }
}

fn find_api_emu_dll_folder(game_exe: &str, dll_names: &[&str]) -> Option<PathBuf> {
    let exe_path = Path::new(game_exe);
    let start = exe_path.parent()?;
    if let Ok(entries) = std::fs::read_dir(start) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if dll_names.contains(&name.to_lowercase().as_str()) {
                    return Some(start.to_path_buf());
                }
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(start) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let sub = entry.path();
                if let Ok(sub_entries) = std::fs::read_dir(&sub) {
                    for se in sub_entries.flatten() {
                        if let Some(name) = se.file_name().to_str() {
                            if dll_names.contains(&name.to_lowercase().as_str()) {
                                return Some(sub);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Check if the game folder contains original Steam API DLLs
pub fn has_original_steam_dlls(game_exe: &str) -> bool {
    let dlls = &["libsteam_api.so", "steam_api.dll", "steamapi64.dll"];
    find_api_emu_dll_folder(game_exe, dlls).is_some()
}

/// Check if the game folder contains original GOG Galaxy DLLs
pub fn has_original_gog_dlls(game_exe: &str) -> bool {
    let dlls = &["Galaxy.dll", "Galaxy64.dll"];
    find_api_emu_dll_folder(game_exe, dlls).is_some()
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

pub fn ensure_skeleton(save_dir: &str) {
    let root = api_emulators_dir(save_dir);
    let dirs = [
        "steam",
        "gog",
    ];
    for d in &dirs {
        let _ = std::fs::create_dir_all(root.join(d));
    }
}

// ========= Install / Uninstall =========

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

pub fn install_gse(
    save_dir: &str,
    game_exe: &str,
    app_id: &str,
    languages: &[String],
    version: &str,
) -> Result<(), String> {
    if !has_original_steam_dlls(game_exe) {
        return Err(format!(
            "No original Steam DLL found in game folder. Cannot install API emulator."
        ));
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
            let _ = copy_file(&gen_interfaces, &dst);
        }
        // Run it from the steam_settings directory
        let _ = std::process::Command::new(&dst)
            .current_dir(&settings_dir)
            .output();
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
        let _ = std::fs::write(&lang_path, content);
    }

    // Step 6: Create achievement symlink
    let ach_dir = crate::parser::achievements_dir(save_dir, app_id);
    if !ach_dir.exists() {
        if let Some(parent) = ach_dir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        #[cfg(unix)]
        let _ = std::os::unix::fs::symlink(&settings_dir, &ach_dir);
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
        return Err(format!(
            "No original GOG Galaxy DLL found in game folder. Cannot install API emulator."
        ));
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

    crate::platforms::gog::generate_galaxy_emu_config(
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

pub fn is_gse_installed(game_exe: &str) -> bool {
    let all_files: Vec<&str> = GSE_VERSION_FILES.to_vec();
    if let Some(folder) = find_api_emu_dll_folder(game_exe, &all_files) {
        folder.join("steam_settings").is_dir()
    } else {
        false
    }
}

pub fn is_nge_installed(game_exe: &str) -> bool {
    let all_files: Vec<&str> = NGE_VERSION_FILES.to_vec();
    if let Some(folder) = find_api_emu_dll_folder(game_exe, &all_files) {
        folder.join("ngalaxye_settings").is_dir()
    } else {
        false
    }
}
