use ira_db::DbConn;
use ira_models::AppSender;
use ira_models::{GameLaunchConfig, TrophySource, UfsRootOverride, UfsSaveFile, WineConfig};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
        super::env_builder::add_overlay_env_standalone(env, overlay_shm, font_family);
        super::env_builder::wrap_with_standalone_overlay(cmd, &capture_env);
    } else {
        super::env_builder::add_overlay_env(env, overlay_shm, font_family);
    }
}

pub fn launch_game(
    launch: &GameLaunchConfig,
    wine: Option<&WineConfig>,
    ctx: &LaunchContext,
) -> Result<i32, String> {
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let game_dir = if launch.working_dir.is_empty() {
        std::path::Path::new(&launch.exe)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().to_string())
    } else {
        Some(launch.working_dir.clone())
    };

    if !launch.pre_launch.is_empty() {
        run_pre_launch(
            &launch.pre_launch,
            game_dir.as_deref(),
            &ctx.save_dir,
            ctx.game_id,
        )?;
    }

    // Centralize game saves via symlinks if enabled globally
    if ctx.centralize_saves && !ctx.ufs_savefiles.is_empty() {
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

    let (mut command, mut env) = if wine.is_some_and(|w| w.enabled) {
        let wine = wine.unwrap();
        let wine_exe = super::wine_launch::find_wine_binary(&wine.version, &wine.custom_wine_path)?;
        if launch.exe.is_empty() {
            return Err("No executable specified".to_string());
        }
        let args: Vec<String> = if launch.args.is_empty() {
            Vec::new()
        } else {
            shlex::split(&launch.args).ok_or_else(|| "Failed to parse arguments".to_string())?
        };

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
            env.retain(|(k, _)| k != "PROTON_VERB");
            env.push(("PROTON_VERB".to_string(), "waitforexitandrun".to_string()));
            if !ctx.app_id.is_empty() {
                env.retain(|(k, _)| k != "GAMEID");
                env.push(("GAMEID".to_string(), ctx.app_id.to_string()));
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
            for reg_cmd in super::wine_launch::build_wine_reg_commands(wine, &wine_exe) {
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

        // Centralize emulator saves via symlinks in the Wine prefix
        match ctx.trophy_source {
            TrophySource::Gse => super::emulator_saves::setup_gbe_saves(&pfx, &ctx.save_dir),
            TrophySource::Nge => super::emulator_saves::setup_nge_saves(&pfx, &ctx.save_dir),
            _ => {}
        }

        (cmd, env)
    } else {
        super::native_launch::validate_executable(&launch.exe)?;
        let args: Vec<String> = if launch.args.is_empty() {
            Vec::new()
        } else {
            shlex::split(&launch.args).ok_or_else(|| "Failed to parse arguments".to_string())?
        };
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
        (cmd, env)
    };

    let input_profile = launch.input_profile.as_deref();
    // Same file ira_input::calibration_store_path writes; the launcher does
    // not depend on that crate, so the name lives here too.
    let calibration = std::path::Path::new(&ctx.save_dir).join("controller_calibration.json");
    super::env_builder::wrap_with_input_mode(
        &mut command,
        launch.input_mode,
        input_profile,
        Some(calibration.to_str().unwrap_or_default()),
        launch.input_pause_unfocused.unwrap_or(true),
    )?;

    // Set PWD to the game's working directory
    if let Some(ref dir) = game_dir {
        env.retain(|(k, _)| k != "PWD");
        env.push(("PWD".to_string(), dir.clone()));
    }

    // Match Lutris: expose the game name/dir for pre/post-launch scripts and games.
    env.retain(|(k, _)| k != "GAME_NAME");
    env.push(("GAME_NAME".to_string(), ctx.game_name.clone()));
    if let Some(ref dir) = game_dir {
        env.retain(|(k, _)| k != "GAME_DIRECTORY");
        env.push(("GAME_DIRECTORY".to_string(), dir.clone()));
    }

    // Lutris's update_proton_env propagates LC_ALL → HOST_LC_ALL for Proton.
    // Applied after user env overrides so a per-game LC_ALL wins. Defaults to
    // the host locale so umu/pressure-vessel get a valid HOST_LC_ALL.
    if wine.is_some_and(|w| w.enabled) {
        let lc_all = env
            .iter()
            .find(|(k, _)| k == "LC_ALL")
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
            .or_else(|| std::env::var("LC_ALL").ok().filter(|v| !v.is_empty()))
            .or_else(|| std::env::var("LANG").ok().filter(|v| !v.is_empty()));
        if let Some(lc) = lc_all {
            env.retain(|(k, _)| k != "LC_ALL");
            env.push(("LC_ALL".to_string(), lc.clone()));
            env.retain(|(k, _)| k != "HOST_LC_ALL");
            env.push(("HOST_LC_ALL".to_string(), lc));
        }
    }

    let log_path = super::wrapper::game_log_path(&ctx.save_dir, ctx.game_id);

    let child = super::wrapper::spawn_game(&command, &env, game_dir.as_deref(), Some(&log_path))?;
    let child_pid = child.id() as i32;

    ctx.running_games
        .lock()
        .map_err(|e| e.to_string())?
        .insert(ctx.game_id, child_pid);

    let game_id = ctx.game_id;
    let variant_id = ctx.variant_id;
    let mc = super::wrapper::MonitorContext {
        sender: ctx.sender.clone(),
        game_id,
        variant_id,
        count_playtime: ctx.count_playtime,
        started_at,
        db: ctx.db.clone(),
        running_games: ctx.running_games.clone(),
        env: env.clone(),
        command: command.clone(),
    };
    std::thread::spawn(move || {
        super::wrapper::monitor_process(child, child_pid, mc);
    });

    Ok(child_pid)
}

/// Run a pre-launch command synchronously via `sh -c`.
/// Uses the game's working directory. Aborts launch on non-zero exit.
fn run_pre_launch(
    cmd: &str,
    cwd: Option<&str>,
    save_dir: &str,
    game_id: i64,
) -> Result<(), String> {
    let log_path = super::wrapper::game_log_path(save_dir, game_id);
    let mut child = std::process::Command::new("sh");
    child.arg("-c").arg(cmd);
    if let Some(dir) = cwd {
        child.current_dir(dir);
    }
    if let Some(parent) = std::path::Path::new(&log_path).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("Failed to create log directory {}: {}", parent.display(), e);
        }
    }
    match child.output() {
        Ok(out) => {
            if let Ok(mut f) = std::fs::File::create(&log_path) {
                use std::io::Write;
                let _ = f.write_all(&out.stdout);
                let _ = f.write_all(&out.stderr);
            }
            if out.status.success() {
                return Ok(());
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{}\n{}", stdout.trim(), stderr.trim());
            let snippet = if combined.trim().is_empty() {
                "no output".to_string()
            } else {
                combined
                    .trim()
                    .lines()
                    .take(10)
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Err(format!(
                "Pre-launch command failed (exit code {:?}):\n{}",
                out.status.code(),
                snippet
            ))
        }
        Err(e) => Err(format!(
            "Failed to run pre-launch command {cmd:?} with cwd {:?}: {} (kind={:?}, raw_os_error={:?})",
            cwd,
            e,
            e.kind(),
            e.raw_os_error()
        )),
    }
}
