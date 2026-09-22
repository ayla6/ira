use crate::wine_launch;
use ira_models::{ControllerInputMode, GameLaunchConfig, WineConfig};

/// Sets `key` to `value`, replacing any previous entry.
fn env_set(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    env.retain(|(k, _)| k != key);
    env.push((key.to_string(), value.to_string()));
}

/// Prepends `value` to `key`, merging with an existing entry
/// colon-separated (LD_PRELOAD / LD_LIBRARY_PATH style).
fn env_prepend(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    let merged = match env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()) {
        Some(prev) if !prev.is_empty() => format!("{}:{}", value, prev),
        _ => value.to_string(),
    };
    env_set(env, key, &merged);
}

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

/// The parent process environment minus build/dev variables that must never
/// reach a launched process (CARGO/RUST toolchain vars), with PATH and
/// LD_LIBRARY_PATH stripped of dev directories. Wine-specific handling is up
/// to callers.
pub fn clean_parent_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| {
            // Filter out build/dev environment variables that shouldn't reach the game
            k != "CARGO"
                && !k.starts_with("CARGO_")
                && k != "RUSTUP"
                && !k.starts_with("RUSTUP_")
                && !k.starts_with("RUST_")
                && k != "OUT_DIR"
        })
        .filter(|(k, v)| match k.as_str() {
            // For non-Proton launches: remove if only dev paths
            // (cargo, rustup, target dirs); otherwise keep filtered below.
            "LD_LIBRARY_PATH" | "PATH" => !filter_dev_paths(v).is_empty(),
            _ => true,
        })
        .map(|(k, v)| {
            if k == "LD_LIBRARY_PATH" || k == "PATH" {
                (k, filter_dev_paths(&v).join(":"))
            } else {
                (k, v)
            }
        })
        .collect()
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

    let mut env = clean_parent_env();
    if is_proton {
        // For Proton/umu, don't pass host LD_LIBRARY_PATH at all.
        // pressure-vessel builds its own STEAM_RUNTIME_LIBRARY_PATH
        // from this — host paths cause library conflicts in the container.
        env.retain(|(k, _)| k != "LD_LIBRARY_PATH");
    }

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
                env_prepend(&mut env, "LD_PRELOAD", &denuvo_so);
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
        env_prepend(env, "LD_PRELOAD", &launch.ld_preload);
    }
    if !launch.ld_library_path.is_empty() {
        env_prepend(env, "LD_LIBRARY_PATH", &launch.ld_library_path);
    }

    if !launch.gpu.is_empty() {
        for (k, v) in crate::gpu::build_gpu_env(&launch.gpu) {
            env.retain(|(ek, _)| ek != &k);
            env.push((k, v));
        }
    }
}

