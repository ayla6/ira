use crate::wine_launch;
use ira_models::{ControllerInputMode, GameLaunchConfig, WineConfig};

fn has_exec(name: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).find(|d| d.join(name).is_file()))
        .is_some()
}

/// Splits a `:`-separated path list and drops empty entries plus development
/// directories (cargo, rustup, target dirs), so the dev environment never
/// leaks into launched games.
fn filter_dev_paths(v: &str) -> Vec<&str> {
    v.split(':')
        .filter(|p| {
            !p.is_empty()
                && !p.contains("/.cargo/")
                && !p.contains("/.rustup/")
                && !p.contains("/target/")
        })
        .collect()
}

/// Returns `(layer_json_dir, shim_so_path)` if the overlay files are found.
/// In development, generates a temporary JSON manifest with the correct
/// `library_path` (the static JSON points to release/, which is wrong in debug).
fn overlay_paths() -> Option<(String, String)> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    // Development: .so files are in target/debug or target/release alongside the exe.
    let dev_shim = exe_dir.join("libira_overlay_shim.so");
    let dev_vk = exe_dir.join("libira_overlay_vk.so");
    if dev_shim.is_file() && dev_vk.is_file() {
        // Generate a temporary JSON manifest with the correct absolute library_path.
        // The static JSON in crates/overlay-vk/ points to release/, which is wrong
        // for debug builds. Writing our own avoids modifying the source file.
        let tmp_dir = std::env::temp_dir().join("ira_overlay");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let json_path = tmp_dir.join("ira_overlay.json");
        let vk_abs = dev_vk.canonicalize().unwrap_or(dev_vk.clone());
        let json_content = format!(
            r#"{{
    "file_format_version": "1.0.0",
    "layer": {{
        "name": "VK_LAYER_IRA_OVERLAY",
        "type": "GLOBAL",
        "api_version": "1.3.0",
        "library_path": "{}",
        "implementation_version": "1",
        "description": "Ira game overlay",
        "functions": {{
            "vkNegotiateLoaderLayerInterfaceVersion": "vkNegotiateLoaderLayerInterfaceVersion"
        }}
    }}
}}"#,
            vk_abs.to_string_lossy()
        );
        match std::fs::write(&json_path, &json_content) {
            Ok(_) => {
                eprintln!(
                    "ira-overlay: JSON manifest written to {} (library_path={})",
                    json_path.display(),
                    vk_abs.display()
                );
                return Some((
                    tmp_dir.to_string_lossy().into(),
                    dev_shim.to_string_lossy().into(),
                ));
            }
            Err(e) => {
                eprintln!("ira-overlay: failed to write JSON manifest: {e}");
            }
        }
    }

    // Release: files installed in an overlay/ subdirectory alongside the exe.
    let overlay_dir = exe_dir.join("overlay");
    let rel_shim = overlay_dir.join("libira_overlay_shim.so");
    let rel_json = overlay_dir.join("ira_overlay.json");
    if rel_shim.is_file() && rel_json.is_file() {
        return Some((
            overlay_dir.to_string_lossy().into(),
            rel_shim.to_string_lossy().into(),
        ));
    }

    None
}

