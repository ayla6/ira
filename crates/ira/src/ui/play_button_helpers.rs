use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ira_config::Config;
use ira_db::DbConn;
use ira_models::{AppSender, WineConfig};

use super::state::SharedState;

pub(super) struct LaunchCtx<'a> {
    pub db: &'a DbConn,
    pub save_dir: &'a str,
    pub game_id: i64,
    pub db_id: i64,
    pub game_name: &'a str,
    pub game_kind: ira_models::GameKind,
    pub sender: &'a AppSender,
    pub running_games: &'a Arc<Mutex<HashMap<i64, i32>>>,
    pub started_at: i64,
    pub overlay_shm: Option<String>,
    pub overlay_global_enabled: bool,
}

fn spawn_and_monitor(ctx: &LaunchCtx, cmd: &[String], env: &[(String, String)], err_label: &str) -> Result<(), String> {
    let log_path = ira_launcher::wrapper::game_log_path(ctx.save_dir, ctx.game_id);
    match ira_launcher::wrapper::spawn_game(cmd, env, None, Some(&log_path)) {
        Ok(child) => {
            let pid = child.id() as i32;
            ctx.running_games.lock().unwrap().insert(ctx.game_id, pid);
            let mc = ira_launcher::wrapper::MonitorContext {
                sender: ctx.sender.clone(),
                game_id: ctx.game_id,
                variant_id: None,
                count_playtime: true,
                started_at: ctx.started_at,
                db: ctx.db.clone(),
                running_games: ctx.running_games.clone(),
            };
            std::thread::spawn(move || {
                ira_launcher::wrapper::monitor_process(child, pid, mc);
            });
            Ok(())
        }
        Err(e) => Err(format!("Failed to launch {}: {}", err_label, e)),
    }
}

/// Build env vars for emulator launches, including overlay env vars if enabled.
/// Checks per-game overlay override first, then falls back to the global source setting.
fn build_emulator_env(ctx: &LaunchCtx) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    let overlay_enabled = ira_db::get_game_config(ctx.db, ctx.db_id)
        .ok()
        .flatten()
        .and_then(|(launch, _, _)| launch.overlay_enabled)
        .unwrap_or(ctx.overlay_global_enabled);
    eprintln!("ira-overlay: emulator launch overlay_enabled={} (global={}, game_id={})", overlay_enabled, ctx.overlay_global_enabled, ctx.game_id);
    if overlay_enabled {
        ira_launcher::env_builder::add_overlay_env(&mut env, ctx.overlay_shm.as_deref());
        eprintln!("ira-overlay: overlay env vars added to emulator launch");
    }
    env
}

/// If the command is a Flatpak invocation, inject overlay env vars as `--env` flags
/// and add `--filesystem` for the layer directory and .so files. Flatpak sandboxes
/// don't inherit arbitrary env vars from the parent process, so `VK_LAYER_PATH` etc.
/// set via `Command::env()` never reach the sandboxed app.
///
/// `LD_PRELOAD` is stripped — Flatpak blocks it for security. The overlay still
/// works without the shim (Wayland keyboard + evdev gamepad provide input).
fn inject_flatpak_overlay_env(cmd: &mut Vec<String>, env: &mut Vec<(String, String)>) {
    if cmd.len() < 3 || cmd[0] != "flatpak" || cmd[1] != "run" {
        return;
    }

    let overlay_keys = ["VK_LAYER_PATH", "VK_INSTANCE_LAYERS", "IRA_OVERLAY_SHM"];
    let mut env_flags: Vec<String> = Vec::new();
    let mut fs_dirs: Vec<String> = Vec::new();

    for key in &overlay_keys {
        if let Some(pos) = env.iter().position(|(k, _)| k == key) {
            let value = env[pos].1.clone();
            env_flags.push(format!("--env={}={}", key, value));
            if *key == "VK_LAYER_PATH" {
                fs_dirs.push(value);
            }
        }
    }

    // LD_PRELOAD won't work in Flatpak sandbox, but the .so directory
    // also contains the Vulkan layer .so — grant filesystem access so
    // the Vulkan loader can dlopen it.
    if let Some(pos) = env.iter().position(|(k, _)| k == "LD_PRELOAD") {
        let preload_path = env[pos].1.clone();
        if let Some(parent) = std::path::Path::new(&preload_path).parent() {
            fs_dirs.push(parent.to_string_lossy().into_owned());
        }
        env.retain(|(k, _)| k != "LD_PRELOAD");
    }

    for dir in &fs_dirs {
        env_flags.push(format!("--filesystem={}", dir));
    }

    // Insert --env flags after "run" and before the flatpak ID
    let insert_pos = 2;
    for (i, flag) in env_flags.into_iter().enumerate() {
        cmd.insert(insert_pos + i, flag);
    }
}

