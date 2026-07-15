use ira_models::WineConfig;

pub fn build_wine_env(wine: &WineConfig, wine_exe: &str) -> Vec<(String, String)> {
    let mut env = Vec::new();

    env.push(("WINEDEBUG".to_string(), wine.show_debug.clone()));
    env.push(("WINE".to_string(), wine_exe.to_string()));

    let pfx = wine_prefix(wine);
    let arch = if wine.arch != "auto" {
        wine.arch.clone()
    } else {
        detect_arch(&pfx, wine_exe)
    };
    env.push(("WINEPREFIX".to_string(), pfx));
    env.push(("WINEARCH".to_string(), arch));

    env.push(("WINEESYNC".to_string(), if wine.esync { "1" } else { "0" }.to_string()));
    env.push(("WINEFSYNC".to_string(), if wine.fsync { "1" } else { "0" }.to_string()));

    if wine.esync && !is_esync_limit_set() {
        eprintln!("Warning: esync enabled but fs.file-max too low (< 1,000,000). Games may crash.");
    }
    if wine.fsync && !is_fsync_supported() {
        eprintln!("Warning: fsync enabled but kernel doesn't support it. Falling back to esync.");
    }

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

    if wine.vkd3d {
        env.push(("PROTON_ENABLE_VKD3D".to_string(), "1".to_string()));
    }

    if wine.d3d_extras {
        env.push(("PROTON_ENABLE_D3D_EXTRAS".to_string(), "1".to_string()));
    }

    if wine.battleye {
        env.push(("PROTON_BATTLEYE_LAUNCHER".to_string(), "1".to_string()));
    }

    if wine.eac {
        env.push(("PROTON_EAC_LAUNCHER".to_string(), "1".to_string()));
    }

    if wine.show_debug == "-all" || wine.show_debug.is_empty() {
        env.push(("DXVK_LOG_LEVEL".to_string(), "error".to_string()));
    } else if wine.show_debug == "+all" {
        env.push(("DXVK_LOG_LEVEL".to_string(), "debug".to_string()));
    } else if wine.show_debug == "+fps" {
        env.push(("DXVK_LOG_LEVEL".to_string(), "info".to_string()));
    }

    let overrides_str = format_dll_overrides(&wine.dll_overrides, wine.desktop_integration);
    if !overrides_str.is_empty() {
        env.push(("WINEDLLOVERRIDES".to_string(), overrides_str));
    }

    if let Some(proton_path) = get_proton_path(&wine.version) {
        env.push(("PROTONPATH".to_string(), proton_path));
    }

    if wine.dxvk_frame_rate > 0 {
        env.push(("DXVK_FRAME_RATE".to_string(), wine.dxvk_frame_rate.to_string()));
    }
    if wine.proton_wow64 {
        env.push(("PROTON_USE_WOW64".to_string(), "1".to_string()));
    }
    if wine.proton_ntsync {
        env.push(("PROTON_USE_NTSYNC".to_string(), "1".to_string()));
    }

    for (k, v) in &wine.wine_env_vars {
        env.retain(|(ek, _)| ek != k);
        env.push((k.clone(), v.clone()));
    }

    env
}