pub fn build_env(
    launch: &GameLaunchConfig,
    wine: Option<&WineConfig>,
    wine_exe: &str,
    save_dir: &str,
    game_id: i64,
    _app_id: &str,
    command: &mut Vec<String>,
) -> Vec<(String, String)> {
    let has_wine = wine.is_some_and(|w| w.enabled);
    let is_proton = has_wine
        && (crate::wine_detect::is_proton_version(&wine.unwrap().version)
            || crate::wine_detect::is_proton_binary(wine_exe));

    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            // Filter out build/dev environment variables that shouldn't reach the game
            k != "CARGO"
                && !k.starts_with("CARGO_")
                && k != "RUSTUP"
                && !k.starts_with("RUSTUP_")
                && !k.starts_with("RUST_")
                && k != "OUT_DIR"
        })
        .filter(|(k, v)| {
            if k == "LD_LIBRARY_PATH" {
                if is_proton {
                    // For Proton/umu, don't pass host LD_LIBRARY_PATH at all.
                    // pressure-vessel builds its own STEAM_RUNTIME_LIBRARY_PATH
                    // from this — host paths cause library conflicts in the container.
                    return false;
                }
                // For non-Proton: remove if only dev paths (cargo, rustup, target dirs)
                !filter_dev_paths(v).is_empty()
            } else if k == "PATH" {
                // Strip cargo/rustup dev directories from PATH so they never
                // reach the game (e.g. /home/ayla/.cargo/bin under cargo run).
                !filter_dev_paths(v).is_empty()
            } else {
                true
            }
        })
        .map(|(k, v)| {
            if k == "LD_LIBRARY_PATH" || k == "PATH" {
                (k, filter_dev_paths(&v).join(":"))
            } else {
                (k, v)
            }
        })
        .collect();

    if let Some(w) = wine {
        if w.enabled {
            let wine_env = wine_launch::build_wine_env(w, wine_exe);
            // Remove any existing keys that wine_env overrides, then extend
            for (k, _) in &wine_env {
                env.retain(|(ek, _)| ek != k);
            }
            env.extend(wine_env);
        }
    }

    apply_launch_overrides(&mut env, launch);

    if let Some(w) = wine {
        if w.enabled && !w.denuvo_api.is_empty() {
            let denuvo_so = format!(
                "{}/api_emulators/denuvo/{}",
                save_dir,
                w.denuvo_api.trim_start_matches('/')
            );
            if std::path::Path::new(&denuvo_so).is_file() {
                let existing = env
                    .iter()
                    .find(|(k, _)| k == "LD_PRELOAD")
                    .map(|(_, v)| v.clone());
                let merged = match existing {
                    Some(prev) if !prev.is_empty() => format!("{}:{}", denuvo_so, prev),
                    _ => denuvo_so,
                };
                env.retain(|(k, _)| k != "LD_PRELOAD");
                env.push(("LD_PRELOAD".to_string(), merged));
            } else {
                eprintln!(
                    "Denuvo emulator not found: {} (denuvo_api='{}')",
                    denuvo_so, w.denuvo_api
                );
            }
        }
    }

    let shader_dir = format!("{}/shader_cache/{}", save_dir, game_id);
    let _ = std::fs::create_dir_all(&shader_dir);
    env.push(("__GL_SHADER_DISK_CACHE".to_string(), "1".to_string()));
    env.push(("__GL_SHADER_DISK_CACHE_PATH".to_string(), shader_dir));

    let default_wine = WineConfig::default();
    let wine_cfg = wine.unwrap_or(&default_wine);
    apply_performance(command, &mut env, launch, wine_cfg);

    env
}

/// Applies per-game environment, loader paths, and GPU selection to an
/// already-constructed launch environment. Emulator launches use this shared
/// path too; they do not go through `build_env`.
pub fn apply_launch_overrides(env: &mut Vec<(String, String)>, launch: &GameLaunchConfig) {
    for (k, v) in &launch.env_vars {
        env.retain(|(ek, _)| ek != k);
        env.push((k.clone(), v.clone()));
    }

    if !launch.ld_preload.is_empty() {
        let existing = env
            .iter()
            .find(|(k, _)| k == "LD_PRELOAD")
            .map(|(_, v)| v.clone());
        let merged = match existing {
            Some(prev) if !prev.is_empty() => format!("{}:{}", launch.ld_preload, prev),
            _ => launch.ld_preload.clone(),
        };
        env.retain(|(k, _)| k != "LD_PRELOAD");
        env.push(("LD_PRELOAD".to_string(), merged));
    }
    if !launch.ld_library_path.is_empty() {
        let existing = env
            .iter()
            .find(|(k, _)| k == "LD_LIBRARY_PATH")
            .map(|(_, v)| v.clone());
        let merged = match existing {
            Some(prev) if !prev.is_empty() => format!("{}:{}", launch.ld_library_path, prev),
            _ => launch.ld_library_path.clone(),
        };
        env.retain(|(k, _)| k != "LD_LIBRARY_PATH");
        env.push(("LD_LIBRARY_PATH".to_string(), merged));
    }

    if !launch.gpu.is_empty() {
        for (k, v) in crate::gpu::build_gpu_env(&launch.gpu) {
            env.retain(|(ek, _)| ek != &k);
            env.push((k, v));
        }
    }
}

