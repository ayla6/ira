use crate::AppMessage;
use gtk4::prelude::*;
use super::state::SharedState;

pub fn play_button(state: &SharedState, lutris_id: i64) -> gtk4::Button {
    let running_games = state.borrow().running_games.clone();
    let sender = state.borrow().sender.clone();

    let game_info = state.borrow().games.iter()
        .find(|g| g.lutris_id == lutris_id)
        .map(|g| (g.kind.clone(), g.game_path.clone(), g.name.clone(), g.shadps4_version.clone(), g.db_id))
        .unwrap_or_default();

    let global_shadps4_exe = state.borrow().cfg.shadps4_executable.clone();
    let state_c = state.clone();

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
    let rg = running_games.clone();
    let s = sender.clone();
    btn.connect_clicked(move |btn| {
        let uri = format!("lutris:rungameid/{}", lutris_id);
        let mut map = rg.lock().unwrap();
        if let Some(mut child) = map.remove(&lutris_id) {
            drop(map);
            let _ = std::process::Command::new("kill")
                .arg(child.id().to_string())
                .spawn();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            icon_click.set_icon_name(Some("media-playback-start-symbolic"));
            label_click.set_text("Play");
            btn.add_css_class("suggested-action");
            s.send(AppMessage::GameStopped(lutris_id)).ok();
        } else {
            drop(map);
            let (kind, game_path, game_name, per_game_version, db_id) = &game_info;
            if kind == "ps4" {
                let exe = if !per_game_version.is_empty() {
                    per_game_version.as_str()
                } else if !global_shadps4_exe.is_empty() {
                    &global_shadps4_exe
                } else {
                    "shadps4"
                };
                match std::process::Command::new(exe)
                    .arg("-g")
                    .arg(game_path)
                    .spawn()
                {
                    Ok(child) => {
                        rg.lock().unwrap().insert(lutris_id, child);
                        icon_click.set_icon_name(Some("window-close-symbolic"));
                        label_click.set_text("Stop");
                        btn.remove_css_class("suggested-action");

                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;
                        let db_c = state_c.borrow().db.clone();
                        let _ = crate::db::set_last_played(&db_c, *db_id, now);
                        if let Some(g) = state_c.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                            g.lastplayed = now;
                        }

                        let rg_mon = rg.clone();
                        let s_mon = s.clone();
                        let id = lutris_id;
                        std::thread::spawn(move || {
                            loop {
                                std::thread::sleep(std::time::Duration::from_secs(2));
                                let mut map = rg_mon.lock().unwrap();
                                if let Some(child) = map.get_mut(&id) {
                                    match child.try_wait() {
                                        Ok(Some(_)) => {
                                            map.remove(&id);
                                            drop(map);
                                            s_mon.send(AppMessage::GameStopped(id)).ok();
                                            return;
                                        }
                                        Ok(None) => {}
                                        Err(_) => {
                                            map.remove(&id);
                                            drop(map);
                                            s_mon.send(AppMessage::GameStopped(id)).ok();
                                            return;
                                        }
                                    }
                                } else {
                                    return;
                                }
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to launch shadPS4: {}", e);
                    }
                }
            } else {
                let uri = format!("lutris:rungameid/{}", lutris_id);
                match std::process::Command::new("lutris").arg(&uri).spawn() {
                    Ok(child) => {
                        rg.lock().unwrap().insert(lutris_id, child);
                        icon_click.set_icon_name(Some("window-close-symbolic"));
                        label_click.set_text("Stop");
                        btn.remove_css_class("suggested-action");

                        let rg_mon = rg.clone();
                        let s_mon = s.clone();
                        let id = lutris_id;
                        std::thread::spawn(move || {
                            loop {
                                std::thread::sleep(std::time::Duration::from_secs(2));
                                let mut map = rg_mon.lock().unwrap();
                                if let Some(child) = map.get_mut(&id) {
                                    match child.try_wait() {
                                        Ok(Some(_)) => {
                                            map.remove(&id);
                                            drop(map);
                                            s_mon.send(AppMessage::GameStopped(id)).ok();
                                            return;
                                        }
                                        Ok(None) => {}
                                        Err(_) => {
                                            map.remove(&id);
                                            drop(map);
                                            s_mon.send(AppMessage::GameStopped(id)).ok();
                                            return;
                                        }
                                    }
                                } else {
                                    return;
                                }
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to launch {}: {}", uri, e);
                    }
                }
            }
        }
    });

    btn
}
