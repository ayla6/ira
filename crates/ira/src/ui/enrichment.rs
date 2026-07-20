use ira_api::SteamDataClient;
use ira_db::DbConn;
use ira_watcher::AchievementWatcher;
use crate::AppMessage;
use crate::AppSender;
use crate::Game;
use crate::GameEntry;

use crate::game_loader::load_game;
use std::sync::Mutex;
use std::sync::Arc;

static RA_ENRICH_LOCK: Mutex<()> = Mutex::new(());

pub struct EnrichGameParams {
    pub app_id: String,
    pub trophy_source: ira_models::TrophySource,
    pub platform_id: String,
    pub db_id: i64,
    pub title: String,
    pub steam: Arc<SteamDataClient>,
    pub watcher: Option<AchievementWatcher>,
    pub sender: AppSender,
    pub save_dir: String,
    pub db: DbConn,
    pub ra_username: String,
    pub ra_token: String,
    pub ra_password: String,
    /// If provided, use this game instead of calling load_game (skips achievement loading).
    /// Pass Some for background enrichment, None for on-demand loading.
    pub game: Option<Game>,
}

pub fn enrich_game_async(params: EnrichGameParams) {
    let EnrichGameParams { app_id, trophy_source, platform_id, db_id, title, steam, watcher, sender, save_dir, db, ra_username, ra_token, ra_password, game } = params;
    std::thread::spawn(move || {
        let _s = tracing::info_span!("enrich_game_async", app_id = %app_id, db_id = db_id).entered();
        if trophy_source == ira_models::TrophySource::Ra {
            let _guard = RA_ENRICH_LOCK.lock().unwrap();
            enrich_ra(EnrichRaParams {
                app_id: &app_id, trophy_source: trophy_source.as_str(), platform_id: &platform_id,
                db_id, title: &title, save_dir: &save_dir, sender: &sender,
                ra_username: &ra_username, ra_token: &ra_token, ra_password: &ra_password, db: &db,
            });
            return;
        }

        let entry = ira_db::find_by_db_id(&db, db_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                let mut e = GameEntry::for_reload(db_id, ira_models::GameKind::Other, trophy_source, &app_id, "", &platform_id);
                e.title = title.clone();
                e
            });

        if trophy_source.has_steam_enrichment() {
            let meta_path = ira_parser::achievements_dir(&save_dir, &app_id).join("achievements.json");
            if !meta_path.exists() {
                if let Err(e) = steam.generate_steam_settings(&app_id) {
                    eprintln!("Could not generate achievements for {}: {}", app_id, e);
                    if let Some(parent) = meta_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&meta_path, "[]");
                }
            }
        }

        let game_provided = game.is_some();
        let mut game = if let Some(g) = game {
            g
        } else {
            match load_game(&entry, &save_dir) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("Failed reloading {}: {}", app_id, e);
                    return;
                }
            }
        };

        if !game_provided && trophy_source.has_steam_enrichment() {
            let has_missing_icons = game.achievements.iter().any(|a| a.icon_path.is_empty());
            if has_missing_icons {
                let _s = tracing::info_span!("enrich_redownload_icons", app_id = %app_id).entered();
                if let Err(e) = steam.generate_steam_settings(&app_id) {
                    eprintln!("Could not re-download achievement icons for {}: {}", app_id, e);
                } else if let Ok(reloaded) = load_game(&entry, &save_dir) {
                    game.achievements = reloaded.achievements;
                }
            }
        }

        if trophy_source.has_steam_enrichment() {
            let appdetails_path = ira_parser::data_dir(&save_dir, &app_id).join("appdetails.json");
            let needs_app_details = game.name.starts_with("App ID:") || !appdetails_path.exists();
            if needs_app_details {
                if let Some(mut details) = steam.fetch_app_details(&app_id) {
                    if !details.name.is_empty() && game.name.starts_with("App ID:") {
                        game.name = details.name.clone();
                    }
                    if !details.dlcs.is_empty() {
                        steam.ensure_dlc_images(&app_id, &mut details.dlcs);
                        let path = ira_parser::data_dir(&save_dir, &app_id).join("appdetails.json");
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

            if game.icon_path.is_empty() && trophy_source == ira_models::TrophySource::SteamNative {
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

            if !game.achievements.is_empty() && game.achievements.iter().any(|a| a.global_percent == 0.0) {
                if let Some(pcts) = steam.fetch_global_achievements(&app_id) {
                    for a in &mut game.achievements {
                        if let Some(&pct) = pcts.get(&a.name) {
                            a.global_percent = pct;
                        }
                    }
                }
            }

            if game.release_date.is_empty() || game.metacritic_score < 0 {
                if let Some(details) = steam.fetch_steam_store_data(&app_id) {
                    if let Some(rd) = details.release_date {
                        if !rd.coming_soon && game.release_date.is_empty() {
                            game.release_date = rd.date.clone();
                            game.release_timestamp = ira_parser::parse_steam_release_date(&rd.date);
                        }
                    }
                    if let Some(mc) = details.metacritic {
                        if game.metacritic_score < 0 {
                            game.metacritic_score = mc.score as i64;
                        }
                    }
                }
            }

            if game.steam_review_score < 0 {
                if let Some(review) = steam.fetch_steam_reviews(&app_id) {
                    game.steam_review_score = review.review_score as i64;
                    game.steam_review_count = review.total_reviews as i64;
                }
            }

            if let Err(e) = ira_db::store_game_metadata(
                &db,
                game.db_id,
                &game.release_date,
                game.release_timestamp,
                game.metacritic_score,
                game.steam_review_score,
                game.steam_review_count,
            ) {
                eprintln!("Failed to store game metadata: {}", e);
            }

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
/// 4. Convert ICO to WebP and save to the game's data directory
fn fetch_steam_game_icon(
    app_id: &str,
    save_dir: &str,
    steam: &std::sync::Arc<ira_api::SteamDataClient>,
) -> Option<String> {
    let _s = tracing::info_span!("fetch_steam_game_icon", app_id = %app_id).entered();
    let app_id_num: u32 = app_id.parse().ok()?;
    let clienticon = ira_platforms::steam::get_clienticon(app_id_num)?;
    if clienticon.is_empty() { return None; }

    let dest_webp = ira_parser::data_dir(save_dir, app_id).join("icon.webp");
    if dest_webp.is_file() { return Some(dest_webp.to_string_lossy().into_owned()); }

    let _ = std::fs::create_dir_all(dest_webp.parent()?);

    let ico_in_games = ira_platforms::steam::steam_install_dir()
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
            let tmp = dest_webp.with_extension("tmp_ico");
            if steam.download_file(&url, &tmp).is_err() {
                return None;
            }
            match std::fs::read(&tmp) {
                Ok(b) => { let _ = std::fs::remove_file(&tmp); b }
                Err(_) => { let _ = std::fs::remove_file(&tmp); return None; }
            }
        }
    };

    let tmp_ico = dest_webp.with_extension("ico");
    if std::fs::write(&tmp_ico, &ico_bytes).is_err() { return None; }

    ira_parser::convert_to_lossless_webp(&tmp_ico);
    if dest_webp.is_file() {
        Some(dest_webp.to_string_lossy().into_owned())
    } else if tmp_ico.is_file() {
        Some(tmp_ico.to_string_lossy().into_owned())
    } else {
        None
    }
}

struct EnrichRaParams<'a> {
    app_id: &'a str,
    trophy_source: &'a str,
    platform_id: &'a str,
    db_id: i64,
    title: &'a str,
    save_dir: &'a str,
    sender: &'a AppSender,
    ra_username: &'a str,
    ra_token: &'a str,
    ra_password: &'a str,
    db: &'a DbConn,
}

fn enrich_ra(params: EnrichRaParams) {
    let EnrichRaParams { app_id, trophy_source, platform_id, db_id, title, save_dir, sender, ra_username, ra_token, ra_password, db } = params;
    let _s = tracing::info_span!("enrich_ra", app_id = %app_id, db_id = db_id).entered();
    if ira_platforms::retroachievements::RaClient::auth_is_broken() {
        return;
    }

    if ra_username.is_empty() || (ra_token.is_empty() && ra_password.is_empty()) {
        eprintln!("RA: skipping enrichment for {} — username_len={} token_len={} password_len={}",
            app_id, ra_username.len(), ra_token.len(), ra_password.len());
        return;
    }

    let entry = ira_db::find_by_db_id(db, db_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| GameEntry::for_reload(db_id, ira_models::GameKind::Retro, ira_models::TrophySource::from_string(trophy_source), "", app_id, platform_id));
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

    ira_platforms::retroachievements::enrich_ra_game(&mut game, save_dir, ra_username, ra_token, ra_password);

    let _ = sender.send(AppMessage::EnrichedGame(game));
}
