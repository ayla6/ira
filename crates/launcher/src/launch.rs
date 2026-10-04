use ira_db::DbConn;
use ira_input_ipc::DaemonClient;
use ira_models::{AppSender, ControllerInputMode};
use ira_models::{GameLaunchConfig, TrophySource, UfsRootOverride, UfsSaveFile, WineConfig};

use std::collections::HashMap;
use std::process::Child;
use std::sync::{Arc, Mutex};

use crate::env_ops::env_set;
use crate::procfs::unix_now;

/// A prepared launch: the final command plus its full environment.
type CommandEnv = (Vec<String>, Vec<(String, String)>);

/// Owned per-launch plumbing shared by the daemon and wrapper spawn paths.
struct SpawnPlan {
    env: Vec<(String, String)>,
    command: Vec<String>,
    game_dir: Option<String>,
    log_file: String,
    pause_watch: Option<super::freeze::FreezeCfg>,
}

pub struct LaunchContext {
    pub game_name: String,
    pub sender: AppSender,
    pub game_id: i64,
    pub variant_id: Option<i64>,
    pub count_playtime: bool,
    pub app_id: String,
    pub trophy_source: TrophySource,
    pub ufs_savefiles: Vec<UfsSaveFile>,
    pub ufs_rootoverrides: Vec<UfsRootOverride>,
    pub centralize_saves: bool,
    pub db: DbConn,
    pub save_dir: String,
    pub running_games: Arc<Mutex<HashMap<i64, i32>>>,
    pub overlay_enabled: bool,
    pub overlay_shm: Option<String>,
    pub overlay_font_family: Option<String>,
    /// Resolved pause-while-overlay-visible setting (game → platform →
    /// global). Consumed two ways: `IRA_GBE_PAUSE_ON_OVERLAY` for
    /// Goldberg-emulated games (cooperative pause in our fork), and the
    /// process-group freeze watcher for external-host overlay mode.
    pub pause_game_on_overlay: bool,
}

/// Attach the ira-overlay to a prepared command/environment pair.
///
/// Shared by the Wine and native launch branches. Gamescope commands wrap the
/// standalone overlay process around the compositor; the game-only env vars
/// were already moved past Gamescope's `--` separator by `apply_performance`.
/// Any other command injects the Vulkan layer directly into the game environment.
fn attach_overlay(
    enabled: bool,
    cmd: &mut Vec<String>,
    env: &mut Vec<(String, String)>,
    overlay_shm: Option<&str>,
    font_family: Option<&str>,
) {
    if !enabled {
        return;
    }
    if super::env_builder::uses_gamescope(cmd) {
        let mut capture_env = Vec::new();
        super::env_builder::add_overlay_env_without_ui(&mut capture_env, overlay_shm, font_family);
        super::env_builder::add_overlay_env_external(env, overlay_shm, font_family);
        super::env_builder::wrap_with_host_overlay(cmd, &capture_env);
    } else {
        super::env_builder::add_overlay_env(env, overlay_shm, font_family);
    }
}

