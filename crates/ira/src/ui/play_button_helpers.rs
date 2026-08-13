use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gio::prelude::ListModelExt;
use glib::object::Cast;
use gtk4::gdk::prelude::{DisplayExt, MonitorExt};
use gtk4::gdk::{Display, Monitor};
use ira_config::Config;
use ira_db::DbConn;
use ira_input::{InputProfile, VirtualGamepadBackend};
use ira_models::{AppSender, ControllerInputMode, WineConfig};

use super::input_profile_store::{read_profile, write_profile};
use super::state::SharedState;

fn detect_screen_resolution() -> (u32, u32) {
    Display::default()
        .and_then(|d| {
            let monitors = d.monitors();
            (0..monitors.n_items())
                .find_map(|i| monitors.item(i).and_then(|m| m.downcast::<Monitor>().ok()))
        })
        .map(|m| {
            let g = m.geometry();
            (g.width() as u32, g.height() as u32)
        })
        .unwrap_or((1920, 1080))
}

pub(super) struct LaunchCtx<'a> {
    pub db: &'a DbConn,
    pub save_dir: &'a str,
    pub game_id: i64,
    pub db_id: i64,
    pub game_name: &'a str,
    pub game_kind: ira_models::GameKind,
    pub trophy_source: ira_models::TrophySource,
    pub ufs_savefiles: Vec<ira_models::UfsSaveFile>,
    pub ufs_rootoverrides: Vec<ira_models::UfsRootOverride>,
    pub centralize_saves: bool,
    pub sender: &'a AppSender,
    pub running_games: &'a Arc<Mutex<HashMap<i64, i32>>>,
    pub started_at: i64,
    pub overlay_shm: Option<String>,
    pub overlay_global_enabled: bool,
    pub overlay_font_family: Option<String>,
    pub gamescope_default: bool,
    pub gamemode_default: bool,
    pub mangohud_default: bool,
    pub gamescope_w_default: u32,
    pub gamescope_h_default: u32,
    pub gamescope_fps_default: u32,
    pub gamescope_upscaling_default: String,
    pub gpu_default: String,
    pub controller_input_mode: ControllerInputMode,
    pub controller_input_profile: Option<String>,
}

pub(super) struct PcControllerProfiles<'a> {
    pub linux: (Option<ControllerInputMode>, &'a str),
    pub wine: (Option<ControllerInputMode>, &'a str),
}

fn input_backend(mode: ControllerInputMode) -> Option<VirtualGamepadBackend> {
    match mode {
        ControllerInputMode::Disabled => None,
        ControllerInputMode::VirtualXInput => Some(VirtualGamepadBackend::XInput),
        ControllerInputMode::VirtualDirectInput => Some(VirtualGamepadBackend::DirectInput),
    }
}

fn resolved_input_mode(
    game_kind: ira_models::GameKind,
    game_override: Option<ControllerInputMode>,
    controller_default: ControllerInputMode,
) -> ControllerInputMode {
    game_override.unwrap_or_else(|| {
        if game_kind == ira_models::GameKind::Steam {
            ControllerInputMode::Disabled
        } else {
            controller_default
        }
    })
}

fn console_input_mode(
    game_override: Option<ControllerInputMode>,
    console_override: Option<ControllerInputMode>,
    controller_default: ControllerInputMode,
    console_profile: Option<&str>,
) -> ControllerInputMode {
    if let Some(mode) = game_override.or(console_override) {
        mode
    } else if console_profile.is_some_and(|profile| !profile.is_empty()) {
        // The selected profile owns its backend; this only enables the broker.
        ControllerInputMode::VirtualXInput
    } else {
        controller_default
    }
}

fn resolve_input_profile(
    ctx: &LaunchCtx,
    mode: ControllerInputMode,
    selected_profile: Option<&str>,
    default_profile: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(backend) = input_backend(mode) else {
        return Ok(None);
    };
    let selected_profile = selected_profile.filter(|path| !path.is_empty());
    let generated_path = || {
        std::path::Path::new(ctx.save_dir)
            .join("controller_defaults")
            .join(match backend {
                VirtualGamepadBackend::XInput => "resolved-xinput.json",
                VirtualGamepadBackend::DirectInput => "resolved-directinput.json",
            })
    };
    let profile_path = selected_profile
        .map(std::path::PathBuf::from)
        .or_else(|| default_profile.map(std::path::PathBuf::from))
        .unwrap_or_else(generated_path);
    if !profile_path.is_file() {
        let mut profile = InputProfile::default_gamepad_for_backend(backend);
        profile.name = format!("Built-in {:?} profile", backend);
        write_profile(&profile_path, &profile)?;
    }
    read_profile(&profile_path)?;
    Ok(Some(profile_path.to_string_lossy().into_owned()))
}

fn spawn_and_monitor(
    ctx: &LaunchCtx,
    cmd: &[String],
    env: &[(String, String)],
    err_label: &str,
) -> Result<(), String> {
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
                env: env.to_vec(),
                command: cmd.to_vec(),
            };
            std::thread::spawn(move || {
                ira_launcher::wrapper::monitor_process(child, pid, mc);
            });
            Ok(())
        }
        Err(e) => Err(format!("Failed to launch {}: {}", err_label, e)),
    }
}

