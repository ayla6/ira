use gtk4::prelude::*;
use adw::prelude::*;
use crate::strings as S;
use super::state::SharedState;
use super::sidebar::rebuild_sidebar;

pub fn show_create_group_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(Some("New Collection"), Some("Enter a name for the collection:"));

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("Collection name"));
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    entry.set_margin_top(8);
    entry.set_margin_bottom(8);
    dialog.set_extra_child(Some(&entry));

    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("create", "Create");
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    let entry_clone = entry.clone();
    let state_clone = state.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp != "create" {
            return;
        }
        let name = entry_clone.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let db = state_clone.borrow().db.clone();
        match ira_db::create_group(&db, &name) {
            Ok(_) => {
                let groups = ira_db::get_all_groups(&db).unwrap_or_default();
                state_clone.borrow_mut().groups = groups;
                rebuild_sidebar(&state_clone);
            }
            Err(e) => {
                eprintln!("Failed to create group: {}", e);
            }
        }
    });

    dialog.present(Some(&window));
}

pub fn show_rename_group_dialog(state: &SharedState, group_id: i64, current_name: &str) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(Some("Rename Collection"), Some("Enter a new name:"));

    let entry = gtk4::Entry::new();
    entry.set_text(current_name);
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    entry.set_margin_top(8);
    entry.set_margin_bottom(8);
    dialog.set_extra_child(Some(&entry));

    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("rename", "Rename");
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");

    let entry_clone = entry.clone();
    let state_clone = state.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp != "rename" {
            return;
        }
        let name = entry_clone.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let db = state_clone.borrow().db.clone();
        match ira_db::rename_group(&db, group_id, &name) {
            Ok(_) => {
                let groups = ira_db::get_all_groups(&db).unwrap_or_default();
                state_clone.borrow_mut().groups = groups;
                rebuild_sidebar(&state_clone);
            }
            Err(e) => {
                eprintln!("Failed to rename group: {}", e);
            }
        }
    });

    dialog.present(Some(&window));
}

pub fn show_delete_group_dialog(state: &SharedState, group_id: i64, name: &str) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(
        Some("Delete Collection"),
        Some(&format!("Delete \"{}\"? Games in this collection will not be removed.", name)),
    );

    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let state_clone = state.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp != "delete" {
            return;
        }
        let db = state_clone.borrow().db.clone();
        match ira_db::delete_group(&db, group_id) {
            Ok(_) => {
                let groups = ira_db::get_all_groups(&db).unwrap_or_default();
                state_clone.borrow_mut().groups = groups;
                state_clone.borrow_mut().group_members.remove(&group_id);
                state_clone.borrow_mut().selected_group = ira_models::GroupSelection::AllGames;
                let selected_id = state_clone.borrow().selected_id.clone();
                rebuild_sidebar(&state_clone);
                if selected_id.is_empty() {
                    super::grid_view::show_grid_view(&state_clone);
                }
            }
            Err(e) => {
                eprintln!("Failed to delete group: {}", e);
            }
        }
    });

    dialog.present(Some(&window));
}
