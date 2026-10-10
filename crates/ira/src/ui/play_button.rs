use crate::AppMessage;
use gtk4::prelude::*;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use super::css::*;
use super::input_profile_store::read_profile;
use super::play_button_helpers;
use super::state::SharedState;
use ira_models::ControllerInputMode;

const PLAY_BTN_HEIGHT: i32 = 48;
const PLAY_BTN_H_MARGIN: i32 = 16;
const PLAY_BTN_ICON_SIZE: i32 = 20;
const PLAY_BTN_LABEL_WIDTH: i32 = 5;

struct LaunchGameInfo {
    game: ira_models::Game,
    per_game_version: String,
    per_game_ra_core: String,
    per_game_emu: String,
    platform_id: String,
}

struct LaunchRequest {
    running_games: Arc<Mutex<HashMap<i64, i32>>>,
    launching_games: Arc<Mutex<HashSet<i64>>>,
    sender: ira_models::AppSender,
    db: ira_db::DbConn,
    save_dir: String,
    cfg: ira_config::Config,
    global_shadps4_exe: String,
    global_rpcs3_exe: String,
    global_vita3k_exe: String,
    global_cemu_exe: String,
    global_azahar_exe: String,
    overlay_global_enabled: bool,
    overlay_font_family: Option<String>,
    system_defaults: ira_config::SystemDefaults,
    controller_registry: Arc<ira_input::ControllerRegistry>,
    app_default_wine: ira_models::WineConfig,
    default_native_env_vars: Vec<(String, String)>,
    screen_resolution: (u32, u32),
    playtime_members: Vec<i64>,
    variant_id: Option<i64>,
    disc_override: Option<i64>,
    in_big_picture: bool,
    game_info: LaunchGameInfo,
}

struct LaunchingGameGuard {
    launching_games: Arc<Mutex<HashSet<i64>>>,
    db_id: i64,
}

impl Drop for LaunchingGameGuard {
    fn drop(&mut self) {
        match self.launching_games.lock() {
            Ok(mut games) => {
                games.remove(&self.db_id);
            }
            Err(error) => eprintln!("Failed to clear launching game state: {error}"),
        }
    }
}

struct LaunchProcessArgs<'a, 'b> {
    ctx: &'a play_button_helpers::LaunchCtx<'b>,
    cfg: &'a ira_config::Config,
    kind: ira_models::GameKind,
    platform_id: &'a str,
    game_path: &'a str,
    per_game_version: &'a str,
    per_game_emu: &'a str,
    per_game_ra_core: &'a str,
    global_shadps4_exe: &'a str,
    global_rpcs3_exe: &'a str,
    global_vita3k_exe: &'a str,
    global_cemu_exe: &'a str,
    global_azahar_exe: &'a str,
    in_big_picture: bool,
    variant_id: Option<i64>,
    variant_count_playtime: bool,
    app_default_wine: &'a ira_models::WineConfig,
    default_native_env_vars: &'a [(String, String)],
    app_id: &'a str,
}

fn build_play_btn_hbox() -> gtk4::Box {
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_valign(gtk4::Align::Center);
    hbox.set_halign(gtk4::Align::Center);
    hbox.set_margin_start(PLAY_BTN_H_MARGIN);
    hbox.set_margin_end(PLAY_BTN_H_MARGIN);

    let icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
    icon.set_pixel_size(PLAY_BTN_ICON_SIZE);
    hbox.append(&icon);

    let label = gtk4::Label::new(Some(&crate::tr!("Play")));
    label.add_css_class(CSS_PLAY_BTN_LABEL);
    label.set_width_chars(PLAY_BTN_LABEL_WIDTH);
    hbox.append(&label);

    hbox
}

fn set_running_state(
    icon: &gtk4::Image,
    label: &gtk4::Label,
    btn: &impl IsA<gtk4::Widget>,
    running: bool,
) {
    if running {
        btn.remove_css_class(CSS_SUGGESTED_ACTION);
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text(&crate::tr!("Stop"));
    } else {
        icon.set_icon_name(Some("media-playback-start-symbolic"));
        label.set_text(&crate::tr!("Play"));
        btn.add_css_class(CSS_SUGGESTED_ACTION);
    }
}

/// The icon and label of a hbox built by [`build_play_btn_hbox`]. All
/// three button shapes share the lookup — first child is the icon, last
/// the label — so it lives here instead of thrice inline.
fn play_btn_icon_label(hbox: &gtk4::Box) -> (gtk4::Image, gtk4::Label) {
    let icon = hbox
        .first_child()
        .and_then(|c| c.downcast::<gtk4::Image>().ok())
        .unwrap();
    let label = hbox
        .last_child()
        .and_then(|c| c.downcast::<gtk4::Label>().ok())
        .unwrap();
    (icon, label)
}

