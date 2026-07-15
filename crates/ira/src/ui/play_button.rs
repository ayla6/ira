use crate::AppMessage;
use gtk4::prelude::*;
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
                .map(|g| (g.kind.clone(), g.game_path.clone(), g.name.clone(), g.shadps4_version.clone(), g.db_id, g.app_id.clone(), g.platform_id.clone(), g.ra_core.clone(), g.emulator_override.clone()))
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

    if kind == ira_models::RETRO {
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
        let cmd = ira_platforms::emulator_detect::build_launch_command(exe, &game_path, core, cc.fullscreen, fullscreen_flag);
        match ira_launcher::wrapper::spawn_game(&cmd, &[], None) {
            Ok(child) => {
                let pid = child.id() as i32;
                running_games.lock().unwrap().insert(game_id, pid);
                let sender_c = sender.clone();
                let db_c = db.clone();
                let rg = running_games.clone();
                std::thread::spawn(move || {
                    ira_launcher::wrapper::monitor_process(
                        child, pid, &sender_c, game_id, started_at, db_c, rg,
                    );
                });
            }
            Err(e) => return Err(format!("Failed to launch {}: {}", game_name, e)),
        }
    } else if kind == ira_models::PS4 {
        let exe = if !per_game_version.is_empty() {
            per_game_version.as_str()
        } else if !global_shadps4_exe.is_empty() {
            &global_shadps4_exe
        } else {
            "shadps4"
        };
        let cmd = vec![exe.to_string(), "-g".to_string(), game_path.to_string()];
        match ira_launcher::wrapper::spawn_game(&cmd, &[], None) {
            Ok(child) => {
                let pid = child.id() as i32;
                running_games.lock().unwrap().insert(game_id, pid);
                let sender_c = sender.clone();
                let db_c = db.clone();
                let rg = running_games.clone();
                std::thread::spawn(move || {
                    ira_launcher::wrapper::monitor_process(
                        child, pid, &sender_c, game_id, started_at, db_c, rg,
                    );
                });
            }
            Err(e) => return Err(format!("Failed to launch shadPS4: {}", e)),
        }
    } else if kind == ira_models::STEAM {
        let cmd = vec!["steam".to_string(), "-applaunch".to_string(), app_id.clone()];
        match ira_launcher::wrapper::spawn_game(&cmd, &[], None) {
            Ok(_child) => {
            }
            Err(_) => {
                let uri = format!("steam://run/{}", app_id);
                let cmd = vec!["xdg-open".to_string(), uri];
                if let Err(e) = ira_launcher::wrapper::spawn_game(&cmd, &[], None) {
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
                    db: db.clone(),
                    save_dir: save_dir.clone(),
                    running_games: running_games.clone(),
                },
            )?;
        } else {
            let uri = format!("lutris:rungameid/{}", game_id);
            let cmd = vec!["lutris".to_string(), uri.clone()];
            match ira_launcher::wrapper::spawn_game(&cmd, &[], None) {
                Ok(child) => {
                    let pid = child.id() as i32;
                    running_games.lock().unwrap().insert(game_id, pid);
                    let sender_c = sender.clone();
                    let db_c = db.clone();
                    let rg = running_games.clone();
                    std::thread::spawn(move || {
                        ira_launcher::wrapper::monitor_process(
                            child, pid, &sender_c, game_id, started_at, db_c, rg,
                        );
                    });
                }
                Err(e) => return Err(format!("Failed to launch {}: {}", uri, e)),
            }
        }
    }

    let _ = ira_db::set_last_played(&db, db_id, started_at);
    if let Some(g) = state.borrow_mut().games.iter_mut().find(|g| g.db_id == game_id) {
        g.last_played = started_at;
    }

    Ok(())
}

pub fn play_button(state: &SharedState, db_id: i64) -> gtk4::Widget {
    let running_games = state.borrow().running_games.clone();
    let sender = state.borrow().sender.clone();
    let st = state.clone();

    let variants = ira_db::get_variants(&state.borrow().db, db_id).unwrap_or_default();
    let has_variants = !variants.is_empty();

    let is_running = running_games.lock().unwrap().contains_key(&db_id);

    if !has_variants {
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

    let default_vid = ira_db::get_default_variant(&state.borrow().db, db_id);
    let default_target = match default_vid {
        Some(vid) => format!("{}", vid),
        None => "none".to_string(),
    };

    let actions = gio::SimpleActionGroup::new();
    let action = gio::SimpleAction::new_stateful(
        "variant",
        Some(glib::VariantTy::STRING),
        &glib::Variant::from(&default_target),
    );

    // Only save to DB and explicitly trigger change_state — the default
    // handler calls change_state after activate, but calling it here too
    // ensures the state updates immediately so the menu re-renders.
    let st_c = st.clone();
    action.connect_activate(move |action, param| {
        if let Some(param) = param {
            let target_str = param.get::<String>().unwrap_or_default();
            let vid = if target_str == "none" {
                None
            } else {
                target_str.parse::<i64>().ok()
            };
            ira_db::set_default_variant(&st_c.borrow().db, db_id, vid);
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
    split.connect_clicked(move |btn| {
        let is_running = st_launch.borrow().running_games.lock().unwrap().contains_key(&db_id);
        if is_running {
            stop_game(&st_launch, db_id);
            icon_click.set_icon_name(Some("media-playback-start-symbolic"));
            label_click.set_text("Play");
            btn.add_css_class("suggested-action");
        } else {
            let vid = ira_db::get_default_variant(&st_launch.borrow().db, db_id);
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