pub fn launch_game(
    launch: &GameLaunchConfig,
    wine: Option<&WineConfig>,
    ctx: &LaunchContext,
) -> Result<i32, String> {
    let started_at = unix_now();
    let game_dir = resolve_game_dir(launch);

    if !launch.pre_launch.is_empty() {
        super::pre_launch::run_pre_launch(
            &launch.pre_launch,
            game_dir.as_deref(),
            &ctx.save_dir,
            ctx.game_id,
            launch.pre_launch_wait.unwrap_or(true),
        )?;
    }

    centralize_saves(wine, ctx);

    let wine_enabled = wine.is_some_and(|w| w.enabled);
    let (mut command, mut env) = if wine_enabled {
        build_wine_launch(launch, wine.unwrap(), ctx)?
    } else {
        build_native_launch(launch, ctx)?
    };

    apply_pause_env(&mut env, ctx);

    // Daemon-side toggle chord for gamescope launches, where every
    // in-process toggle path stands down. The daemon swallows the chord
    // and reports it; the monitor below flips the SHM flag.
    super::env_builder::maybe_add_daemon_toggle_env(&mut env, &command, ctx.overlay_enabled);

    let host_child = spawn_overlay_host(&command, ctx);

    // Same file ira_input::calibration_store_path writes; the launcher does
    // not depend on that crate, so the name lives here too.
    let calibration = std::path::Path::new(&ctx.save_dir).join("controller_calibration.json");
    let input_launch = super::input_daemon::InputLaunch {
        mode: launch.input_mode,
        profile: launch.input_profile.clone(),
        calibration: Some(calibration.to_string_lossy().into_owned()),
        pause_unfocused: launch.input_pause_unfocused.unwrap_or(true),
    };
    match super::input_daemon::launch_via_daemon(
        &command,
        &env,
        game_dir.as_deref(),
        &input_launch,
        ctx.game_id,
    ) {
        Ok(client) => {
            // Daemon-supervised launches arm the freezer backend only (see
            // run_daemon_session): computed before the plan moves the command.
            let pause_watch = super::freeze::watch_config_daemon(
                ctx.pause_game_on_overlay,
                super::env_builder::uses_gamescope(&command),
                ctx.overlay_shm.as_deref(),
                ctx.game_id,
            );
            let plan = SpawnPlan {
                env,
                command,
                game_dir,
                // Daemon sessions pump output over IPC; nothing to tail.
                log_file: String::new(),
                pause_watch,
            };
            run_daemon_session(client, ctx, started_at, plan, &launch.post_exit, host_child);
            return Ok(0);
        }
        Err(reason) => {
            if matches!(launch.input_mode, Some(ControllerInputMode::Enabled)) {
                eprintln!("launch: input daemon unavailable ({reason}); using the wrapper");
            }
        }
    }
    let wrapped = WrappedSpawn {
        ctx,
        launch,
        started_at,
        command: &mut command,
        env: &mut env,
        game_dir,
        wine_enabled,
        host_child,
    };
    spawn_wrapped(wrapped)
}

fn resolve_game_dir(launch: &GameLaunchConfig) -> Option<String> {
    if launch.working_dir.is_empty() {
        std::path::Path::new(&launch.exe)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().to_string())
    } else {
        Some(launch.working_dir.clone())
    }
}

/// Centralize game saves via symlinks if enabled globally.
fn centralize_saves(wine: Option<&WineConfig>, ctx: &LaunchContext) {
    if !ctx.centralize_saves || ctx.ufs_savefiles.is_empty() {
        return;
    }
    let pfx = wine
        .filter(|w| w.enabled)
        .map(super::wine_launch::wine_prefix);
    super::game_saves::setup_game_saves(
        &ctx.ufs_savefiles,
        &ctx.ufs_rootoverrides,
        &ctx.app_id,
        &ctx.save_dir,
        pfx.as_deref(),
    );
}

fn parse_launch_args(launch: &GameLaunchConfig) -> Result<Vec<String>, String> {
    if launch.args.is_empty() {
        Ok(Vec::new())
    } else {
        shlex::split(&launch.args).ok_or_else(|| "Failed to parse arguments".to_string())
    }
}

fn build_wine_launch(
    launch: &GameLaunchConfig,
    wine: &WineConfig,
    ctx: &LaunchContext,
) -> Result<CommandEnv, String> {
    let wine_exe = super::wine_launch::find_wine_binary(&wine.version, &wine.custom_wine_path)?;
    if launch.exe.is_empty() {
        return Err("No executable specified".to_string());
    }
    let args = parse_launch_args(launch)?;

    let is_proton = super::wine_detect::is_proton_version(&wine.version)
        || super::wine_detect::is_proton_binary(&wine_exe);

    // For Proton versions, the command uses umu-run, but WINE env var
    // must still point to the actual Proton wine binary (umu reads it).
    let mut cmd = if wine.umu_enabled || is_proton {
        let umu = super::wine_detect::find_umu_binary()?;
        let mut c = vec![umu];
        if !launch.exe.is_empty() {
            c.push(launch.exe.clone());
        }
        c.extend_from_slice(&args);
        c
    } else {
        super::wine_launch::build_wine_command(&wine_exe, &launch.exe, &args, wine)
    };
    let mut env = super::env_builder::build_env(
        launch,
        Some(wine),
        &wine_exe,
        &ctx.save_dir,
        ctx.game_id,
        &ctx.app_id,
        &mut cmd,
    );

    // Set umu env vars for Proton versions (automatic) or explicit umu_enabled
    if wine.umu_enabled || is_proton {
        env_set(&mut env, "PROTON_VERB", "waitforexitandrun");
        if !ctx.app_id.is_empty() {
            env_set(&mut env, "GAMEID", &ctx.app_id);
        }
    }

    attach_overlay(
        ctx.overlay_enabled,
        &mut cmd,
        &mut env,
        ctx.overlay_shm.as_deref(),
        ctx.overlay_font_family.as_deref(),
    );

    let pfx = super::wine_launch::wine_prefix(wine);
    let prefix_ready = std::path::Path::new(&pfx).join("system.reg").is_file();

    if !prefix_ready && !wine.umu_enabled && !is_proton {
        run_wine_reg_commands(wine, &wine_exe, &env);
    }

    // Centralize emulator saves via symlinks in the Wine prefix
    match ctx.trophy_source {
        TrophySource::Gse => super::emulator_saves::setup_gbe_saves(&pfx, &ctx.save_dir),
        TrophySource::Nge => super::emulator_saves::setup_nge_saves(&pfx, &ctx.save_dir),
        _ => {}
    }

    Ok((cmd, env))
}

