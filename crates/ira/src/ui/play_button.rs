use crate::AppMessage;
use gtk4::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use super::state::SharedState;

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
    let (running_games, sender, game_info, global_shadps4_exe, db, save_dir, app_default_wine, default_native_env_vars, cfg_clone) = {
        let s = state.borrow();
        (
            s.running_games.clone(),
            s.sender.clone(),
            s.games.iter()
                .find(|g| g.db_id == game_id)
                .map(|g| (g.kind, g.game_path.clone(), g.name.clone(), g.shadps4_version.clone(), g.db_id, g.app_id.clone(), g.platform_id.clone(), g.ra_core.clone(), g.emulator_override.clone()))
                .unwrap_or_default(),
            s.cfg.shadps4_executable.clone(),
            s.db.clone(),
            s.save_dir.clone(),
            s.cfg.default_wine_config.clone(),
            s.cfg.default_native_env_vars.clone(),
            s.cfg.clone(),
        )
    };

    if running_games.lock().unwrap().contains_key(&game_id) {
        return Ok(());
    }

    let (kind, game_path, game_name, per_game_version, db_id, app_id, platform_id, per_game_ra_core, per_game_emu) = game_info;

    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let variant_info = variant_id
        .and_then(|vid| ira_db::get_variants(&db, db_id).ok()?.into_iter().find(|v| v.id == vid))
        .map(|v| (v.show_as_entry, v.count_playtime));
    let (variant_show_as_entry, variant_count_playtime) = variant_info.unwrap_or((false, true));

    if kind == ira_models::GameKind::Retro {
        let cc = cfg_clone.console(&platform_id);
        let exe = if !per_game_emu.is_empty() {
            &per_game_emu
        } else if cc.executable.is_empty() {
            return Err(format!("No emulator configured for {}", platform_id));
        } else {
            &cc.executable
        };
        let core = if !per_game_ra_core.is_empty() {
            &per_game_ra_core
        } else {
            &cc.ra_core
        };
        let fullscreen_flag = ira_models::find_console(&platform_id)
            .map(|d| d.fullscreen_flag)
            .unwrap_or("--fullscreen");
        let rom_path = {
            let discs = ira_db::get_discs(&db, db_id).unwrap_or_default();
            let default_disc_id = ira_db::get_default_disc(&db, db_id);
            discs.iter()
                .find(|d| Some(d.id) == default_disc_id)
                .map(|d| d.rom_path.clone())
                .unwrap_or(game_path)
        };
        let cmd = ira_platforms::emulator_detect::build_launch_command(exe, &rom_path, core, cc.fullscreen, fullscreen_flag);
        let log_path = ira_launcher::wrapper::game_log_path(&save_dir, game_id);
        match ira_launcher::wrapper::spawn_game(&cmd, &[], None, Some(&log_path)) {
            Ok(child) => {
                let pid = child.id() as i32;
                running_games.lock().unwrap().insert(game_id, pid);
                let mc = ira_launcher::wrapper::MonitorContext {
                    sender: sender.clone(),
                    game_id,
                    variant_id: None,
                    count_playtime: true,
                    started_at,
                    db: db.clone(),
                    running_games: running_games.clone(),
                };
                std::thread::spawn(move || {
                    ira_launcher::wrapper::monitor_process(child, pid, mc);
                });
            }
            Err(e) => return Err(format!("Failed to launch {}: {}", game_name, e)),
        }
    } else if kind == ira_models::GameKind::Ps4 {
        let exe = if !per_game_version.is_empty() {
            per_game_version.as_str()
        } else if !global_shadps4_exe.is_empty() {
            &global_shadps4_exe
        } else {
            "shadps4"
        };
        let cmd = vec![exe.to_string(), "-g".to_string(), game_path.to_string()];
        let log_path = ira_launcher::wrapper::game_log_path(&save_dir, game_id);
        match ira_launcher::wrapper::spawn_game(&cmd, &[], None, Some(&log_path)) {
            Ok(child) => {
                let pid = child.id() as i32;
                running_games.lock().unwrap().insert(game_id, pid);
                let mc = ira_launcher::wrapper::MonitorContext {
                    sender: sender.clone(),
                    game_id,
                    variant_id: None,
                    count_playtime: true,
                    started_at,
                    db: db.clone(),
                    running_games: running_games.clone(),
                };
                std::thread::spawn(move || {
                    ira_launcher::wrapper::monitor_process(child, pid, mc);
                });
            }
            Err(e) => return Err(format!("Failed to launch shadPS4: {}", e)),
        }
    } else if kind == ira_models::GameKind::Steam {
        let cmd = vec!["steam".to_string(), "-applaunch".to_string(), app_id.clone()];
        match ira_launcher::wrapper::spawn_game(&cmd, &[], None, None) {
            Ok(_child) => {
            }
            Err(_) => {
                let uri = format!("steam://run/{}", app_id);
                let cmd = vec!["xdg-open".to_string(), uri];
                if let Err(e) = ira_launcher::wrapper::spawn_game(&cmd, &[], None, None) {
                    return Err(format!("Failed to launch Steam game: {}", e));
                }
            }
        }
    } else {
        let (mut launch, mut wine, profile_id) = ira_db::get_game_config(&db, db_id)
            .ok()
            .flatten()
            .unwrap_or_default();

        if let Some(pid) = profile_id {
            if let Ok(Some(profile)) = ira_db::get_profile(&db, pid) {
                wine.version = profile.wine_version;
                wine.custom_wine_path = profile.custom_wine_path;
                wine.prefix = profile.prefix;
                wine.arch = profile.arch;
                wine.umu_enabled = profile.umu_enabled;
            }
        }

        wine = wine.merge_with_default(&app_default_wine);

        if let Some(vid) = variant_id {
            if let Ok(variants) = ira_db::get_variants(&db, db_id) {
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
            let mut merged = default_native_env_vars.clone();
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
                    game_name: game_name.clone(),
                    sender,
                    game_id,
                    variant_id,
                    count_playtime: variant_count_playtime,
                    app_id: app_id.clone(),
                    db: db.clone(),
                    save_dir: save_dir.clone(),
                    running_games: running_games.clone(),
                },
            )?;
        } else {
            return Err(format!("No launch config saved for '{}'. Configure the game's launch settings first.", game_name));
        }
    }

    if !variant_count_playtime {
        // Variant doesn't count playtime (e.g. modding tool) — skip last_played
    } else if variant_show_as_entry {
        if let Some(vid) = variant_id {
            if let Err(e) = ira_db::set_variant_last_played(&db, vid, started_at) {
                eprintln!("Failed to update variant last played: {}", e);
            }
        }
        if let Some(g) = state.borrow_mut().games.iter_mut().find(|g| g.db_id == game_id && g.variant_id == variant_id) {
            g.last_played = started_at;
        }
    } else {
        if let Err(e) = ira_db::set_last_played(&db, db_id, started_at) {
            eprintln!("Failed to update last played: {}", e);
        }
        if let Some(g) = state.borrow_mut().games.iter_mut().find(|g| g.db_id == game_id && g.variant_id.is_none()) {
            g.last_played = started_at;
        }
    }

    Ok(())
}