/// Build env vars for emulator launches, apply performance wrappers (gamemode/
/// mangohud/gamescope), and set up overlay env (VK layer or standalone mode).
/// Checks per-game overlay override first, then falls back to the global source setting.
fn build_emulator_env_and_wrap(
    ctx: &LaunchCtx,
    cmd: &mut Vec<String>,
    console_mode: Option<ControllerInputMode>,
    console_profile: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            !k.starts_with("CARGO_") && !k.starts_with("RUSTUP_") && !k.starts_with("RUST_")
        })
        .filter(|(k, v)| {
            if k == "LD_LIBRARY_PATH" {
                let filtered: Vec<&str> = v
                    .split(':')
                    .filter(|p| {
                        !p.is_empty() && !p.contains("/.rustup/") && !p.contains("/target/")
                    })
                    .collect();
                !filtered.is_empty()
            } else {
                true
            }
        })
        .map(|(k, v)| {
            if k == "LD_LIBRARY_PATH" {
                let filtered: Vec<&str> = v
                    .split(':')
                    .filter(|p| {
                        !p.is_empty() && !p.contains("/.rustup/") && !p.contains("/target/")
                    })
                    .collect();
                (k, filtered.join(":"))
            } else {
                (k, v)
            }
        })
        .collect();

    let (launch, wine, _profile_id) = ira_db::get_game_config(ctx.db, ctx.db_id)
        .ok()
        .flatten()
        .unwrap_or_default();

    let mut launch = launch;
    if launch.gamemode.is_none() {
        launch.gamemode = Some(ctx.gamemode_default);
    }
    if launch.mangohud.is_none() {
        launch.mangohud = Some(ctx.mangohud_default);
    }
    if launch.gamescope.is_none() {
        launch.gamescope = Some(ctx.gamescope_default);
    }
    if launch.gamescope_w.is_none() {
        launch.gamescope_w = Some(ctx.gamescope_w_default);
    }
    if launch.gamescope_h.is_none() {
        launch.gamescope_h = Some(ctx.gamescope_h_default);
    }
    if launch.gamescope_w == Some(0) || launch.gamescope_h == Some(0) {
        let (sw, sh) = detect_screen_resolution();
        if launch.gamescope_w == Some(0) {
            launch.gamescope_w = Some(sw);
        }
        if launch.gamescope_h == Some(0) {
            launch.gamescope_h = Some(sh);
        }
    }
    if launch.gamescope_fps.is_none() {
        launch.gamescope_fps = Some(ctx.gamescope_fps_default);
    }
    if launch.gamescope_upscaling.is_none() {
        launch.gamescope_upscaling = Some(ctx.gamescope_upscaling_default.clone());
    }
    if launch.gpu.is_empty() && !ctx.gpu_default.is_empty() {
        launch.gpu = ctx.gpu_default.clone();
    }

    let overlay_enabled = launch.overlay_enabled.unwrap_or(ctx.overlay_global_enabled);

    // Determine overlay mode before applying performance wrappers.
    let will_use_gamescope = ira_launcher::env_builder::will_use_gamescope(&launch);

    eprintln!(
        "ira-overlay: overlay_enabled={} will_use_gamescope={} gamescope_cfg={:?}",
        overlay_enabled, will_use_gamescope, launch.gamescope
    );

    if overlay_enabled {
        if will_use_gamescope {
            ira_launcher::env_builder::add_overlay_env_standalone(
                &mut env,
                ctx.overlay_shm.as_deref(),
                ctx.overlay_font_family.as_deref(),
            );
        } else {
            ira_launcher::env_builder::add_overlay_env(
                &mut env,
                ctx.overlay_shm.as_deref(),
                ctx.overlay_font_family.as_deref(),
            );
        }
    }

    inject_flatpak_overlay_env(cmd, &mut env);

    ira_launcher::env_builder::apply_performance(cmd, &mut env, &launch, &wine);

    if overlay_enabled && ira_launcher::env_builder::uses_gamescope(cmd) {
        ira_launcher::env_builder::wrap_with_standalone_overlay(cmd);
    }

    let input_mode = console_input_mode(
        launch.input_mode,
        console_mode,
        ctx.controller_input_mode,
        console_profile,
    );
    let input_profile = resolve_input_profile(
        ctx,
        input_mode,
        launch.input_profile.as_deref(),
        console_profile.or(ctx.controller_input_profile.as_deref()),
    )?;
    if input_profile.is_some() {
        ira_launcher::env_builder::wrap_with_input(cmd, input_profile.as_deref())?;
        eprintln!(
            "ira-input: enabled for {}{}",
            ctx.game_name,
            input_profile
                .map(|profile| format!(" using {profile}"))
                .unwrap_or_default()
        );
    }

    Ok(env)
}