fn run_wine_reg_commands(wine: &WineConfig, wine_exe: &str, env: &[(String, String)]) {
    for reg_cmd in super::wine_launch::build_wine_reg_commands(wine, wine_exe) {
        let mut child = std::process::Command::new(&reg_cmd[0]);
        for arg in &reg_cmd[1..] {
            child.arg(arg);
        }
        child.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        match child.status() {
            Ok(s) if !s.success() && s.code() != Some(1) => {
                eprintln!(
                    "Wine reg command failed (exit {:?}): {:?}",
                    s.code(),
                    reg_cmd
                );
            }
            Err(e) => {
                eprintln!("Failed to run wine reg command: {}", e);
            }
            _ => {}
        }
    }
}

fn build_native_launch(
    launch: &GameLaunchConfig,
    ctx: &LaunchContext,
) -> Result<CommandEnv, String> {
    super::native_launch::validate_executable(&launch.exe)?;
    let args = parse_launch_args(launch)?;
    let mut cmd = super::native_launch::build_native_command(&launch.exe, &args);
    let mut env = super::env_builder::build_env(
        launch,
        None,
        "",
        &ctx.save_dir,
        ctx.game_id,
        &ctx.app_id,
        &mut cmd,
    );

    // Centralize GBE saves via symlinks in GBE's native save directory
    if ctx.trophy_source == TrophySource::Gse {
        super::emulator_saves::setup_gbe_saves_native(&ctx.save_dir);
    }

    attach_overlay(
        ctx.overlay_enabled,
        &mut cmd,
        &mut env,
        ctx.overlay_shm.as_deref(),
        ctx.overlay_font_family.as_deref(),
    );
    Ok((cmd, env))
}

/// Cooperative pause for Goldberg-emulated games (see `gbe_pause_env`).
/// Non-Goldberg games with the setting on cannot pause cooperatively, so
/// say so once instead of failing silently.
fn apply_pause_env(env: &mut Vec<(String, String)>, ctx: &LaunchContext) {
    if let Some((k, v)) = gbe_pause_env(
        ctx.pause_game_on_overlay,
        ctx.trophy_source,
        ctx.overlay_shm.as_deref(),
    ) {
        eprintln!("ira-pause: setting {k}=1 for Goldberg game '{}'", ctx.game_name);
        env_set(env, &k, &v);
    } else if ctx.pause_game_on_overlay {
        eprintln!(
            "ira-pause: enabled for '{}' but not Goldberg-emulated (trophy={:?}) — no cooperative pause",
            ctx.game_name, ctx.trophy_source
        );
    }
}

/// The GTK host renders the panel out-of-process for injected mode. Under
/// gamescope it runs inside the session (wrapped above), otherwise it
/// spawns here as a sibling of the game and is reaped on game exit.
fn spawn_overlay_host(command: &[String], ctx: &LaunchContext) -> Option<Child> {
    if ctx.overlay_enabled && !super::env_builder::uses_gamescope(command) {
        ctx.overlay_shm
            .as_deref()
            .and_then(super::overlay_host::spawn_host)
    } else {
        None
    }
}

fn monitor_context_for(
    ctx: &LaunchContext,
    started_at: i64,
    post_exit: &str,
    plan: SpawnPlan,
) -> super::wrapper::MonitorContext {
    super::wrapper::MonitorContext {
        sender: ctx.sender.clone(),
        game_id: ctx.game_id,
        variant_id: ctx.variant_id,
        count_playtime: ctx.count_playtime,
        started_at,
        db: ctx.db.clone(),
        running_games: ctx.running_games.clone(),
        env: plan.env,
        command: plan.command,
        post_exit: post_exit.to_string(),
        working_dir: plan.game_dir,
        log_file: plan.log_file,
        pause_watch: plan.pause_watch,
    }
}