/// Removes variables that must apply only to the game inside Gamescope.
pub fn take_gamescope_game_env(env: &mut Vec<(String, String)>) -> Vec<(String, String)> {
    const GAME_KEYS: [&str; 9] = [
        "DRI_PRIME",
        "__NV_PRIME_RENDER_OFFLOAD",
        "__GLX_VENDOR_LIBRARY_NAME",
        "__VK_LAYER_NV_optimus",
        "VK_ICD_FILENAMES",
        "VK_DRIVER_FILES",
        "DXVK_FILTER_DEVICE_UUID",
        "DXVK_FILTER_DEVICE_NAME",
        "LD_PRELOAD",
    ];
    let mut overrides = Vec::new();
    for key in GAME_KEYS {
        if let Some((_, value)) = env.iter().find(|(existing, _)| existing == key) {
            overrides.push((key.to_string(), value.clone()));
        }
        env.retain(|(existing, _)| existing != key);
    }
    overrides
}

/// Restores game-only variables when Gamescope is not wrapped with the
/// standalone overlay after all.
pub fn restore_gamescope_game_env(
    env: &mut Vec<(String, String)>,
    game_env: Vec<(String, String)>,
) {
    for (key, value) in game_env {
        env.retain(|(existing, _)| existing != &key);
        env.push((key, value));
    }
}

/// Wraps the command with gamemode/mangohud/gamescope if configured.
/// Reads system settings from GameLaunchConfig (not WineConfig — these are
/// system-level settings that apply to ALL games, not just Wine).
/// Adds mangohud env vars to `env`.
/// Returns `true` if gamescope is used (indicating standalone overlay mode).
pub fn apply_performance(
    command: &mut Vec<String>,
    env: &mut Vec<(String, String)>,
    launch: &GameLaunchConfig,
    _wine_cfg: &WineConfig,
) -> bool {
    let mut extra_prefix: Vec<String> = Vec::new();

    // By the time apply_performance is called, gamemode/mangohud/gamescope
    // should have been resolved from None to the system default.
    if launch.gamemode.unwrap_or(false) && has_exec("gamemoderun") {
        extra_prefix.push("gamemoderun".to_string());
    }
    let mangohud_enabled = launch.mangohud.unwrap_or(false) && has_exec("mangohud");
    if mangohud_enabled {
        env.retain(|(k, _)| k != "MANGOHUD");
        env.push(("MANGOHUD".to_string(), "1".to_string()));
        env.retain(|(k, _)| k != "MANGOHUD_DLSYM");
        env.push(("MANGOHUD_DLSYM".to_string(), "1".to_string()));
    }

    if launch.gamescope.unwrap_or(false) && has_exec("gamescope") {
        let mut gs_args = vec!["gamescope".to_string()];

        let w = launch.gamescope_w.unwrap_or(0);
        let h = launch.gamescope_h.unwrap_or(0);
        if w > 0 && h > 0 {
            gs_args.push("-W".to_string());
            gs_args.push(w.to_string());
            gs_args.push("-H".to_string());
            gs_args.push(h.to_string());
        }

        let fps = launch.gamescope_fps.unwrap_or(0);
        if fps > 0 {
            gs_args.push("-r".to_string());
            gs_args.push(fps.to_string());
        }

        if let Some(upscaling) = &launch.gamescope_upscaling {
            gs_args.push("-F".to_string());
            gs_args.push(upscaling.to_string());
        }

        gs_args.push("--fullscreen".to_string());

        // With gamescope, use --mangoapp instead of mangohud in the command.
        // mangoapp is the gamescope-native overlay; it reads the same MANGOHUD env vars.
        // mangohud does not work inside gamescope — only mangoapp does.
        if mangohud_enabled && has_exec("mangoapp") {
            gs_args.push("--mangoapp".to_string());
        }

        if !launch.gamescope_flags.is_empty() {
            if let Some(flags) = shlex::split(&launch.gamescope_flags) {
                gs_args.extend(flags);
            }
        }
        gs_args.push("--".to_string());
        gs_args.extend(extra_prefix);
        gs_args.append(command);
        *command = gs_args;
        true
    } else {
        if mangohud_enabled {
            extra_prefix.push("mangohud".to_string());
        }
        let mut final_cmd: Vec<String> = Vec::new();
        final_cmd.extend(extra_prefix);
        final_cmd.append(command);
        *command = final_cmd;
        false
    }
}

