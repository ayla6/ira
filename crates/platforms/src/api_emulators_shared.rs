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

/// Recursively walk `base_folder` and return every directory that directly
/// contains at least one of `dll_names` (case-insensitive). Iterative
/// depth-first search — no recursion limit concerns.
pub fn find_dll_dirs_recursive(base_folder: &Path, dll_names: &[&str]) -> Vec<PathBuf> {
    let lower_names: Vec<String> = dll_names.iter().map(|s| s.to_lowercase()).collect();
    let mut results = Vec::new();
    let mut stack = vec![base_folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut found_here = false;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        stack.push(entry.path());
                    } else if !found_here {
                        if let Some(name) = entry.file_name().to_str() {
                            if lower_names.contains(&name.to_lowercase()) {
                                found_here = true;
                            }
                        }
                    }
                }
            }
        }
        if found_here {
            results.push(dir);
        }
    }
    results
}

/// Find the directory containing one of `dll_names` inside a game install.
///
/// Tries the shallow exe-relative scan first (fast path for games whose DLLs
/// sit next to or one level under the exe), then falls back to a recursive
/// scan of `game_folder`. Prefers the deepest match — for nested installs
/// (e.g. Unreal Engine games) the API DLLs live several levels below the exe.
pub(crate) fn find_game_dll_folder(game_exe: &str, game_folder: &str, dll_names: &[&str]) -> Option<PathBuf> {
    if let Some(folder) = find_api_emu_dll_folder(game_exe, dll_names) {
        return Some(folder);
    }
    if game_folder.is_empty() {
        return None;
    }
    let mut dirs = find_dll_dirs_recursive(Path::new(game_folder), dll_names);
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    dirs.into_iter().next()
}

/// Backup filename variants for a given DLL name.
/// For `steam_api64.dll` returns:
///   `steam_api64.dll.bak`, `steam_api64.bak.dll`,
///   `steam_api64.owo.dll`, `steam_api64.dll.owo`
fn backup_variants(dll_name: &str) -> Vec<String> {
    let (stem, ext) = match dll_name.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => return Vec::new(),
    };
    vec![
        format!("{}.{}.bak", stem, ext),
        format!("{}.bak.{}", stem, ext),
        format!("{}.owo.{}", stem, ext),
        format!("{}.{}.owo", stem, ext),
    ]
}

/// Check whether `dir` contains any emulator backup file for the given DLL
/// names. Returns true if the emulator appears to already be installed
/// (originals backed up as `.dll.bak`, `.bak.dll`, `.owo.dll`, or `.dll.owo`).
pub fn has_emulator_backups(dir: &Path, dll_names: &[&str]) -> bool {
    for dll in dll_names {
        for variant in backup_variants(dll) {
            if dir.join(&variant).exists() {
                return true;
            }
        }
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_variants_dll() {
        let v = backup_variants("steam_api64.dll");
        assert_eq!(v, vec![
            "steam_api64.dll.bak".to_string(),
            "steam_api64.bak.dll".to_string(),
            "steam_api64.owo.dll".to_string(),
            "steam_api64.dll.owo".to_string(),
        ]);
    }

    #[test]
    fn test_backup_variants_so() {
        let v = backup_variants("libsteam_api.so");
        assert_eq!(v, vec![
            "libsteam_api.so.bak".to_string(),
            "libsteam_api.bak.so".to_string(),
            "libsteam_api.owo.so".to_string(),
            "libsteam_api.so.owo".to_string(),
        ]);
    }

    #[test]
    fn test_backup_variants_no_extension() {
        let v = backup_variants("noext");
        assert!(v.is_empty());
    }

    #[test]
    fn test_has_emulator_backups_detects_all_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        for name in &["steam_api64.dll.bak", "steam_api64.bak.dll", "steam_api64.owo.dll"] {
            std::fs::write(dir.join(name), b"x").unwrap();
            assert!(has_emulator_backups(dir, &["steam_api64.dll"]), "failed for {}", name);
            std::fs::remove_file(dir.join(name)).unwrap();
        }
        assert!(!has_emulator_backups(dir, &["steam_api64.dll"]));
    }

    #[test]
    fn test_find_dll_dirs_recursive_finds_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // bin/win64/steam_api64.dll
        let nested = root.join("bin").join("win64");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("steam_api64.dll"), b"x").unwrap();
        // root also has a libsteam_api.so
        std::fs::write(root.join("libsteam_api.so"), b"x").unwrap();

        let found = find_dll_dirs_recursive(root, &["steam_api64.dll", "libsteam_api.so"]);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&root.to_path_buf()));
        assert!(found.contains(&nested));
    }

    #[test]
    fn test_find_dll_dirs_recursive_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("Steam_API64.DLL"), b"x").unwrap();
        let found = find_dll_dirs_recursive(root, &["steam_api64.dll"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], root);
    }

    #[test]
    fn test_find_dll_dirs_recursive_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let found = find_dll_dirs_recursive(tmp.path(), &["steam_api64.dll"]);
        assert!(found.is_empty());
    }
}