/// The click shared by every play-button shape: stop a running game,
/// else launch (with `variant_id` for the variant shape, `None`
/// otherwise) and repaint the button to the outcome.
fn toggle_play_clicked(
    state: &SharedState,
    db_id: i64,
    variant_id: Option<i64>,
    sender: &ira_models::AppSender,
    icon: &gtk4::Image,
    label: &gtk4::Label,
    btn: &impl IsA<gtk4::Widget>,
) {
    if is_game_running(state, db_id) {
        stop_game(state, db_id);
        set_running_state(icon, label, btn, false);
    } else {
        match launch_game(state, db_id, variant_id) {
            Ok(true) => {
                set_running_state(icon, label, btn, true);
                btn.set_sensitive(false);
            }
            Ok(false) => {}
            Err(e) => {
                eprintln!("Failed to launch game: {}", e);
                let _ = sender.send(AppMessage::GameLaunchFailed { db_id, error: e });
            }
        }
    }
}

fn is_game_running(state: &SharedState, db_id: i64) -> bool {
    state
        .borrow()
        .running_games
        .lock()
        .map(|m| m.contains_key(&db_id))
        .unwrap_or(false)
}

fn is_game_launching(state: &SharedState, db_id: i64) -> bool {
    state
        .borrow()
        .launching_games
        .lock()
        .map(|games| games.contains(&db_id))
        .unwrap_or(false)
}

fn sorted_controller_snapshot(
    mut devices: Vec<ira_input::DeviceInfo>,
) -> Vec<ira_input::DeviceInfo> {
    devices.sort_unstable_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.vendor.cmp(&right.vendor))
            .then_with(|| left.product.cmp(&right.product))
            .then_with(|| left.version.cmp(&right.version))
            .then_with(|| left.name.cmp(&right.name))
    });
    devices
}

pub(super) fn active_controller_input(
    cfg: &ira_config::Config,
    save_dir: &str,
    controller_registry: &ira_input::ControllerRegistry,
) -> (ControllerInputMode, Option<String>) {
    sorted_controller_snapshot(controller_registry.snapshot())
        .into_iter()
        .find_map(|device| {
            let key = ira_config::Config::controller_key(device.vendor, device.product);
            let defaults = cfg.controller_defaults.get(&key)?;
            if defaults.mode == ControllerInputMode::Disabled {
                return None;
            }
            let configured = std::path::PathBuf::from(&defaults.profile);
            let path = if configured.is_file() && read_profile(&configured).is_ok() {
                configured
            } else {
                super::input_profile_store::find_controller_default_profile(save_dir, &key)?
            };
            let profile = path.is_file().then(|| path.to_string_lossy().into_owned());
            Some((defaults.mode, profile))
        })
        .unwrap_or((ControllerInputMode::Disabled, None))
}

pub fn stop_game(state: &SharedState, game_id: i64) {
    // A Steam game is spawned and owned by the Steam client, so its own
    // steam://stop is the only clean stop — killing the process tree would
    // reach into Steam's runtime. Dropping the registry entry unmarks the
    // game right away, and the launch supervisor reports the session over
    // once it notices (or the game's processes are gone).
    let steam_app_id = {
        let s = state.borrow();
        s.games
            .iter()
            .find(|g| g.db_id == game_id)
            .filter(|g| g.kind == ira_models::GameKind::Steam)
            .map(|g| g.steam_api_id().to_string())
    };
    if let Some(app_id) = steam_app_id {
        state
            .borrow()
            .running_games
            .lock()
            .unwrap()
            .remove(&game_id);
        ira_input::steam::request_steam_stop(&app_id);
        return;
    }
    let running_games = state.borrow().running_games.clone();
    let pid = running_games.lock().unwrap().get(&game_id).copied();
    if let Some(pid) = pid {
        if pid <= 0 {
            if let Err(error) = ira_launcher::input_daemon::stop_session(game_id) {
                eprintln!("stop: failed to cancel game {}: {}", game_id, error);
                return;
            }
            running_games.lock().unwrap().remove(&game_id);
            return;
        }
        running_games.lock().unwrap().remove(&game_id);
        let (wine_exe, wine_prefix, env) = {
            let s = state.borrow();
            let game = s.games.iter().find(|g| g.db_id == game_id);
            let db_id = game.map(|g| g.db_id).unwrap_or(0);
            let config = super::helpers::logged_game_config(&s.db, db_id);
            let app_default = s.cfg.default_wine_config.clone();
            let (exe, prefix, env_vars) = if let Some((_, mut wine, _)) = config {
                wine = wine.merge_with_default(&app_default);
                if game
                    .map(|g| g.kind == ira_models::GameKind::Wine)
                    .unwrap_or(false)
                    && wine.enabled
                {
                    let exe = ira_launcher::wine_launch::find_wine_binary(
                        &wine.version,
                        &wine.custom_wine_path,
                    )
                    .ok();
                    let prefix = ira_launcher::wine_launch::wine_prefix(&wine);
                    let env = ira_launcher::wine_launch::build_wine_env(
                        &wine,
                        exe.as_deref().unwrap_or(""),
                    );
                    (exe, Some(prefix), env)
                } else {
                    (None, None, Vec::new())
                }
            } else {
                (None, None, Vec::new())
            };
            (exe, prefix, env_vars)
        };
        ira_launcher::wrapper::stop_game(pid, wine_exe.as_deref(), wine_prefix.as_deref(), &env);
    }
}

