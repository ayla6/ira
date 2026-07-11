pub mod wrapper;
pub mod env_builder;
pub mod wine_launch;
pub mod native_launch;

use crate::models::{GameLaunchConfig, WineConfig};
use crate::db::DbConn;
use crate::AppSender;

use std::sync::{Arc, Mutex};
use std::collections::HashMap;

pub fn launch_game(
    launch: &GameLaunchConfig,
    wine: Option<&WineConfig>,
    _game_name: &str,
    sender: AppSender,
    game_id: i64,
    lutris_id: i64,
    db: DbConn,
    save_dir: &str,
    running_games: Arc<Mutex<HashMap<i64, i32>>>,
) -> Result<i32, String> {
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let (command, env) = if wine.map_or(false, |w| w.enabled) {
        let wine = wine.unwrap();
        let wine_exe = wine_launch::find_wine_binary(&wine.version, &wine.custom_wine_path)?;
        if launch.exe.is_empty() {
            return Err("No executable specified".to_string());
        }
        let args: Vec<String> = if launch.args.is_empty() {
            Vec::new()
        } else {
            shlex::split(&launch.args).ok_or_else(|| "Failed to parse arguments".to_string())?
        };
        let mut cmd = wine_launch::build_wine_command(&wine_exe, &launch.exe, &args, wine);
        let env = env_builder::build_env(launch, Some(wine), &wine_exe, save_dir, game_id, &mut cmd);

        let pfx = wine_launch::wine_prefix(wine);
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

        if !prefix_ready {
            for reg_cmd in wine_launch::build_wine_reg_commands(wine, &wine_exe) {
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
        native_launch::validate_executable(&launch.exe)?;
        let args: Vec<String> = if launch.args.is_empty() {
            Vec::new()
        } else {
            shlex::split(&launch.args).ok_or_else(|| "Failed to parse arguments".to_string())?
        };
        let mut cmd = native_launch::build_native_command(&launch.exe, &args);
        let env = env_builder::build_env(launch, None, "", save_dir, game_id, &mut cmd);
        (cmd, env)
    };

    let game_dir = if launch.working_dir.is_empty() {
        std::path::Path::new(&launch.exe).parent().map(|p| p.to_string_lossy().to_string())
    } else {
        Some(launch.working_dir.clone())
    };

    let child = wrapper::spawn_game(&command, &env, game_dir.as_deref())?;
    let child_pid = child.id() as i32;

    running_games.lock().map_err(|e| e.to_string())?.insert(lutris_id, child_pid);

    let sender_c = sender.clone();
    let db_c = db.clone();
    let rg = running_games.clone();
    std::thread::spawn(move || {
        wrapper::monitor_process(child, child_pid, &sender_c, lutris_id, started_at, db_c, game_id, rg);
    });

    Ok(child_pid)
}
