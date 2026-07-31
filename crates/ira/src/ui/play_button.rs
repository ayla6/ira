use crate::AppMessage;
use gtk4::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use super::play_button_helpers;
use super::state::SharedState;
use super::css::*;

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

    let label = gtk4::Label::new(Some("Play"));
    label.add_css_class(CSS_PLAY_BTN_LABEL);
    label.set_width_chars(PLAY_BTN_LABEL_WIDTH);
    hbox.append(&label);

    hbox
}

fn set_running_state(icon: &gtk4::Image, label: &gtk4::Label, btn: &impl IsA<gtk4::Widget>, running: bool) {
    if running {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text("Stop");
        btn.remove_css_class(CSS_SUGGESTED_ACTION);
    } else {
        icon.set_icon_name(Some("media-playback-start-symbolic"));
        label.set_text("Play");
        btn.add_css_class(CSS_SUGGESTED_ACTION);
    }
}

fn is_game_running(state: &SharedState, db_id: i64) -> bool {
    state.borrow().running_games.lock().map(|m| m.contains_key(&db_id)).unwrap_or(false)
}

pub fn stop_game(state: &SharedState, game_id: i64) {
    let pid = state.borrow().running_games.lock().unwrap().remove(&game_id);
    if let Some(pid) = pid {
        let (wine_exe, wine_prefix, env) = {
            let s = state.borrow();
            let game = s.games.iter().find(|g| g.db_id == game_id);
            let db_id = game.map(|g| g.db_id).unwrap_or(0);
            let config = ira_db::get_game_config(&s.db, db_id).ok().flatten();
            let app_default = s.cfg.default_wine_config.clone();
            let (exe, prefix, env_vars) = if let Some((_, mut wine, _)) = config {
                wine = wine.merge_with_default(&app_default);
                if wine.enabled {
                    let exe = ira_launcher::wine_launch::find_wine_binary(&wine.version, &wine.custom_wine_path).ok();
                    let prefix = ira_launcher::wine_launch::wine_prefix(&wine);
                    let env = ira_launcher::wine_launch::build_wine_env(&wine, exe.as_deref().unwrap_or(""));
                    (exe, Some(prefix), env)
                } else {
                    (None, None, Vec::new())
                }
            } else {
                (None, None, Vec::new())
            };
            (exe, prefix, env_vars)
        };
        ira_launcher::wrapper::stop_game_with_wine(
            pid,
            wine_exe.as_deref(),
            wine_prefix.as_deref(),
            &env,
        );
    }
}

