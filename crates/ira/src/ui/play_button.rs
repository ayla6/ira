use crate::AppMessage;
use gtk4::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use super::css::*;
use super::input_profile_store::read_profile;
use super::play_button_helpers;
use super::state::SharedState;
use ira_models::ControllerInputMode;

const PLAY_BTN_HEIGHT: i32 = 48;
const PLAY_BTN_H_MARGIN: i32 = 16;
const PLAY_BTN_ICON_SIZE: i32 = 20;
const PLAY_BTN_LABEL_WIDTH: i32 = 5;

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
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text(&crate::tr!("Stop"));
        btn.remove_css_class(CSS_SUGGESTED_ACTION);
    } else {
        icon.set_icon_name(Some("media-playback-start-symbolic"));
        label.set_text(&crate::tr!("Play"));
        btn.add_css_class(CSS_SUGGESTED_ACTION);
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
    let pid = state
        .borrow()
        .running_games
        .lock()
        .unwrap()
        .remove(&game_id);
    if let Some(pid) = pid {
        let (wine_exe, wine_prefix, env) = {
            let s = state.borrow();
            let game = s.games.iter().find(|g| g.db_id == game_id);
            let db_id = game.map(|g| g.db_id).unwrap_or(0);
            let config = ira_db::get_game_config(&s.db, db_id).ok().flatten();
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
    let (
        running_games,
        sender,
        game_info,
        global_shadps4_exe,
        global_rpcs3_exe,
        global_vita3k_exe,
        global_cemu_exe,
        global_azahar_exe,
        db,
        save_dir,
        app_default_wine,
        default_native_env_vars,
        cfg_clone,
        overlay_shm,
        overlay_global_enabled,
        overlay_font_family,
        system_defaults,
        controller_registry,
    ) = {
        let s = state.borrow();
        let game = s.games.iter().find(|g| g.db_id == game_id);
        let source_id = game.and_then(|g| match g.kind {
            ira_models::GameKind::Steam => Some("steam"),
            ira_models::GameKind::Retro => Some(g.platform_id.as_str()),
            ira_models::GameKind::Ps4 => Some("ps4"),
            ira_models::GameKind::Ps3 => Some("ps3"),
            ira_models::GameKind::PsVita => Some("psvita"),
            ira_models::GameKind::WiiU => Some("wiiu"),
            ira_models::GameKind::ThreeDS => Some("3ds"),
            ira_models::GameKind::Switch => Some("switch"),
            _ => None,
        });
        let overlay_global_enabled =
            source_id.map_or(s.cfg.overlay.enabled, |id| s.cfg.overlay.source_enabled(id));
        let mut system_defaults = s.cfg.default_system.clone();
        system_defaults.gamescope = source_id
            .and_then(|id| s.cfg.overlay.source_gamescope.get(id).copied())
            .unwrap_or(system_defaults.gamescope);
        let overlay_shm = game.and_then(|game| {
            let launch = ira_db::get_game_config(&s.db, game.db_id)
                .ok()
                .flatten()
                .map(|(launch, _, _)| launch)
                .unwrap_or_default();
            crate::overlay::write_game_shm(
                game,
                &s.cfg.overlay,
                launch.overlay_encoder,
                launch.overlay_recording_quality,
            )
        });
        let game_info = game
            .map(|g| {
                (
                    g.kind,
                    g.game_path.clone(),
                    g.name.clone(),
                    g.shadps4_version.clone(),
                    g.db_id,
                    g.app_id.clone(),
                    g.platform_id.clone(),
                    g.ra_core.clone(),
                    g.emulator_override.clone(),
                    g.trophy_source,
                )
            })
            .unwrap_or_default();
        (
            s.running_games.clone(),
            s.sender.clone(),
            game_info,
            s.cfg.shadps4_executable.clone(),
            s.cfg.rpcs3_executable.clone(),
            s.cfg.vita3k_executable.clone(),
            s.cfg.cemu_executable.clone(),
            s.cfg.azahar_executable.clone(),
            s.db.clone(),
            s.save_dir.clone(),
            s.cfg.default_wine_config.clone(),
            s.cfg.default_native_env_vars.clone(),
            s.cfg.clone(),
            overlay_shm,
            overlay_global_enabled,
            s.cfg.overlay.font_family.clone(),
            system_defaults,
            s.controller_registry.clone(),
        )
    };

    if is_game_running(state, game_id) {
        return Ok(false);
    }

    let (
        kind,
        game_path,
        game_name,
        per_game_version,
        db_id,
        app_id,
        platform_id,
        per_game_ra_core,
        per_game_emu,
        trophy_source,
    ) = game_info;

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

    let ctx = play_button_helpers::LaunchCtx {
        db: &db,
        save_dir: &save_dir,
        game_id,
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
        system_defaults,
        controller_input_mode,
        controller_input_profile,
    };

    if matches!(
        kind,
        ira_models::GameKind::Retro | ira_models::GameKind::Switch
    ) {
        play_button_helpers::launch_retro(
            &ctx,
            &cfg_clone,
            &platform_id,
            &per_game_emu,
            &per_game_ra_core,
            &game_path,
        )?;
    } else if kind == ira_models::GameKind::Ps4 {
        play_button_helpers::launch_ps4(
            &ctx,
            &per_game_version,
            &global_shadps4_exe,
            &game_path,
            cfg_clone.console("ps4").controller_mode,
            (!cfg_clone.console("ps4").controller_profile.is_empty())
                .then_some(cfg_clone.console("ps4").controller_profile.as_str()),
        )?;
    } else if kind == ira_models::GameKind::Ps3 {
        play_button_helpers::launch_ps3(
            &ctx,
            &per_game_emu,
            &global_rpcs3_exe,
            &game_path,
            cfg_clone.console("ps3").controller_mode,
            (!cfg_clone.console("ps3").controller_profile.is_empty())
                .then_some(cfg_clone.console("ps3").controller_profile.as_str()),
        )?;
    } else if kind == ira_models::GameKind::PsVita {
        play_button_helpers::launch_vita3k(
            &ctx,
            &global_vita3k_exe,
            &game_path,
            cfg_clone.console("psvita").controller_mode,
            (!cfg_clone.console("psvita").controller_profile.is_empty())
                .then_some(cfg_clone.console("psvita").controller_profile.as_str()),
        )?;
    } else if kind == ira_models::GameKind::WiiU {
        play_button_helpers::launch_cemu(
            &ctx,
            &per_game_emu,
            &global_cemu_exe,
            &game_path,
            cfg_clone.console("wiiu").controller_mode,
            (!cfg_clone.console("wiiu").controller_profile.is_empty())
                .then_some(cfg_clone.console("wiiu").controller_profile.as_str()),
        )?;
    } else if kind == ira_models::GameKind::ThreeDS {
        play_button_helpers::launch_azahar(
            &ctx,
            &per_game_emu,
            &global_azahar_exe,
            &game_path,
            cfg_clone.console("3ds").controller_mode,
            (!cfg_clone.console("3ds").controller_profile.is_empty())
                .then_some(cfg_clone.console("3ds").controller_profile.as_str()),
        )?;
    } else if kind == ira_models::GameKind::Steam {
        let result = play_button_helpers::launch_steam(&ctx, &app_id);
        if result == Ok(true) {
            let _ = sender.send(crate::AppMessage::GameStarted(db_id, variant_id));
        }
        return result;
    } else {
        play_button_helpers::launch_other(
            &ctx,
            &app_default_wine,
            variant_id,
            variant_count_playtime,
            &default_native_env_vars,
            play_button_helpers::PcControllerProfiles {
                linux: (
                    cfg_clone.linux_controller_mode,
                    cfg_clone.linux_controller_profile.as_str(),
                ),
                wine: (
                    cfg_clone.wine_controller_mode,
                    cfg_clone.wine_controller_profile.as_str(),
                ),
            },
            &app_id,
        )?;
    }

    play_button_helpers::update_last_played(
        state,
        &ctx,
        variant_id,
        variant_count_playtime,
        variant_show_as_entry,
    );

    // The sidebar playing style is driven by GameStarted, and launch_game is
    // the single launch path (sidebar click, context menu and play buttons),
    // so report the start here to keep every entry point in sync.
    let _ = sender.send(crate::AppMessage::GameStarted(db_id, variant_id));

    Ok(true)
}

/// Opens this game's configured emulator with no game loaded (the
/// "Open emulator without game" menu entries). The process is detached:
/// no playtime is recorded, no play state changes, no messages are sent.
pub fn open_emulator_no_game(state: &SharedState, db_id: i64) -> Result<(), String> {
    if is_game_running(state, db_id) {
        return Err(crate::tr!(
            "Stop the running game before opening its emulator"
        ));
    }

    let (
        sender,
        game_info,
        global_shadps4_exe,
        global_rpcs3_exe,
        global_vita3k_exe,
        global_cemu_exe,
        global_azahar_exe,
        db,
        save_dir,
        cfg_clone,
        overlay_shm,
        overlay_global_enabled,
        overlay_font_family,
        system_defaults,
        controller_registry,
        running_games,
    ) = {
        let s = state.borrow();
        let game = s.games.iter().find(|g| g.db_id == db_id);
        let source_id = game.and_then(|g| match g.kind {
            ira_models::GameKind::Steam => Some("steam"),
            ira_models::GameKind::Retro => Some(g.platform_id.as_str()),
            ira_models::GameKind::Ps4 => Some("ps4"),
            ira_models::GameKind::Ps3 => Some("ps3"),
            ira_models::GameKind::PsVita => Some("psvita"),
            ira_models::GameKind::WiiU => Some("wiiu"),
            ira_models::GameKind::ThreeDS => Some("3ds"),
            ira_models::GameKind::Switch => Some("switch"),
            _ => None,
        });
        let overlay_global_enabled =
            source_id.map_or(s.cfg.overlay.enabled, |id| s.cfg.overlay.source_enabled(id));
        let mut system_defaults = s.cfg.default_system.clone();
        system_defaults.gamescope = source_id
            .and_then(|id| s.cfg.overlay.source_gamescope.get(id).copied())
            .unwrap_or(system_defaults.gamescope);
        let overlay_shm = game.and_then(|game| {
            let launch = ira_db::get_game_config(&s.db, game.db_id)
                .ok()
                .flatten()
                .map(|(launch, _, _)| launch)
                .unwrap_or_default();
            crate::overlay::write_game_shm(
                game,
                &s.cfg.overlay,
                launch.overlay_encoder,
                launch.overlay_recording_quality,
            )
        });
        let game_info = game
            .map(|g| {
                (
                    g.kind,
                    g.name.clone(),
                    g.shadps4_version.clone(),
                    g.platform_id.clone(),
                    g.emulator_override.clone(),
                    g.trophy_source,
                )
            })
            .unwrap_or_default();
        (
            s.sender.clone(),
            game_info,
            s.cfg.shadps4_executable.clone(),
            s.cfg.rpcs3_executable.clone(),
            s.cfg.vita3k_executable.clone(),
            s.cfg.cemu_executable.clone(),
            s.cfg.azahar_executable.clone(),
            s.db.clone(),
            s.save_dir.clone(),
            s.cfg.clone(),
            overlay_shm,
            overlay_global_enabled,
            s.cfg.overlay.font_family.clone(),
            system_defaults,
            s.controller_registry.clone(),
            s.running_games.clone(),
        )
    };

    let (kind, game_name, per_game_version, platform_id, per_game_emu, trophy_source) = game_info;

    let (controller_input_mode, controller_input_profile) =
        active_controller_input(&cfg_clone, &save_dir, &controller_registry);

    let ctx = play_button_helpers::LaunchCtx {
        db: &db,
        save_dir: &save_dir,
        game_id: db_id,
        db_id,
        game_name: &game_name,
        game_kind: kind,
        trophy_source,
        ufs_savefiles: Vec::new(),
        ufs_rootoverrides: Vec::new(),
        centralize_saves: false,
        sender: &sender,
        running_games: &running_games,
        started_at: 0,
        overlay_shm,
        overlay_global_enabled,
        overlay_font_family,
        system_defaults,
        controller_input_mode,
        controller_input_profile,
    };

    play_button_helpers::launch_emulator_no_game(
        &ctx,
        &cfg_clone,
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

    let variants = ira_db::get_variants(&db, db_id).unwrap_or_default();
    let discs = ira_db::get_discs(&db, db_id).unwrap_or_default();
    let has_variants = !variants.is_empty();
    let has_discs = !discs.is_empty();

    let is_running = is_game_running(state, db_id);

    if !has_variants && !has_discs {
        return build_simple_play_button(state, db_id, &sender, is_running);
    }

    if has_discs {
        return build_disc_play_button(state, db_id, &discs, &sender, is_running);
    }

    build_variant_play_button(state, db_id, &variants, variant_id, &sender, is_running)
}

fn build_simple_play_button(
    state: &SharedState,
    db_id: i64,
    sender: &ira_models::AppSender,
    is_running: bool,
) -> gtk4::Widget {
    let btn = gtk4::Button::new();
    btn.set_valign(gtk4::Align::Center);
    btn.set_height_request(PLAY_BTN_HEIGHT);

    let hbox = build_play_btn_hbox();
    let icon = hbox
        .first_child()
        .and_then(|c| c.downcast::<gtk4::Image>().ok())
        .unwrap();
    let label = hbox
        .last_child()
        .and_then(|c| c.downcast::<gtk4::Label>().ok())
        .unwrap();

    btn.set_child(Some(&hbox));

    if !is_running {
        btn.add_css_class(CSS_SUGGESTED_ACTION);
    } else {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text(&crate::tr!("Stop"));
    }

    let icon_click = icon;
    let label_click = label;
    let st = state.clone();
    let sender_c = sender.clone();
    btn.connect_clicked(move |btn| {
        let is_running = is_game_running(&st, db_id);
        if is_running {
            stop_game(&st, db_id);
            set_running_state(&icon_click, &label_click, btn, false);
        } else {
            match launch_game(&st, db_id, None) {
                Ok(true) => {
                    set_running_state(&icon_click, &label_click, btn, true);
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("Failed to launch game: {}", e);
                    let _ = sender_c.send(AppMessage::AddGameError(e));
                }
            }
        }
    });

    btn.upcast()
}

fn build_disc_play_button(
    state: &SharedState,
    db_id: i64,
    discs: &[ira_models::GameDisc],
    sender: &ira_models::AppSender,
    is_running: bool,
) -> gtk4::Widget {
    let split = adw::SplitButton::new();

    let hbox = build_play_btn_hbox();
    let icon = hbox
        .first_child()
        .and_then(|c| c.downcast::<gtk4::Image>().ok())
        .unwrap();
    let label = hbox
        .last_child()
        .and_then(|c| c.downcast::<gtk4::Label>().ok())
        .unwrap();

    split.set_child(Some(&hbox));
    split.set_height_request(PLAY_BTN_HEIGHT);
    split.set_valign(gtk4::Align::Center);
    split.set_dropdown_tooltip(&crate::tr!("Select disc"));

    if !is_running {
        split.add_css_class(CSS_SUGGESTED_ACTION);
    } else {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text(&crate::tr!("Stop"));
    }

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
        let is_running = is_game_running(&st_launch, db_id);
        if is_running {
            stop_game(&st_launch, db_id);
            set_running_state(&icon_click, &label_click, btn, false);
        } else {
            match launch_game(&st_launch, db_id, None) {
                Ok(true) => {
                    set_running_state(&icon_click, &label_click, btn, true);
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("Failed to launch game: {}", e);
                    let _ = sender_c.send(AppMessage::AddGameError(e));
                }
            }
        }
    });

    split.upcast()
}

fn build_variant_play_button(
    state: &SharedState,
    db_id: i64,
    variants: &[ira_models::GameVariant],
    variant_id: Option<i64>,
    sender: &ira_models::AppSender,
    is_running: bool,
) -> gtk4::Widget {
    let split = adw::SplitButton::new();

    let hbox = build_play_btn_hbox();
    let icon = hbox
        .first_child()
        .and_then(|c| c.downcast::<gtk4::Image>().ok())
        .unwrap();
    let label = hbox
        .last_child()
        .and_then(|c| c.downcast::<gtk4::Label>().ok())
        .unwrap();

    split.set_child(Some(&hbox));
    split.set_height_request(PLAY_BTN_HEIGHT);
    split.set_valign(gtk4::Align::Center);
    split.set_dropdown_tooltip(&crate::tr!("Select variant"));

    if !is_running {
        split.add_css_class(CSS_SUGGESTED_ACTION);
    } else {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text(&crate::tr!("Stop"));
    }

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
        let is_running = is_game_running(&st_launch, db_id);
        if is_running {
            stop_game(&st_launch, db_id);
            set_running_state(&icon_click, &label_click, btn, false);
        } else {
            let vid = current_variant_launch.get();
            match launch_game(&st_launch, db_id, vid) {
                Ok(true) => {
                    set_running_state(&icon_click, &label_click, btn, true);
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("Failed to launch game: {}", e);
                    let _ = sender_c.send(AppMessage::AddGameError(e));
                }
            }
        }
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
            let _ = sender_open.send(AppMessage::AddGameError(e));
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
    use super::sorted_controller_snapshot;
    use ira_input::DeviceInfo;
    use std::path::PathBuf;

    fn device(path: &str) -> DeviceInfo {
        DeviceInfo {
            path: PathBuf::from(path),
            name: String::new(),
            vendor: 0,
            product: 0,
            version: 0,
            has_evdev_gyro: false,
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
}