/// If the command is a Flatpak invocation, inject overlay env vars as `--env` flags
/// and add `--filesystem` for the layer directory and .so files. Flatpak sandboxes
/// don't inherit arbitrary env vars from the parent process, so `VK_LAYER_PATH` etc.
/// set via `Command::env()` never reach the sandboxed app.
///
/// `LD_PRELOAD` is stripped — Flatpak blocks it for security. The overlay still
/// works without the shim (Wayland keyboard + evdev gamepad provide input).
fn inject_flatpak_overlay_env(cmd: &mut Vec<String>, env: &mut Vec<(String, String)>) {
    let insert_pos = if cmd.len() >= 3 && cmd[0] == "flatpak" && cmd[1] == "run" {
        2
    } else if cmd.len() >= 5 && cmd[0] == "flatpak-spawn" && cmd[2] == "flatpak" && cmd[3] == "run"
    {
        4
    } else {
        return;
    };

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
    let configured_core = if !per_game_ra_core.is_empty() {
        per_game_ra_core
    } else {
        &cc.ra_core
    };
    let resolved_core = if ira_platforms::emulator_detect::is_retroarch(exe) {
        ira_platforms::emulator_detect::resolve_ra_core_for_console(platform_id, configured_core)
            .ok_or_else(|| {
                format!(
                    "No RetroArch core is installed for {platform_id}. Install one with RetroArch's Core Downloader."
                )
            })?
    } else {
        String::new()
    };
    let fullscreen_flag = ira_models::find_console(platform_id)
        .map(|d| d.fullscreen_flag)
        .unwrap_or("--fullscreen");
    let rom_path = {
        let discs = ira_db::get_discs(ctx.db, ctx.db_id).unwrap_or_default();
        let default_disc_id = ira_db::get_default_disc(ctx.db, ctx.db_id).ok().flatten();
        let raw = discs
            .iter()
            .find(|d| Some(d.id) == default_disc_id)
            .map(|d| d.rom_path.clone())
            .unwrap_or_else(|| game_path.to_string());
        if raw.is_empty() || std::path::Path::new(&raw).is_absolute() {
            raw
        } else {
            cfg.rom_folder(platform_id)
                .join(raw)
                .to_string_lossy()
                .into_owned()
        }
    };
    let rom_root = std::path::Path::new(&rom_path).parent();
    let mut cmd = ira_platforms::emulator_detect::build_launch_command_with_filesystem(
        exe,
        &rom_path,
        &resolved_core,
        cc.fullscreen,
        fullscreen_flag,
        rom_root,
    );
    let env = build_emulator_env_and_wrap(
        ctx,
        &mut cmd,
        cc.controller_mode,
        (!cc.controller_profile.is_empty()).then_some(cc.controller_profile.as_str()),
    )?;
    spawn_and_monitor(ctx, &cmd, &env, ctx.game_name)
}

