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

pub fn launch_game(state: &SharedState, lutris_id: i64, variant_id: Option<i64>) -> Result<(), String> {
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

        if let Some(vid) = variant_id {
            if let Ok(variants) = crate::db::get_variants(&db, db_id) {
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

pub fn play_button(state: &SharedState, lutris_id: i64, db_id: i64) -> gtk4::Widget {
    let running_games = state.borrow().running_games.clone();
    let sender = state.borrow().sender.clone();
    let st = state.clone();

    let variants = crate::db::get_variants(&state.borrow().db, db_id).unwrap_or_default();
    let has_variants = !variants.is_empty();
    let default_vid = crate::db::get_default_variant(&state.borrow().db, db_id);

    let variant_ids: Vec<Option<i64>> = {
        let mut v: Vec<Option<i64>> = vec![None];
        for var in &variants {
            v.push(Some(var.id));
        }
        v
    };
    let variant_labels: Vec<String> = {
        let mut v: Vec<String> = vec!["Base game".to_string()];
        for var in &variants {
            v.push(var.name.clone());
        }
        v
    };

    let is_running = running_games.lock().unwrap().contains_key(&lutris_id);

    if !has_variants {
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

        if is_running {
            icon.set_icon_name(Some("window-close-symbolic"));
            label.set_text("Stop");
        } else {
            btn.add_css_class("suggested-action");
        }

        let icon_click = icon.clone();
        let label_click = label.clone();
        btn.connect_clicked(move |btn| {
            let is_running = st.borrow().running_games.lock().unwrap().contains_key(&lutris_id);
            if is_running {
                stop_game(&st, lutris_id);
                icon_click.set_icon_name(Some("media-playback-start-symbolic"));
                label_click.set_text("Play");
                btn.add_css_class("suggested-action");
            } else {
                match launch_game(&st, lutris_id, None) {
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

    let icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
    icon.set_pixel_size(20);
    hbox.append(&icon);

    let label = gtk4::Label::new(Some("Play"));
    label.add_css_class("play-btn-label");
    hbox.append(&label);

    split.set_child(Some(&hbox));
    split.set_size_request(130, 48);
    split.set_valign(gtk4::Align::Center);

    if is_running {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text("Stop");
    } else {
        split.add_css_class("suggested-action");
    }

    let popover = gtk4::Popover::new();
    let list = gtk4::ListBox::new();
    list.set_margin_start(6);
    list.set_margin_end(6);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    list.add_css_class("boxed-list");

    let selected_idx = variant_ids
        .iter()
        .position(|v| *v == default_vid)
        .unwrap_or(0);

    for (i, name) in variant_labels.iter().enumerate() {
        let row = gtk4::ListBoxRow::new();
        let lbl = gtk4::Label::new(Some(name));
        lbl.set_xalign(0.0);
        lbl.set_margin_start(8);
        lbl.set_margin_end(8);
        lbl.set_margin_top(6);
        lbl.set_margin_bottom(6);
        row.set_child(Some(&lbl));
        if i == selected_idx {
            row.add_css_class("selected");
        }
        list.append(&row);
    }

    popover.set_child(Some(&list));
    split.set_popover(Some(&popover));

    let icon_click = icon.clone();
    let label_click = label.clone();
    let st_launch = st.clone();
    split.connect_clicked(move |btn| {
        let is_running = st_launch.borrow().running_games.lock().unwrap().contains_key(&lutris_id);
        if is_running {
            stop_game(&st_launch, lutris_id);
            icon_click.set_icon_name(Some("media-playback-start-symbolic"));
            label_click.set_text("Play");
            btn.add_css_class("suggested-action");
        } else {
            let vid = crate::db::get_default_variant(&st_launch.borrow().db, db_id);
            match launch_game(&st_launch, lutris_id, vid) {
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

    let st_popover = st.clone();
    let popover_clone = popover.clone();
    list.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            let idx = row.index() as usize;
            if idx < variant_ids.len() {
                let vid = variant_ids[idx];
                crate::db::set_default_variant(&st_popover.borrow().db, db_id, vid);
            }
        }
        popover_clone.popdown();
    });

    split.upcast()
}
