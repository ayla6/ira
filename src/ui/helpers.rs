use adw::prelude::{AlertDialogExt, AdwDialogExt};
use gtk4::prelude::*;
use crate::Game;
use crate::strings as S;
use crate::models::{AppMessage, AppSender};
use std::collections::HashMap;
use std::process::Child;
use std::sync::{Arc, Mutex};

use super::state::SharedState;

pub fn merge_game_enrichment(existing: &Game, updated: &mut Game) {
    if !existing.name.is_empty() && !existing.name.starts_with("App ID:") {
        updated.name = existing.name.clone();
    }

    updated.hidden = existing.hidden;
    updated.lutris_id = existing.lutris_id;
    updated.slug = existing.slug.clone();
    updated.playtime = existing.playtime;
    updated.lastplayed = existing.lastplayed;
    updated.lutris_name = existing.lutris_name.clone();
    updated.manual_unmatch = existing.manual_unmatch;
    updated.sort_title = existing.sort_title.clone();
    if updated.icon_path.is_empty() {
        updated.icon_path = existing.icon_path.clone();
    }
    if updated.hero_image_path.is_empty() {
        updated.hero_image_path = existing.hero_image_path.clone();
    }
    if updated.grid_path.is_empty() {
        updated.grid_path = existing.grid_path.clone();
    }
    if updated.header_path.is_empty() {
        updated.header_path = existing.header_path.clone();
    }
    if updated.logo_path.is_empty() {
        updated.logo_path = existing.logo_path.clone();
    }
    if updated.logo_position.is_empty() {
        updated.logo_position = existing.logo_position.clone();
    }
    if updated.logo_size == 0 {
        updated.logo_size = existing.logo_size;
    }

    if !existing.achievements.is_empty() {
        let existing_pcts: HashMap<String, f64> = existing
            .achievements
            .iter()
            .map(|a| (a.name.clone(), a.global_percent))
            .collect();
        for a in &mut updated.achievements {
            if a.global_percent == 0.0 {
                if let Some(&pct) = existing_pcts.get(&a.name) {
                    a.global_percent = pct;
                }
            }
        }
    }
}

pub fn open_folder(path: &str) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

pub fn confirm_dialog(
    parent: &adw::ApplicationWindow,
    title: &str,
    body: &str,
    confirm_label: &str,
    appearance: adw::ResponseAppearance,
    on_confirm: impl Fn() + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(body));
    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("confirm", confirm_label);
    dialog.set_response_appearance("confirm", appearance);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, resp| {
        if resp == "confirm" {
            on_confirm();
        }
    });
    dialog.present(Some(parent));
}

pub trait Clearable {
    fn clear_all_children(&self);
}

impl Clearable for gtk4::Box {
    fn clear_all_children(&self) {
        while let Some(child) = self.first_child() {
            self.remove(&child);
        }
    }
}

impl Clearable for gtk4::ListBox {
    fn clear_all_children(&self) {
        while let Some(child) = self.first_child() {
            self.remove(&child);
        }
    }
}

impl Clearable for gtk4::FlowBox {
    fn clear_all_children(&self) {
        while let Some(child) = self.first_child() {
            self.remove(&child);
        }
    }
}

pub fn clear_children(w: &impl Clearable) {
    w.clear_all_children();
}

pub fn refresh_settings_images_page(
    state: &SharedState,
    db_id: i64,
    build_page: impl Fn(&SharedState, &Game, &adw::Window) -> gtk4::Widget,
) {
    if let Some((ref sw, ref ss, sdb_id)) = state.borrow().settings_data.clone() {
        if sdb_id == db_id && sw.is_visible() {
            if let Some(old) = ss.child_by_name("images") {
                ss.remove(&old);
            }
            if let Some(game) = state.borrow().games.iter().find(|g| g.db_id == db_id).cloned() {
                let new_page = build_page(state, &game, &sw);
                ss.add_named(&new_page, Some("images"));
            }
        }
    }
}

pub fn monitor_running_game(
    sender: AppSender,
    running: Arc<Mutex<HashMap<i64, Child>>>,
    lutris_id: i64,
    child: Child,
) {
    running.lock().unwrap().insert(lutris_id, child);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let mut map = running.lock().unwrap();
            if let Some(ch) = map.get_mut(&lutris_id) {
                match ch.try_wait() {
                    Ok(Some(_)) | Err(_) => {
                        map.remove(&lutris_id);
                        drop(map);
                        sender.send(AppMessage::GameStopped(lutris_id)).ok();
                        return;
                    }
                    Ok(None) => {}
                }
            } else {
                return;
            }
        }
    });
}