pub(super) fn launch_ps4(
    ctx: &LaunchCtx,
    per_game_version: &str,
    global_shadps4_exe: &str,
    game_path: &str,
    console_mode: Option<ControllerInputMode>,
    console_profile: Option<&str>,
) -> Result<(), String> {
    let exe = ira_platforms::ps4::resolve_shadps4_executable(per_game_version, global_shadps4_exe);
    if !ira_platforms::ps4::shadps4_executable_available(&exe) {
        return Err(format!(
            "shadPS4 executable was not found: {exe}. Install shadPS4 or select an available version in Settings."
        ));
    }
    let args = vec!["-g".to_string(), game_path.to_string()];
    let mut cmd = ira_platforms::emulator_detect::build_command_with_filesystem(
        &exe,
        &args,
        Some(std::path::Path::new(game_path)),
    );
    let env = build_emulator_env_and_wrap(ctx, &mut cmd, console_mode, console_profile)?;
    spawn_and_monitor(ctx, &cmd, &env, "shadPS4")
}

pub(super) fn launch_ps3(
    ctx: &LaunchCtx,
    per_game_emu: &str,
    global_rpcs3_exe: &str,
    game_path: &str,
    console_mode: Option<ControllerInputMode>,
    console_profile: Option<&str>,
) -> Result<(), String> {
    let exe = if !per_game_emu.is_empty() {
        per_game_emu
    } else if !global_rpcs3_exe.is_empty() {
        global_rpcs3_exe
    } else {
        "rpcs3"
    };
    let args = vec!["--no-gui".to_string(), game_path.to_string()];
    let mut cmd = ira_platforms::emulator_detect::build_command_with_filesystem(
        exe,
        &args,
        Some(std::path::Path::new(game_path)),
    );
    let env = build_emulator_env_and_wrap(ctx, &mut cmd, console_mode, console_profile)?;
    spawn_and_monitor(ctx, &cmd, &env, "RPCS3")
}

pub(super) fn launch_vita3k(
    ctx: &LaunchCtx,
    global_executable: &str,
    game_path: &str,
    console_mode: Option<ControllerInputMode>,
    console_profile: Option<&str>,
) -> Result<(), String> {
    let exe = if global_executable.is_empty() {
        "vita3k"
    } else {
        global_executable
    };
    let args = vec!["-r".to_string(), game_path.to_string()];
    let mut cmd = ira_platforms::emulator_detect::build_command_with_filesystem(
        exe,
        &args,
        Some(std::path::Path::new(game_path)),
    );
    let env = build_emulator_env_and_wrap(ctx, &mut cmd, console_mode, console_profile)?;
    spawn_and_monitor(ctx, &cmd, &env, "Vita3K")
}

pub(super) fn launch_cemu(
    ctx: &LaunchCtx,
    global_executable: &str,
    game_path: &str,
    console_mode: Option<ControllerInputMode>,
    console_profile: Option<&str>,
) -> Result<(), String> {
    let exe = if global_executable.is_empty() {
        "cemu"
    } else {
        global_executable
    };
    let args = vec!["-g".to_string(), game_path.to_string()];
    let mut cmd = ira_platforms::emulator_detect::build_command_with_filesystem(
        exe,
        &args,
        Some(std::path::Path::new(game_path)),
    );
    let env = build_emulator_env_and_wrap(ctx, &mut cmd, console_mode, console_profile)?;
    spawn_and_monitor(ctx, &cmd, &env, "Cemu")
}

