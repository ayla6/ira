use std::path::{Path, PathBuf};

/// A redistributable package found under `_CommonRedist` (e.g. DotNet, vcredist).
#[derive(Debug, Clone)]
pub struct RedistPackage {
    pub name: String,
    pub installers: Vec<PathBuf>,
}

/// Scan `<steamapps>/common/Steamworks Shared/_CommonRedist` for redistributable
/// packages. Always ignores the `DirectX` directory. Returns one entry per
/// non-empty package directory, with the `.exe` installers found inside
/// (recursively, sorted).
pub fn detect_redists(steamapps_dir: &Path) -> Vec<RedistPackage> {
    let base = steamapps_dir
        .join("common")
        .join("Steamworks Shared")
        .join("_CommonRedist");
    detect_redists_in_base(&base)
}

/// Scan `<game_dir>/_CommonRedist` for redistributable packages.
/// Same logic as `detect_redists` but for games not in a Steam library.
pub fn detect_redists_in_game_folder(game_dir: &Path) -> Vec<RedistPackage> {
    detect_redists_in_base(&game_dir.join("_CommonRedist"))
}

/// If redist installer paths point to a shared `_CommonRedist` (e.g. in
/// `Steamworks Shared`), copy that directory into `game_dir/_CommonRedist`
/// and remap the installer paths. If the installers are already inside the
/// game dir, they're returned unchanged.
pub fn localize_redists(game_dir: &Path, packages: Vec<RedistPackage>) -> Vec<RedistPackage> {
    if packages.is_empty() {
        return packages;
    }

    let local_base = game_dir.join("_CommonRedist");
    let game_dir_canonical = game_dir.canonicalize().unwrap_or_else(|_| game_dir.to_path_buf());

    // Check if installers are already inside the game folder
    let already_local = packages.iter().all(|p| {
        p.installers.iter().all(|inst| {
            inst.canonicalize()
                .map(|c| c.starts_with(&game_dir_canonical))
                .unwrap_or(false)
        })
    });
    if already_local {
        return packages;
    }

    // Find the _CommonRedist source directory from the first installer path
    let source_base = packages
        .iter()
        .flat_map(|p| &p.installers)
        .find_map(|inst| find_common_redist_ancestor(inst));

    let Some(source_base) = source_base else {
        return packages;
    };

    // Copy _CommonRedist to game_dir if it doesn't already exist
    if !local_base.exists() {
        if let Err(e) = copy_dir_recursive(&source_base, &local_base) {
            eprintln!("Failed to copy _CommonRedist to game folder: {}", e);
            return packages;
        }
    }

    // Remap installer paths from source_base to local_base
    packages
        .into_iter()
        .map(|pkg| RedistPackage {
            name: pkg.name,
            installers: pkg
                .installers
                .into_iter()
                .map(|inst| remap_path(&inst, &source_base, &local_base))
                .collect(),
        })
        .collect()
}

