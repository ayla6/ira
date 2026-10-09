//! Gamescope composition: game-only environment splitting, display policy,
//! argument assembly, and the gamemode/mangohud/gamescope wrapping.

use crate::env_ops::{env_set, has_exec};
use ira_models::{GameLaunchConfig, WineConfig};

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

/// True when the environment carries a gamescope session socket:
/// gamescope always serves its nested compositor on `gamescope-*`.
pub(crate) fn session_socket_in_env(env: &[(String, String)]) -> bool {
    env.iter()
        .any(|(k, v)| k == "WAYLAND_DISPLAY" && v.starts_with("gamescope"))
}

/// Position right after the `run` of a flatpak invocation inside
/// `command`, wherever it sits (plain, `flatpak-spawn`, or past a
/// gamescope `--`). `None` when the command is not a flatpak launch.
pub(crate) fn flatpak_run_insert_pos(command: &[String]) -> Option<usize> {
    command
        .windows(2)
        .position(|w| {
            (w[0] == "flatpak" || w[0] == "flatpak-spawn" || w[0] == "--host")
                && w[1] == "run"
        })
        .map(|pos| pos + 2)
}

/// Pins a flatpak game to the session's X11 server. A sandboxed game
/// reaches displays through its sandbox sockets: left alone it prefers
/// Wayland, where RetroArch lands on a "null" display server whose
/// surface the compositor never sizes to the output (tiny game), or —
/// with no Wayland socket at all — it falls back to the desktop's
/// `wayland-0` and escapes the session entirely. `--socket=x11` mounts
/// the gamescope Xwayland socket (`DISPLAY` already points at it) and
/// `--nosocket=wayland` keeps every Wayland socket out, so the game
/// deterministically takes the X11 fullscreen path. No-op for
/// non-flatpak commands. Proven against `flatpak:org.libretro.RetroArch`
/// in-session: display server `x11`, window at full output size.
pub fn pin_flatpak_for_gamescope(command: &mut Vec<String>) {
    if let Some(pos) = flatpak_run_insert_pos(command) {
        command.insert(pos, "--nosocket=wayland".to_string());
        command.insert(pos, "--socket=x11".to_string());
    }
}

/// Display policy for a launch inside gamescope territory (a session, a
/// wrap, or both): flatpak games are pinned to the X11 server, everything
/// else native loses the outer Wayland socket so toolkits take the X11
/// path. Call after `apply_performance`, before any host-overlay wrap.
pub fn apply_gamescope_display_policy(
    command: &mut Vec<String>,
    env: &mut Vec<(String, String)>,
    in_session: bool,
) {
    let wrapped = uses_gamescope(command);
    if in_session || wrapped {
        pin_flatpak_for_gamescope(command);
    }
    if !wrapped && flatpak_run_insert_pos(command).is_none() {
        strip_session_wayland(env);
    }
}

/// Drops the outer session's Wayland socket from an UNWRAPPED native
/// game's environment. Inside a gamescope session the game must go
/// through the session's Xwayland server (`DISPLAY`): native toolkits
/// that prefer Wayland land on a display server the session build does
/// not size to the output (RetroArch's "null" server renders tiny).
/// No-op on the desktop and for gamescope-wrapped launches (the nested
/// case unsets past the `--` instead, and the compositor itself needs
/// the socket to find what it nests in). Flatpak launches keep the
/// socket and are pinned to X11 by `pin_flatpak_for_gamescope` instead.
pub fn strip_session_wayland(env: &mut Vec<(String, String)>) {
    if session_socket_in_env(env) {
        env.retain(|(k, _)| k != "WAYLAND_DISPLAY");
    }
}