pub(super) fn launch_retro(
    ctx: &LaunchCtx,
    cfg: &Config,
    platform_id: &str,
    per_game_emu: &str,
    per_game_ra_core: &str,
    game_path: &str,
) -> Result<(), String> {
    let cc = cfg.console(platform_id);
    let exe = if !per_game_emu.is_empty() {
        per_game_emu
    } else if cc.executable.is_empty() {
        return Err(format!("No emulator configured for {}", platform_id));
    } else {
        &cc.executable
    };
    let core = if !per_game_ra_core.is_empty() {
        per_game_ra_core
    } else {
        &cc.ra_core
    };
    let fullscreen_flag = ira_models::find_console(platform_id)
        .map(|d| d.fullscreen_flag)
        .unwrap_or("--fullscreen");
    let rom_path = {
        let discs = ira_db::get_discs(ctx.db, ctx.db_id).unwrap_or_default();
        let default_disc_id = ira_db::get_default_disc(ctx.db, ctx.db_id);
        discs.iter()
            .find(|d| Some(d.id) == default_disc_id)
            .map(|d| d.rom_path.clone())
            .unwrap_or_else(|| game_path.to_string())
    };
    let mut cmd = ira_platforms::emulator_detect::build_launch_command(exe, &rom_path, core, cc.fullscreen, fullscreen_flag);
    let mut env = build_emulator_env(ctx);
    inject_flatpak_overlay_env(&mut cmd, &mut env);
    spawn_and_monitor(ctx, &cmd, &env, ctx.game_name)
}

pub(super) fn launch_ps4(
    ctx: &LaunchCtx,
    per_game_version: &str,
    global_shadps4_exe: &str,
    game_path: &str,
) -> Result<(), String> {
    let exe = if !per_game_version.is_empty() {
        per_game_version
    } else if !global_shadps4_exe.is_empty() {
        global_shadps4_exe
    } else {
        "shadps4"
    };
    let mut cmd = vec![exe.to_string(), "-g".to_string(), game_path.to_string()];
    let mut env = build_emulator_env(ctx);
    inject_flatpak_overlay_env(&mut cmd, &mut env);
    spawn_and_monitor(ctx, &cmd, &env, "shadPS4")
}

pub(super) fn launch_ps3(
    ctx: &LaunchCtx,
    per_game_emu: &str,
    global_rpcs3_exe: &str,
    game_path: &str,
) -> Result<(), String> {
    let exe = if !per_game_emu.is_empty() {
        per_game_emu
    } else if !global_rpcs3_exe.is_empty() {
        global_rpcs3_exe
    } else {
        "rpcs3"
    };
    let mut cmd = vec![exe.to_string(), "--no-gui".to_string(), game_path.to_string()];
    let mut env = build_emulator_env(ctx);
    inject_flatpak_overlay_env(&mut cmd, &mut env);
    spawn_and_monitor(ctx, &cmd, &env, "RPCS3")
}

pub(super) fn launch_steam(app_id: &str) -> Result<(), String> {
    let cmd = vec!["steam".to_string(), "-applaunch".to_string(), app_id.to_string()];
    match ira_launcher::wrapper::spawn_game(&cmd, &[], None, None) {
        Ok(_child) => {}
        Err(_) => {
            let uri = format!("steam://run/{}", app_id);
            let cmd = vec!["xdg-open".to_string(), uri];
            if let Err(e) = ira_launcher::wrapper::spawn_game(&cmd, &[], None, None) {
                return Err(format!("Failed to launch Steam game: {}", e));
            }
        }
    }
    Ok(())
}

