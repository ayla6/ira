use ira_models::WineConfig;

use crate::wine_detect::{detect_arch, get_proton_path};
use crate::wine_dlls::format_dll_overrides;

pub use crate::wine_detect::{detect_wine_versions, find_wine_binary};

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