/// Assembles the variables re-applied past Gamescope's `--` separator: the
/// GPU/preload keys that must not reach the compositor, plus a Wayland
/// display override. On the desktop Gamescope overrides `DISPLAY` for its
/// children but never `WAYLAND_DISPLAY`, so a Wayland-native game would
/// inherit the desktop's `wayland-0` through Ira's environment and escape
/// the gamescope window entirely; its own compositor socket is always
/// `gamescope-0`. Inside a session the override flips: the inherited
/// `gamescope-0` belongs to the OUTER compositor, and a game inside a
/// nested gamescope would escape the inner session through it — so the
/// socket is unset past the `--` instead and the game falls back to the
/// inner Xwayland `DISPLAY`. The returned unsets name those removals.
/// The override must live past the `--` separator, since Gamescope itself reads
/// `WAYLAND_DISPLAY` to find the compositor it nests in.
fn gamescope_game_env(env: &mut Vec<(String, String)>) -> (Vec<(String, String)>, Vec<String>) {
    let mut game_env = take_gamescope_game_env(env);
    if session_socket_in_env(env) {
        return (game_env, vec!["WAYLAND_DISPLAY".to_string()]);
    }
    env_set(&mut game_env, "WAYLAND_DISPLAY", "gamescope-0");
    (game_env, Vec::new())
}

/// Extra variables for the game inside Gamescope beyond the compositor-safe
/// set: the WSI toggle is opt-in (some games break with it forced on), so an
/// unset or off toggle contributes nothing and never stomps the session env.
pub(crate) fn gamescope_wsi_env(wsi: Option<bool>) -> Vec<(String, String)> {
    if wsi == Some(true) {
        vec![("ENABLE_GAMESCOPE_WSI".to_string(), "1".to_string())]
    } else {
        Vec::new()
    }
}

/// Prefixes `game_env` onto the command that runs inside Gamescope — directly
/// after its `--` separator, so the compositor never sees the variables while
/// the game and its wrappers do. `unset_env` keys are removed with
/// `/usr/bin/env -u` before the assignments apply.
fn apply_game_env_inside_gamescope(
    command: &mut Vec<String>,
    game_env: &[(String, String)],
    unset_env: &[String],
) {
    if game_env.is_empty() && unset_env.is_empty() {
        return;
    }
    let Some(sep) = command.iter().position(|arg| arg == "--") else {
        eprintln!("launch: gamescope command has no `--`; game-only env vars not applied");
        return;
    };
    let mut prefixed = vec!["/usr/bin/env".to_string()];
    for key in unset_env {
        prefixed.push("-u".to_string());
        prefixed.push(key.clone());
    }
    prefixed.extend(game_env.iter().map(|(key, value)| format!("{key}={value}")));
    let inner = command.split_off(sep + 1);
    command.extend(prefixed);
    command.extend(inner);
}

/// Whether a launch with this gamescope flag wraps in a gamescope
/// compositor, with the system default filling in an unset flag. Inside
/// a session (`in_session`) only an explicit per-game opt-in nests —
/// the session itself is already the compositor.
pub fn will_wrap_gamescope(flag: Option<bool>, default: bool, in_session: bool) -> bool {
    let want = if in_session {
        flag == Some(true)
    } else {
        flag.unwrap_or(default)
    };
    want && has_exec("gamescope")
}

