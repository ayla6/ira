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

fn find_emu_dll_folder(game_exe: &str, dll_names: &[&str]) -> Option<PathBuf> {
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

pub fn install_gse(
    emu_dir: &str,
    variant: &str,
    game_exe: &str,
    app_id: &str,
) -> Result<(), String> {
    let variant_dir = Path::new(emu_dir).join("gse").join(variant);
    let (arch_files, src_arch) = if game_exe.ends_with(".exe") || game_exe.ends_with(".bat") {
        let is64 = game_exe.contains("64") || std::fs::metadata(game_exe)
            .map(|m| m.len() > 1_500_000).unwrap_or(false);
        if is64 {
            (GSE_WIN_FILES_X64, "x64")
        } else {
            (GSE_WIN_FILES_X86, "x86")
        }
    } else {
        let is64 = std::env::consts::ARCH == "x86_64";
        if is64 {
            (GSE_LINUX_FILES, "x64")
        } else {
            (GSE_LINUX_FILES, "x86")
        }
    };

    let src_dir = variant_dir.join(src_arch);
    let dll_folder = find_emu_dll_folder(game_exe, arch_files)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    for file in arch_files {
        let dst = dll_folder.join(file);
        backup_file(&dst)?;
        let src = src_dir.join(file);
        if src.exists() {
            copy_file(&src, &dst)?;
        }
    }

    let settings_dir = dll_folder.join("steam_settings");
    std::fs::create_dir_all(&settings_dir).map_err(|e| format!("create steam_settings: {}", e))?;

    let appid_path = settings_dir.join("steam_appid.txt");
    if !appid_path.exists() {
        std::fs::write(&appid_path, app_id).map_err(|e| format!("write steam_appid.txt: {}", e))?;
    }

    let tools_dir = Path::new(emu_dir).join("gse").join("tools");
    let gen_interfaces = tools_dir.join("generate_interfaces");
    if gen_interfaces.is_file() {
        let dst = dll_folder.join("generate_interfaces");
        if !dst.exists() {
            let _ = copy_file(&gen_interfaces, &dst);
        }
    }

    Ok(())
}

pub fn uninstall_gse(game_exe: &str) -> Result<(), String> {
    let all_files = [GSE_LINUX_FILES, GSE_WIN_FILES_X64, GSE_WIN_FILES_X86].concat();
    let dll_folder = find_emu_dll_folder(game_exe, &all_files)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    for file in &all_files {
        let path = dll_folder.join(file);
        restore_backup(&path)?;
    }
    Ok(())
}

pub fn install_nge(
    emu_dir: &str,
    game_exe: &str,
    product_id: &str,
) -> Result<(), String> {
    let src_dir = Path::new(emu_dir).join("nge");
    let (arch_files, src_arch) = if game_exe.ends_with(".exe") || game_exe.ends_with(".bat") {
        let is64 = game_exe.contains("64") || std::fs::metadata(game_exe)
            .map(|m| m.len() > 1_500_000).unwrap_or(false);
        if is64 {
            (NGE_WIN_FILES_X64, "x64")
        } else {
            (NGE_WIN_FILES_X86, "x86")
        }
    } else {
        return Err("Nemirtingas emulator is Windows-only (no Linux .so)".to_string());
    };

    let src_arch_dir = if src_dir.join(src_arch).is_dir() {
        src_dir.join(src_arch)
    } else {
        src_dir
    };

    let dll_folder = find_emu_dll_folder(game_exe, arch_files)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    for file in arch_files {
        let dst = dll_folder.join(file);
        backup_file(&dst)?;
        let src = src_arch_dir.join(file);
        if src.exists() {
            copy_file(&src, &dst)?;
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
    let all_files = [NGE_WIN_FILES_X64, NGE_WIN_FILES_X86].concat();
    let dll_folder = find_emu_dll_folder(game_exe, &all_files)
        .or_else(|| Path::new(game_exe).parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Cannot determine game DLL folder".to_string())?;

    for file in &all_files {
        let path = dll_folder.join(file);
        restore_backup(&path)?;
    }
    Ok(())
}

pub fn is_gse_installed(game_exe: &str) -> bool {
    let all_files = [GSE_LINUX_FILES, GSE_WIN_FILES_X64, GSE_WIN_FILES_X86].concat();
    if let Some(folder) = find_emu_dll_folder(game_exe, &all_files) {
        folder.join("steam_settings").is_dir()
    } else {
        false
    }
}

pub fn is_nge_installed(game_exe: &str) -> bool {
    let all_files = [NGE_WIN_FILES_X64, NGE_WIN_FILES_X86].concat();
    if let Some(folder) = find_emu_dll_folder(game_exe, &all_files) {
        folder.join("ngalaxye_settings").is_dir()
    } else {
        false
    }
}
