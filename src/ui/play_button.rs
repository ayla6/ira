use crate::AppMessage;
use gtk4::prelude::*;
use super::state::SharedState;

pub fn stop_game(state: &SharedState, lutris_id: i64) {
    let pid = state.borrow().running_games.lock().unwrap().remove(&lutris_id);
    if let Some(pid) = pid {
        let (wine_exe, wine_prefix, env) = {
            let s = state.borrow();
            let game = s.games.iter().find(|g| g.lutris_id == lutris_id);
            let db_id = game.map(|g| g.db_id).unwrap_or(0);
            let config = crate::db::get_game_config(&s.db, db_id).ok().flatten();
            let app_default = s.cfg.default_wine_config.clone();
            let (exe, prefix, env_vars) = if let Some((_, mut wine, _)) = config {
                wine = wine.merge_with_default(&app_default);
                if wine.enabled {
                    let exe = crate::launcher::wine_launch::find_wine_binary(&wine.version, &wine.custom_wine_path).ok();
                    let prefix = crate::launcher::wine_launch::wine_prefix(&wine);
                    let env = crate::launcher::wine_launch::build_wine_env(&wine, exe.as_deref().unwrap_or(""));
                    (exe, Some(prefix), env)
                } else {
                    (None, None, Vec::new())
                }
            } else {
                (None, None, Vec::new())
            };
            (exe, prefix, env_vars)
        };
        crate::launcher::wrapper::stop_game_with_wine(
            pid,
            wine_exe.as_deref(),
            wine_prefix.as_deref(),
            &env,
        );
    }
}

pub fn launch_game(state: &SharedState, lutris_id: i64) -> Result<(), String> {
    let (running_games, sender, game_info, global_shadps4_exe, db, save_dir, app_default_wine, default_native_env_vars) = {
        let s = state.borrow();
        (
            s.running_games.clone(),
            s.sender.clone(),
            s.games.iter()
                .find(|g| g.lutris_id == lutris_id)
                .map(|g| (g.kind.clone(), g.game_path.clone(), g.name.clone(), g.shadps4_version.clone(), g.db_id))
                .unwrap_or_default(),
            s.cfg.shadps4_executable.clone(),
            s.db.clone(),
            s.save_dir.clone(),
            s.cfg.default_wine_config.clone(),
            s.cfg.default_native_env_vars.clone(),
        )
    };

    if running_games.lock().unwrap().contains_key(&lutris_id) {
        return Ok(());
    }

    let (kind, game_path, game_name, per_game_version, db_id) = game_info;

    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if kind == "ps4" {
        let exe = if !per_game_version.is_empty() {
            per_game_version.as_str()
        } else if !global_shadps4_exe.is_empty() {
            &global_shadps4_exe
        } else {
            "shadps4"
        };
        let cmd = vec![exe.to_string(), "-g".to_string(), game_path.to_string()];
        match crate::launcher::wrapper::spawn_game(&cmd, &[], None) {
            Ok(child) => {
                let pid = child.id() as i32;
                running_games.lock().unwrap().insert(lutris_id, pid);
                let sender_c = sender.clone();
                let db_c = db.clone();
                let rg = running_games.clone();
                std::thread::spawn(move || {
                    crate::launcher::wrapper::monitor_process(
                        child, pid, &sender_c, lutris_id, started_at, db_c, db_id, rg,
                    );
                });
            }
            Err(e) => return Err(format!("Failed to launch shadPS4: {}", e)),
        }
    } else {
        let (mut launch, mut wine, profile_id) = crate::db::get_game_config(&db, db_id)
            .ok()
            .flatten()
            .unwrap_or_default();

        if let Some(pid) = profile_id {
            if let Ok(Some(profile)) = crate::db::get_profile(&db, pid) {
                wine.version = profile.wine_version;
                wine.custom_wine_path = profile.custom_wine_path;
                wine.prefix = profile.prefix;
                wine.arch = profile.arch;
            }
        }

        wine = wine.merge_with_default(&app_default_wine);

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
            crate::launcher::launch_game(
                &launch, wine_opt, &game_name, sender, db_id, lutris_id,
                db.clone(), &save_dir, running_games,
            )?;
        } else {
            let uri = format!("lutris:rungameid/{}", lutris_id);
            let cmd = vec!["lutris".to_string(), uri.clone()];
            match crate::launcher::wrapper::spawn_game(&cmd, &[], None) {
                Ok(child) => {
                    let pid = child.id() as i32;
                    running_games.lock().unwrap().insert(lutris_id, pid);
                    let sender_c = sender.clone();
                    let db_c = db.clone();
                    let rg = running_games.clone();
                    std::thread::spawn(move || {
                        crate::launcher::wrapper::monitor_process(
                            child, pid, &sender_c, lutris_id, started_at, db_c, db_id, rg,
                        );
                    });
                }
                Err(e) => return Err(format!("Failed to launch {}: {}", uri, e)),
            }
        }
    }

    let _ = crate::db::set_last_played(&db, db_id, started_at);
    if let Some(g) = state.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
        g.lastplayed = started_at;
    }

    Ok(())
}

pub fn play_button(state: &SharedState, lutris_id: i64) -> gtk4::Button {
    let running_games = state.borrow().running_games.clone();
    let sender = state.borrow().sender.clone();

    let btn = gtk4::Button::new();
    btn.set_valign(gtk4::Align::Center);
    btn.set_size_request(130, 48);

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_valign(gtk4::Align::Center);
    hbox.set_halign(gtk4::Align::Center);

    let icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
    icon.set_pixel_size(20);
    hbox.append(&icon);

    let label = gtk4::Label::new(Some("Play"));
    label.add_css_class("play-btn-label");
    hbox.append(&label);

    btn.set_child(Some(&hbox));

    let is_running = running_games.lock().unwrap().contains_key(&lutris_id);
    if is_running {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text("Stop");
    } else {
        btn.add_css_class("suggested-action");
    }

    let icon_click = icon.clone();
    let label_click = label.clone();
    let st = state.clone();
    btn.connect_clicked(move |btn| {
        let is_running = st.borrow().running_games.lock().unwrap().contains_key(&lutris_id);
        if is_running {
            stop_game(&st, lutris_id);
            icon_click.set_icon_name(Some("media-playback-start-symbolic"));
            label_click.set_text("Play");
            btn.add_css_class("suggested-action");
        } else {
            match launch_game(&st, lutris_id) {
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

    btn
}