/// The daemon supervises the game from here on; our monitor only consumes
/// its event stream. The real pid arrives with the SessionStarted event,
/// but the accepted launch must count as running right away: the UI
/// rebuilds the play button from this map as soon as GameStarted lands,
/// and an entry that only appears seconds later flipped the button back
/// to "Play" mid-launch.
fn run_daemon_session(
    client: DaemonClient,
    ctx: &LaunchContext,
    started_at: i64,
    plan: SpawnPlan,
    post_exit: &str,
    host_child: Option<Child>,
) {
    ctx.running_games.lock().unwrap().insert(ctx.game_id, 0);
    // Daemon-supervised launches arm the freezer backend only: the
    // daemon owns the pid, so the SIGSTOP fallback stays forbidden
    // (signals from outside could race session teardown). The
    // freezer sends no signals, and fatal signals still land on
    // frozen tasks, so teardown is unaffected.
    if plan.pause_watch.is_some() {
        eprintln!("ira-pause: freeze watcher armed for '{}'", ctx.game_name);
    }
    let mc = monitor_context_for(ctx, started_at, post_exit, plan);
    std::thread::spawn(move || super::wrapper::monitor_session(client, mc));
    if let Some(host) = host_child {
        super::overlay_host::reap_when_game_exits(host, ctx.running_games.clone(), ctx.game_id);
    }
}

/// Everything the wrapper fallback spawn needs, bundled so the helper
/// stays under the argument limit.
struct WrappedSpawn<'a> {
    ctx: &'a LaunchContext,
    launch: &'a GameLaunchConfig,
    started_at: i64,
    command: &'a mut Vec<String>,
    env: &'a mut Vec<(String, String)>,
    game_dir: Option<String>,
    wine_enabled: bool,
    host_child: Option<Child>,
}

fn spawn_wrapped(req: WrappedSpawn<'_>) -> Result<i32, String> {
    let WrappedSpawn {
        ctx,
        launch,
        started_at,
        command,
        env,
        game_dir,
        wine_enabled,
        host_child,
    } = req;
    // The game bypasses the daemon, so the daemon's desktop-default session
    // would keep remapping the pad underneath it — the exact bug that makes
    // "input remapping: disabled" look ignored. Hold the desktop behaviour
    // off for the game's lifetime; dropping the client releases it.
    let desktop_hold = if matches!(launch.input_mode, Some(ControllerInputMode::Enabled)) {
        None
    } else {
        super::input_daemon::hold_desktop_for_game()
    };
    let calibration = std::path::Path::new(&ctx.save_dir).join("controller_calibration.json");
    super::env_builder::wrap_with_input_mode(
        command,
        launch.input_mode,
        launch.input_profile.as_deref(),
        Some(calibration.to_str().unwrap_or_default()),
        launch.input_pause_unfocused.unwrap_or(true),
    );

    apply_script_env(env, game_dir.as_deref(), &ctx.game_name, wine_enabled);

    let log_path = super::wrapper::game_log_path(&ctx.save_dir, ctx.game_id);

    let child = match super::wrapper::spawn_game(command, env, game_dir.as_deref(), Some(&log_path)) {
        Ok(child) => child,
        Err(e) => {
            if let Some(mut host) = host_child {
                let _ = host.kill();
                let _ = host.wait();
            }
            return Err(e);
        }
    };
    let child_pid = child.id() as i32;

    ctx.running_games
        .lock()
        .map_err(|e| e.to_string())?
        .insert(ctx.game_id, child_pid);

    // Freeze only when the un-toggle survives: gamescope/external-host
    // mode, where ira-overlay-ui owns the visible window and mirrors hide
    // back into SHM. In-process mode would deadlock (frozen game can't see
    // the un-toggle), so it pauses cooperatively or not at all.
    let pause_watch = super::freeze::watch_config(
        ctx.pause_game_on_overlay,
        super::env_builder::uses_gamescope(command),
        true,
        ctx.overlay_shm.as_deref(),
        ctx.game_id,
    );
    if pause_watch.is_some() {
        eprintln!("ira-pause: freeze watcher armed for '{}'", ctx.game_name);
    }
    let plan = SpawnPlan {
        env: env.clone(),
        command: command.clone(),
        game_dir,
        log_file: log_path,
        pause_watch,
    };
    let mc = monitor_context_for(ctx, started_at, &launch.post_exit, plan);
    std::thread::spawn(move || {
        // Held until monitoring ends: the pad stays native for exactly as
        // long as the game lives.
        let _desktop_hold = desktop_hold;
        super::wrapper::monitor_process(child, child_pid, mc);
    });
    if let Some(host) = host_child {
        super::overlay_host::reap_when_game_exits(host, ctx.running_games.clone(), ctx.game_id);
    }

    Ok(child_pid)
}

