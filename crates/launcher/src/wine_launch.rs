use ira_models::WineConfig;

use crate::wine_detect::{detect_arch, get_proton_path};
use crate::wine_dlls::format_dll_overrides;

pub use crate::wine_detect::{detect_wine_versions, find_wine_binary};

pub fn build_wine_env(wine: &WineConfig, wine_exe: &str) -> Vec<(String, String)> {
    let mut env = Vec::new();

    let is_proton = crate::wine_detect::is_proton_version(&wine.version)
        || crate::wine_detect::is_proton_binary(wine_exe);

    env.push(("WINEDEBUG".to_string(), wine.show_debug.clone()));
    env.push(("WINE".to_string(), wine_exe.to_string()));

    let pfx = wine_prefix(wine);
    let arch = if is_proton {
        "win64".to_string()
    } else if wine.arch != "auto" {
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

    if wine.graphics == "wayland" {
        env.push(("WINE_ENABLE_WAYLAND".to_string(), "1".to_string()));
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

    if let Some(proton_path) = get_proton_path(&wine.version, wine_exe) {
        env.push(("PROTONPATH".to_string(), proton_path));
    }

    // Proton-specific env vars
    if is_proton {
        // Enable umu logging
        env.retain(|(k, _)| k != "UMU_LOG");
        env.push(("UMU_LOG".to_string(), "1".to_string()));
        env.retain(|(k, _)| k != "UMU_RUNTIME_UPDATE");
        env.push(("UMU_RUNTIME_UPDATE".to_string(), "0".to_string()));

        // Proton needs PROTON_USE_WINED3D when DXVK is not enabled
        if !wine.dxvk {
            env.retain(|(k, _)| k != "PROTON_USE_WINED3D");
            env.push(("PROTON_USE_WINED3D".to_string(), "1".to_string()));
        }

        // DXVK D3D8 support when DXVK is enabled
        if wine.dxvk {
            env.retain(|(k, _)| k != "PROTON_DXVK_D3D8");
            env.push(("PROTON_DXVK_D3D8".to_string(), "1".to_string()));
        }

        // Disable LSteam client integration (we're not Steam)
        if wine.proton_disable_lsteamclient {
            env.retain(|(k, _)| k != "PROTON_DISABLE_LSTEAMCLIENT");
            env.push(("PROTON_DISABLE_LSTEAMCLIENT".to_string(), "1".to_string()));
        }

        // Set wayland explicitly (0 or 1, not absent)
        env.retain(|(k, _)| k != "PROTON_ENABLE_WAYLAND");
        if wine.graphics == "wayland" {
            env.push(("PROTON_ENABLE_WAYLAND".to_string(), "1".to_string()));
        } else {
            env.push(("PROTON_ENABLE_WAYLAND".to_string(), "0".to_string()));
        }

        // Set mono/gecko cache dirs from the Proton installation. Lutris sets
        // these unconditionally; wine falls back to its own detection if absent.
        let wine_path = std::path::Path::new(wine_exe);
        if let Some(files_dir) = wine_path.parent().and_then(|p| p.parent()) {
            let mono = files_dir.join("mono");
            let gecko = files_dir.join("gecko");
            env.retain(|(k, _)| k != "WINE_MONO_CACHE_DIR");
            env.push(("WINE_MONO_CACHE_DIR".to_string(), mono.to_string_lossy().to_string()));
            env.retain(|(k, _)| k != "WINE_GECKO_CACHE_DIR");
            env.push(("WINE_GECKO_CACHE_DIR".to_string(), gecko.to_string_lossy().to_string()));
        }
    }

    if wine.dxvk_frame_rate > 0 {
        env.push(("DXVK_FRAME_RATE".to_string(), wine.dxvk_frame_rate.to_string()));
    }
    // Always pass DXVK_HUD explicitly: some Proton builds enable the HUD by
    // default, so setting it to 0 when the toggle is off is required to disable it.
    env.retain(|(k, _)| k != "DXVK_HUD");
    env.push(("DXVK_HUD".to_string(), if wine.dxvk_hud { "1" } else { "0" }.to_string()));
    if wine.proton_wow64 {
        env.retain(|(k, _)| k != "PROTON_USE_WOW64");
        env.push(("PROTON_USE_WOW64".to_string(), "1".to_string()));
    }
    if wine.proton_ntsync {
        env.retain(|(k, _)| k != "PROTON_USE_NTSYNC");
        env.push(("PROTON_USE_NTSYNC".to_string(), "1".to_string()));
    }

    env
}

pub fn build_wine_command(wine_exe: &str, game_exe: &str, args: &[String], _wine: &WineConfig) -> Vec<String> {
    let mut cmd = vec![wine_exe.to_string()];
    if game_exe.to_ascii_lowercase().ends_with(".msi") {
        cmd.push("msiexec".to_string());
        cmd.push("/i".to_string());
    }
    cmd.push(game_exe.to_string());
    cmd.extend_from_slice(args);
    cmd
}

pub fn wine_prefix(wine: &WineConfig) -> String {
    wine.prefix.clone()
}

/// Generate a unique prefix path from a game slug.
///
/// If `base_dir` is empty, falls back to `~/.local/share/ira/prefixes`.
/// The slug is sanitized: only alphanumeric + dashes kept, lowercased.
/// If `{base}/{slug}` already exists, tries `{slug}1`, `{slug}2`, etc.
pub fn generate_prefix_path(base_dir: &str, slug: &str) -> String {
    let base = if base_dir.is_empty() {
        let xdg = std::env::var("XDG_DATA_HOME").ok().filter(|s| !s.is_empty());
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        match xdg {
            Some(x) => format!("{}/ira/prefixes", x),
            None => format!("{}/.local/share/ira/prefixes", home),
        }
    } else {
        base_dir.to_string()
    };

    let clean: String = slug
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    let clean: String = clean.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    let candidate = format!("{}/{}", base, clean);
    if !std::path::Path::new(&candidate).exists() {
        return candidate;
    }
    let mut i = 1;
    loop {
        let c = format!("{}/{}{}", base, clean, i);
        if !std::path::Path::new(&c).exists() {
            return c;
        }
        i += 1;
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

    // Virtual desktop removed — barely works on newer Wine versions.
    // Always clean up any leftover Explorer registry keys from older configs.
    let desktop_name = "Default";
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
    fn test_generate_prefix_path_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_str().unwrap();
        let p = generate_prefix_path(base, "Halo: Combat Evolved");
        assert_eq!(p, format!("{}/halo-combat-evolved", base));
        assert!(!std::path::Path::new(&p).exists());
    }

    #[test]
    fn test_generate_prefix_path_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_str().unwrap();
        let first = generate_prefix_path(base, "Halo");
        std::fs::create_dir_all(&first).unwrap();
        let second = generate_prefix_path(base, "Halo");
        assert_eq!(second, format!("{}/halo1", base));
        std::fs::create_dir_all(&second).unwrap();
        let third = generate_prefix_path(base, "Halo");
        assert_eq!(third, format!("{}/halo2", base));
    }

    #[test]
    fn test_generate_prefix_path_sanitizes() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_str().unwrap();
        let p = generate_prefix_path(base, "Game!!! @#$%");
        assert_eq!(p, format!("{}/game", base));
    }
}