pub fn launch_game(
    state: &SharedState,
    game_id: i64,
    variant_id: Option<i64>,
) -> Result<bool, String> {
    if maybe_show_disc_picker(state, game_id, variant_id) {
        return Ok(false);
    }
    launch_game_with_disc(state, game_id, variant_id, None)
}

/// Boot one disc of a multi-disc game: the picker calls this after the
/// player chooses, so it never re-opens the picker itself.
pub fn launch_game_disc(
    state: &SharedState,
    game_id: i64,
    variant_id: Option<i64>,
    disc_id: i64,
) -> Result<bool, String> {
    launch_game_with_disc(state, game_id, variant_id, Some(disc_id))
}

/// Show the disc picker for a multi-disc game on a single-disc emulator
/// and report whether one was shown. Dolphin boots every disc at once,
/// so its games never stop here.
fn maybe_show_disc_picker(state: &SharedState, game_id: i64, variant_id: Option<i64>) -> bool {
    let (db, cfg, kind, platform_id, per_game_emu, game_name) = {
        let s = state.borrow();
        let game = s.games.iter().find(|g| g.db_id == game_id);
        let Some(game) = game else {
            return false;
        };
        (
            s.db.clone(),
            s.cfg.clone(),
            game.kind,
            game.platform_id.id().to_string(),
            game.emulator_override.clone(),
            game.name.clone(),
        )
    };
    if !matches!(
        kind,
        ira_models::GameKind::Retro | ira_models::GameKind::Switch
    ) {
        return false;
    }
    let discs = ira_db::get_discs(&db, game_id).unwrap_or_default();
    if discs.len() <= 1 {
        return false;
    }
    let exe = play_button_helpers::retro_exe(&cfg, &platform_id, &per_game_emu);
    if !play_button_helpers::needs_disc_picker(exe, discs.len()) {
        return false;
    }
    if super::big_picture::is_big_picture() {
        super::big_picture::show_disc_picker(state, game_id, variant_id);
    } else {
        super::disc_picker::present(state, game_id, variant_id, &game_name, &discs);
    }
    true
}

fn launch_game_with_disc(
    state: &SharedState,
    game_id: i64,
    variant_id: Option<i64>,
    disc_override: Option<i64>,
) -> Result<bool, String> {
    if is_game_running(state, game_id) || is_game_launching(state, game_id) {
        return Ok(false);
    }
    let request = snapshot_launch_request(state, game_id, variant_id, disc_override)?;
    let game_kind = request.game_info.game.kind;
    let exes = play_button_helpers::EmulatorExes {
        platform_id: &request.game_info.platform_id,
        per_game_version: &request.game_info.per_game_version,
        per_game_emu: &request.game_info.per_game_emu,
        shadps4: &request.global_shadps4_exe,
        rpcs3: &request.global_rpcs3_exe,
        vita3k: &request.global_vita3k_exe,
        cemu: &request.global_cemu_exe,
        azahar: &request.global_azahar_exe,
    };
    if let Some(page_id) =
        play_button_helpers::missing_emulator_page(&request.cfg, &exes, game_kind)
    {
        let _ = request.sender.send(crate::AppMessage::EmulatorMissing {
            game_name: request.game_info.game.name.clone(),
            page_id,
        });
        return Ok(false);
    }
    let launching_games = request.launching_games.clone();
    if !launching_games.lock().unwrap().insert(game_id) {
        return Ok(false);
    }
    let sender = request.sender.clone();
    std::thread::spawn(move || {
        if let Err(error) = launch_game_on_worker(request) {
            let _ = sender.send(crate::AppMessage::GameLaunchFailed {
                db_id: game_id,
                error,
            });
        }
    });
    Ok(true)
}

fn snapshot_launch_request(
    state: &SharedState,
    game_id: i64,
    variant_id: Option<i64>,
    disc_override: Option<i64>,
) -> Result<LaunchRequest, String> {
    let screen_resolution = play_button_helpers::detect_screen_resolution();
    let in_big_picture = super::big_picture::is_big_picture();
    let s = state.borrow();
    let game = s
        .games
        .iter()
        .find(|game| game.db_id == game_id)
        .cloned()
        .ok_or_else(|| crate::tr!("Game not found"))?;
    let source_id = game.overlay_source_id();
    let mut system_defaults = s.cfg.default_system.clone();
    system_defaults.gamescope = source_id
        .and_then(|id| s.cfg.overlay.source_gamescope.get(id).copied())
        .unwrap_or(system_defaults.gamescope);
    let playtime_members = s
        .playtime_links
        .get(&game.db_id)
        .cloned()
        .unwrap_or_else(|| vec![game.db_id]);
    Ok(LaunchRequest {
        running_games: s.running_games.clone(),
        launching_games: s.launching_games.clone(),
        sender: s.sender.clone(),
        db: s.db.clone(),
        save_dir: s.save_dir.clone(),
        cfg: s.cfg.clone(),
        global_shadps4_exe: s.cfg.shadps4_executable.clone(),
        global_rpcs3_exe: s.cfg.rpcs3_executable.clone(),
        global_vita3k_exe: s.cfg.vita3k_executable.clone(),
        global_cemu_exe: s.cfg.cemu_executable.clone(),
        global_azahar_exe: s.cfg.azahar_executable.clone(),
        overlay_global_enabled: source_id
            .map_or(s.cfg.overlay.enabled, |id| s.cfg.overlay.source_enabled(id)),
        overlay_font_family: s.cfg.overlay.font_family.clone(),
        system_defaults,
        controller_registry: s.controller_registry.clone(),
        app_default_wine: s.cfg.default_wine_config.clone(),
        default_native_env_vars: s.cfg.default_native_env_vars.clone(),
        screen_resolution,
        playtime_members,
        variant_id,
        disc_override,
        in_big_picture,
        game_info: LaunchGameInfo {
            per_game_version: game.shadps4_version.clone(),
            per_game_ra_core: game.ra_core.clone(),
            per_game_emu: game.emulator_override.clone(),
            platform_id: game.platform_id.id().to_string(),
            game,
        },
    })
}

