mod bench;
mod config;
mod db;
mod gamesetup;
mod images;
mod parser;
mod steam;
mod strings;
mod ui;
mod watcher;

use crate::db::GameEntry;
use crate::parser::Game;
use crate::steam::SteamClient;
use crate::ui::{build_ui, enrich_game_async, handle_app_message, restore_content, SharedState};
use crate::watcher::AchievementWatcher;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

pub enum AppMessage {
    EnrichedGame(Game),
    NewGame(Game),
    WatcherGameUpdated(Game),
    AddGameError(String),
    GameRemoved { app_id: String },
}

fn main() {
    let app = adw::Application::new(
        Some("com.github.achievement.viewer"),
        gio::ApplicationFlags::empty(),
    );

    let state_holder: Rc<RefCell<Option<SharedState>>> = Rc::new(RefCell::new(None));

    app.connect_activate({
        let state_holder = state_holder.clone();
        move |app| {
            if let Some(state) = state_holder.borrow().as_ref() {
                let win = state.borrow().window.clone();
                win.present();
                restore_content(state);
                return;
            }
            let state = activate(app);
            *state_holder.borrow_mut() = Some(state);
        }
    });

    app.run();
}

fn activate(app: &adw::Application) -> SharedState {
    let cfg = config::load_config();

    let db = db::init_db(&format!("{}/gse.db", ui::SAVE_DIR));

    // If DB is empty, populate from existing directory structure
    if db::load_all_games(&db).map(|v| v.is_empty()).unwrap_or(true) {
        populate_db_from_dirs(&db, ui::SAVE_DIR);
    } else {
        // DB exists but some titles may be empty — re-scan to fill gaps
        populate_db_from_dirs(&db, ui::SAVE_DIR);
    }

    let steam = Arc::new(SteamClient::new(
        cfg.steam_api_key.clone(),
        cfg.steam_griddb_api_key.clone(),
        &format!("{}/data", ui::SAVE_DIR),
    ));

    let games = parser::load_games(&db, ui::SAVE_DIR);

    let (sender, receiver) = mpsc::channel::<AppMessage>();

    let cfg_for_watcher = Arc::new(cfg.clone());
    let watcher = match AchievementWatcher::new(cfg_for_watcher, sender.clone(), ui::SAVE_DIR.to_string()) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("Live achievement watching unavailable: {}", e);
            None
        }
    };

    let game_names = watcher.as_ref().map(|w| w.game_names()).unwrap_or_else(|| {
        Arc::new(Mutex::new(std::collections::HashMap::new()))
    });

    for g in &games {
        game_names.lock().unwrap().insert(g.app_id.clone(), g.name.clone());
    }

    let state = build_ui(
        app,
        games,
        cfg,
        steam.clone(),
        watcher.clone(),
        db.clone(),
        sender.clone(),
        game_names,
    );

    let receiver = RefCell::new(receiver);
    let state_clone = state.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        while let Ok(msg) = receiver.borrow_mut().try_recv() {
            handle_app_message(&state_clone, msg);
        }
        glib::ControlFlow::Continue
    });

    let games_snapshot: Vec<Game> = state.borrow().games.clone();
    for game in &games_snapshot {
        if let Some(ref watcher) = watcher {
            let entry = GameEntry {
                id: game.db_id,
                kind: game.kind.clone(),
                steam_id: game.app_id.clone(),
                platform_id: game.platform_id.clone(),
                title: game.name.clone(),
                lutris_id: None,
            };
            watcher.watch(&entry, &game.achievements);
        }
        enrich_game_async(
            game.app_id.clone(),
            game.kind.clone(),
            game.platform_id.clone(),
            game.db_id,
            steam.clone(),
            watcher.clone(),
            sender.clone(),
        );
    }

    if std::env::var("AV_BENCH").is_ok() {
        bench::run_bench(state.clone());
    }

    state
}

fn populate_db_from_dirs(db: &db::DbConn, save_dir: &str) {
    use std::path::Path;

    let read_title = |app_id: &str| -> String {
        let data_dir = Path::new(save_dir).join("data").join(app_id);
        if let Ok(data) = std::fs::read(data_dir.join("appdetails.json")) {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&data) {
                if let Some(name) = json.get("name").and_then(|v| v.as_str()) {
                    if !name.is_empty() {
                        return name.to_string();
                    }
                }
            }
        }
        String::new()
    };

    let steam_dir = format!("{}/steam", save_dir);
    if let Ok(entries) = std::fs::read_dir(&steam_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let app_id = match entry.file_name().to_str() {
                Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                _ => continue,
            };
            let title = read_title(&app_id);
            let _ = db::add_game(db, "steam", &app_id, &app_id, &title);
        }
    }

    let gog_dir = format!("{}/gog", save_dir);
    if let Ok(galaxy_entries) = std::fs::read_dir(&gog_dir) {
        for galaxy_entry in galaxy_entries.flatten() {
            if !galaxy_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let galaxy_path = galaxy_entry.path();
            if let Ok(product_entries) = std::fs::read_dir(&galaxy_path) {
                for product_entry in product_entries.flatten() {
                    if !product_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    let product_dir = product_entry.path();
                    let product_id = match product_entry.file_name().to_str() {
                        Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                        _ => continue,
                    };
                    let app_id = match std::fs::read_to_string(product_dir.join("steam_appid.txt")) {
                        Ok(s) => s.trim().to_string(),
                        Err(_) => continue,
                    };
                    if app_id.parse::<i64>().is_err() {
                        continue;
                    }
                    let title = read_title(&app_id);
                    let _ = db::add_game(db, "gog", &app_id, &product_id, &title);
                }
            }
        }
    }
}
