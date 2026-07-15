use std::path::{Path, PathBuf};

pub(crate) fn backup_file(path: &Path) -> Result<(), String> {
    let bak = path.with_extension(format!(
        "{}.bak",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    if !bak.exists() && path.exists() {
        std::fs::rename(path, &bak).map_err(|e| format!("backup failed: {}", e))?;
    }
    Ok(())
}

pub(crate) fn restore_backup(path: &Path) -> Result<(), String> {
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

pub(crate) fn copy_file(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::copy(src, dst).map_err(|e| format!("copy {:?} \u{2192} {:?}: {}", src, dst, e))?;
    Ok(())
}

pub fn api_emulators_dir(save_dir: &str) -> PathBuf {
    Path::new(save_dir).join("api_emulators")
}

pub(crate) fn detect_arch(game_exe: &str) -> &'static str {
    if game_exe.ends_with(".exe") || game_exe.ends_with(".bat") {
        let is64 = game_exe.contains("64") || std::fs::metadata(game_exe)
            .map(|m| m.len() > 1_500_000).unwrap_or(false);
        if is64 { "x64" } else { "x86" }
    } else {
        if std::env::consts::ARCH == "x86_64" { "x64" } else { "x86" }
    }
}

pub(crate) fn is_windows(game_exe: &str) -> bool {
    game_exe.ends_with(".exe") || game_exe.ends_with(".bat")
}

pub(crate) fn find_api_emu_dll_folder(game_exe: &str, dll_names: &[&str]) -> Option<PathBuf> {
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
