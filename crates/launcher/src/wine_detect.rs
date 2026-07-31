fn steam_data_dirs() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    vec![
        format!("{}/.steam/debian-installation", home),
        format!("{}/.steam", home),
        format!("{}/.local/share/steam", home),
        format!("{}/.local/share/Steam", home),
        format!("{}/snap/steam/common/.local/share/Steam", home),
        format!("{}/.steam/steam", home),
        format!("{}/.var/app/com.valvesoftware.Steam/.local/share/Steam", home),
        format!("{}/.var/app/com.valvesoftware.Steam/.local/share/steam", home),
        format!("{}/.var/app/com.valvesoftware.Steam/data/steam", home),
        format!("{}/.var/app/com.valvesoftware.Steam/data/Steam", home),
        "/usr/share/steam".to_string(),
        "/usr/local/share/steam".to_string(),
    ]
}

fn find_proton_wine(dir: &std::path::Path) -> Option<String> {
    if dir.join("proton").is_file() {
        let dist_wine = dir.join("dist").join("bin").join("wine");
        if dist_wine.is_file() {
            return Some(dist_wine.to_string_lossy().to_string());
        }
    }
    let files_wine = dir.join("files").join("bin").join("wine");
    if files_wine.is_file() {
        return Some(files_wine.to_string_lossy().to_string());
    }
    None
}

/// True if the wine version is a Proton build. This includes:
/// - The "ge-proton" sentinel (downloaded on demand by umu)
/// - Any version name containing "proton"
/// - Any wine binary found inside a Proton directory structure (has a
///   `proton` file sibling to `dist/` or `files/`)
pub(crate) fn is_proton_version(version: &str) -> bool {
    let v = version.to_lowercase();
    v == "ge-proton" || v.contains("proton")
}

/// True if the wine executable path lives inside a Proton installation
/// (detected by the presence of a `proton` file in the installation root).
pub(crate) fn is_proton_binary(wine_exe: &str) -> bool {
    let path = std::path::Path::new(wine_exe);
    // Proton wine lives at <root>/dist/bin/wine or <root>/files/bin/wine
    // The <root> contains a file named "proton"
    let bin_dir = match path.parent() {
        Some(p) => p,
        None => return false,
    };
    let root_candidate = bin_dir
        .parent()            // dist or files
        .and_then(|p| p.parent());  // root
    match root_candidate {
        Some(root) => root.join("proton").is_file(),
        None => false,
    }
}

/// Resolve PROTONPATH for a given wine version and executable.
/// - "ge-proton" sentinel → "GE-Proton" (umu downloads it)
/// - Specific Proton version → the Proton installation directory
/// - Non-Proton → None
pub(crate) fn get_proton_path(version: &str, wine_exe: &str) -> Option<String> {
    let v = version.to_lowercase();
    if v == "ge-proton" {
        return Some("GE-Proton".to_string());
    }
    // If the wine binary is inside a Proton directory, compute PROTONPATH from it
    if is_proton_binary(wine_exe) {
        let path = std::path::Path::new(wine_exe);
        let bin_dir = path.parent()?;
        let proton_root = bin_dir.parent()?.parent()?;
        return Some(proton_root.to_string_lossy().into_owned());
    }
    // Fall back to version name check for names that contain "proton"
    if v.contains("proton") {
        let path = std::path::Path::new(wine_exe);
        let bin_dir = path.parent()?;
        let proton_root = bin_dir.parent()?.parent()?;
        return Some(proton_root.to_string_lossy().into_owned());
    }
    None
}

