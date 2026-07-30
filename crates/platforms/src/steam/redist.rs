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
    let Ok(entries) = std::fs::read_dir(&base) else {
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
}