/// Walk up from a path to find the nearest `_CommonRedist` ancestor.
fn find_common_redist_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = path.parent()?;
    loop {
        if current.file_name().and_then(|n| n.to_str()) == Some("_CommonRedist") {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

/// Replace the `source_base` prefix of `path` with `local_base`.
fn remap_path(path: &Path, source_base: &Path, local_base: &Path) -> PathBuf {
    if let Ok(rel) = path.strip_prefix(source_base) {
        local_base.join(rel)
    } else {
        path.to_path_buf()
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    let entries = std::fs::read_dir(src).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn detect_redists_in_base(base: &Path) -> Vec<RedistPackage> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut names: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.eq_ignore_ascii_case("DirectX") {
            continue;
        }
        names.push((name, entry.path()));
    }
    names.sort_by(|a, b| a.0.cmp(&b.0));

    let mut packages = Vec::new();
    for (name, dir) in names {
        let installers = find_exe_installers(&dir);
        if !installers.is_empty() {
            packages.push(RedistPackage { name, installers });
        }
    }
    packages
}

/// Recursively collect `.exe` files under `dir`, sorted alphabetically.
fn find_exe_installers(dir: &Path) -> Vec<PathBuf> {
    let mut exes = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
            {
                exes.push(path);
            }
        }
    }
    exes.sort();
    exes
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(path: &Path) {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(path, b"x").unwrap();
    }

    fn steamapps_with_redists() -> TempDir {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("common").join("Steamworks Shared").join("_CommonRedist");
        touch(&base.join("DirectX").join("Jun2010").join("DXSETUP.exe"));
        touch(&base.join("DotNet").join("3.5").join("dotnetfx35.exe"));
        touch(&base.join("vcredist").join("2012").join("vcredist_x64.exe"));
        touch(&base.join("vcredist").join("2012").join("vcredist_x86.exe"));
        tmp
    }

    #[test]
    fn test_detect_redists_ignores_directx() {
        let tmp = steamapps_with_redists();
        let packages = detect_redists(tmp.path());
        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["DotNet", "vcredist"]);
        assert!(!names.contains(&"DirectX"));
    }

    #[test]
    fn test_detect_redists_collects_exes_recursively() {
        let tmp = steamapps_with_redists();
        let packages = detect_redists(tmp.path());
        let vcredist = packages.iter().find(|p| p.name == "vcredist").unwrap();
        assert_eq!(vcredist.installers.len(), 2);
        assert!(vcredist.installers.iter().all(|p| p.extension().unwrap() == "exe"));
    }

    #[test]
    fn test_detect_redists_missing_folder() {
        let tmp = TempDir::new().unwrap();
        assert!(detect_redists(tmp.path()).is_empty());
    }

    #[test]
    fn test_detect_redists_skips_empty_packages() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("common").join("Steamworks Shared").join("_CommonRedist");
        std::fs::create_dir_all(base.join("Empty")).unwrap();
        touch(&base.join("DotNet").join("dotnetfx35.exe"));
        let packages = detect_redists(tmp.path());
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "DotNet");
    }

    #[test]
    fn test_detect_redists_in_game_folder() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("_CommonRedist");
        touch(&base.join("DirectX").join("DXSETUP.exe"));
        touch(&base.join("vcredist").join("vcredist_x64.exe"));
        let packages = detect_redists_in_game_folder(tmp.path());
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "vcredist");
    }

    #[test]
    fn test_detect_redists_in_game_folder_missing() {
        let tmp = TempDir::new().unwrap();
        assert!(detect_redists_in_game_folder(tmp.path()).is_empty());
    }

    #[test]
    fn test_localize_redists_copies_to_game_folder() {
        let shared = TempDir::new().unwrap();
        let redist_base = shared.path().join("_CommonRedist");
        touch(&redist_base.join("vcredist").join("vcredist_x64.exe"));

        let game_dir = TempDir::new().unwrap();
        let packages = detect_redists_in_base(&redist_base);
        assert!(!packages.is_empty());

        let localized = localize_redists(game_dir.path(), packages);
        // _CommonRedist should now exist in the game folder
        assert!(game_dir.path().join("_CommonRedist").exists());
        // Installer paths should be remapped to game folder
        assert!(localized[0].installers[0].starts_with(game_dir.path()));
    }

    #[test]
    fn test_localize_redists_already_local() {
        let game_dir = TempDir::new().unwrap();
        let redist_base = game_dir.path().join("_CommonRedist");
        touch(&redist_base.join("vcredist").join("vcredist_x64.exe"));

        let packages = detect_redists_in_game_folder(game_dir.path());
        let localized = localize_redists(game_dir.path(), packages);

        // Paths should be unchanged (already in game folder)
        assert!(localized[0].installers[0].starts_with(game_dir.path()));
    }

    #[test]
    fn test_localize_redists_empty() {
        let game_dir = TempDir::new().unwrap();
        let localized = localize_redists(game_dir.path(), Vec::new());
        assert!(localized.is_empty());
        assert!(!game_dir.path().join("_CommonRedist").exists());
    }
}