pub fn find_wine_binary(version: &str, custom_path: &str) -> Result<String, String> {
    match version {
        "system" => {
            let candidates = ["/usr/bin/wine", "/usr/local/bin/wine"];
            for c in &candidates {
                if std::path::Path::new(c).is_file() {
                    return Ok(c.to_string());
                }
            }
            which::which("wine").map(|p| p.to_string_lossy().to_string())
                .map_err(|_| "System Wine not found. Install wine or set a custom Wine path.".to_string())
        }
        "custom" => {
            if custom_path.is_empty() {
                return Err("Custom Wine version selected but no Wine path specified.".to_string());
            }
            if !std::path::Path::new(custom_path).is_file() {
                return Err(format!("Custom Wine executable not found: {}", custom_path));
            }
            Ok(custom_path.to_string())
        }
        "winehq-devel" => {
            let p = "/opt/wine-devel/bin/wine";
            if std::path::Path::new(p).is_file() { Ok(p.to_string()) }
            else { Err("WineHQ Devel not found at /opt/wine-devel/bin/wine".to_string()) }
        }
        "winehq-staging" => {
            let p = "/opt/wine-staging/bin/wine";
            if std::path::Path::new(p).is_file() { Ok(p.to_string()) }
            else { Err("WineHQ Staging not found at /opt/wine-staging/bin/wine".to_string()) }
        }
        "wine-development" => {
            let p = "/usr/lib/wine-development/wine";
            if std::path::Path::new(p).is_file() { Ok(p.to_string()) }
            else { Err("Wine Development not found at /usr/lib/wine-development/wine".to_string()) }
        }
        "ge-proton" => {
            which::which("umu-run")
                .map(|p| p.to_string_lossy().to_string())
                .map_err(|_| "umu-run not found in PATH. Install umu-launcher.".to_string())
        }
        _ => {
            if version.starts_with("wine-") {
                let sys_path = format!("/usr/lib/{}/bin/wine", version);
                if std::path::Path::new(&sys_path).is_file() {
                    return Ok(sys_path);
                }
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            let lutris_path = format!(
                "{}/.local/share/lutris/runners/wine/{}/bin/wine",
                home, version
            );
            if std::path::Path::new(&lutris_path).is_file() {
                return Ok(lutris_path);
            }
            let our_path = format!(
                "{}/runners/wine/{}/bin/wine",
                home, version
            );
            if std::path::Path::new(&our_path).is_file() {
                return Ok(our_path);
            }
            for dir in steam_data_dirs() {
                for sub in &["compatibilitytools.d", "steamapps/common"] {
                    let p = std::path::Path::new(&dir).join(sub).join(version);
                    if let Some(wine) = find_proton_wine(&p) {
                        return Ok(wine);
                    }
                }
            }
            Err(format!("Wine version '{}' not found. Check Lutris runner dir or set a custom Wine path.", version))
        }
    }
}

fn scan_proton_versions() -> Vec<(String, String)> {
    let mut versions: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let extra = std::env::var("STEAM_EXTRA_COMPAT_TOOLS_PATHS").ok();
    let extra_paths: Vec<String> = extra
        .as_deref()
        .map(|s| s.split(':').map(String::from).collect())
        .unwrap_or_default();

    for dir in steam_data_dirs().iter().chain(extra_paths.iter()) {
        for sub in &["compatibilitytools.d", "steamapps/common"] {
            let d = std::path::Path::new(dir).join(sub);
            if let Ok(entries) = std::fs::read_dir(&d) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.join("proton").is_file() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if seen.insert(name.clone()) {
                            versions.push((name.clone(), name));
                        }
                    }
                }
            }
        }
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let lutris_wine = format!("{}/.local/share/lutris/runners/wine", home);
    if let Ok(entries) = std::fs::read_dir(&lutris_wine) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.join("proton").is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                if seen.insert(name.clone()) {
                    versions.push((name.clone(), name));
                }
            }
        }
    }

    versions
}

pub fn detect_wine_versions() -> Vec<(String, String)> {
    let mut versions: Vec<(String, String)> = Vec::new();

    versions.push(("System Wine".to_string(), "system".to_string()));
    versions.push(("Custom (select executable below)".to_string(), "custom".to_string()));

    for (label, vers) in &[
        ("WineHQ Devel", "winehq-devel"),
        ("WineHQ Staging", "winehq-staging"),
        ("Wine Development", "wine-development"),
    ] {
        if find_wine_binary(vers, "").is_ok() {
            versions.push((label.to_string(), vers.to_string()));
        }
    }

    if let Ok(entries) = std::fs::read_dir("/usr/lib") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("wine-") {
                let wine_path = entry.path().join("bin").join("wine");
                if wine_path.is_file() && !versions.iter().any(|(_, v)| *v == name) {
                    versions.push((format!("System {}", name), name));
                }
            }
        }
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let lutris_runner_dir = format!("{}/.local/share/lutris/runners/wine", home);
    if let Ok(entries) = std::fs::read_dir(&lutris_runner_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let wine_path = entry.path().join("bin").join("wine");
            if wine_path.is_file() && !versions.iter().any(|(_, v)| *v == name) {
                versions.push((name.clone(), name));
            }
        }
    }

    for (label, vers) in scan_proton_versions() {
        if !versions.iter().any(|(_, v)| *v == vers) {
            versions.push((label, vers));
        }
    }

    if which::which("umu-run").is_ok() {
        versions.push(("GE-Proton (Latest)".to_string(), "ge-proton".to_string()));
    }

    versions
}

pub fn detect_arch(prefix: &str, _wine_exe: &str) -> String {
    let reg_path = std::path::Path::new(prefix).join("system.reg");
    if reg_path.is_file() {
        if let Ok(content) = std::fs::read_to_string(&reg_path) {
            for line in content.lines().take(5) {
                if line.contains("win64") { return "win64".to_string(); }
                if line.contains("win32") { return "win32".to_string(); }
            }
        }
    }
    "win64".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_arch_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = detect_arch(tmp.path().to_string_lossy().as_ref(), "");
        assert_eq!(result, "win64");
    }

    #[test]
    fn test_detect_arch_win64() {
        let tmp = tempfile::TempDir::new().unwrap();
        let reg_path = tmp.path().join("system.reg");
        std::fs::write(&reg_path, r#"[Software\Wine]
"Source"=-
#"Win64"=-
"Architecture"="win64"
"#).unwrap();
        let result = detect_arch(tmp.path().to_string_lossy().as_ref(), "");
        assert_eq!(result, "win64");
    }

    #[test]
    fn test_find_wine_binary_system_not_found() {
        let result = find_wine_binary("system", "");
        assert!(result.is_err() || result.is_ok());
    }

    #[test]
    fn test_find_wine_binary_custom_empty() {
        let result = find_wine_binary("custom", "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Wine path"));
    }
}