fn launch_game_on_worker(request: LaunchRequest) -> Result<(), String> {
    let LaunchRequest {
        running_games,
        launching_games,
        sender,
        db,
        save_dir,
        cfg: cfg_clone,
        global_shadps4_exe,
        global_rpcs3_exe,
        global_vita3k_exe,
        global_cemu_exe,
        global_azahar_exe,
        overlay_global_enabled,
        overlay_font_family,
        system_defaults,
        controller_registry,
        app_default_wine,
        default_native_env_vars,
        screen_resolution,
        playtime_members,
        variant_id,
        disc_override,
        in_big_picture,
        game_info,
        ..
    } = request;
    let game = game_info.game;
    let kind = game.kind;
    let game_path = game.game_path.clone();
    let game_name = game.name.clone();
    let db_id = game.db_id;
    let _launching_guard = LaunchingGameGuard {
        launching_games,
        db_id,
    };
    let app_id = game.steam_api_id().to_string();
    let platform_id = game_info.platform_id;
    let per_game_version = game_info.per_game_version;
    let per_game_ra_core = game_info.per_game_ra_core;
    let per_game_emu = game_info.per_game_emu;
    let trophy_source = game.trophy_source;
    let launch = ira_db::get_game_config(&db, db_id)
        .ok()
        .flatten()
        .map(|(launch, _, _)| launch)
        .unwrap_or_default();
    let shm_game = ira_db::find_by_db_id(&db, db_id)
        .ok()
        .flatten()
        .and_then(|entry| crate::game_loader::load_game(&entry, &save_dir).ok())
        .unwrap_or_else(|| game.clone());
    let overlay_shm = crate::overlay::write_game_shm(
        &shm_game,
        launch_display_playtime(&db, &game, &playtime_members),
        &cfg_clone.overlay,
        launch.overlay_encoder,
        launch.overlay_recording_quality,
    );

    let (ufs_savefiles, ufs_rootoverrides) =
        crate::game_loader::read_app_details(&save_dir, &app_id)
            .map(|d| (d.ufs_savefiles, d.ufs_rootoverrides))
            .unwrap_or_default();

    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let variant_info = variant_id
        .and_then(|vid| {
            ira_db::get_variants(&db, db_id)
                .ok()?
                .into_iter()
                .find(|v| v.id == vid)
        })
        .map(|v| (v.show_as_entry, v.count_playtime));
    let (variant_show_as_entry, variant_count_playtime) = variant_info.unwrap_or((false, true));
    let (controller_input_mode, controller_input_profile) =
        active_controller_input(&cfg_clone, &save_dir, &controller_registry);

    let pause_platform = match kind {
        ira_models::GameKind::Wine => cfg_clone.wine_pause_game_on_overlay,
        ira_models::GameKind::Linux => cfg_clone.linux_pause_game_on_overlay,
        _ => cfg_clone.console(&platform_id).pause_game_on_overlay,
    };
    let pause_global = cfg_clone.pause_game_on_overlay;

    let ctx = play_button_helpers::LaunchCtx {
        db: &db,
        save_dir: &save_dir,
        game_id: db_id,
        db_id,
        game_name: &game_name,
        game_kind: kind,
        trophy_source,
        ufs_savefiles,
        ufs_rootoverrides,
        centralize_saves: cfg_clone.centralize_game_saves,
        sender: &sender,
        running_games: &running_games,
        started_at,
        overlay_shm,
        overlay_global_enabled,
        overlay_font_family,
        pause_platform,
        pause_global,
        system_defaults,
        controller_input_mode,
        controller_input_profile,
        screen_resolution,
        disc_override,
    };

    let launched = launch_process(LaunchProcessArgs {
        ctx: &ctx,
        cfg: &cfg_clone,
        kind,
        platform_id: &platform_id,
        game_path: &game_path,
        per_game_version: &per_game_version,
        per_game_emu: &per_game_emu,
        per_game_ra_core: &per_game_ra_core,
        global_shadps4_exe: &global_shadps4_exe,
        global_rpcs3_exe: &global_rpcs3_exe,
        global_vita3k_exe: &global_vita3k_exe,
        global_cemu_exe: &global_cemu_exe,
        global_azahar_exe: &global_azahar_exe,
        in_big_picture,
        variant_id,
        variant_count_playtime,
        app_default_wine: &app_default_wine,
        default_native_env_vars: &default_native_env_vars,
        app_id: &app_id,
    })?;
    if !launched {
        return Err(crate::tr!("Game launch did not start").to_string());
    }
    if kind == ira_models::GameKind::Steam {
        let _ = sender.send(crate::AppMessage::GameStarted(db_id, variant_id));
        return Ok(());
    }

    if let Some((variant_id, last_played)) = play_button_helpers::update_last_played(
        &ctx,
        variant_id,
        variant_count_playtime,
        variant_show_as_entry,
    ) {
        let _ = sender.send(crate::AppMessage::GameLastPlayedUpdated {
            game_id: db_id,
            variant_id,
            started_at: last_played,
        });
    }

    // The sidebar playing style is driven by GameStarted, and launch_game is
    // the single launch path (sidebar click, context menu and play buttons),
    // so report the start here to keep every entry point in sync.
    let _ = sender.send(crate::AppMessage::GameStarted(db_id, variant_id));

    Ok(())
}

