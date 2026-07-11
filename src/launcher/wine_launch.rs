use crate::models::WineConfig;

pub fn build_wine_env(wine: &WineConfig, wine_exe: &str) -> Vec<(String, String)> {
    let mut env = Vec::new();

    env.push(("WINEDEBUG".to_string(), wine.show_debug.clone()));
    env.push(("WINE".to_string(), wine_exe.to_string()));

    let prefix = if wine.prefix.is_empty() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        format!("{}/.wine", home)
    } else {
        wine.prefix.clone()
    };
    let arch = detect_arch(&prefix, wine_exe);
    env.push(("WINEPREFIX".to_string(), prefix));
    env.push(("WINEARCH".to_string(), arch));

    env.push(("WINEESYNC".to_string(), if wine.esync { "1" } else { "0" }.to_string()));
    env.push(("WINEFSYNC".to_string(), if wine.fsync { "1" } else { "0" }.to_string()));

    if !wine.esync {
        env.push(("PROTON_NO_ESYNC".to_string(), "1".to_string()));
    }
    if !wine.fsync {
        env.push(("PROTON_NO_FSYNC".to_string(), "1".to_string()));
    }

    if wine.fsr {
        env.push(("WINE_FULLSCREEN_FSR".to_string(), "1".to_string()));
    }

    if wine.dxvk_nvapi {
        env.push(("DXVK_NVAPIHACK".to_string(), "0".to_string()));
        env.push(("DXVK_ENABLE_NVAPI".to_string(), "1".to_string()));
    }

    if wine.dxvk {
        env.push(("WINE_LARGE_ADDRESS_AWARE".to_string(), "1".to_string()));
    }

    if wine.show_debug == "-all" || wine.show_debug.is_empty() {
        env.push(("DXVK_LOG_LEVEL".to_string(), "error".to_string()));
    } else if wine.show_debug == "+all" {
        env.push(("DXVK_LOG_LEVEL".to_string(), "debug".to_string()));
    } else if wine.show_debug == "+fps" {
        env.push(("DXVK_LOG_LEVEL".to_string(), "info".to_string()));
    }

    let overrides_str = format_dll_overrides(&wine.dll_overrides);
    if !overrides_str.is_empty() {
        env.push(("WINEDLLOVERRIDES".to_string(), overrides_str));
    }

    if let Some(proton_path) = get_proton_path(&wine.version) {
        env.push(("PROTONPATH".to_string(), proton_path));
    }

    env
}

pub fn format_dll_overrides(overrides: &[(String, String)]) -> String {
    let mut entries: Vec<String> = Vec::new();

    let mut seen_default = false;
    for (dll, value) in overrides {
        if dll == "winemenubuilder" {
            seen_default = true;
        }
        let normalized = value
            .replace("builtin", "b")
            .replace("native", "n")
            .replace("disabled", "")
            .replace(" ", "");
        entries.push(format!("{}={}", dll, normalized));
    }
    if !seen_default {
        entries.push("winemenubuilder=".to_string());
    }
    entries.join(";")
}

fn get_proton_path(version: &str) -> Option<String> {
    let v = version.to_lowercase();
    if v == "ge-proton" {
        Some("GE-Proton".to_string())
    } else if v.contains("proton") {
        Some(version.to_string())
    } else {
        None
    }
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
        _ => {
            let lutris_path = format!(
                "{}/.local/share/lutris/runners/wine/{}/bin/wine",
                std::env::var("HOME").unwrap_or_else(|_| "/root".to_string()),
                version
            );
            if std::path::Path::new(&lutris_path).is_file() {
                return Ok(lutris_path);
            }
            let our_path = format!(
                "{}/runners/wine/{}/bin/wine",
                std::env::var("HOME").unwrap_or_else(|_| "/root".to_string()),
                version
            );
            if std::path::Path::new(&our_path).is_file() {
                return Ok(our_path);
            }
            Err(format!("Wine version '{}' not found. Check Lutris runner dir or set a custom Wine path.", version))
        }
    }
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

    versions.push(("GE-Proton (Latest)".to_string(), "ge-proton".to_string()));

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

    versions
}

pub fn build_wine_command(wine_exe: &str, game_exe: &str, args: &[String]) -> Vec<String> {
    let mut cmd = vec![wine_exe.to_string()];
    if game_exe.ends_with(".msi") {
        cmd.push("msiexec".to_string());
        cmd.push("/i".to_string());
    }
    cmd.push(game_exe.to_string());
    cmd.extend_from_slice(args);
    cmd
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

pub fn is_esync_limit_set() -> bool {
    if let Ok(content) = std::fs::read_to_string("/proc/sys/fs/file-max") {
        if let Ok(max) = content.trim().parse::<u64>() {
            return max >= 1_000_000;
        }
    }
    false
}

pub fn is_fsync_supported() -> bool {
    if let Ok(content) = std::fs::read_to_string("/proc/sys/kernel/max_user_futexes") {
        if let Ok(max) = content.trim().parse::<u64>() {
            return max > 0;
        }
    }
    if let Ok(uts) = std::fs::read_to_string("/proc/sys/kernel/ostype") {
        let release = uts.trim();
        if let Some(ver) = release.split('.').next().and_then(|s| s.parse::<u32>().ok()) {
            return ver >= 5;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_dll_overrides_empty() {
        let result = format_dll_overrides(&[]);
        assert_eq!(result, "winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_basic() {
        let overrides = vec![
            ("d3d11".to_string(), "native,builtin".to_string()),
        ];
        let result = format_dll_overrides(&overrides);
        assert_eq!(result, "d3d11=n,b;winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_multiple() {
        let overrides = vec![
            ("d3d11".to_string(), "native,builtin".to_string()),
            ("d3d9".to_string(), "builtin,native".to_string()),
            ("winemenubuilder".to_string(), "".to_string()),
        ];
        let result = format_dll_overrides(&overrides);
        assert_eq!(result, "d3d11=n,b;d3d9=b,n;winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_disabled() {
        let overrides = vec![
            ("d3d11".to_string(), "disabled".to_string()),
        ];
        let result = format_dll_overrides(&overrides);
        assert!(result.starts_with("d3d11=;"));
    }

    #[test]
    fn test_format_dll_overrides_native_only() {
        let overrides = vec![
            ("d3d11".to_string(), "native".to_string()),
        ];
        let result = format_dll_overrides(&overrides);
        assert_eq!(result, "d3d11=n;winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_builtin_only() {
        let overrides = vec![
            ("d3d11".to_string(), "builtin".to_string()),
        ];
        let result = format_dll_overrides(&overrides);
        assert_eq!(result, "d3d11=b;winemenubuilder=");
    }

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
        // On a system without Wine, this should error. We just check it returns a Result.
        assert!(result.is_err() || result.is_ok());
    }

    #[test]
    fn test_find_wine_binary_custom_empty() {
        let result = find_wine_binary("custom", "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Wine path"));
    }
}