/// The gamescope argument list for a launch, sans wrappers: `gamescope`
/// through the `--` separator. The configured resolution is the GAME's
/// render resolution (nested `-w`/`-h`), never the output (`-W`/`-H`
/// pin the compositor window instead and leave the game rendering at
/// the default size, which upscales blurry to the output). The output
/// follows the window — fullscreen on launch — so a 720p game upscales
/// to the display. `0` means auto and is omitted, letting the game see
/// the output resolution natively. Pure apart from the mangoapp probe,
/// so tests can assert the exact argv.
pub(crate) fn gamescope_args(launch: &GameLaunchConfig, mangoapp: bool) -> Vec<String> {
    let mut gs_args = vec!["gamescope".to_string()];

    let w = launch.gamescope_w.unwrap_or(0);
    let h = launch.gamescope_h.unwrap_or(0);
    if w > 0 {
        gs_args.push("-w".to_string());
        gs_args.push(w.to_string());
    }
    if h > 0 {
        gs_args.push("-h".to_string());
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

    // Wayland clients need an explicit opt-in on gamescope's side: without
    // it gamescope unsets WAYLAND_DISPLAY for its children (X11-only) and
    // reports XDG_SESSION_TYPE=x11. With it they get gamescope's own
    // socket (always advertised as GAMESCOPE_WAYLAND_DISPLAY) and
    // XDG_SESSION_TYPE=wayland.
    if launch.gamescope_wayland.unwrap_or(false) {
        gs_args.push("--expose-wayland".to_string());
    }

    gs_args.push("--fullscreen".to_string());

    // With gamescope, use --mangoapp instead of mangohud in the command.
    // mangoapp is the gamescope-native overlay; it reads the same MANGOHUD env vars.
    // mangohud does not work inside gamescope — only mangoapp does.
    if mangoapp {
        gs_args.push("--mangoapp".to_string());
    }

    if !launch.gamescope_flags.is_empty() {
        if let Some(flags) = shlex::split(&launch.gamescope_flags) {
            gs_args.extend(flags);
        }
    }
    gs_args.push("--".to_string());
    gs_args
}

/// Reads system settings from GameLaunchConfig (not WineConfig — these are
/// system-level settings that apply to ALL games, not just Wine).
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
    if launch.gamescope.unwrap_or(false) && has_exec("gamescope") {
        let mut gs_args = gamescope_args(launch, mangohud_enabled && has_exec("mangoapp"));
        gs_args.extend(extra_prefix);
        gs_args.append(command);
        *command = gs_args;
        // Gamescope must see the system default GPU: pinning the compositor
        // itself to a secondary GPU breaks its presentation to the desktop
        // compositor. The game-only variables are re-applied past the `--`
        // separator so only the game inherits them.
        let (mut game_env, unset_env) = gamescope_game_env(env);
        game_env.extend(gamescope_wsi_env(launch.gamescope_wsi));
        apply_game_env_inside_gamescope(command, &game_env, &unset_env);
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

        apply_game_env_inside_gamescope(&mut command, &game_env, &[]);

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

        apply_game_env_inside_gamescope(&mut command, &game_env, &[]);

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

        apply_game_env_inside_gamescope(&mut command, &game_env, &[]);

        assert_eq!(command, ["gamescope", "-W", "1920"]);
    }

    #[test]
    fn test_apply_game_env_inside_gamescope_empty_env_is_noop() {
        let mut command = command();

        apply_game_env_inside_gamescope(&mut command, &[], &[]);

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

    fn gamescope_launch() -> GameLaunchConfig {
        GameLaunchConfig {
            gamescope: Some(true),
            gamescope_w: Some(1280),
            gamescope_h: Some(720),
            gamescope_fps: Some(60),
            gamescope_upscaling: Some("fsr".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_gamescope_args_use_nested_resolution() {
        let args = gamescope_args(&gamescope_launch(), false);

        // The game renders at the nested resolution; the output follows
        // the window. `-W`/`-H` would pin the output instead and leave
        // the game rendering small.
        assert!(args.contains(&"-w".to_string()));
        assert!(args.contains(&"1280".to_string()));
        assert!(args.contains(&"-h".to_string()));
        assert!(args.contains(&"720".to_string()));
        assert!(!args.contains(&"-W".to_string()));
        assert!(!args.contains(&"-H".to_string()));
        assert!(args.contains(&"--fullscreen".to_string()));
        assert!(args.contains(&"-r".to_string()));
        assert!(args.contains(&"60".to_string()));
        assert!(args.contains(&"-F".to_string()));
        assert!(args.contains(&"fsr".to_string()));
        assert_eq!(args.last().unwrap(), "--");
    }

    #[test]
    fn test_gamescope_args_omit_zero_resolution() {
        let launch = GameLaunchConfig {
            gamescope: Some(true),
            ..Default::default()
        };

        let args = gamescope_args(&launch, false);

        assert!(!args.contains(&"-w".to_string()));
        assert!(!args.contains(&"-h".to_string()));
        assert!(!args.contains(&"-W".to_string()));
        assert!(!args.contains(&"-H".to_string()));
        assert!(args.contains(&"--fullscreen".to_string()));
    }

    #[test]
    fn test_gamescope_args_partial_resolution_passes_through() {
        let launch = GameLaunchConfig {
            gamescope: Some(true),
            gamescope_w: Some(1280),
            ..Default::default()
        };

        let args = gamescope_args(&launch, false);

        assert!(args.contains(&"-w".to_string()));
        assert!(!args.contains(&"-h".to_string()));
    }

    #[test]
    fn test_gamescope_args_expose_wayland_opt_in() {
        // Arrange: Wayland support explicitly enabled…
        let launch = GameLaunchConfig {
            gamescope: Some(true),
            gamescope_wayland: Some(true),
            ..Default::default()
        };
        // Act
        let args = gamescope_args(&launch, false);
        // Assert: the flag lands before the `--` separator…
        assert!(args.contains(&"--expose-wayland".to_string()));
        assert!(
            args.iter().position(|a| a == "--expose-wayland").unwrap()
                < args.iter().position(|a| a == "--").unwrap()
        );
        // …and stays out unless opted in.
        let plain = gamescope_args(&gamescope_launch(), false);
        assert!(!plain.contains(&"--expose-wayland".to_string()));
        let off = GameLaunchConfig {
            gamescope: Some(true),
            gamescope_wayland: Some(false),
            ..Default::default()
        };
        assert!(!gamescope_args(&off, false).contains(&"--expose-wayland".to_string()));
    }

    #[test]
    fn test_gamescope_wsi_env_opt_in_only() {
        assert_eq!(
            gamescope_wsi_env(Some(true)),
            vec![("ENABLE_GAMESCOPE_WSI".to_string(), "1".to_string())]
        );
        // Off or unset contributes nothing — never stomps the session env.
        assert!(gamescope_wsi_env(Some(false)).is_empty());
        assert!(gamescope_wsi_env(None).is_empty());
    }

    #[test]
    fn test_gamescope_game_env_pins_wayland_display_on_desktop() {
        let mut env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string()),
            ("DRI_PRIME".to_string(), "1".to_string()),
        ];

        let (game_env, unset_env) = gamescope_game_env(&mut env);

        // Exactly one override, pointing at Gamescope's own compositor —
        // the desktop's wayland-0 must not leak through to the game.
        assert!(unset_env.is_empty());
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
    fn test_gamescope_game_env_unsets_session_socket_when_nested() {
        let mut env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("WAYLAND_DISPLAY".to_string(), "gamescope-0".to_string()),
            ("DISPLAY".to_string(), ":2".to_string()),
        ];

        let (game_env, unset_env) = gamescope_game_env(&mut env);

        // The inherited socket belongs to the outer compositor: the game
        // inside a nested gamescope must not see it.
        assert_eq!(unset_env, vec!["WAYLAND_DISPLAY".to_string()]);
        assert!(!game_env.iter().any(|(key, _)| key == "WAYLAND_DISPLAY"));
    }

    #[test]
    fn test_apply_game_env_inside_gamescope_emits_unset_first() {
        let mut command = vec![
            "gamescope".to_string(),
            "--".to_string(),
            "game".to_string(),
        ];
        let game_env = vec![("DRI_PRIME".to_string(), "1".to_string())];
        let unset_env = vec!["WAYLAND_DISPLAY".to_string()];

        apply_game_env_inside_gamescope(&mut command, &game_env, &unset_env);

        assert_eq!(
            command,
            [
                "gamescope",
                "--",
                "/usr/bin/env",
                "-u",
                "WAYLAND_DISPLAY",
                "DRI_PRIME=1",
                "game"
            ]
        );
    }

    #[test]
    fn test_will_wrap_gamescope_session_needs_explicit_opt_in() {
        assert!(!will_wrap_gamescope(None, true, true));
        assert!(!will_wrap_gamescope(Some(false), true, true));
    }

    #[test]
    fn test_strip_session_wayland_removes_session_socket_only() {
        let mut env = vec![
            ("WAYLAND_DISPLAY".to_string(), "gamescope-0".to_string()),
            ("DISPLAY".to_string(), ":2".to_string()),
        ];

        strip_session_wayland(&mut env);

        assert_eq!(env, [("DISPLAY".to_string(), ":2".to_string())]);
    }

    #[test]
    fn test_strip_session_wayland_keeps_desktop_socket() {
        let mut env = vec![("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string())];

        strip_session_wayland(&mut env);

        assert_eq!(
            env,
            [("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string())]
        );
    }

    #[test]
    fn test_pin_flatpak_for_gamescope_inserts_socket_flags() {
        let mut command = vec![
            "flatpak".to_string(),
            "run".to_string(),
            "org.libretro.RetroArch".to_string(),
            "-L".to_string(),
            "core.so".to_string(),
        ];

        pin_flatpak_for_gamescope(&mut command);

        assert_eq!(
            command,
            [
                "flatpak",
                "run",
                "--socket=x11",
                "--nosocket=wayland",
                "org.libretro.RetroArch",
                "-L",
                "core.so"
            ]
        );
    }

    #[test]
    fn test_pin_flatpak_for_gamescope_finds_wrap_past_separator() {
        let mut command = vec![
            "gamescope".to_string(),
            "-w".to_string(),
            "1280".to_string(),
            "--".to_string(),
            "flatpak".to_string(),
            "run".to_string(),
            "org.libretro.RetroArch".to_string(),
        ];

        pin_flatpak_for_gamescope(&mut command);

        assert_eq!(
            command,
            [
                "gamescope",
                "-w",
                "1280",
                "--",
                "flatpak",
                "run",
                "--socket=x11",
                "--nosocket=wayland",
                "org.libretro.RetroArch"
            ]
        );
    }

    #[test]
    fn test_pin_flatpak_for_gamescope_ignores_native_commands() {
        let mut command = vec!["retroarch".to_string(), "-L".to_string(), "core.so".to_string()];

        pin_flatpak_for_gamescope(&mut command);

        assert_eq!(command, ["retroarch", "-L", "core.so"]);
    }

    #[test]
    fn test_display_policy_pins_flatpak_and_keeps_session_socket() {
        let mut command = vec![
            "flatpak".to_string(),
            "run".to_string(),
            "org.libretro.RetroArch".to_string(),
        ];
        let mut env = vec![("WAYLAND_DISPLAY".to_string(), "gamescope-0".to_string())];

        apply_gamescope_display_policy(&mut command, &mut env, true);

        assert!(command.contains(&"--socket=x11".to_string()));
        assert!(command.contains(&"--nosocket=wayland".to_string()));
        assert!(env.iter().any(|(k, _)| k == "WAYLAND_DISPLAY"));
    }

    #[test]
    fn test_display_policy_strips_native_session_socket() {
        let mut command = vec!["retroarch".to_string(), "game.rom".to_string()];
        let mut env = vec![
            ("WAYLAND_DISPLAY".to_string(), "gamescope-0".to_string()),
            ("DISPLAY".to_string(), ":2".to_string()),
        ];

        apply_gamescope_display_policy(&mut command, &mut env, true);

        assert_eq!(env, [("DISPLAY".to_string(), ":2".to_string())]);
    }

    #[test]
    fn test_display_policy_leaves_desktop_alone() {
        let mut command = vec![
            "flatpak".to_string(),
            "run".to_string(),
            "org.libretro.RetroArch".to_string(),
        ];
        let mut env = vec![("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string())];

        apply_gamescope_display_policy(&mut command, &mut env, false);

        assert!(!command.contains(&"--socket=x11".to_string()));
        assert_eq!(
            env,
            [("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string())]
        );
    }
}