fn launch_process(args: LaunchProcessArgs<'_, '_>) -> Result<bool, String> {
    let LaunchProcessArgs {
        ctx,
        cfg,
        kind,
        platform_id,
        game_path,
        per_game_version,
        per_game_emu,
        per_game_ra_core,
        global_shadps4_exe,
        global_rpcs3_exe,
        global_vita3k_exe,
        global_cemu_exe,
        global_azahar_exe,
        in_big_picture,
        variant_id,
        variant_count_playtime,
        app_default_wine,
        default_native_env_vars,
        app_id,
    } = args;
    if matches!(
        kind,
        ira_models::GameKind::Retro | ira_models::GameKind::Switch
    ) {
        play_button_helpers::launch_retro(
            ctx,
            cfg,
            platform_id,
            per_game_emu,
            per_game_ra_core,
            game_path,
        )?;
    } else if kind == ira_models::GameKind::Ps4 {
        play_button_helpers::launch_ps4(
            ctx,
            per_game_version,
            global_shadps4_exe,
            game_path,
            cfg.console_fullscreen("ps4", in_big_picture),
            play_button_helpers::ConsoleInput {
                mode: cfg.console("ps4").controller_mode,
                profile: (!cfg.console("ps4").controller_profile.is_empty())
                    .then_some(cfg.console("ps4").controller_profile.as_str()),
                pause_unfocused: cfg.console("ps4").input_pause_unfocused,
            },
        )?;
    } else if kind == ira_models::GameKind::Ps3 {
        play_button_helpers::launch_ps3(
            ctx,
            per_game_emu,
            global_rpcs3_exe,
            game_path,
            cfg.console_fullscreen("ps3", in_big_picture),
            play_button_helpers::ConsoleInput {
                mode: cfg.console("ps3").controller_mode,
                profile: (!cfg.console("ps3").controller_profile.is_empty())
                    .then_some(cfg.console("ps3").controller_profile.as_str()),
                pause_unfocused: cfg.console("ps3").input_pause_unfocused,
            },
        )?;
    } else if kind == ira_models::GameKind::PsVita {
        play_button_helpers::launch_vita3k(
            ctx,
            global_vita3k_exe,
            game_path,
            cfg.console_fullscreen("psvita", in_big_picture),
            play_button_helpers::ConsoleInput {
                mode: cfg.console("psvita").controller_mode,
                profile: (!cfg.console("psvita").controller_profile.is_empty())
                    .then_some(cfg.console("psvita").controller_profile.as_str()),
                pause_unfocused: cfg.console("psvita").input_pause_unfocused,
            },
        )?;
    } else if kind == ira_models::GameKind::WiiU {
        play_button_helpers::launch_cemu(
            ctx,
            per_game_emu,
            global_cemu_exe,
            game_path,
            cfg.console_fullscreen("wiiu", in_big_picture),
            play_button_helpers::ConsoleInput {
                mode: cfg.console("wiiu").controller_mode,
                profile: (!cfg.console("wiiu").controller_profile.is_empty())
                    .then_some(cfg.console("wiiu").controller_profile.as_str()),
                pause_unfocused: cfg.console("wiiu").input_pause_unfocused,
            },
        )?;
    } else if kind == ira_models::GameKind::ThreeDS {
        play_button_helpers::launch_azahar(
            ctx,
            per_game_emu,
            global_azahar_exe,
            game_path,
            cfg.console_fullscreen("3ds", in_big_picture),
            play_button_helpers::ConsoleInput {
                mode: cfg.console("3ds").controller_mode,
                profile: (!cfg.console("3ds").controller_profile.is_empty())
                    .then_some(cfg.console("3ds").controller_profile.as_str()),
                pause_unfocused: cfg.console("3ds").input_pause_unfocused,
            },
        )?;
    } else if kind == ira_models::GameKind::Steam {
        return play_button_helpers::launch_steam(ctx, app_id);
    } else {
        play_button_helpers::launch_other(
            ctx,
            app_default_wine,
            variant_id,
            variant_count_playtime,
            default_native_env_vars,
            play_button_helpers::PcControllerProfiles {
                linux: (
                    cfg.linux_controller_mode,
                    cfg.linux_controller_profile.as_str(),
                ),
                wine: (
                    cfg.wine_controller_mode,
                    cfg.wine_controller_profile.as_str(),
                ),
            },
            app_id,
        )?;
    }
    Ok(true)
}

