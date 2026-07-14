use crate::api::SteamClient;
use crate::db::DbConn;
use crate::watcher::AchievementWatcher;
use crate::AppMessage;
use crate::AppSender;
use crate::GameEntry;
use crate::models::has_steam_enrichment;
use crate::parser::load_game;
use std::sync::Mutex;

static RA_ENRICH_LOCK: Mutex<()> = Mutex::new(());

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
    db: DbConn,
    ra_username: String,
    ra_token: String,
    ra_password: String,
) {
    std::thread::spawn(move || {
        if trophy_source == crate::models::RA {
            let _guard = RA_ENRICH_LOCK.lock().unwrap();
            enrich_ra(&app_id, &trophy_source, &platform_id, db_id, lutris_id, &title, &save_dir, &sender, &ra_username, &ra_token, &ra_password, &db);
            return;
        }

        let entry = crate::db::find_by_steam_id(&db, &app_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                let mut e = GameEntry::for_reload(db_id, "", &trophy_source, &app_id, &platform_id);
                e.title = title.clone();
                e
            });

        if has_steam_enrichment(&trophy_source) {
            let meta_path = crate::parser::achievements_dir(&save_dir, &app_id).join("achievements.json");
            if !meta_path.exists() {
                if let Err(e) = steam.generate_steam_settings(&app_id) {
                    eprintln!("Could not generate achievements for {}: {}", app_id, e);
                }
            }
        }

        let Ok(mut game) = load_game(&entry, &save_dir) else {
            eprintln!("Failed reloading {}", app_id);
            return;
        };

        if has_steam_enrichment(&trophy_source) {
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

            if game.icon_path.is_empty() && trophy_source == crate::models::STEAM_NATIVE {
                if let Some(png) = fetch_steam_game_icon(&app_id, &save_dir, &steam) {
                    game.icon_path = png;
                }
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

            if let Some(details) = steam.fetch_steam_store_data(&app_id) {
                if let Some(rd) = details.release_date {
                    if !rd.coming_soon {
                        game.release_date = rd.date.clone();
                        game.release_timestamp = crate::parser::parse_steam_release_date(&rd.date);
                    }
                }
                if let Some(mc) = details.metacritic {
                    game.metacritic_score = mc.score as i64;
                }
            }

            if let Some(review) = steam.fetch_steam_reviews(&app_id) {
                game.steam_review_score = review.review_score as i64;
                game.steam_review_count = review.total_reviews as i64;
            }

            let _ = crate::db::store_game_metadata(
                &db,
                game.db_id,
                &game.release_date,
                game.release_timestamp,
                game.metacritic_score,
                game.steam_review_score,
                game.steam_review_count,
            );

            if let Some(ref watcher) = watcher {
                watcher.watch(&entry, &game.achievements);
            }
        }

        let _ = sender.send(AppMessage::EnrichedGame(game));
    });
}

/// Fetch the game icon from Steam's local clienticon cache or CDN.
/// 1. Look up clienticon hash from appinfo.vdf
/// 2. Try local steam/games/<hash>.ico
/// 3. Fall back to Steam CDN download
/// 4. Convert ICO to PNG and save to the game's data directory
fn fetch_steam_game_icon(
    app_id: &str,
    save_dir: &str,
    steam: &std::sync::Arc<crate::api::SteamClient>,
) -> Option<String> {
    let app_id_num: u32 = app_id.parse().ok()?;
    let clienticon = crate::platforms::steam::get_clienticon(app_id_num)?;
    if clienticon.is_empty() { return None; }

    let dest_png = crate::parser::data_dir(save_dir, app_id).join("icon.png");
    if dest_png.is_file() { return Some(dest_png.to_string_lossy().into_owned()); }

    let _ = std::fs::create_dir_all(dest_png.parent()?);

    let ico_in_games = crate::platforms::steam::steam_install_dir()
        .map(|d| d.join("steam").join("games").join(format!("{}.ico", clienticon)));

    let ico_bytes = if let Some(ref path) = ico_in_games {
        if path.is_file() {
            std::fs::read(path).ok()
        } else {
            None
        }
    } else {
        None
    };

    let ico_bytes = match ico_bytes {
        Some(b) => b,
        None => {
            let url = format!("https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/{}/{}.ico", app_id, clienticon);
            let tmp = dest_png.with_extension("tmp_ico");
            if steam.download_file(&url, &tmp).is_err() {
                return None;
            }
            match std::fs::read(&tmp) {
                Ok(b) => { let _ = std::fs::remove_file(&tmp); b }
                Err(_) => { let _ = std::fs::remove_file(&tmp); return None; }
            }
        }
    };

    let tmp_ico = dest_png.with_extension("ico");
    if std::fs::write(&tmp_ico, &ico_bytes).is_err() { return None; }

    match crate::parser::convert_ico_to_png(&tmp_ico) {
        Ok(png_path) => {
            let _ = std::fs::rename(&png_path, &dest_png);
            let _ = std::fs::remove_file(&tmp_ico);
            Some(dest_png.to_string_lossy().into_owned())
        }
        Err(_) => {
            let _ = std::fs::remove_file(&tmp_ico);
            None
        }
    }
}

fn enrich_ra(
    app_id: &str,
    trophy_source: &str,
    platform_id: &str,
    db_id: i64,
    _lutris_id: i64,
    title: &str,
    save_dir: &str,
    sender: &AppSender,
    ra_username: &str,
    ra_token: &str,
    ra_password: &str,
    db: &DbConn,
) {
    if crate::platforms::retroachievements::RaClient::auth_is_broken() {
        return;
    }

    if ra_username.is_empty() || (ra_token.is_empty() && ra_password.is_empty()) {
        eprintln!("RA: skipping enrichment for {} — username_len={} token_len={} password_len={}",
            app_id, ra_username.len(), ra_token.len(), ra_password.len());
        return;
    }

    let entry = crate::db::find_by_steam_id(&db, app_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| GameEntry::for_reload(db_id, crate::models::RETRO, trophy_source, app_id, platform_id));
    let mut game = match load_game(&entry, save_dir) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("RA: failed to load game {}: {}", app_id, e);
            return;
        }
    };
    if !title.is_empty() {
        game.name = title.to_string();
    }

    crate::platforms::retroachievements::enrich_ra_game(&mut game, save_dir, ra_username, ra_token, ra_password);

    let _ = sender.send(AppMessage::EnrichedGame(game));
}