pub(super) fn launch_other(
    ctx: &LaunchCtx,
    app_default_wine: &WineConfig,
    variant_id: Option<i64>,
    variant_count_playtime: bool,
    default_native_env_vars: &[(String, String)],
    app_id: &str,
) -> Result<(), String> {
    let (mut launch, mut wine, profile_id) = ira_db::get_game_config(ctx.db, ctx.db_id)
        .ok()
        .flatten()
        .unwrap_or_default();

    if let Some(pid) = profile_id {
        if let Ok(Some(profile)) = ira_db::get_profile(ctx.db, pid) {
            wine.version = profile.wine_version;
            wine.custom_wine_path = profile.custom_wine_path;
            wine.prefix = profile.prefix;
            wine.arch = profile.arch;
            wine.umu_enabled = profile.umu_enabled;
        }
    }

    wine = wine.merge_with_default(app_default_wine);

    // Only Wine games should use Wine. Linux/Other games must launch natively
    // even if a default Wine config with enabled=true exists.
    if ctx.game_kind != ira_models::GameKind::Wine {
        wine.enabled = false;
    }

    if let Some(vid) = variant_id {
        if let Ok(variants) = ira_db::get_variants(ctx.db, ctx.db_id) {
            if let Some(var) = variants.iter().find(|v| v.id == vid) {
                if !var.exe.is_empty() {
                    launch.exe = var.exe.clone();
                }
                if !var.working_dir.is_empty() {
                    launch.working_dir = var.working_dir.clone();
                }
                if !var.args.is_empty() {
                    launch.args = var.args.clone();
                }
                if !var.env_vars.is_empty() {
                    launch.env_vars = var.env_vars.clone();
                }
                if !var.pre_launch.is_empty() {
                    launch.pre_launch = var.pre_launch.clone();
                }
            }
        }
    }

    if !default_native_env_vars.is_empty() {
        let mut merged = default_native_env_vars.to_vec();
        for (k, v) in &launch.env_vars {
            merged.retain(|(ek, _)| ek != k);
            merged.push((k.clone(), v.clone()));
        }
        launch.env_vars = merged;
    }

    if !launch.exe.is_empty() {
        let overlay_enabled = launch.overlay_enabled.unwrap_or(ctx.overlay_global_enabled);
        let wine_opt = if wine.enabled { Some(&wine) } else { None };
        ira_launcher::launch_game(
            &launch,
            wine_opt,
            &ira_launcher::LaunchContext {
                game_name: ctx.game_name.to_string(),
                sender: ctx.sender.clone(),
                game_id: ctx.game_id,
                variant_id,
                count_playtime: variant_count_playtime,
                app_id: app_id.to_string(),
                db: ctx.db.clone(),
                save_dir: ctx.save_dir.to_string(),
                running_games: ctx.running_games.clone(),
                overlay_enabled,
                overlay_shm: if overlay_enabled { ctx.overlay_shm.clone() } else { None },
            },
        )?;
    } else {
        return Err(format!("No launch config saved for '{}'. Configure the game's launch settings first.", ctx.game_name));
    }
    Ok(())
}

pub(super) fn update_last_played(
    state: &SharedState,
    ctx: &LaunchCtx,
    variant_id: Option<i64>,
    variant_count_playtime: bool,
    variant_show_as_entry: bool,
) {
    if variant_count_playtime && variant_show_as_entry {
        if let Some(vid) = variant_id {
            if let Err(e) = ira_db::set_variant_last_played(ctx.db, vid, ctx.started_at) {
                eprintln!("Failed to update variant last played: {}", e);
            }
        }
        if let Some(g) = state.borrow_mut().games.iter_mut().find(|g| g.db_id == ctx.game_id && g.variant_id == variant_id) {
            g.last_played = ctx.started_at;
        }
    } else if variant_count_playtime {
        if let Err(e) = ira_db::set_last_played(ctx.db, ctx.db_id, ctx.started_at) {
            eprintln!("Failed to update last played: {}", e);
        }
        if let Some(g) = state.borrow_mut().games.iter_mut().find(|g| g.db_id == ctx.game_id && g.variant_id.is_none()) {
            g.last_played = ctx.started_at;
        }
    }
}
