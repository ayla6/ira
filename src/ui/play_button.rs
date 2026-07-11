use crate::AppMessage;
use gtk4::prelude::*;
use super::state::SharedState;

pub fn stop_game(state: &SharedState, lutris_id: i64) {
    let pid = state.borrow().running_games.lock().unwrap().remove(&lutris_id);
    if let Some(pid) = pid {
        unsafe { libc::kill(pid, libc::SIGTERM); }
    }
}

pub fn launch_game(state: &SharedState, lutris_id: i64) -> Result<(), String> {
    let (running_games, sender, game_info, global_shadps4_exe, db, save_dir) = {
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
        )
    };

    if running_games.lock().unwrap().contains_key(&lutris_id) {
        return Ok(());
    }

    let (kind, game_path, game_name, per_game_version, db_id) = &game_info;

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
        match std::process::Command::new(exe).arg("-g").arg(game_path).spawn() {
            Ok(child) => {
                let pid = child.id() as i32;
                running_games.lock().unwrap().insert(lutris_id, pid);
                super::helpers::monitor_running_game(
                    sender, running_games, lutris_id, child, db.clone(), *db_id, started_at,
                );
            }
            Err(e) => return Err(format!("Failed to launch shadPS4: {}", e)),
        }
    } else {
        let (launch, wine) = crate::db::get_game_config(&db, *db_id)
            .ok()
            .flatten()
            .unwrap_or_default();

        if !launch.exe.is_empty() {
            let wine_opt = if wine.enabled { Some(&wine) } else { None };
            crate::launcher::launch_game(
                &launch, wine_opt, game_name, sender, *db_id, lutris_id,
                db.clone(), &save_dir, running_games,
            )?;
        } else {
            let uri = format!("lutris:rungameid/{}", lutris_id);
            match std::process::Command::new("lutris").arg(&uri).spawn() {
                Ok(child) => {
                    let pid = child.id() as i32;
                    running_games.lock().unwrap().insert(lutris_id, pid);
                    super::helpers::monitor_running_game(
                        sender, running_games, lutris_id, child, db.clone(), *db_id, started_at,
                    );
                }
                Err(e) => return Err(format!("Failed to launch {}: {}", uri, e)),
            }
        }
    }

    let _ = crate::db::set_last_played(&db, *db_id, started_at);
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