fn launch_display_playtime(db: &ira_db::DbConn, game: &ira_models::Game, members: &[i64]) -> f64 {
    if members.len() <= 1 {
        return game.playtime;
    }
    members
        .iter()
        .filter_map(|id| {
            ira_db::find_by_db_id(db, *id).unwrap_or_else(|error| {
                eprintln!("Failed to read linked game {id}: {error}");
                None
            })
        })
        .map(|entry| entry.playtime)
        .sum()
}

/// Opens this game's configured emulator with no game loaded (the
/// "Open emulator without game" menu entries). The process is detached:
/// no playtime is recorded and no play state changes; failures are reported
/// through the application message channel.
pub fn open_emulator_no_game(state: &SharedState, db_id: i64) -> Result<(), String> {
    if is_game_running(state, db_id) || is_game_launching(state, db_id) {
        return Err(crate::tr!(
            "Stop the running game before opening its emulator"
        ));
    }

    let request = snapshot_launch_request(state, db_id, None, None)?;
    let game_kind = request.game_info.game.kind;
    if !game_kind.has_standalone_emulator() {
        return Err(crate::tr!(
            "'{}' does not run through an emulator that can be opened on its own"
        )
        .replacen("{}", &request.game_info.game.name, 1));
    }
    let exes = play_button_helpers::EmulatorExes {
        platform_id: &request.game_info.platform_id,
        per_game_version: &request.game_info.per_game_version,
        per_game_emu: &request.game_info.per_game_emu,
        shadps4: &request.global_shadps4_exe,
        rpcs3: &request.global_rpcs3_exe,
        vita3k: &request.global_vita3k_exe,
        cemu: &request.global_cemu_exe,
        azahar: &request.global_azahar_exe,
    };
    if let Some(page_id) =
        play_button_helpers::missing_emulator_page(&request.cfg, &exes, game_kind)
    {
        let _ = request.sender.send(crate::AppMessage::EmulatorMissing {
            game_name: request.game_info.game.name.clone(),
            page_id,
        });
        return Ok(());
    }

    let launching_games = request.launching_games.clone();
    if !launching_games.lock().unwrap().insert(db_id) {
        return Ok(());
    }
    let sender = request.sender.clone();
    std::thread::spawn(move || {
        let result = launch_emulator_no_game_on_worker(request);
        match result {
            Ok(()) => {
                let _ = sender.send(crate::AppMessage::EmulatorOpened(db_id));
            }
            Err(error) => {
                let _ = sender.send(crate::AppMessage::EmulatorOpenFailed { db_id, error });
            }
        }
    });
    Ok(())
}

fn launch_emulator_no_game_on_worker(request: LaunchRequest) -> Result<(), String> {
    let LaunchRequest {
        running_games,
        launching_games,
        sender,
        db,
        save_dir,
        cfg,
        global_shadps4_exe,
        global_rpcs3_exe,
        global_vita3k_exe,
        global_cemu_exe,
        global_azahar_exe,
        overlay_global_enabled,
        overlay_font_family,
        system_defaults,
        controller_registry,
        screen_resolution,
        playtime_members,
        game_info,
        ..
    } = request;
    let game = game_info.game;
    let db_id = game.db_id;
    let _launching_guard = LaunchingGameGuard {
        launching_games,
        db_id,
    };
    let game_name = game.name.clone();
    let platform_id = game_info.platform_id;
    let per_game_version = game_info.per_game_version;
    let per_game_emu = game_info.per_game_emu;
    let launch = ira_db::get_game_config(&db, db_id)
        .ok()
        .flatten()
        .map(|(launch, _, _)| launch)
        .unwrap_or_default();
    let shm_game = ira_db::find_by_db_id(&db, db_id)
        .ok()
        .flatten()
        .and_then(|entry| crate::game_loader::load_game(&entry, &save_dir).ok())
        .unwrap_or_else(|| game.clone());
    let overlay_shm = crate::overlay::write_game_shm(
        &shm_game,
        launch_display_playtime(&db, &game, &playtime_members),
        &cfg.overlay,
        launch.overlay_encoder,
        launch.overlay_recording_quality,
    );
    let (controller_input_mode, controller_input_profile) =
        active_controller_input(&cfg, &save_dir, &controller_registry);
    let ctx = play_button_helpers::LaunchCtx {
        db: &db,
        save_dir: &save_dir,
        game_id: db_id,
        db_id,
        game_name: &game_name,
        game_kind: game.kind,
        trophy_source: game.trophy_source,
        ufs_savefiles: Vec::new(),
        ufs_rootoverrides: Vec::new(),
        centralize_saves: false,
        sender: &sender,
        running_games: &running_games,
        started_at: 0,
        overlay_shm,
        overlay_global_enabled,
        overlay_font_family,
        pause_platform: None,
        pause_global: false,
        system_defaults,
        controller_input_mode,
        controller_input_profile,
        screen_resolution,
        disc_override: None,
    };
    play_button_helpers::launch_emulator_no_game(
        &ctx,
        &cfg,
        &play_button_helpers::EmulatorExes {
            platform_id: &platform_id,
            per_game_version: &per_game_version,
            per_game_emu: &per_game_emu,
            shadps4: &global_shadps4_exe,
            rpcs3: &global_rpcs3_exe,
            vita3k: &global_vita3k_exe,
            cemu: &global_cemu_exe,
            azahar: &global_azahar_exe,
        },
    )
}