/// Adds overlay env vars (VK_LAYER_PATH, VK_INSTANCE_LAYERS, LD_PRELOAD, IRA_OVERLAY_SHM,
/// IRA_OVERLAY_FONT_FAMILY) to an existing env list. Call this after `build_env` when
/// overlay is enabled. Does nothing if the overlay files are not found on disk.
pub fn add_overlay_env(
    env: &mut Vec<(String, String)>,
    overlay_shm: Option<&str>,
    font_family: Option<&str>,
) {
    let Some((layer_dir, shim_path)) = overlay_paths() else {
        eprintln!("ira-overlay: enabled but files not found — skipping injection");
        return;
    };

    env.retain(|(k, _)| k != "VK_LAYER_PATH");
    env.push(("VK_LAYER_PATH".to_string(), layer_dir.clone()));
    env.retain(|(k, _)| k != "VK_INSTANCE_LAYERS");
    env.push((
        "VK_INSTANCE_LAYERS".to_string(),
        "VK_LAYER_IRA_OVERLAY".to_string(),
    ));

    eprintln!("ira-overlay: injecting VK layer (path={layer_dir}) + shim + SHM");

    let existing = env
        .iter()
        .find(|(k, _)| k == "LD_PRELOAD")
        .map(|(_, v)| v.clone());
    let merged = match existing {
        Some(prev) if !prev.is_empty() => format!("{}:{}", shim_path, prev),
        _ => shim_path,
    };
    env.retain(|(k, _)| k != "LD_PRELOAD");
    env.push(("LD_PRELOAD".to_string(), merged));

    if let Some(shm) = overlay_shm {
        env.retain(|(k, _)| k != "IRA_OVERLAY_SHM");
        env.push(("IRA_OVERLAY_SHM".to_string(), shm.to_string()));
    }

    // Resolve font family: user config → system default via fontconfig → fallback.
    let font = font_family
        .map(str::to_string)
        .or_else(detect_system_font)
        .unwrap_or_else(|| "sans-serif".to_string());
    env.retain(|(k, _)| k != "IRA_OVERLAY_FONT_FAMILY");
    env.push(("IRA_OVERLAY_FONT_FAMILY".to_string(), font));
}

/// Injects overlay components into a game running inside Gamescope while
/// leaving UI rendering to the standalone overlay process.
pub fn add_overlay_env_without_ui(
    env: &mut Vec<(String, String)>,
    overlay_shm: Option<&str>,
    font_family: Option<&str>,
) {
    add_overlay_env(env, overlay_shm, font_family);
    env.retain(|(key, _)| key != "IRA_OVERLAY_DISABLE_UI");
    env.push(("IRA_OVERLAY_DISABLE_UI".to_string(), "1".to_string()));
}