pub(super) fn launch_steam(ctx: &LaunchCtx, app_id: &str) -> Result<(), String> {
    let mut cmd = vec![
        "steam".to_string(),
        "-applaunch".to_string(),
        app_id.to_string(),
    ];
    let (launch, _, _) = ira_db::get_game_config(ctx.db, ctx.db_id)
        .ok()
        .flatten()
        .unwrap_or_default();
    let input_mode = resolved_input_mode(
        ira_models::GameKind::Steam,
        launch.input_mode,
        ctx.controller_input_mode,
    );
    let input_profile = resolve_input_profile(
        ctx,
        input_mode,
        launch.input_profile.as_deref(),
        ctx.controller_input_profile.as_deref(),
    )?;
    if input_backend(input_mode).is_some() {
        ira_launcher::env_builder::wrap_with_input_mode(
            &mut cmd,
            Some(input_mode),
            input_profile.as_deref(),
        )?;
        let separator = cmd
            .iter()
            .position(|argument| argument == "--")
            .expect("input wrapper must include a command separator");
        cmd.splice(
            separator..separator,
            ["--steam-app-id".to_string(), app_id.to_string()],
        );
        let env = std::env::vars().collect::<Vec<_>>();
        return spawn_and_monitor(ctx, &cmd, &env, "Steam game");
    }
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
    pc_controller_profiles: PcControllerProfiles<'_>,
    app_id: &str,
) -> Result<(), String> {
    let (mut launch, mut wine, profile_id) = ira_db::get_game_config(ctx.db, ctx.db_id)
        .ok()
        .flatten()
        .unwrap_or_default();

    if launch.gamemode.is_none() {
        launch.gamemode = Some(ctx.gamemode_default);
    }
    if launch.mangohud.is_none() {
        launch.mangohud = Some(ctx.mangohud_default);
    }
    if launch.gamescope.is_none() {
        launch.gamescope = Some(ctx.gamescope_default);
    }
    if launch.gamescope_w.is_none() {
        launch.gamescope_w = Some(ctx.gamescope_w_default);
    }
    if launch.gamescope_h.is_none() {
        launch.gamescope_h = Some(ctx.gamescope_h_default);
    }
    if launch.gamescope_w == Some(0) || launch.gamescope_h == Some(0) {
        let (sw, sh) = detect_screen_resolution();
        if launch.gamescope_w == Some(0) {
            launch.gamescope_w = Some(sw);
        }
        if launch.gamescope_h == Some(0) {
            launch.gamescope_h = Some(sh);
        }
    }
    if launch.gamescope_fps.is_none() {
        launch.gamescope_fps = Some(ctx.gamescope_fps_default);
    }
    if launch.gamescope_upscaling.is_none() {
        launch.gamescope_upscaling = Some(ctx.gamescope_upscaling_default.clone());
    }
    if launch.gpu.is_empty() && !ctx.gpu_default.is_empty() {
        launch.gpu = ctx.gpu_default.clone();
    }

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

    let (game_type_mode, game_type_profile) = match ctx.game_kind {
        ira_models::GameKind::Wine => pc_controller_profiles.wine,
        ira_models::GameKind::Linux => pc_controller_profiles.linux,
        _ => (None, ""),
    };
    let input_mode = launch
        .input_mode
        .or(game_type_mode)
        .unwrap_or_else(|| resolved_input_mode(ctx.game_kind, None, ctx.controller_input_mode));
    launch.input_profile = resolve_input_profile(
        ctx,
        input_mode,
        launch.input_profile.as_deref(),
        (!game_type_profile.is_empty())
            .then_some(game_type_profile)
            .or(ctx.controller_input_profile.as_deref()),
    )?;
    launch.input_mode = Some(input_mode);

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
                trophy_source: ctx.trophy_source,
                ufs_savefiles: ctx.ufs_savefiles.clone(),
                ufs_rootoverrides: ctx.ufs_rootoverrides.clone(),
                centralize_saves: ctx.centralize_saves,
                db: ctx.db.clone(),
                save_dir: ctx.save_dir.to_string(),
                running_games: ctx.running_games.clone(),
                overlay_enabled,
                overlay_shm: if overlay_enabled {
                    ctx.overlay_shm.clone()
                } else {
                    None
                },
                overlay_font_family: ctx.overlay_font_family.clone(),
            },
        )?;
    } else {
        return Err(format!(
            "No launch config saved for '{}'. Configure the game's launch settings first.",
            ctx.game_name
        ));
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
        if let Some(g) = state
            .borrow_mut()
            .games
            .iter_mut()
            .find(|g| g.db_id == ctx.game_id && g.variant_id == variant_id)
        {
            g.last_played = ctx.started_at;
        }
    } else if variant_count_playtime {
        if let Err(e) = ira_db::set_last_played(ctx.db, ctx.db_id, ctx.started_at) {
            eprintln!("Failed to update last played: {}", e);
        }
        if let Some(g) = state
            .borrow_mut()
            .games
            .iter_mut()
            .find(|g| g.db_id == ctx.game_id && g.variant_id.is_none())
        {
            g.last_played = ctx.started_at;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{console_input_mode, input_backend, resolved_input_mode};
    use ira_input::VirtualGamepadBackend;
    use ira_models::ControllerInputMode;

    #[test]
    fn test_input_backend_resolves_modes() {
        assert_eq!(input_backend(ControllerInputMode::Disabled), None);
        assert_eq!(
            input_backend(ControllerInputMode::VirtualXInput),
            Some(VirtualGamepadBackend::XInput)
        );
        assert_eq!(
            input_backend(ControllerInputMode::VirtualDirectInput),
            Some(VirtualGamepadBackend::DirectInput)
        );
    }

    #[test]
    fn test_console_input_mode_prioritizes_game_and_console_overrides() {
        assert_eq!(
            console_input_mode(
                None,
                None,
                ControllerInputMode::Disabled,
                Some("/layouts/ps1.json"),
            ),
            ControllerInputMode::VirtualXInput
        );
        assert_eq!(
            console_input_mode(
                Some(ControllerInputMode::Disabled),
                Some(ControllerInputMode::VirtualXInput),
                ControllerInputMode::VirtualXInput,
                Some("/layouts/ps1.json"),
            ),
            ControllerInputMode::Disabled
        );
        assert_eq!(
            console_input_mode(
                None,
                Some(ControllerInputMode::Disabled),
                ControllerInputMode::VirtualXInput,
                Some("/layouts/ps1.json"),
            ),
            ControllerInputMode::Disabled
        );
    }

    #[test]
    fn test_pc_input_mode_prefers_game_type_override() {
        let game_mode = Some(ControllerInputMode::VirtualDirectInput);
        let game_type_mode = Some(ControllerInputMode::Disabled);
        let mode = game_mode.or(game_type_mode).unwrap_or_else(|| {
            resolved_input_mode(
                ira_models::GameKind::Linux,
                None,
                ControllerInputMode::VirtualXInput,
            )
        });
        assert_eq!(mode, ControllerInputMode::VirtualDirectInput);
    }

    #[test]
    fn test_steam_inheritance_keeps_steam_input_authoritative() {
        assert_eq!(
            resolved_input_mode(
                ira_models::GameKind::Steam,
                None,
                ControllerInputMode::VirtualXInput,
            ),
            ControllerInputMode::Disabled
        );
        assert_eq!(
            resolved_input_mode(
                ira_models::GameKind::Steam,
                Some(ControllerInputMode::VirtualDirectInput),
                ControllerInputMode::VirtualXInput,
            ),
            ControllerInputMode::VirtualDirectInput
        );
        assert_eq!(
            resolved_input_mode(
                ira_models::GameKind::Ps4,
                None,
                ControllerInputMode::VirtualXInput,
            ),
            ControllerInputMode::VirtualXInput
        );
    }
}