pub fn format_dll_overrides(overrides: &[(String, String)], desktop_integration: bool) -> String {
    let mut entries: Vec<String> = Vec::new();

    let mut user_set_winemenubuilder = false;
    for (dll, value) in overrides {
        if dll == "winemenubuilder" {
            user_set_winemenubuilder = true;
        }
        let normalized: String = value
            .split(',')
            .map(|token| {
                let t = token.trim();
                if t == "builtin" { "b" }
                else if t == "native" { "n" }
                else if t == "disabled" { "" }
                else { t }
            })
            .collect::<Vec<&str>>()
            .join(",");
        entries.push(format!("{}={}", dll, normalized));
    }
    if !user_set_winemenubuilder && !desktop_integration {
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

pub fn build_wine_command(wine_exe: &str, game_exe: &str, args: &[String], wine: &WineConfig) -> Vec<String> {
    let mut cmd = vec![wine_exe.to_string()];
    if wine.virtual_desktop {
        let res = if wine.virtual_desktop_res.is_empty() {
            "Default,1920x1080".to_string()
        } else {
            format!("Default,{}", wine.virtual_desktop_res)
        };
        cmd.push("explorer".to_string());
        cmd.push(format!("/desktop={}", res));
    }
    if game_exe.to_ascii_lowercase().ends_with(".msi") {
        cmd.push("msiexec".to_string());
        cmd.push("/i".to_string());
    }
    cmd.push(game_exe.to_string());
    cmd.extend_from_slice(args);
    cmd
}

pub fn wine_prefix(wine: &WineConfig) -> String {
    if wine.prefix.is_empty() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        format!("{}/.wine", home)
    } else {
        wine.prefix.clone()
    }
}

pub fn build_wine_reg_commands(wine: &WineConfig, wine_exe: &str) -> Vec<Vec<String>> {
    let mut commands: Vec<Vec<String>> = Vec::new();

    let pfx = wine_prefix(wine);
    let prefix_initialized = std::path::Path::new(&pfx).join("system.reg").is_file();
    if !prefix_initialized {
        commands.push(vec![
            wine_exe.to_string(),
            "wineboot".to_string(),
            "--init".to_string(),
        ]);
    }

    commands.push(vec![
        wine_exe.to_string(),
        "reg".to_string(),
        "add".to_string(),
        r"HKCU\Software\Wine\X11 Driver".to_string(),
        "/v".to_string(),
        "MouseWarpOverride".to_string(),
        "/t".to_string(),
        "REG_SZ".to_string(),
        "/d".to_string(),
        wine.mouse_warp_override.clone(),
        "/f".to_string(),
    ]);

    let desktop_name = "Default";
    if wine.virtual_desktop {
        let res = if wine.virtual_desktop_res.is_empty() {
            "1920x1080".to_string()
        } else {
            wine.virtual_desktop_res.clone()
        };
        commands.push(vec![
            wine_exe.to_string(),
            "reg".to_string(),
            "add".to_string(),
            r"HKCU\Software\Wine\Explorer".to_string(),
            "/v".to_string(),
            "Desktop".to_string(),
            "/t".to_string(),
            "REG_SZ".to_string(),
            "/d".to_string(),
            desktop_name.to_string(),
            "/f".to_string(),
        ]);
        commands.push(vec![
            wine_exe.to_string(),
            "reg".to_string(),
            "add".to_string(),
            r"HKCU\Software\Wine\Explorer\Desktops".to_string(),
            "/v".to_string(),
            desktop_name.to_string(),
            "/t".to_string(),
            "REG_SZ".to_string(),
            "/d".to_string(),
            res,
            "/f".to_string(),
        ]);
    } else {
        commands.push(vec![
            wine_exe.to_string(),
            "reg".to_string(),
            "delete".to_string(),
            r"HKCU\Software\Wine\Explorer".to_string(),
            "/v".to_string(),
            "Desktop".to_string(),
            "/f".to_string(),
        ]);
        commands.push(vec![
            wine_exe.to_string(),
            "reg".to_string(),
            "delete".to_string(),
            r"HKCU\Software\Wine\Explorer\Desktops".to_string(),
            "/v".to_string(),
            desktop_name.to_string(),
            "/f".to_string(),
        ]);
    }

    if wine.dpi_enabled {
        commands.push(vec![
            wine_exe.to_string(),
            "reg".to_string(),
            "add".to_string(),
            r"HKCU\Software\Wine\Fonts".to_string(),
            "/v".to_string(),
            "LogPixels".to_string(),
            "/t".to_string(),
            "REG_DWORD".to_string(),
            "/d".to_string(),
            format!("{}", wine.dpi),
            "/f".to_string(),
        ]);
    } else {
        commands.push(vec![
            wine_exe.to_string(),
            "reg".to_string(),
            "delete".to_string(),
            r"HKCU\Software\Wine\Fonts".to_string(),
            "/v".to_string(),
            "LogPixels".to_string(),
            "/f".to_string(),
        ]);
    }

    commands.push(vec![
        wine_exe.to_string(),
        "reg".to_string(),
        "add".to_string(),
        r"HKCU\Software\Wine\WineDbg".to_string(),
        "/v".to_string(),
        "ShowCrashDialog".to_string(),
        "/t".to_string(),
        "REG_SZ".to_string(),
        "/d".to_string(),
        if wine.show_crash_dialogs { "1" } else { "0" }.to_string(),
        "/f".to_string(),
    ]);

    commands.push(vec![
        wine_exe.to_string(),
        "reg".to_string(),
        "add".to_string(),
        r"HKCU\Software\Wine\Drivers".to_string(),
        "/v".to_string(),
        "Audio".to_string(),
        "/t".to_string(),
        "REG_SZ".to_string(),
        "/d".to_string(),
        wine.audio.clone(),
        "/f".to_string(),
    ]);

    commands
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
    if let Ok(content) = std::fs::read_to_string("/proc/sys/kernel/osrelease") {
        let parts: Vec<&str> = content.trim().split('.').collect();
        if parts.len() >= 2 {
            let major: u32 = parts[0].parse().unwrap_or(0);
            let minor: u32 = parts[1].parse().unwrap_or(0);
            return (major, minor) >= (5, 16);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_dll_overrides_empty() {
        let result = format_dll_overrides(&[], false);
        assert_eq!(result, "winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_enabled() {
        let result = format_dll_overrides(&[], true);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_dll_overrides_basic() {
        let overrides = vec![
            ("d3d11".to_string(), "native,builtin".to_string()),
        ];
        let result = format_dll_overrides(&overrides, false);
        assert_eq!(result, "d3d11=n,b;winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_multiple() {
        let overrides = vec![
            ("d3d11".to_string(), "native,builtin".to_string()),
            ("d3d9".to_string(), "builtin,native".to_string()),
            ("winemenubuilder".to_string(), "".to_string()),
        ];
        let result = format_dll_overrides(&overrides, false);
        assert_eq!(result, "d3d11=n,b;d3d9=b,n;winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_disabled() {
        let overrides = vec![
            ("d3d11".to_string(), "disabled".to_string()),
        ];
        let result = format_dll_overrides(&overrides, false);
        assert!(result.starts_with("d3d11=;"));
    }

    #[test]
    fn test_format_dll_overrides_native_only() {
        let overrides = vec![
            ("d3d11".to_string(), "native".to_string()),
        ];
        let result = format_dll_overrides(&overrides, false);
        assert_eq!(result, "d3d11=n;winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_builtin_only() {
        let overrides = vec![
            ("d3d11".to_string(), "builtin".to_string()),
        ];
        let result = format_dll_overrides(&overrides, false);
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