/// Queries fontconfig (`fc-match`) for the system's default sans-serif font family.
fn detect_system_font() -> Option<String> {
    let output = std::process::Command::new("fc-match")
        .args(["-f", "%{family}", "sans-serif"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let family = String::from_utf8_lossy(&output.stdout)
        .split(',')
        .next()?
        .trim()
        .to_string();
    if family.is_empty() || family == "sans-serif" {
        None
    } else {
        Some(family)
    }
}

// ─── Standalone overlay (gamescope mode) ───

/// Finds the standalone overlay binary next to the main executable.
fn standalone_binary_path() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    let dev_bin = exe_dir.join("ira-overlay-standalone");
    if dev_bin.is_file() {
        return Some(dev_bin.to_string_lossy().into());
    }

    let rel_bin = exe_dir.join("overlay").join("ira-overlay-standalone");
    if rel_bin.is_file() {
        return Some(rel_bin.to_string_lossy().into());
    }

    None
}

/// Adds environment used by the standalone overlay. Unlike direct mode, this
/// does not inject a Vulkan layer or preload library into the game.
pub fn add_overlay_env_standalone(
    env: &mut Vec<(String, String)>,
    overlay_shm: Option<&str>,
    font_family: Option<&str>,
) {
    eprintln!("ira-overlay: preparing standalone overlay (SHM, no game injection)");

    // In standalone mode, the VK layer must NOT be loaded — the standalone
    // overlay process has its own Vulkan instance. If VK_INSTANCE_LAYERS is
    // set (e.g. from a previous non-standalone launch in the same session),
    // the layer would hook the game's Vulkan calls and conflict.
    env.retain(|(k, _)| k != "VK_INSTANCE_LAYERS" && k != "VK_LAYER_PATH");

    if let Some(shm) = overlay_shm {
        env.retain(|(k, _)| k != "IRA_OVERLAY_SHM");
        env.push(("IRA_OVERLAY_SHM".to_string(), shm.to_string()));
    }

    let font = font_family
        .map(str::to_string)
        .or_else(detect_system_font)
        .unwrap_or_else(|| "sans-serif".to_string());
    env.retain(|(k, _)| k != "IRA_OVERLAY_FONT_FAMILY");
    env.push(("IRA_OVERLAY_FONT_FAMILY".to_string(), font));
}

fn input_binary_path() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let candidates = [
        exe_dir.join("ira-input"),
        exe_dir.join("input").join("ira-input"),
    ];
    if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
        return Some(path.to_string_lossy().into_owned());
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join("ira-input"))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

/// Wraps a final game command with the host-side input broker.
/// The broker stays outside Wine, Proton, umu, gamescope, and Flatpak.
pub fn wrap_with_input(
    command: &mut Vec<String>,
    profile: Option<&str>,
    calibration: Option<&str>,
    pause_unfocused: bool,
    motion_udp: bool,
) -> Result<(), String> {
    let binary = input_binary_path()
        .ok_or_else(|| "input remapping enabled but ira-input was not found".to_string())?;
    wrap_command_with_input(command, &binary, profile, calibration, pause_unfocused, motion_udp);
    Ok(())
}

/// Wraps the command only when a virtual controller backend is selected.
pub fn wrap_with_input_mode(
    command: &mut Vec<String>,
    mode: Option<ControllerInputMode>,
    profile: Option<&str>,
    calibration: Option<&str>,
    pause_unfocused: bool,
    motion_udp: bool,
) -> Result<(), String> {
    let binary = match mode {
        Some(ControllerInputMode::VirtualXInput)
        | Some(ControllerInputMode::VirtualDirectInput)
        | Some(ControllerInputMode::VirtualSwitchPro)
        | Some(ControllerInputMode::VirtualDualShock4)
        | Some(ControllerInputMode::VirtualDualSense) => Some(input_binary_path()),
        None | Some(ControllerInputMode::Disabled) => None,
    };
    if let Some(binary) = binary {
        let binary = binary
            .ok_or_else(|| "input remapping enabled but ira-input was not found".to_string())?;
        wrap_command_with_input(command, &binary, profile, calibration, pause_unfocused, motion_udp);
    }
    Ok(())
}