/// Script-visible environment: the game directory plus Lutris-compatible
/// names for pre/post-launch scripts, and the Proton locale propagation
/// (`LC_ALL` → `HOST_LC_ALL`, per-game value winning).
fn apply_script_env(
    env: &mut Vec<(String, String)>,
    game_dir: Option<&str>,
    game_name: &str,
    wine_enabled: bool,
) {
    // Set PWD to the game's working directory
    if let Some(dir) = game_dir {
        env_set(env, "PWD", dir);
    }

    // Match Lutris: expose the game name/dir for pre/post-launch scripts and games.
    env_set(env, "GAME_NAME", game_name);
    if let Some(dir) = game_dir {
        env_set(env, "GAME_DIRECTORY", dir);
    }

    // Lutris's update_proton_env propagates LC_ALL → HOST_LC_ALL for Proton.
    // Applied after user env overrides so a per-game LC_ALL wins. Defaults to
    // the host locale so umu/pressure-vessel get a valid HOST_LC_ALL.
    if wine_enabled {
        let lc_all = env
            .iter()
            .find(|(k, _)| k == "LC_ALL")
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
            .or_else(|| std::env::var("LC_ALL").ok().filter(|v| !v.is_empty()))
            .or_else(|| std::env::var("LANG").ok().filter(|v| !v.is_empty()));
        if let Some(lc) = lc_all {
            env_set(env, "LC_ALL", &lc);
            env_set(env, "HOST_LC_ALL", &lc);
        }
    }
}

/// Cooperative-pause flag for our Goldberg fork: when the resolved
/// pause-while-overlay-visible setting is on for a Goldberg-emulated game
/// whose overlay SHM exists, the fork watches `overlay_visible` and posts
/// `GameOverlayActivated_t` so the game pauses itself exactly as on real
/// Steam. Returns the env pair to inject, or `None` when any precondition
/// fails (other games have no fork to read the flag; without SHM there is
/// no visibility signal to follow).
pub(crate) fn gbe_pause_env(
    pause_game_on_overlay: bool,
    trophy_source: TrophySource,
    overlay_shm: Option<&str>,
) -> Option<(String, String)> {
    if !pause_game_on_overlay {
        return None;
    }
    if trophy_source != TrophySource::Gse {
        return None;
    }
    if overlay_shm.is_none_or(|s| s.is_empty()) {
        return None;
    }
    Some(("IRA_GBE_PAUSE_ON_OVERLAY".to_string(), "1".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gbe_pause_env_set_for_gse_with_overlay() {
        // Arrange: opted-in Goldberg game with overlay SHM present.
        // Act
        let env = gbe_pause_env(true, TrophySource::Gse, Some("/ira_overlay_42"));
        // Assert
        assert_eq!(
            env,
            Some((
                "IRA_GBE_PAUSE_ON_OVERLAY".to_string(),
                "1".to_string()
            ))
        );
    }

    #[test]
    fn test_gbe_pause_env_unset_when_disabled() {
        assert_eq!(gbe_pause_env(false, TrophySource::Gse, Some("/ira_overlay_42")), None);
    }

    #[test]
    fn test_gbe_pause_env_unset_without_gse_or_shm() {
        // Non-Goldberg games have no fork to read the flag…
        assert_eq!(
            gbe_pause_env(true, TrophySource::SteamNative, Some("/ira_overlay_42")),
            None
        );
        assert_eq!(gbe_pause_env(true, TrophySource::Nge, Some("/ira_overlay_42")), None);
        // …and without overlay SHM there is no visibility signal to follow.
        assert_eq!(gbe_pause_env(true, TrophySource::Gse, None), None);
    }
}
