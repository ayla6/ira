use gtk4::prelude::*;
use adw::prelude::*;
use crate::AppMessage;
use crate::AppSender;
use crate::Game;
use crate::MergedAchievement;
use crate::GameEntry;
use crate::parser::set_achievement_earned;
use super::state::{SharedState, SAVE_DIR};
use super::enrichment::enrich_game_async;
use super::helpers::confirm_dialog;
use crate::strings as S;

pub fn match_game_to_steam(state: &SharedState, lutris_id: i64, steam_app_id: String, lutris_name: String) {
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::db::upsert_matching(&db, lutris_id, &steam_app_id, "steam", &steam_app_id) {
            eprintln!("match_game_to_steam: upsert_matching failed: {}", e);
            return;
        }
        if let Err(e) = steam.generate_steam_settings(&steam_app_id) {
            eprintln!("match_game_to_steam: generate_steam_settings failed: {}", e);
        }
        match crate::db::find_by_lutris_id(&db, lutris_id) {
            Ok(Some(entry)) => {
                match crate::parser::load_game(&entry, SAVE_DIR) {
                    Ok(mut game) => {
                        if game.name.is_empty() || game.name.starts_with("App ID:") {
                            game.name = lutris_name.clone();
                        }
                        game.lutris_id = lutris_id;
                        let name = game.name.clone();
                        if let Some(ref watcher) = watcher {
                            watcher.watch(&entry, &game.achievements);
                        }
                        let _ = sender.send(AppMessage::NewGame(game));
                        enrich_game_async(
                            steam_app_id.clone(),
                            "steam".to_string(),
                            steam_app_id.clone(),
                            entry.id,
                            lutris_id,
                            name,
                            steam,
                            watcher,
                            sender,
                        );
                    }
                    Err(e) => eprintln!("match_game_to_steam: load_game failed: {}", e),
                }
            }
            Ok(None) => eprintln!("match_game_to_steam: find_by_lutris_id returned None for lutris_id={}", lutris_id),
            Err(e) => eprintln!("match_game_to_steam: find_by_lutris_id error: {}", e),
        }
    });
}

pub fn match_game_to_sgdb(state: &SharedState, lutris_id: i64, sgdb_id: String, lutris_name: String) {
    let steam = state.borrow().steam.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::db::upsert_matching(&db, lutris_id, &sgdb_id, "sgdb", &sgdb_id) {
            eprintln!("match_game_to_sgdb: upsert_matching failed: {}", e);
            return;
        }
        let (icon, hero, grid, logo, header) = steam.ensure_sgdb_assets(&sgdb_id);

        if let Ok(Some(entry)) = crate::db::find_by_lutris_id(&db, lutris_id) {
            let mut game = Game {
                app_id: sgdb_id.clone(),
                kind: "sgdb".to_string(),
                platform_id: sgdb_id.clone(),
                db_id: entry.id,
                name: if entry.title.is_empty() { lutris_name.clone() } else { entry.title.clone() },
                icon_path: icon,
                hero_image_path: hero,
                grid_path: grid,
                header_path: header,
                logo_path: logo,
                achievements: Vec::new(),
                earned_count: 0,
                total_count: 0,
                hidden: entry.hidden,
                lutris_id,
                slug: String::new(),
                playtime: 0.0,
                lastplayed: 0,
                logo_position: entry.logo_position.clone(),
                logo_size: entry.logo_size,
                lutris_name: lutris_name.clone(),
                manual_unmatch: false,
                sort_title: entry.sort_title.clone(),
                game_path: String::new(),
                sgdb_id: String::new(),
                shadps4_version: String::new(),
            };
            let _ = sender.send(AppMessage::NewGame(game));
        }
    });
}

pub fn confirm_mark_unlocked(state: &SharedState, kind: &str, app_id: &str, platform_id: &str, ach: &MergedAchievement, reload: impl Fn() + 'static) {
    let window = state.borrow().window.clone();
    let ach_name = ach.name.clone();
    let kind = kind.to_string();
    let app_id = app_id.to_string();
    let platform_id = platform_id.to_string();
    confirm_dialog(
        &window,
        S::MARK_UNLOCKED,
        &format!(
            "This will mark \u{201C}{}\u{201D} as earned without a real unlock time. \
             Use this only if you already unlocked it previously (e.g. before using this tool).",
            ach.display_name
        ),
        S::MARK_AS_UNLOCKED,
        adw::ResponseAppearance::Destructive,
        move || {
            if let Err(e) = set_achievement_earned(SAVE_DIR, &kind, &app_id, &platform_id, &ach_name, true) {
                eprintln!("Failed to mark achievement as unlocked: {}", e);
                return;
            }
            reload();
        },
    );
}
