mod bench;
mod config;
mod gamesetup;
mod images;
mod parser;
mod steam;
mod ui;
mod watcher;

use crate::config::Config;
use crate::parser::{load_games, Game};
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
    WatcherNewGameDir { app_id: String, game_dir: String },
    AddGameError(String),
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
                state.borrow().window.present();
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
    let steam = Arc::new(SteamClient::new(
        cfg.steam_api_key.clone(),
        cfg.steam_griddb_api_key.clone(),
        &format!("{}/data", ui::SAVE_DIR),
    ));

    let games = load_games(ui::SAVE_DIR);

    let (sender, receiver) = mpsc::channel::<AppMessage>();

    let cfg_for_watcher = Arc::new(cfg.clone());
    let watcher = match AchievementWatcher::new(cfg_for_watcher, sender.clone()) {
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
        let app_id = game.app_id.clone();
        let game_dir = format!("{}/{}", ui::SAVE_DIR, app_id);
        if let Some(ref watcher) = watcher {
            watcher.watch(&app_id, &game_dir, &game.achievements);
        }
        enrich_game_async(
            app_id,
            game_dir,
            steam.clone(),
            watcher.clone(),
            sender.clone(),
            false,
        );
    }

    if let Some(ref watcher) = watcher {
        if let Err(e) = watcher.watch_root(ui::SAVE_DIR) {
            eprintln!("Could not watch saves directory for new games: {}", e);
        }
    }

    if std::env::var("AV_BENCH").is_ok() {
        bench::run_bench(state.clone());
    }

    state
}