/// Removes variables that must apply only to the game inside Gamescope
/// (GPU selection, LD_PRELOAD) so the compositor itself never inherits them.
/// The returned pairs are re-applied to the game command past Gamescope's
/// `--` separator via `apply_game_env_inside_gamescope`.
fn take_gamescope_game_env(env: &mut Vec<(String, String)>) -> Vec<(String, String)> {
    const GAME_KEYS: [&str; 10] = [
        "DRI_PRIME",
        "__NV_PRIME_RENDER_OFFLOAD",
        "__GLX_VENDOR_LIBRARY_NAME",
        "__VK_LAYER_NV_optimus",
        "VK_ICD_FILENAMES",
        "VK_DRIVER_FILES",
        "MESA_VK_DEVICE_SELECT",
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

/// Assembles the variables re-applied past Gamescope's `--` separator: the
/// GPU/preload keys that must not reach the compositor, plus a Wayland
/// display override. Gamescope overrides `DISPLAY` for its children but
/// never `WAYLAND_DISPLAY`, so a Wayland-native game would inherit the
/// desktop's `wayland-0` through Ira's environment and escape the gamescope
/// window entirely; its own compositor socket is always `gamescope-0`. The
/// override must live past the `--` separator, since Gamescope itself reads
/// `WAYLAND_DISPLAY` to find the compositor it nests in.
fn gamescope_game_env(env: &mut Vec<(String, String)>) -> Vec<(String, String)> {
    let mut game_env = take_gamescope_game_env(env);
    env_set(&mut game_env, "WAYLAND_DISPLAY", "gamescope-0");
    game_env
}

/// Prefixes `game_env` onto the command that runs inside Gamescope — directly
/// after its `--` separator, so the compositor never sees the variables while
/// the game and its wrappers do.
fn apply_game_env_inside_gamescope(
    command: &mut Vec<String>,
    game_env: &[(String, String)],
) {
    if game_env.is_empty() {
        return;
    }
    let Some(sep) = command.iter().position(|arg| arg == "--") else {
        eprintln!("launch: gamescope command has no `--`; game-only env vars not applied");
        return;
    };
    let mut prefixed = vec!["/usr/bin/env".to_string()];
    prefixed.extend(game_env.iter().map(|(key, value)| format!("{key}={value}")));
    let inner = command.split_off(sep + 1);
    command.extend(prefixed);
    command.extend(inner);
}

/// Wraps the command with gamemode/mangohud/gamescope if configured.
/// Reads system settings from GameLaunchConfig (not WineConfig — these are
/// system-level settings that apply to ALL games, not just Wine).
/// Adds mangohud env vars to `env`.
/// Returns `true` if gamescope is used (indicating external overlay mode).
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
    if !launch.command_prefix.is_empty() {
        match shlex::split(&launch.command_prefix) {
            Some(words) => extra_prefix.extend(words),
            None => eprintln!(
                "launch: failed to parse command prefix {:?}; launching without it",
                launch.command_prefix
            ),
        }
    }
    let mangohud_enabled = launch.mangohud.unwrap_or(false) && has_exec("mangohud");
    if mangohud_enabled {
        env_set(env, "MANGOHUD", "1");
        env_set(env, "MANGOHUD_DLSYM", "1");
    }

    if launch.gamescope.unwrap_or(false) && !has_exec("gamescope") {
        eprintln!("launch: gamescope requested but binary not found; launching without it");
    }
    if launch.gamescope.unwrap_or(false) && has_exec("gamescope") {        let mut gs_args = vec!["gamescope".to_string()];

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
        // Gamescope must see the system default GPU: pinning the compositor
        // itself to a secondary GPU breaks its presentation to the desktop
        // compositor. The game-only variables are re-applied past the `--`
        // separator so only the game inherits them.
        let game_env = gamescope_game_env(env);
        apply_game_env_inside_gamescope(command, &game_env);
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

    env_set(env, "VK_LAYER_PATH", &layer_dir);
    env_set(env, "VK_INSTANCE_LAYERS", "VK_LAYER_IRA_OVERLAY");

    eprintln!("ira-overlay: injecting VK layer (path={layer_dir}) + shim + SHM");

    env_prepend(env, "LD_PRELOAD", &shim_path);

    if let Some(shm) = overlay_shm {
        env_set(env, "IRA_OVERLAY_SHM", shm);
    }

    // Resolve font family: user config → system default via fontconfig → fallback.
    let font = font_family
        .map(str::to_string)
        .or_else(detect_system_font)
        .unwrap_or_else(|| "sans-serif".to_string());
    env_set(env, "IRA_OVERLAY_FONT_FAMILY", &font);
}

/// Injects overlay components into a game running inside Gamescope while
/// leaving UI rendering to the host overlay window.
pub fn add_overlay_env_without_ui(
    env: &mut Vec<(String, String)>,
    overlay_shm: Option<&str>,
    font_family: Option<&str>,
) {
    add_overlay_env(env, overlay_shm, font_family);
    env_set(env, "IRA_OVERLAY_DISABLE_UI", "1");
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

// ─── Host overlay (gamescope mode) ───

/// Adds environment for the gamescope session. Unlike direct mode, this
/// does not inject a Vulkan layer or preload library into the session —
/// the GTK host must never load the layer into itself.
pub fn add_overlay_env_external(
    env: &mut Vec<(String, String)>,
    overlay_shm: Option<&str>,
    font_family: Option<&str>,
) {
    eprintln!("ira-overlay: preparing host overlay (SHM, no session injection)");

    // In external mode, the VK layer must NOT be loaded — the host process
    // creates no Vulkan instance. If VK_INSTANCE_LAYERS is set (e.g. from a
    // previous non-external launch in the same session), the layer would
    // hook the host's library handles pointlessly.
    env.retain(|(k, _)| k != "VK_INSTANCE_LAYERS" && k != "VK_LAYER_PATH");

    if let Some(shm) = overlay_shm {
        env_set(env, "IRA_OVERLAY_SHM", shm);
    }

    let font = font_family
        .map(str::to_string)
        .or_else(detect_system_font)
        .unwrap_or_else(|| "sans-serif".to_string());
    env_set(env, "IRA_OVERLAY_FONT_FAMILY", &font);
}

pub(crate) fn input_binary_path() -> Option<String> {
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
) -> Result<(), String> {
    let binary = input_binary_path()
        .ok_or_else(|| "input remapping enabled but ira-input was not found".to_string())?;
    wrap_command_with_input(command, &binary, profile, calibration, pause_unfocused);
    Ok(())
}

/// Wraps the command only when input remapping is explicitly enabled; which
/// virtual controller the game sees is decided by the profile itself. A
/// missing ira-input binary degrades to launching unwrapped — remapping is
/// optional garnish, the game itself must start.
pub fn wrap_with_input_mode(
    command: &mut Vec<String>,
    mode: Option<ControllerInputMode>,
    profile: Option<&str>,
    calibration: Option<&str>,
    pause_unfocused: bool,
) {
    if let Some(ControllerInputMode::Enabled) = mode {
        match input_binary_path() {
            Some(binary) => {
                wrap_command_with_input(command, &binary, profile, calibration, pause_unfocused)
            }
            None => eprintln!(
                "launch: input remapping is enabled but ira-input was not found; \
                 launching without input remapping"
            ),
        }
    }
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
        Some(ControllerInputMode::Enabled) => {
            wrap_command_with_input(command, binary, profile, calibration, true)
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
    wrapped.push("--".to_string());
    wrapped.extend(game_command);
    *command = wrapped;
}

/// Wraps a gamescope command so the GTK host runs inside gamescope
/// alongside the game. The host inherits `DISPLAY` from gamescope's
/// internal XWayland server and marks its window as
/// `GAMESCOPE_EXTERNAL_OVERLAY`, so gamescope composites it on top of the
/// game as a separate plane (like mangoapp) — no Vulkan compositing, no
/// canvas copy.
///
/// Transforms: `gamescope -- wine ...`
/// Into:       `gamescope -- sh -c 'IRA_OVERLAY_GAMESCOPE=1 ira-overlay-ui & exec "$@"' -- wine ...`
/// The host is forced onto the X11 backend: gamescope's external-overlay
/// atom is X11-only, and on Wayland the window would appear as a regular
/// (main-plane) window instead of an overlay.
/// Splits off the game command so the host can re-wrap it.
/// Gamescope commands carry the game after the `--` separator (everything
/// before it is gamescope args); a plain command — a launch under an
/// already-running gamescope session — is the game itself.
fn split_game_command(command: &mut Vec<String>) -> Vec<String> {
    match command.iter().position(|a| a == "--") {
        Some(sep) => command.split_off(sep + 1),
        None => std::mem::take(command),
    }
}

pub fn wrap_with_host_overlay(
    command: &mut Vec<String>,
    capture_env: &[(String, String)],
) -> bool {
    let Some(bin) = super::overlay_host::host_binary_path() else {
        eprintln!("ira-overlay: host binary not found, skipping");
        return false;
    };

    // Gamescope commands carry the game after the `--` separator (everything
    // before it is gamescope args). A plain command — a launch under an
    // already-running gamescope session — is the game itself.
    let game_cmd = split_game_command(command);

    let quoted_bin = shlex::try_quote(&bin)
        .map(|c| c.into_owned())
        .unwrap_or(bin);
    let env_prefix = capture_env
        .iter()
        .map(|(key, value)| {
            let quoted = shlex::try_quote(value)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| value.clone());
            format!("{key}={quoted} ")
        })
        .collect::<String>();
    let sh_script = format!(
        "cleanup() {{ status=$?; trap - EXIT INT TERM HUP; if [ -n \"${{overlay_pid:-}}\" ]; then kill \"$overlay_pid\" 2>/dev/null; wait \"$overlay_pid\" 2>/dev/null; fi; exit \"$status\"; }}; trap cleanup EXIT; trap 'exit 143' INT TERM HUP; IRA_OVERLAY_GAMESCOPE=1 GDK_BACKEND=x11 GSK_RENDERER=cairo {quoted_bin} & overlay_pid=$!; {env_prefix}\"$@\""
    );

    command.push("/usr/bin/sh".to_string());
    command.push("-c".to_string());
    command.push(sh_script);
    command.push("--".to_string());
    command.extend(game_cmd);
    eprintln!("ira-overlay: gamescope host wrapped ({quoted_bin})");
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
    fn test_split_game_command_separates_gamescope_wrap() {
        let mut cmd = vec![
            "gamescope".to_string(),
            "--fullscreen".to_string(),
            "--".to_string(),
            "game".to_string(),
        ];
        let game = split_game_command(&mut cmd);
        assert_eq!(cmd, vec!["gamescope", "--fullscreen", "--"]);
        assert_eq!(game, vec!["game"]);
    }

    #[test]
    fn test_split_game_command_takes_plain_command_whole() {
        let mut cmd = vec!["rpcs3".to_string(), "--fullscreen".to_string()];
        let game = split_game_command(&mut cmd);
        assert!(cmd.is_empty());
        assert_eq!(game, vec!["rpcs3", "--fullscreen"]);
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
        wrap_with_input_mode_for_binary(
            &mut command,
            None,
            Some("profile"),
            None,
            "/bin/ira-input",
        );
        assert_eq!(command, vec!["game", "--fullscreen"]);
    }

    #[test]
    fn test_input_mode_enabled_wraps_command_with_profile() {
        let mut command = command();
        wrap_with_input_mode_for_binary(
            &mut command,
            Some(ControllerInputMode::Enabled),
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
                "--",
                "game",
                "--fullscreen"
            ]
        );
    }

    #[test]
    fn test_input_mode_enabled_without_profile_omits_flag() {
        let mut command = command();
        wrap_with_input_mode_for_binary(
            &mut command,
            Some(ControllerInputMode::Enabled),
            None,
            None,
            "/bin/ira-input",
        );
        assert_eq!(
            command,
            vec![
                "/bin/ira-input",
                "--pause-unfocused",
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
    fn test_gamescope_game_env_pins_wayland_display_to_gamescope() {
        let mut env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string()),
            ("DRI_PRIME".to_string(), "1".to_string()),
        ];

        let game_env = gamescope_game_env(&mut env);

        // Exactly one override, pointing at Gamescope's own compositor —
        // the desktop's wayland-0 must not leak through to the game.
        assert_eq!(
            game_env
                .iter()
                .filter(|(key, _)| key == "WAYLAND_DISPLAY")
                .count(),
            1
        );
        assert!(game_env.contains(&(
            "WAYLAND_DISPLAY".to_string(),
            "gamescope-0".to_string()
        )));
        assert!(game_env.contains(&("DRI_PRIME".to_string(), "1".to_string())));
    }

    #[test]
    fn test_apply_game_env_inside_gamescope_prefixes_inner_command() {
        let mut command = vec![
            "gamescope".to_string(),
            "-W".to_string(),
            "1920".to_string(),
            "--".to_string(),
            "umu-run".to_string(),
            "game.exe".to_string(),
        ];
        let game_env = vec![
            ("DRI_PRIME".to_string(), "1".to_string()),
            ("VK_DRIVER_FILES".to_string(), "/icd/nvidia.json".to_string()),
        ];

        apply_game_env_inside_gamescope(&mut command, &game_env);

        assert_eq!(
            command,
            [
                "gamescope",
                "-W",
                "1920",
                "--",
                "/usr/bin/env",
                "DRI_PRIME=1",
                "VK_DRIVER_FILES=/icd/nvidia.json",
                "umu-run",
                "game.exe"
            ]
        );
    }

    #[test]
    fn test_apply_game_env_inside_gamescope_keeps_prefix_wrappers() {
        let mut command = vec![
            "gamescope".to_string(),
            "--".to_string(),
            "gamemoderun".to_string(),
            "game".to_string(),
        ];
        let game_env = vec![("LD_PRELOAD".to_string(), "/game/helper.so".to_string())];

        apply_game_env_inside_gamescope(&mut command, &game_env);

        assert_eq!(
            command,
            [
                "gamescope",
                "--",
                "/usr/bin/env",
                "LD_PRELOAD=/game/helper.so",
                "gamemoderun",
                "game"
            ]
        );
    }

    #[test]
    fn test_apply_game_env_inside_gamescope_without_separator_is_noop() {
        let mut command = vec!["gamescope".to_string(), "-W".to_string(), "1920".to_string()];
        let game_env = vec![("DRI_PRIME".to_string(), "1".to_string())];

        apply_game_env_inside_gamescope(&mut command, &game_env);

        assert_eq!(command, ["gamescope", "-W", "1920"]);
    }

    #[test]
    fn test_apply_game_env_inside_gamescope_empty_env_is_noop() {
        let mut command = command();

        apply_game_env_inside_gamescope(&mut command, &[]);

        assert_eq!(command, ["game", "--fullscreen"]);
    }

    #[test]
    fn test_apply_performance_without_gamescope_keeps_game_env() {
        let launch = GameLaunchConfig::default();
        let mut cmd = command();
        let mut env = vec![("DRI_PRIME".to_string(), "1".to_string())];

        assert!(!apply_performance(&mut cmd, &mut env, &launch, &WineConfig::default()));

        assert_eq!(cmd, ["game", "--fullscreen"]);
        assert_eq!(env, [("DRI_PRIME".to_string(), "1".to_string())]);
    }

    #[test]
    fn test_external_overlay_does_not_inject_game_libraries() {
        let mut env = Vec::new();

        add_overlay_env_external(&mut env, Some("/ira_overlay_1"), Some("Sans"));

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