pub fn play_button(state: &SharedState, db_id: i64, variant_id: Option<i64>) -> gtk4::Widget {
    let (sender, db) = {
        let s = state.borrow();
        (s.sender.clone(), s.db.clone())
    };

    let variants = ira_db::get_variants(&db, db_id).unwrap_or_else(|e| {
        eprintln!("Failed to read variants for db_id {}: {}", db_id, e);
        Vec::new()
    });
    let discs = ira_db::get_discs(&db, db_id).unwrap_or_else(|e| {
        eprintln!("Failed to read discs for db_id {}: {}", db_id, e);
        Vec::new()
    });
    let has_variants = !variants.is_empty();
    let has_discs = !discs.is_empty();

    let is_running = is_game_running(state, db_id);
    let is_launching = is_game_launching(state, db_id);
    let is_active = is_running || is_launching;

    let button = if !has_variants && !has_discs {
        build_simple_play_button(state, db_id, &sender, is_active)
    } else if has_discs {
        build_disc_play_button(state, db_id, &discs, &sender, is_active)
    } else {
        build_variant_play_button(state, db_id, &variants, variant_id, &sender, is_active)
    };
    if is_launching {
        button.set_sensitive(false);
    }
    button
}

fn build_simple_play_button(
    state: &SharedState,
    db_id: i64,
    sender: &ira_models::AppSender,
    is_active: bool,
) -> gtk4::Widget {
    let btn = gtk4::Button::new();
    btn.set_valign(gtk4::Align::Center);
    btn.set_height_request(PLAY_BTN_HEIGHT);

    let hbox = build_play_btn_hbox();
    let (icon, label) = play_btn_icon_label(&hbox);

    btn.set_child(Some(&hbox));

    set_running_state(&icon, &label, &btn, is_active);

    let icon_click = icon;
    let label_click = label;
    let st = state.clone();
    let sender_c = sender.clone();
    btn.connect_clicked(move |btn| {
        toggle_play_clicked(&st, db_id, None, &sender_c, &icon_click, &label_click, btn);
    });

    btn.upcast()
}

fn build_disc_play_button(
    state: &SharedState,
    db_id: i64,
    discs: &[ira_models::GameDisc],
    sender: &ira_models::AppSender,
    is_active: bool,
) -> gtk4::Widget {
    let split = adw::SplitButton::new();

    let hbox = build_play_btn_hbox();
    let (icon, label) = play_btn_icon_label(&hbox);

    split.set_child(Some(&hbox));
    split.set_height_request(PLAY_BTN_HEIGHT);
    split.set_valign(gtk4::Align::Center);
    split.set_dropdown_tooltip(&crate::tr!("Select disc"));

    set_running_state(&icon, &label, &split, is_active);

    let default_did = ira_db::get_default_disc(&state.borrow().db, db_id)
        .ok()
        .flatten();
    let default_target = match default_did {
        Some(did) => format!("{}", did),
        None => "0".to_string(),
    };

    let actions = gio::SimpleActionGroup::new();
    let action = gio::SimpleAction::new_stateful(
        "disc",
        Some(glib::VariantTy::STRING),
        &glib::Variant::from(&default_target),
    );

    let st_c = state.clone();
    action.connect_activate(move |action, param| {
        if let Some(param) = param {
            let target_str = param.get::<String>().unwrap_or_default();
            let did = target_str.parse::<i64>().ok();
            if let Err(e) = ira_db::set_default_disc(&st_c.borrow().db, db_id, did) {
                eprintln!("Failed to set default disc: {e}");
            }
            action.change_state(param);
        }
    });
    actions.add_action(&action);

    let menu = gio::Menu::new();
    for disc in discs {
        let name = if disc.label.is_empty() {
            crate::tr!("Disc {}").replacen("{}", &disc.disc_number.to_string(), 1)
        } else {
            disc.label.clone()
        };
        menu.append(Some(&name), Some(&format!("play.disc::{}", disc.id)));
    }
    add_open_emulator_menu_item(state, db_id, &actions, &menu, sender);

    split.insert_action_group("play", Some(&actions));
    split.set_menu_model(Some(&menu));

    let icon_click = icon;
    let label_click = label;
    let st_launch = state.clone();
    let sender_c = sender.clone();
    split.connect_clicked(move |btn| {
        toggle_play_clicked(
            &st_launch,
            db_id,
            None,
            &sender_c,
            &icon_click,
            &label_click,
            btn,
        );
    });

    split.upcast()
}