#[cfg(test)]
fn wrap_with_input_mode_for_binary(
    command: &mut Vec<String>,
    mode: Option<ControllerInputMode>,
    profile: Option<&str>,
    calibration: Option<&str>,
    binary: &str,
) {
    match mode {
        Some(ControllerInputMode::VirtualXInput)
        | Some(ControllerInputMode::VirtualDirectInput)
        | Some(ControllerInputMode::VirtualSwitchPro)
        | Some(ControllerInputMode::VirtualDualShock4)
        | Some(ControllerInputMode::VirtualDualSense) => {
            wrap_command_with_input(command, binary, profile, calibration, true, true)
        }
        None | Some(ControllerInputMode::Disabled) => {}
    }
}

fn wrap_command_with_input(
    command: &mut Vec<String>,
    binary: &str,
    profile: Option<&str>,
    calibration: Option<&str>,
    pause_unfocused: bool,
    motion_udp: bool,
) {
    let game_command = std::mem::take(command);
    let mut wrapped = vec![binary.to_string()];
    if let Some(profile) = profile.filter(|profile| !profile.is_empty()) {
        wrapped.push("--profile".to_string());
        wrapped.push(profile.to_string());
    }
    if let Some(calibration) = calibration.filter(|calibration| !calibration.is_empty()) {
        wrapped.push("--calibration".to_string());
        wrapped.push(calibration.to_string());
    }
    if pause_unfocused {
        wrapped.push("--pause-unfocused".to_string());
    }
    wrapped.push("--motion-port".to_string());
    if motion_udp {
        wrapped.push("26760".to_string());
    } else {
        wrapped.push("0".to_string());
    }
    wrapped.push("--".to_string());
    wrapped.extend(game_command);
    *command = wrapped;
}

/// Wraps a gamescope command so the standalone overlay runs inside gamescope
/// alongside the game. The overlay inherits `DISPLAY` from gamescope's
/// internal XWayland server, allowing it to create an X11 window via XCB.
///
/// The overlay window is marked as `GAMESCOPE_EXTERNAL_OVERLAY` so gamescope
/// composites it on top of the game as a separate plane (like mangoapp).
/// The overlay runs under the Gamescope WSI layer (inheriting
/// `GAMESCOPE_WAYLAND_DISPLAY`): the layer intercepts `vkCreateXcbSurfaceKHR`
/// and presents the overlay's frames to gamescope via Wayland, bypassing
/// XWayland, with pre-multiplied alpha for transparency.
///
/// Transforms: `gamescope -- wine ...`
/// Into:       `gamescope -- sh -c 'ENABLE_GAMESCOPE_WSI=1 ira-overlay-standalone & exec "$@"' -- wine ...`
pub fn wrap_with_standalone_overlay(
    command: &mut Vec<String>,
    game_env: &[(String, String)],
) -> bool {
    let Some(bin) = standalone_binary_path() else {
        eprintln!("ira-overlay: standalone binary not found, skipping");
        return false;
    };

    // Find the `--` separator in the gamescope command.
    // Everything before `--` is gamescope args, everything after is the game command.
    let sep_pos = command.iter().position(|a| a == "--");
    let Some(sep) = sep_pos else {
        eprintln!("ira-overlay: no `--` in gamescope command, skipping standalone wrap");
        return false;
    };

    let game_cmd: Vec<String> = command.split_off(sep + 1);
    // `command` now ends with `... -- `

    let quoted_bin = shlex::try_quote(&bin)
        .map(|c| c.into_owned())
        .unwrap_or(bin);
    let game_env = game_env
        .iter()
        .map(|(key, value)| {
            let quoted = shlex::try_quote(value)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| value.clone());
            format!("{key}={quoted} ")
        })
        .collect::<String>();
    let sh_script = format!(
        "cleanup() {{ status=$?; trap - EXIT INT TERM HUP; if [ -n \"${{overlay_pid:-}}\" ]; then kill \"$overlay_pid\" 2>/dev/null; wait \"$overlay_pid\" 2>/dev/null; fi; exit \"$status\"; }}; trap cleanup EXIT; trap 'exit 143' INT TERM HUP; ENABLE_GAMESCOPE_WSI=1 {quoted_bin} & overlay_pid=$!; {game_env}\"$@\""
    );

    command.push("/usr/bin/sh".to_string());
    command.push("-c".to_string());
    command.push(sh_script);
    command.push("--".to_string());
    command.extend(game_cmd);
    true
}