pub fn play_button(state: &SharedState, db_id: i64, variant_id: Option<i64>) -> gtk4::Widget {
    let running_games = state.borrow().running_games.clone();
    let sender = state.borrow().sender.clone();
    let st = state.clone();

    let variants = ira_db::get_variants(&state.borrow().db, db_id).unwrap_or_default();
    let discs = ira_db::get_discs(&state.borrow().db, db_id).unwrap_or_default();
    let has_variants = !variants.is_empty();
    let has_discs = !discs.is_empty();

    let is_running = running_games.lock().unwrap().contains_key(&db_id);

    if !has_variants && !has_discs {
        let btn = gtk4::Button::new();
        btn.set_valign(gtk4::Align::Center);
        btn.set_height_request(48);

        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        hbox.set_valign(gtk4::Align::Center);
        hbox.set_halign(gtk4::Align::Center);
        hbox.set_margin_start(16);
        hbox.set_margin_end(16);

        let icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
        icon.set_pixel_size(20);
        hbox.append(&icon);

        let label = gtk4::Label::new(Some("Play"));
        label.add_css_class("play-btn-label");
        label.set_width_chars(5);
        hbox.append(&label);

        btn.set_child(Some(&hbox));

        if is_running {
            icon.set_icon_name(Some("window-close-symbolic"));
            label.set_text("Stop");
        } else {
            btn.add_css_class("suggested-action");
        }

        let icon_click = icon.clone();
        let label_click = label.clone();
        btn.connect_clicked(move |btn| {
            let is_running = st.borrow().running_games.lock().unwrap().contains_key(&db_id);
            if is_running {
                stop_game(&st, db_id);
                icon_click.set_icon_name(Some("media-playback-start-symbolic"));
                label_click.set_text("Play");
                btn.add_css_class("suggested-action");
            } else {
                match launch_game(&st, db_id, None) {
                    Ok(()) => {
                        icon_click.set_icon_name(Some("window-close-symbolic"));
                        label_click.set_text("Stop");
                        btn.remove_css_class("suggested-action");
                    }
                    Err(e) => {
                        eprintln!("Failed to launch game: {}", e);
                        let _ = sender.send(AppMessage::AddGameError(e));
                    }
                }
            }
        });

        return btn.upcast();
    }

    if has_discs {
        let split = adw::SplitButton::new();

        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        hbox.set_valign(gtk4::Align::Center);
        hbox.set_halign(gtk4::Align::Center);
        hbox.set_margin_start(16);
        hbox.set_margin_end(16);

        let icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
        icon.set_pixel_size(20);
        hbox.append(&icon);

        let label = gtk4::Label::new(Some("Play"));
        label.add_css_class("play-btn-label");
        label.set_width_chars(5);
        hbox.append(&label);

        split.set_child(Some(&hbox));
        split.set_height_request(48);
        split.set_valign(gtk4::Align::Center);
        split.set_dropdown_tooltip("Select disc");

        if is_running {
            icon.set_icon_name(Some("window-close-symbolic"));
            label.set_text("Stop");
        } else {
            split.add_css_class("suggested-action");
        }

        let default_did = ira_db::get_default_disc(&state.borrow().db, db_id);
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

        let st_c = st.clone();
        action.connect_activate(move |action, param| {
            if let Some(param) = param {
                let target_str = param.get::<String>().unwrap_or_default();
                let did = target_str.parse::<i64>().ok();
                ira_db::set_default_disc(&st_c.borrow().db, db_id, did);
                action.change_state(param);
            }
        });
        actions.add_action(&action);

        let menu = gio::Menu::new();
        for disc in &discs {
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
        let st_launch = st.clone();
        split.connect_clicked(move |btn| {
            let is_running = st_launch.borrow().running_games.lock().unwrap().contains_key(&db_id);
            if is_running {
                stop_game(&st_launch, db_id);
                icon_click.set_icon_name(Some("media-playback-start-symbolic"));
                label_click.set_text("Play");
                btn.add_css_class("suggested-action");
            } else {
                match launch_game(&st_launch, db_id, None) {
                    Ok(()) => {
                        icon_click.set_icon_name(Some("window-close-symbolic"));
                        label_click.set_text("Stop");
                        btn.remove_css_class("suggested-action");
                    }
                    Err(e) => {
                        eprintln!("Failed to launch game: {}", e);
                        let _ = sender.send(AppMessage::AddGameError(e));
                    }
                }
            }
        });

        return split.upcast();
    }

    let split = adw::SplitButton::new();

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_valign(gtk4::Align::Center);
    hbox.set_halign(gtk4::Align::Center);
    hbox.set_margin_start(16);
    hbox.set_margin_end(16);

    let icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
    icon.set_pixel_size(20);
    hbox.append(&icon);

    let label = gtk4::Label::new(Some("Play"));
    label.add_css_class("play-btn-label");
    label.set_width_chars(5);
    hbox.append(&label);

    split.set_child(Some(&hbox));
    split.set_height_request(48);
    split.set_valign(gtk4::Align::Center);
    split.set_dropdown_tooltip("Select variant");

    if is_running {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text("Stop");
    } else {
        split.add_css_class("suggested-action");
    }

    let default_vid = variant_id.or_else(|| {
        let vid = ira_db::get_default_variant(&state.borrow().db, db_id)?;
        variants.iter().find(|v| v.id == vid && v.count_playtime && !v.show_as_entry).map(|v| v.id)
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

    let eligible_default_ids: std::collections::HashSet<i64> = variants.iter()
        .filter(|v| v.count_playtime && !v.show_as_entry)
        .map(|v| v.id)
        .collect();

    // For variant entries, don't persist the selection to DB — just track locally.
    // For base games, persist as before and notify so the game page reloads
    // with the variant's hero + logo (if the variant has custom_images).
    let st_c = st.clone();
    let current_variant_c = current_variant.clone();
    action.connect_activate(move |action, param| {
        if let Some(param) = param {
            let target_str = param.get::<String>().unwrap_or_default();
            let vid = if target_str == "none" {
                None
            } else {
                target_str.parse::<i64>().ok()
            };
            if variant_id.is_none() {
                let can_be_default = vid.is_none_or(|vid| eligible_default_ids.contains(&vid));
                if can_be_default {
                    ira_db::set_default_variant(&st_c.borrow().db, db_id, vid);
                }
                let _ = st_c.borrow().sender.send(crate::AppMessage::VariantSelected(db_id, vid));
            }
            current_variant_c.set(vid);
            action.change_state(param);
        }
    });
    actions.add_action(&action);

    let menu = gio::Menu::new();
    menu.append(Some("Base game"), Some("play.variant::none"));
    for var in &variants {
        menu.append(Some(&var.name), Some(&format!("play.variant::{}", var.id)));
    }

    split.insert_action_group("play", Some(&actions));
    split.set_menu_model(Some(&menu));

    let icon_click = icon.clone();
    let label_click = label.clone();
    let st_launch = st.clone();
    let current_variant_launch = current_variant.clone();
    split.connect_clicked(move |btn| {
        let is_running = st_launch.borrow().running_games.lock().unwrap().contains_key(&db_id);
        if is_running {
            stop_game(&st_launch, db_id);
            icon_click.set_icon_name(Some("media-playback-start-symbolic"));
            label_click.set_text("Play");
            btn.add_css_class("suggested-action");
        } else {
            let vid = current_variant_launch.get();
            match launch_game(&st_launch, db_id, vid) {
                Ok(()) => {
                    icon_click.set_icon_name(Some("window-close-symbolic"));
                    label_click.set_text("Stop");
                    btn.remove_css_class("suggested-action");
                }
                Err(e) => {
                    eprintln!("Failed to launch game: {}", e);
                    let _ = sender.send(AppMessage::AddGameError(e));
                }
            }
        }
    });

    split.upcast()
}
