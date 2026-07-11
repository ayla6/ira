use crate::api::SteamClient;
use crate::watcher::AchievementWatcher;
use crate::AppMessage;
use crate::AppSender;
use crate::GameEntry;
use crate::models::has_steam_enrichment;
use crate::parser::load_game;

pub fn enrich_game_async(
    app_id: String,
    trophy_source: String,
    platform_id: String,
    db_id: i64,
    lutris_id: i64,
    title: String,
    steam: std::sync::Arc<SteamClient>,
    watcher: Option<AchievementWatcher>,
    sender: AppSender,
    save_dir: String,
) {
    std::thread::spawn(move || {
        let mut entry = GameEntry::for_reload(db_id, "", &trophy_source, &app_id, &platform_id, lutris_id);
        entry.title = title;

        let Ok(mut game) = load_game(&entry, &save_dir) else {
            eprintln!("Failed reloading {}", app_id);
            return;
        };

        if has_steam_enrichment(&trophy_source) {
            let meta_path = crate::parser::achievements_dir(&save_dir, &app_id).join("achievements.json");
            if !meta_path.exists() {
                if let Err(e) = steam.generate_steam_settings(&app_id) {
                    eprintln!("Could not generate achievements for {}: {}", app_id, e);
                }
            }

            if game.name.starts_with("App ID:") {
                if let Some(mut details) = steam.fetch_app_details(&app_id) {
                    if !details.name.is_empty() {
                        game.name = details.name.clone();
                    }
                    if !details.dlcs.is_empty() {
                        steam.ensure_dlc_images(&app_id, &mut details.dlcs);
                        let path = crate::parser::data_dir(&save_dir, &app_id).join("appdetails.json");
                        if let Ok(b) = serde_json::to_vec(&details) {
                            let _ = std::fs::write(&path, b);
                        }
                    }
                }
            } else {
                if let Some(mut details) = steam.fetch_app_details(&app_id) {
                    if !details.dlcs.is_empty() {
                        steam.ensure_dlc_images(&app_id, &mut details.dlcs);
                        let path = crate::parser::data_dir(&save_dir, &app_id).join("appdetails.json");
                        if let Ok(b) = serde_json::to_vec(&details) {
                            let _ = std::fs::write(&path, b);
                        }
                    }
                }
            }

            let has_local_icon = !game.icon_path.is_empty();
            let (icon_path, hero_path) = steam.ensure_assets(&app_id, has_local_icon);
            if game.icon_path.is_empty() && !icon_path.is_empty() {
                game.icon_path = icon_path;
            }
            if game.hero_image_path.is_empty() && !hero_path.is_empty() {
                game.hero_image_path = hero_path;
            }

            let (grid_path, header_path, logo_path) = steam.ensure_grids(&app_id);
            if game.grid_path.is_empty() && !grid_path.is_empty() {
                game.grid_path = grid_path;
            }
            if game.header_path.is_empty() && !header_path.is_empty() {
                game.header_path = header_path;
            }
            if game.logo_path.is_empty() && !logo_path.is_empty() {
                game.logo_path = logo_path;
            }

            if let Some(pcts) = steam.fetch_global_achievements(&app_id) {
                for a in &mut game.achievements {
                    if let Some(&pct) = pcts.get(&a.name) {
                        a.global_percent = pct;
                    }
                }
            }

            if let Some(ref watcher) = watcher {
                watcher.watch(&entry, &game.achievements);
            }
        }

        let _ = sender.send(AppMessage::EnrichedGame(game));
    });
}
