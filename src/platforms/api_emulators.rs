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

const GSE_LINUX_FILES: &[&str] = &["libsteam_api.so", "steamclient.so"];
const GSE_WIN_FILES_X64: &[&str] = &["steamapi64.dll", "steamclient64.dll"];
const GSE_WIN_FILES_X86: &[&str] = &["steam_api.dll", "steamclient.dll"];
const NGE_WIN_FILES_X64: &[&str] = &["Galaxy64.dll"];
const NGE_WIN_FILES_X86: &[&str] = &["Galaxy.dll"];

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
    std::fs::copy(src, dst).map_err(|e| format!("copy {:?} → {:?}: {}", src, dst, e))?;
    Ok(())
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

fn find_latest_file(dir: &Path) -> Option<PathBuf> {
    let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        if latest.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
                            latest = Some((mtime, entry.path()));
                        }
                    }
                }
            }
        }
    }
    latest.map(|(_, p)| p)
}

pub fn install_gse(
    save_dir: &str,
    game_exe: &str,
    app_id: &str,
) -> Result<(), String> {
    let emu_root = api_emulators_dir(save_dir).join("steam");
    let arch = detect_arch(game_exe);
    let platform = if is_windows(game_exe) { "windows" } else { "linux" };

    let dll_folder = find_api_emu_dll_folder(game_exe, GSE_LINUX_FILES)
        .or_else(|| find_api_emu_dll_folder(game_exe, GSE_WIN_FILES_X64))
        .or_else(|| find_api_emu_dll_folder(game_exe, GSE_WIN_FILES_X86))
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    let files: &[(&str, &str)] = if is_windows(game_exe) {
        if arch == "x64" {
            &[("steamapi", "steamapi64.dll"), ("steamclient", "steamclient64.dll")]
        } else {
            &[("steamapi", "steam_api.dll"), ("steamclient", "steamclient.dll")]
        }
    } else {
        &[("steamapi", "libsteam_api.so"), ("steamclient", "steamclient.so")]
    };

    for (sub, filename) in files {
        let src_dir = emu_root.join(platform).join(arch).join(sub);
        let dst = dll_folder.join(filename);
        backup_file(&dst)?;
        if src_dir.is_dir() {
            if let Some(src) = find_latest_file(&src_dir) {
                copy_file(&src, &dst)?;
            }
        }
    }

    let settings_dir = dll_folder.join("steam_settings");
    std::fs::create_dir_all(&settings_dir).map_err(|e| format!("create steam_settings: {}", e))?;

    let appid_path = settings_dir.join("steam_appid.txt");
    if !appid_path.exists() {
        std::fs::write(&appid_path, app_id).map_err(|e| format!("write steam_appid.txt: {}", e))?;
    }

    let tools_dir = emu_root.join("tools");
    let gen_interfaces = tools_dir.join("generate_interfaces");
    if gen_interfaces.is_file() {
        let dst = dll_folder.join("generate_interfaces");
        if !dst.exists() {
            let _ = copy_file(&gen_interfaces, &dst);
        }
    }

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
    let all_files: Vec<&str> = [GSE_LINUX_FILES, GSE_WIN_FILES_X64, GSE_WIN_FILES_X86].concat();
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
) -> Result<(), String> {
    if !is_windows(game_exe) {
        return Err("Nemirtingas API emulator is Windows-only (no Linux .so)".to_string());
    }

    let emu_root = api_emulators_dir(save_dir).join("gog");
    let arch = detect_arch(game_exe);
    let filename = if arch == "x64" { "Galaxy64.dll" } else { "Galaxy.dll" };

    let dll_folder = find_api_emu_dll_folder(game_exe, NGE_WIN_FILES_X64)
        .or_else(|| find_api_emu_dll_folder(game_exe, NGE_WIN_FILES_X86))
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    for variant in &["new", "old"] {
        let src_dir = emu_root.join(variant).join(arch);
        if src_dir.is_dir() {
            if let Some(src) = find_latest_file(&src_dir) {
                let dst = dll_folder.join(filename);
                backup_file(&dst)?;
                copy_file(&src, &dst)?;
                break;
            }
        }
    }

    let settings_dir = dll_folder.join("ngalaxye_settings");
    std::fs::create_dir_all(&settings_dir).map_err(|e| format!("create ngalaxye_settings: {}", e))?;

    crate::platforms::gog::generate_galaxy_emu_config(
        &dll_folder.to_string_lossy(),
        product_id,
    )?;

    Ok(())
}

pub fn uninstall_nge(game_exe: &str) -> Result<(), String> {
    let all_files: Vec<&str> = [NGE_WIN_FILES_X64, NGE_WIN_FILES_X86].concat();
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
    let all_files: Vec<&str> = [GSE_LINUX_FILES, GSE_WIN_FILES_X64, GSE_WIN_FILES_X86].concat();
    if let Some(folder) = find_api_emu_dll_folder(game_exe, &all_files) {
        folder.join("steam_settings").is_dir()
    } else {
        false
    }
}

pub fn is_nge_installed(game_exe: &str) -> bool {
    let all_files: Vec<&str> = [NGE_WIN_FILES_X64, NGE_WIN_FILES_X86].concat();
    if let Some(folder) = find_api_emu_dll_folder(game_exe, &all_files) {
        folder.join("ngalaxye_settings").is_dir()
    } else {
        false
    }
}
