use ira_models::{GameLaunchConfig, WineConfig, TrophySource, UfsSaveFile, UfsRootOverride};
use ira_db::DbConn;
use ira_models::AppSender;

use std::sync::{Arc, Mutex};
use std::collections::HashMap;

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
        std::path::Path::new(&launch.exe).parent().map(|p| p.to_string_lossy().to_string())
    } else {
        Some(launch.working_dir.clone())
    };

    if !launch.pre_launch.is_empty() {
        run_pre_launch(&launch.pre_launch, game_dir.as_deref(), &ctx.save_dir, ctx.game_id)?;
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

    let (command, env) = if wine.is_some_and(|w| w.enabled) {
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

        let is_proton = super::wine_detect::is_proton_version(&wine.version);

        // For Proton versions, the command uses umu-run, but WINE env var
        // must still point to the actual Proton wine binary (umu reads it).
        let mut cmd = if wine.umu_enabled || is_proton {
            let mut c = vec!["umu-run".to_string()];
            if !launch.exe.is_empty() {
                c.push(launch.exe.clone());
            }
            c.extend_from_slice(&args);
            c
        } else {
            super::wine_launch::build_wine_command(&wine_exe, &launch.exe, &args, wine)
        };
        let mut env = super::env_builder::build_env(launch, Some(wine), &wine_exe, &ctx.save_dir, ctx.game_id, &ctx.app_id, &mut cmd);

        // Set umu env vars for Proton versions (automatic) or explicit umu_enabled
        if wine.umu_enabled || is_proton {
            env.retain(|(k, _)| k != "PROTON_VERB");
            env.push(("PROTON_VERB".to_string(), "waitforexitandrun".to_string()));
            if !ctx.app_id.is_empty() {
                env.retain(|(k, _)| k != "GAMEID");
                env.push(("GAMEID".to_string(), ctx.app_id.to_string()));
            }
        }

        // Centralize GBE saves via GseSavePath env var
        if ctx.trophy_source == TrophySource::Gse {
            super::emulator_saves::setup_gbe_saves(&mut env, &ctx.save_dir, true);
        }

        if ctx.overlay_enabled {
            if super::env_builder::uses_gamescope(&cmd) {
                super::env_builder::add_overlay_env_standalone(&mut env, ctx.overlay_shm.as_deref(), ctx.overlay_font_family.as_deref());
                super::env_builder::wrap_with_standalone_overlay(&mut cmd);
            } else {
                super::env_builder::add_overlay_env(&mut env, ctx.overlay_shm.as_deref(), ctx.overlay_font_family.as_deref());
            }
        }

        let pfx = super::wine_launch::wine_prefix(wine);
        let prefix_ready = std::path::Path::new(&pfx).join("system.reg").is_file();

        let version_file = std::path::Path::new(&pfx).join(".av_wine_version");
        let version_matches = if version_file.is_file() {
            std::fs::read_to_string(&version_file)
                .map(|v| v.trim() == wine.version)
                .unwrap_or(false)
        } else {
            false
        };

        if prefix_ready && !version_matches {
            eprintln!("Warning: wine version mismatch for prefix {}. Configured: '{}', expected: '{}'", pfx, wine.version, std::fs::read_to_string(&version_file).unwrap_or_default().trim());
        }

        if !prefix_ready && !wine.umu_enabled && !is_proton {
            for reg_cmd in super::wine_launch::build_wine_reg_commands(wine, &wine_exe) {
                let mut child = std::process::Command::new(&reg_cmd[0]);
                for arg in &reg_cmd[1..] {
                    child.arg(arg);
                }
                child.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
                match child.status() {
                    Ok(s) if !s.success() && s.code() != Some(1) => {
                        eprintln!("Wine reg command failed (exit {:?}): {:?}", s.code(), reg_cmd);
                    }
                    Err(e) => {
                        eprintln!("Failed to run wine reg command: {}", e);
                    }
                    _ => {}
                }
            }
            if let Err(e) = std::fs::write(&version_file, &wine.version) {
                eprintln!("Failed to write wine version file: {}", e);
            }
        }

        // Centralize NGE saves via symlinks in the Wine prefix
        if ctx.trophy_source == TrophySource::Nge {
            super::emulator_saves::setup_nge_saves(&pfx, &ctx.save_dir);
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
        let mut env = super::env_builder::build_env(launch, None, "", &ctx.save_dir, ctx.game_id, &ctx.app_id, &mut cmd);

        // Centralize GBE saves via GseSavePath env var (native Linux)
        if ctx.trophy_source == TrophySource::Gse {
            super::emulator_saves::setup_gbe_saves(&mut env, &ctx.save_dir, false);
        }

        if ctx.overlay_enabled {
            if super::env_builder::uses_gamescope(&cmd) {
                super::env_builder::add_overlay_env_standalone(&mut env, ctx.overlay_shm.as_deref(), ctx.overlay_font_family.as_deref());
                super::env_builder::wrap_with_standalone_overlay(&mut cmd);
            } else {
                super::env_builder::add_overlay_env(&mut env, ctx.overlay_shm.as_deref(), ctx.overlay_font_family.as_deref());
            }
        }
        (cmd, env)
    };

    let log_path = super::wrapper::game_log_path(&ctx.save_dir, ctx.game_id);
    let child = super::wrapper::spawn_game(&command, &env, game_dir.as_deref(), Some(&log_path))?;
    let child_pid = child.id() as i32;

    ctx.running_games.lock().map_err(|e| e.to_string())?.insert(ctx.game_id, child_pid);

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
    };
    std::thread::spawn(move || {
        super::wrapper::monitor_process(child, child_pid, mc);
    });

    Ok(child_pid)
}

/// Run a pre-launch command synchronously via `sh -c`.
/// Uses the game's working directory. Aborts launch on non-zero exit.
fn run_pre_launch(cmd: &str, cwd: Option<&str>, save_dir: &str, game_id: i64) -> Result<(), String> {
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
    match std::fs::File::create(&log_path) {
        Ok(f) => {
            let stderr = f.try_clone().unwrap_or_else(|_| {
                std::fs::File::create("/dev/null").unwrap()
            });
            child.stdout(std::process::Stdio::from(f));
            child.stderr(std::process::Stdio::from(stderr));
        }
        Err(e) => eprintln!("Could not open log file {}: {}", log_path, e),
    }
    match child.status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("Pre-launch command failed (exit code {:?})", s.code())),
        Err(e) => Err(format!("Failed to run pre-launch command: {}", e)),
    }
}
