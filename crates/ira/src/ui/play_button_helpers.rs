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
    pub sender: &'a AppSender,
    pub running_games: &'a Arc<Mutex<HashMap<i64, i32>>>,
    pub started_at: i64,
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
    let cmd = ira_platforms::emulator_detect::build_launch_command(exe, &rom_path, core, cc.fullscreen, fullscreen_flag);
    spawn_and_monitor(ctx, &cmd, &[], ctx.game_name)
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
    let cmd = vec![exe.to_string(), "-g".to_string(), game_path.to_string()];

    let overlay_layer_path = format!("{}/../overlay", env!("CARGO_MANIFEST_DIR"));
    let env: Vec<(String, String)> = std::env::vars()
        .chain([
            ("VK_LAYER_PATH".to_string(), overlay_layer_path.clone()),
            ("VK_INSTANCE_LAYERS".to_string(), "VK_LAYER_IRA_OVERLAY".to_string()),
        ])
        .collect();

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
    let cmd = vec![exe.to_string(), "--no-gui".to_string(), game_path.to_string()];
    spawn_and_monitor(ctx, &cmd, &[], "RPCS3")
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