/// Returns `true` if the command starts with `gamescope`.
pub fn uses_gamescope(command: &[String]) -> bool {
    command.first().is_some_and(|c| c == "gamescope")
}

/// Returns `true` if gamescope would be applied for this launch config.
/// Use this to determine overlay mode before calling `apply_performance`.
pub fn will_use_gamescope(launch: &GameLaunchConfig) -> bool {
    launch.gamescope.unwrap_or(false) && has_exec("gamescope")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command() -> Vec<String> {
        vec!["game".to_string(), "--fullscreen".to_string()]
    }

    #[test]
    fn test_input_mode_disabled_does_not_wrap_command() {
        let mut command = command();
        wrap_with_input_mode_for_binary(
            &mut command,
            Some(ControllerInputMode::Disabled),
            None,
            None,
            "/bin/ira-input",
        );
        assert_eq!(command, vec!["game", "--fullscreen"]);
    }

    #[test]
    fn test_input_mode_none_inherits_without_wrapping_command() {
        let mut command = command();
        wrap_with_input_mode_for_binary(&mut command, None, Some("profile"), None, "/bin/ira-input");
        assert_eq!(command, vec!["game", "--fullscreen"]);
    }

    #[test]
    fn test_input_mode_virtual_xinput_wraps_command() {
        let mut command = command();
        wrap_with_input_mode_for_binary(
            &mut command,
            Some(ControllerInputMode::VirtualXInput),
            Some("profile"),
            None,
            "/bin/ira-input",
        );
        assert_eq!(
            command,
            vec![
                "/bin/ira-input",
                "--profile",
                "profile",
                "--pause-unfocused",
                "--motion-port",
                "26760",
                "--",
                "game",
                "--fullscreen"
            ]
        );
    }

    #[test]
    fn test_input_mode_virtual_direct_input_wraps_command() {
        let mut command = command();
        wrap_with_input_mode_for_binary(
            &mut command,
            Some(ControllerInputMode::VirtualDirectInput),
            None,
            None,
            "/bin/ira-input",
        );
        assert_eq!(
            command,
            vec![
                "/bin/ira-input",
                "--pause-unfocused",
                "--motion-port",
                "26760",
                "--",
                "game",
                "--fullscreen"
            ]
        );
    }

    #[test]
    fn test_input_mode_virtual_switch_pro_wraps_command() {
        let mut command = command();
        wrap_with_input_mode_for_binary(
            &mut command,
            Some(ControllerInputMode::VirtualSwitchPro),
            None,
            None,
            "/bin/ira-input",
        );
        assert_eq!(
            command,
            vec![
                "/bin/ira-input",
                "--pause-unfocused",
                "--motion-port",
                "26760",
                "--",
                "game",
                "--fullscreen"
            ]
        );
    }

    #[test]
    fn test_filter_dev_paths_strips_cargo_rustup_target() {
        let filtered = filter_dev_paths(
            "/usr/bin:/home/ayla/.cargo/bin:/home/ayla/.rustup/toolchains/nightly-x86_64/bin:/data/build/target/debug:/usr/local/bin",
        );
        assert_eq!(filtered, vec!["/usr/bin", "/usr/local/bin"]);
    }

    #[test]
    fn test_filter_dev_paths_empty_and_dev_only() {
        assert!(filter_dev_paths("").is_empty());
        assert!(filter_dev_paths("/home/ayla/.cargo/bin").is_empty());
        assert!(filter_dev_paths(":/home/ayla/.rustup/bin/:").is_empty());
    }

    #[test]
    fn test_apply_launch_overrides_merges_user_environment() {
        let launch = GameLaunchConfig {
            env_vars: vec![("GAME_MODE".to_string(), "test".to_string())],
            ld_preload: "/configured/preload.so".to_string(),
            ld_library_path: "/configured/lib".to_string(),
            ..Default::default()
        };
        let mut env = vec![
            ("GAME_MODE".to_string(), "old".to_string()),
            ("LD_PRELOAD".to_string(), "/existing/preload.so".to_string()),
            ("LD_LIBRARY_PATH".to_string(), "/existing/lib".to_string()),
        ];

        apply_launch_overrides(&mut env, &launch);

        assert!(env.contains(&("GAME_MODE".to_string(), "test".to_string())));
        assert!(env.contains(&(
            "LD_PRELOAD".to_string(),
            "/configured/preload.so:/existing/preload.so".to_string()
        )));
        assert!(env.contains(&(
            "LD_LIBRARY_PATH".to_string(),
            "/configured/lib:/existing/lib".to_string()
        )));
    }

    #[test]
    fn test_take_gamescope_game_env_extracts_game_only_values() {
        let mut env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("DRI_PRIME".to_string(), "1".to_string()),
            ("LD_PRELOAD".to_string(), "/game/helper.so".to_string()),
        ];

        let game_env = take_gamescope_game_env(&mut env);

        assert_eq!(env, [("PATH".to_string(), "/usr/bin".to_string())]);
        assert!(game_env.contains(&("DRI_PRIME".to_string(), "1".to_string())));
        assert!(game_env.contains(&("LD_PRELOAD".to_string(), "/game/helper.so".to_string())));
    }

    #[test]
    fn test_restore_gamescope_game_env_restores_game_only_values() {
        let mut env = vec![("PATH".to_string(), "/usr/bin".to_string())];
        let game_env = vec![
            ("DRI_PRIME".to_string(), "1".to_string()),
            ("LD_PRELOAD".to_string(), "/game/helper.so".to_string()),
        ];

        restore_gamescope_game_env(&mut env, game_env);

        assert!(env.contains(&("DRI_PRIME".to_string(), "1".to_string())));
        assert!(env.contains(&("LD_PRELOAD".to_string(), "/game/helper.so".to_string())));
    }

    #[test]
    fn test_standalone_overlay_does_not_inject_game_libraries() {
        let mut env = Vec::new();

        add_overlay_env_standalone(&mut env, Some("/ira_overlay_1"), Some("Sans"));

        assert!(!env.iter().any(|(key, _)| key == "LD_PRELOAD"));
        assert!(!env.iter().any(|(key, _)| key == "VK_INSTANCE_LAYERS"));
        assert!(!env.iter().any(|(key, _)| key == "VK_LAYER_PATH"));
        assert!(env.contains(&("IRA_OVERLAY_SHM".to_string(), "/ira_overlay_1".to_string())));
    }

    #[test]
    fn test_overlay_without_ui_marks_game_environment() {
        let mut env = vec![("IRA_OVERLAY_DISABLE_UI".to_string(), "0".to_string())];

        add_overlay_env_without_ui(&mut env, Some("/ira_overlay_1"), Some("Sans"));

        assert_eq!(
            env.iter().find(|(key, _)| key == "IRA_OVERLAY_DISABLE_UI"),
            Some(&("IRA_OVERLAY_DISABLE_UI".to_string(), "1".to_string()))
        );
    }
}