pub fn launch_game(state: &SharedState, game_id: i64, variant_id: Option<i64>) -> Result<(), String> {
    let (running_games, sender, game_info, global_shadps4_exe, global_rpcs3_exe, db, save_dir, app_default_wine, default_native_env_vars, cfg_clone, overlay_shm, overlay_global_enabled, overlay_font_family, gamescope_default, gamemode_default, mangohud_default, gamescope_w_default, gamescope_h_default, gamescope_fps_default, gamescope_upscaling_default) = {
        let s = state.borrow();
        let game = s.games.iter().find(|g| g.db_id == game_id);
        let overlay_shm = game.and_then(|g| crate::overlay::write_game_shm(g, &s.cfg.overlay));
        let source_id = game.and_then(|g| {
            match g.kind {
                ira_models::GameKind::Steam => Some("steam"),
                ira_models::GameKind::Retro => Some(g.platform_id.as_str()),
                ira_models::GameKind::Ps4 => Some("ps4"),
                ira_models::GameKind::Ps3 => Some("ps3"),
                _ => None,
            }
        });
        let overlay_global_enabled = source_id
            .map_or(s.cfg.overlay.enabled, |id| s.cfg.overlay.source_enabled(id));
        let gamemode_default = s.cfg.default_system.gamemode;
        let mangohud_default = s.cfg.default_system.mangohud;
        let gamescope_default = source_id
            .and_then(|id| s.cfg.overlay.source_gamescope.get(id).copied())
            .unwrap_or(s.cfg.default_system.gamescope);
        let gamescope_w_default = s.cfg.default_system.gamescope_w;
        let gamescope_h_default = s.cfg.default_system.gamescope_h;
        let gamescope_fps_default = s.cfg.default_system.gamescope_fps;
        let gamescope_upscaling_default = s.cfg.default_system.gamescope_upscaling.clone();
        let game_info = game
            .map(|g| (g.kind, g.game_path.clone(), g.name.clone(), g.shadps4_version.clone(), g.db_id, g.app_id.clone(), g.platform_id.clone(), g.ra_core.clone(), g.emulator_override.clone(), g.trophy_source))
            .unwrap_or_default();
        (
            s.running_games.clone(),
            s.sender.clone(),
            game_info,
            s.cfg.shadps4_executable.clone(),
            s.cfg.rpcs3_executable.clone(),
            s.db.clone(),
            s.save_dir.clone(),
            s.cfg.default_wine_config.clone(),
            s.cfg.default_native_env_vars.clone(),
            s.cfg.clone(),
            overlay_shm,
            overlay_global_enabled,
            s.cfg.overlay.font_family.clone(),
            gamescope_default,
            gamemode_default,
            mangohud_default,
            gamescope_w_default,
            gamescope_h_default,
            gamescope_fps_default,
            gamescope_upscaling_default,
        )
    };

    if is_game_running(state, game_id) {
        return Ok(());
    }

    let (kind, game_path, game_name, per_game_version, db_id, app_id, platform_id, per_game_ra_core, per_game_emu, trophy_source) = game_info;

    let (ufs_savefiles, ufs_rootoverrides) = crate::game_loader::read_app_details(&save_dir, &app_id)
        .map(|d| (d.ufs_savefiles, d.ufs_rootoverrides))
        .unwrap_or_default();

    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let variant_info = variant_id
        .and_then(|vid| ira_db::get_variants(&db, db_id).ok()?.into_iter().find(|v| v.id == vid))
        .map(|v| (v.show_as_entry, v.count_playtime));
    let (variant_show_as_entry, variant_count_playtime) = variant_info.unwrap_or((false, true));

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
        gamescope_default,
        gamemode_default,
        mangohud_default,
        gamescope_w_default,
        gamescope_h_default,
        gamescope_fps_default,
        gamescope_upscaling_default,
    };

    if kind == ira_models::GameKind::Retro {
        play_button_helpers::launch_retro(&ctx, &cfg_clone, &platform_id, &per_game_emu, &per_game_ra_core, &game_path)?;
    } else if kind == ira_models::GameKind::Ps4 {
        play_button_helpers::launch_ps4(&ctx, &per_game_version, &global_shadps4_exe, &game_path)?;
    } else if kind == ira_models::GameKind::Ps3 {
        play_button_helpers::launch_ps3(&ctx, &per_game_emu, &global_rpcs3_exe, &game_path)?;
    } else if kind == ira_models::GameKind::Steam {
        play_button_helpers::launch_steam(&app_id)?;
    } else {
        play_button_helpers::launch_other(&ctx, &app_default_wine, variant_id, variant_count_playtime, &default_native_env_vars, &app_id)?;
    }

    play_button_helpers::update_last_played(state, &ctx, variant_id, variant_count_playtime, variant_show_as_entry);

    Ok(())
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
    let icon = hbox.first_child()
        .and_then(|c| c.downcast::<gtk4::Image>().ok())
        .unwrap();
    let label = hbox.last_child()
        .and_then(|c| c.downcast::<gtk4::Label>().ok())
        .unwrap();

    btn.set_child(Some(&hbox));

    if !is_running {
        btn.add_css_class(CSS_SUGGESTED_ACTION);
    } else {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text("Stop");
    }

    let icon_click = icon.clone();
    let label_click = label.clone();
    let st = state.clone();
    let sender_c = sender.clone();
    btn.connect_clicked(move |btn| {
        let is_running = is_game_running(&st, db_id);
        if is_running {
            stop_game(&st, db_id);
            set_running_state(&icon_click, &label_click, btn, false);
        } else {
            match launch_game(&st, db_id, None) {
                Ok(()) => {
                    set_running_state(&icon_click, &label_click, btn, true);
                }
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
    let icon = hbox.first_child()
        .and_then(|c| c.downcast::<gtk4::Image>().ok())
        .unwrap();
    let label = hbox.last_child()
        .and_then(|c| c.downcast::<gtk4::Label>().ok())
        .unwrap();

    split.set_child(Some(&hbox));
    split.set_height_request(PLAY_BTN_HEIGHT);
    split.set_valign(gtk4::Align::Center);
    split.set_dropdown_tooltip("Select disc");

    if !is_running {
        split.add_css_class(CSS_SUGGESTED_ACTION);
    } else {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text("Stop");
    }

    let default_did = ira_db::get_default_disc(&state.borrow().db, db_id).ok().flatten();
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
            format!("Disc {}", disc.disc_number)
        } else {
            disc.label.clone()
        };
        menu.append(Some(&name), Some(&format!("play.disc::{}", disc.id)));
    }

    split.insert_action_group("play", Some(&actions));
    split.set_menu_model(Some(&menu));

    let icon_click = icon.clone();
    let label_click = label.clone();
    let st_launch = state.clone();
    let sender_c = sender.clone();
    split.connect_clicked(move |btn| {
        let is_running = is_game_running(&st_launch, db_id);
        if is_running {
            stop_game(&st_launch, db_id);
            set_running_state(&icon_click, &label_click, btn, false);
        } else {
            match launch_game(&st_launch, db_id, None) {
                Ok(()) => {
                    set_running_state(&icon_click, &label_click, btn, true);
                }
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
    let icon = hbox.first_child()
        .and_then(|c| c.downcast::<gtk4::Image>().ok())
        .unwrap();
    let label = hbox.last_child()
        .and_then(|c| c.downcast::<gtk4::Label>().ok())
        .unwrap();

    split.set_child(Some(&hbox));
    split.set_height_request(PLAY_BTN_HEIGHT);
    split.set_valign(gtk4::Align::Center);
    split.set_dropdown_tooltip("Select variant");

    if !is_running {
        split.add_css_class(CSS_SUGGESTED_ACTION);
    } else {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text("Stop");
    }

    let default_vid = variant_id.or_else(|| ira_db::get_default_variant(&state.borrow().db, db_id).ok().flatten());
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
            let _ = st_c.borrow().sender.send(crate::AppMessage::VariantSelected(db_id, vid));
            current_variant_c.set(vid);
            action.change_state(param);
        }
    });
    actions.add_action(&action);

    let menu = gio::Menu::new();
    menu.append(Some("Base game"), Some("play.variant::none"));
    for var in variants {
        menu.append(Some(&var.name), Some(&format!("play.variant::{}", var.id)));
    }

    split.insert_action_group("play", Some(&actions));
    split.set_menu_model(Some(&menu));

    let icon_click = icon.clone();
    let label_click = label.clone();
    let st_launch = state.clone();
    let current_variant_launch = current_variant.clone();
    let sender_c = sender.clone();
    split.connect_clicked(move |btn| {
        let is_running = is_game_running(&st_launch, db_id);
        if is_running {
            stop_game(&st_launch, db_id);
            set_running_state(&icon_click, &label_click, btn, false);
        } else {
            let vid = current_variant_launch.get();
            match launch_game(&st_launch, db_id, vid) {
                Ok(()) => {
                    set_running_state(&icon_click, &label_click, btn, true);
                }
                Err(e) => {
                    eprintln!("Failed to launch game: {}", e);
                    let _ = sender_c.send(AppMessage::AddGameError(e));
                }
            }
        }
    });

    split.upcast()
}
