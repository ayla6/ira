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
        let mut cmd = wine_launch::build_wine_command(&wine_exe, &launch.exe, &args);
        let env = env_builder::build_env(launch, Some(wine), &wine_exe, save_dir, game_id, &mut cmd);
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