fn build_variant_play_button(
    state: &SharedState,
    db_id: i64,
    variants: &[ira_models::GameVariant],
    variant_id: Option<i64>,
    sender: &ira_models::AppSender,
    is_active: bool,
) -> gtk4::Widget {
    let split = adw::SplitButton::new();

    let hbox = build_play_btn_hbox();
    let (icon, label) = play_btn_icon_label(&hbox);

    split.set_child(Some(&hbox));
    split.set_height_request(PLAY_BTN_HEIGHT);
    split.set_valign(gtk4::Align::Center);
    split.set_dropdown_tooltip(&crate::tr!("Select variant"));

    set_running_state(&icon, &label, &split, is_active);

    let default_vid = variant_id.or_else(|| {
        ira_db::get_default_variant(&state.borrow().db, db_id)
            .ok()
            .flatten()
    });
    let default_target = match default_vid {
        Some(vid) => format!("{}", vid),
        None => "none".to_string(),
    };

    let current_variant: Rc<Cell<Option<i64>>> = Rc::new(Cell::new(default_vid));

    let actions = gio::SimpleActionGroup::new();
    let action = gio::SimpleAction::new_stateful(
        "variant",
        Some(glib::VariantTy::STRING),
        &glib::Variant::from(&default_target),
    );

    let st_c = state.clone();
    let current_variant_c = current_variant.clone();
    action.connect_activate(move |action, param| {
        if let Some(param) = param {
            let target_str = param.get::<String>().unwrap_or_default();
            let vid = if target_str == "none" {
                None
            } else {
                target_str.parse::<i64>().ok()
            };
            if let Err(e) = ira_db::set_default_variant(&st_c.borrow().db, db_id, vid) {
                eprintln!("Failed to set default variant: {e}");
            }
            let _ = st_c
                .borrow()
                .sender
                .send(crate::AppMessage::VariantSelected(db_id, vid));
            current_variant_c.set(vid);
            action.change_state(param);
        }
    });
    actions.add_action(&action);

    let menu = gio::Menu::new();
    menu.append(Some(&crate::tr!("Base game")), Some("play.variant::none"));
    for var in variants {
        menu.append(Some(&var.name), Some(&format!("play.variant::{}", var.id)));
    }
    add_open_emulator_menu_item(state, db_id, &actions, &menu, sender);

    split.insert_action_group("play", Some(&actions));
    split.set_menu_model(Some(&menu));

    let icon_click = icon;
    let label_click = label;
    let st_launch = state.clone();
    let current_variant_launch = current_variant;
    let sender_c = sender.clone();
    split.connect_clicked(move |btn| {
        let vid = current_variant_launch.get();
        toggle_play_clicked(
            &st_launch,
            db_id,
            vid,
            &sender_c,
            &icon_click,
            &label_click,
            btn,
        );
    });

    split.upcast()
}

/// Appends "Open emulator without game" to a play-button dropdown, but only
/// when the game runs through an emulator that can be opened on its own.
fn add_open_emulator_menu_item(
    state: &SharedState,
    db_id: i64,
    actions: &gio::SimpleActionGroup,
    menu: &gio::Menu,
    sender: &ira_models::AppSender,
) {
    let opens_emulator = state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == db_id)
        .map(|g| g.kind.has_standalone_emulator())
        .unwrap_or(false);
    if !opens_emulator {
        return;
    }

    let open_action = gio::SimpleAction::new("open_emulator", None);
    let st_open = state.clone();
    let sender_open = sender.clone();
    open_action.connect_activate(move |_, _| {
        if let Err(e) = open_emulator_no_game(&st_open, db_id) {
            eprintln!("Failed to open emulator: {}", e);
            let _ = sender_open.send(AppMessage::EmulatorOpenFailed { db_id, error: e });
        }
    });
    actions.add_action(&open_action);

    let section = gio::Menu::new();
    section.append(
        Some(&crate::tr!("Open emulator without game")),
        Some("play.open_emulator"),
    );
    menu.append_section(None, &section);
}

#[cfg(test)]
mod tests {
    use super::{sorted_controller_snapshot, LaunchingGameGuard};
    use ira_input::DeviceInfo;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    fn device(path: &str) -> DeviceInfo {
        DeviceInfo {
            path: PathBuf::from(path),
            name: String::new(),
            vendor: 0,
            product: 0,
            version: 0,
            has_evdev_gyro: false,
            uniq: String::new(),
            supported_buttons: Vec::new(),
        }
    }

    #[test]
    fn test_sorted_controller_snapshot_orders_by_path() {
        let devices = sorted_controller_snapshot(vec![
            device("/dev/input/event9"),
            device("/dev/input/event2"),
        ]);

        assert_eq!(devices[0].path, PathBuf::from("/dev/input/event2"));
        assert_eq!(devices[1].path, PathBuf::from("/dev/input/event9"));
    }

    #[test]
    fn test_launching_game_guard_clears_state_on_drop() {
        let launching_games = Arc::new(Mutex::new(HashSet::new()));
        launching_games.lock().unwrap().insert(42);

        {
            let _guard = LaunchingGameGuard {
                launching_games: launching_games.clone(),
                db_id: 42,
            };
        }

        assert!(!launching_games.lock().unwrap().contains(&42));
    }
}
