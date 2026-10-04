//! Overlay environment injection: the Vulkan layer for direct mode, the
//! SHM-only host configuration for gamescope mode, and the daemon toggle.

use crate::env_ops::{env_prepend, env_set};
use crate::gamescope_env::uses_gamescope;

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

fn overlay_font_family(font_family: Option<&str>) -> String {
    font_family
        .map(str::to_string)
        .or_else(detect_system_font)
        .unwrap_or_else(|| "sans-serif".to_string())
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
    env_set(env, "IRA_OVERLAY_FONT_FAMILY", &overlay_font_family(font_family));
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

    env_set(env, "IRA_OVERLAY_FONT_FAMILY", &overlay_font_family(font_family));
}

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
    // The host runs as a gamescope child (DISPLAY already repointed at
    // gamescope's X server), so the desktop display it reads input from
    // must be passed explicitly — along with its authority file, since the
    // inherited XAUTHORITY belongs to the gamescope session, not the desk.
    let host_prefix = [
        ("DISPLAY", "IRA_OVERLAY_DESKTOP_DISPLAY"),
        ("XAUTHORITY", "IRA_OVERLAY_DESKTOP_XAUTHORITY"),
    ]
    .into_iter()
    .filter_map(|(src, dst)| {
        std::env::var(src).ok().map(|value| {
            let quoted = shlex::try_quote(&value)
                .map(|c| c.into_owned())
                .unwrap_or(value);
            format!("{dst}={quoted} ")
        })
    })
    .collect::<String>();
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
        "cleanup() {{ status=$?; trap - EXIT INT TERM HUP; if [ -n \"${{overlay_pid:-}}\" ]; then kill \"$overlay_pid\" 2>/dev/null; wait \"$overlay_pid\" 2>/dev/null; fi; exit \"$status\"; }}; trap cleanup EXIT; trap 'exit 143' INT TERM HUP; IRA_OVERLAY_GAMESCOPE=1 GDK_BACKEND=x11 GSK_RENDERER=cairo {host_prefix}{quoted_bin} & overlay_pid=$!; {env_prefix}\"$@\""
    );

    command.push("/usr/bin/sh".to_string());
    command.push("-c".to_string());
    command.push(sh_script);
    command.push("--".to_string());
    command.extend(game_cmd);
    eprintln!("ira-overlay: gamescope host wrapped ({quoted_bin})");
    true
}

/// Arms daemon-side overlay toggle for this launch's environment: gamescope
/// plus overlay. In gamescope mode every in-process toggle path stands down
/// (`IRA_OVERLAY_DISABLE_UI`: shim keyboard/gamepad hooks, layer evdev), so
/// the daemon is the only gamepad authority and cannot double-flip.
/// Flatpaks need no special case: with no LD_PRELOAD shim and no /dev/input
/// in the sandbox, they were already daemon-or-nothing.
pub fn maybe_add_daemon_toggle_env(
    env: &mut Vec<(String, String)>,
    command: &[String],
    overlay_enabled: bool,
) {
    if !(overlay_enabled && uses_gamescope(command)) {
        return;
    }
    env.retain(|(k, _)| k != ira_overlay_ipc::DAEMON_TOGGLE_ENV);
    env.push((
        ira_overlay_ipc::DAEMON_TOGGLE_ENV.to_string(),
        "1".to_string(),
    ));
    eprintln!("ira-overlay: daemon owns the overlay toggle chord (gamescope)");
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_daemon_toggle_env_gated() {
        // Arrange: gamescope commands with the overlay on, and the rest.
        let wrapped: Vec<String> = ["gamescope", "--", "flatpak", "run"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let native_wrapped: Vec<String> = ["gamescope", "--", "/games/game"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Act + Assert: armed for gamescope+overlay regardless of packaging…
        let mut env = Vec::new();
        maybe_add_daemon_toggle_env(&mut env, &wrapped, true);
        assert!(
            env.iter()
                .any(|(k, v)| k == "IRA_OVERLAY_DAEMON_TOGGLE" && v == "1")
        );
        let mut env = Vec::new();
        maybe_add_daemon_toggle_env(&mut env, &native_wrapped, true);
        assert!(
            env.iter()
                .any(|(k, v)| k == "IRA_OVERLAY_DAEMON_TOGGLE" && v == "1")
        );
        // …but not without the overlay or without gamescope.
        let mut env = Vec::new();
        maybe_add_daemon_toggle_env(&mut env, &wrapped, false);
        assert!(env.is_empty());
        let mut env = Vec::new();
        maybe_add_daemon_toggle_env(
            &mut env,
            &["flatpak".to_string(), "run".to_string()],
            true,
        );
        assert!(env.is_empty());
        let mut env = Vec::new();
        maybe_add_daemon_toggle_env(&mut env, &["game".to_string()], true);
        assert!(env.is_empty());
    }
}
