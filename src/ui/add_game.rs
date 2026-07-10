use crate::api::SteamClient;
use crate::watcher::AchievementWatcher;
use crate::AppMessage;
use crate::AppSender;
use crate::db::DbConn;
use crate::parser::load_game;
use crate::strings as S;

use gtk4::prelude::*;
use adw::prelude::*;
use gio::prelude::*;
use super::state::{SharedState, SAVE_DIR};
use super::enrichment::enrich_game_async;

pub fn show_add_game_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();
    let dialog = gtk4::FileDialog::new();
    dialog.set_title(S::SELECT_GAME_FOLDER);

    let state_clone = state.clone();
    dialog.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let folder = path.to_string_lossy().into_owned();

        if let Some(app_id) = crate::platforms::steam_setup::detect_app_id(&folder) {
            finish_add_game(&state_clone, &folder, &app_id);
        } else if crate::platforms::gog::is_gog_game(&folder) {
            if let Some((_info_dir, product_id, game_name)) = crate::platforms::gog::find_gog_info(&folder) {
                prompt_for_steam_id_gog(&state_clone, &folder, &product_id, &game_name);
            } else {
                prompt_for_app_id(&state_clone, &folder);
            }
        } else {
            prompt_for_app_id(&state_clone, &folder);
        }
    });
}

pub fn prompt_for_steam_id(state: &SharedState, title: &str, body: &str, on_add: impl Fn(&str) + 'static) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(Some(title), Some(body));

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("e.g. 1687950"));
    entry.set_input_purpose(gtk4::InputPurpose::Digits);
    entry.set_margin_top(8);
    entry.set_margin_bottom(8);
    entry.set_margin_start(8);
    entry.set_margin_end(8);
    dialog.set_extra_child(Some(&entry));

    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("add", S::ADD_GAME_BTN);
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response != "add" {
            return;
        }
        on_add(&entry.text());
    });
    dialog.present(Some(&window));
}

pub fn prompt_for_app_id(state: &SharedState, folder: &str) {
    let folder = folder.to_string();
    let state_clone = state.clone();
    prompt_for_steam_id(state, S::ENTER_STEAM_ID, S::ENTER_STEAM_ID_BODY, move |app_id| {
        finish_add_game(&state_clone, &folder, app_id);
    });
}

pub fn prompt_for_steam_id_gog(state: &SharedState, galaxy_folder: &str, product_id: &str, game_name: &str) {
    let galaxy_folder = galaxy_folder.to_string();
    let product_id = product_id.to_string();
    let game_name = game_name.to_string();
    let state_clone = state.clone();
    let body = format!(
        "{}: {}\n{}: {}\n\n{}",
        S::DETECTED_GOG_GAME, game_name,
        S::GOG_PRODUCT_ID, product_id,
        S::ENTER_STEAM_ID_GOG
    );
    prompt_for_steam_id(state, S::ADD_GOG_GAME, &body, move |steam_app_id| {
        finish_add_gog_game(&state_clone, &galaxy_folder, &product_id, &game_name, steam_app_id);
    });
}

fn finalize_added_game(
    app_id: &str,
    kind: &str,
    platform_id: &str,
    steam: std::sync::Arc<SteamClient>,
    watcher: Option<AchievementWatcher>,
    sender: AppSender,
    db: DbConn,
) {
    let entry = match crate::db::find_by_steam_id(&db, app_id) {
        Ok(Some(e)) => e,
        _ => {
            eprintln!("Failed to find game in DB after adding: {}", app_id);
            return;
        }
    };
    match load_game(&entry, SAVE_DIR) {
        Ok(game) => {
            if let Some(ref watcher) = watcher {
                watcher.watch(&entry, &game.achievements);
            }
            let name = game.name.clone();
            let _ = sender.send(AppMessage::NewGame(game));
            enrich_game_async(
                app_id.to_string(),
                kind.to_string(),
                platform_id.to_string(),
                entry.id,
                0,
                name,
                steam,
                watcher,
                sender,
            );
        }
        Err(e) => eprintln!("Failed to load newly added game: {}", e),
    }
}

pub fn finish_add_game(state: &SharedState, folder: &str, app_id: &str) {
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    let folder = folder.to_string();
    let app_id = app_id.to_string();

    std::thread::spawn(move || {
        match crate::platforms::steam_setup::add_game_from_folder(&folder, &app_id, &steam, &db, SAVE_DIR) {
            Ok(_) => finalize_added_game(&app_id, "steam", &app_id, steam, watcher, sender, db),
            Err(e) => {
                eprintln!("Add game failed: {}", e);
                let _ = sender.send(AppMessage::AddGameError(e));
            }
        }
    });
}

pub fn finish_add_gog_game(state: &SharedState, galaxy_folder: &str, product_id: &str, game_name: &str, steam_app_id: &str) {
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    let galaxy_folder = galaxy_folder.to_string();
    let product_id = product_id.to_string();
    let game_name = game_name.to_string();
    let steam_app_id = steam_app_id.to_string();

    std::thread::spawn(move || {
        match crate::platforms::gog_setup::add_gog_game_from_folder(
            &galaxy_folder, &product_id, &game_name, &steam_app_id, &steam, &db, SAVE_DIR,
        ) {
            Ok(_) => finalize_added_game(&steam_app_id, "gog", &product_id, steam, watcher, sender, db),
            Err(e) => {
                eprintln!("GOG add game failed: {}", e);
                let _ = sender.send(AppMessage::AddGameError(e));
            }
        }
    });
}
