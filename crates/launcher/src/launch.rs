use ira_models::{GameLaunchConfig, WineConfig};
use ira_db::DbConn;
use ira_models::AppSender;

use std::sync::{Arc, Mutex};
use std::collections::HashMap;

pub struct LaunchContext {
    pub game_name: String,
    pub sender: AppSender,
    pub game_id: i64,
    pub db: DbConn,
    pub save_dir: String,
    pub running_games: Arc<Mutex<HashMap<i64, i32>>>,
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

        let mut cmd = if wine.umu_enabled {
            let mut c = vec!["umu-run".to_string()];
            if !launch.exe.is_empty() {
                c.push(launch.exe.clone());
            }
            c.extend_from_slice(&args);
            c
        } else {
            super::wine_launch::build_wine_command(&wine_exe, &launch.exe, &args, wine)
        };
        let env = super::env_builder::build_env(launch, Some(wine), &wine_exe, &ctx.save_dir, ctx.game_id, &mut cmd);

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

        if !prefix_ready && !wine.umu_enabled {
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
            let _ = std::fs::write(&version_file, &wine.version);
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
        let env = super::env_builder::build_env(launch, None, "", &ctx.save_dir, ctx.game_id, &mut cmd);
        (cmd, env)
    };

    let game_dir = if launch.working_dir.is_empty() {
        std::path::Path::new(&launch.exe).parent().map(|p| p.to_string_lossy().to_string())
    } else {
        Some(launch.working_dir.clone())
    };

    let log_path = super::wrapper::game_log_path(&ctx.save_dir, ctx.game_id);
    let child = super::wrapper::spawn_game(&command, &env, game_dir.as_deref(), Some(&log_path))?;
    let child_pid = child.id() as i32;

    ctx.running_games.lock().map_err(|e| e.to_string())?.insert(ctx.game_id, child_pid);

    let game_id = ctx.game_id;
    let sender_c = ctx.sender.clone();
    let db_c = ctx.db.clone();
    let rg = ctx.running_games.clone();
    std::thread::spawn(move || {
        super::wrapper::monitor_process(child, child_pid, &sender_c, game_id, started_at, db_c, rg);
    });

    Ok(child_pid)
}
