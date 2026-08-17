use crate::AppMessage;
use crate::AppSender;
use crate::Game;
use crate::GameEntry;
use ira_api::SteamDataClient;
use ira_db::DbConn;

use crate::game_loader::load_game;
use std::sync::Arc;
use std::sync::Mutex;

static RA_ENRICH_LOCK: Mutex<()> = Mutex::new(());

pub struct EnrichGameParams {
    pub app_id: String,
    pub trophy_source: ira_models::TrophySource,
    pub platform_id: String,
    pub db_id: i64,
    pub title: String,
    pub steam: Arc<SteamDataClient>,
    pub sender: AppSender,
    pub save_dir: String,
    pub db: DbConn,
    pub ra_username: String,
    pub ra_web_api_key: String,
    /// If provided, use this game instead of calling load_game (skips achievement loading).
    /// Pass Some for background enrichment, None for on-demand loading.
    pub game: Option<Game>,
}

pub fn enrich_game_async(params: EnrichGameParams) {
    std::thread::spawn(move || {
        enrich_game_blocking(params);
    });
}

pub fn enrich_game_blocking(params: EnrichGameParams) {
    let EnrichGameParams {
        app_id,
        trophy_source,
        platform_id,
        db_id,
        title,
        steam,
        sender,
        save_dir,
        db,
        ra_username,
        ra_web_api_key,
        game,
    } = params;
    let _s = tracing::info_span!("enrich_game", app_id = %app_id, db_id = db_id).entered();
    if trophy_source == ira_models::TrophySource::Ra {
        let _guard = RA_ENRICH_LOCK.lock().unwrap();
        enrich_ra(EnrichRaParams {
            app_id: &app_id,
            trophy_source: trophy_source.as_str(),
            platform_id: &platform_id,
            db_id,
            title: &title,
            save_dir: &save_dir,
            sender: &sender,
            ra_username: &ra_username,
            ra_web_api_key: &ra_web_api_key,
            db: &db,
        });
        return;
    }

    let entry = ira_db::find_by_db_id(&db, db_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            let mut e = GameEntry::for_reload(
                db_id,
                ira_models::GameKind::Other,
                trophy_source,
                &app_id,
                "",
                &platform_id,
            );
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
                eprintln!(
                    "Could not re-download achievement icons for {}: {}",
                    app_id, e
                );
            } else if let Ok(reloaded) = load_game(&entry, &save_dir) {
                game.achievements = reloaded.achievements;
            }
        }
    }

    if trophy_source.has_steam_enrichment() {
        let appdetails_path = ira_parser::data_dir(&save_dir, &app_id).join("appdetails.json");
        let needs_app_details = game.name.starts_with("App ID:") || !appdetails_path.exists();
        if needs_app_details {
            if let Some(details) = steam.fetch_app_details(&app_id) {
                if !details.name.is_empty() && game.name.starts_with("App ID:") {
                    game.set_name(&details.name);
                }
            }
        }

        let has_local_icon = !game.icon_path.is_empty();
        if game.icon_path.is_empty() || game.hero_image_path.is_empty() {
            let (icon_path, hero_path) = steam.ensure_assets(&app_id, has_local_icon);
            if game.icon_path.is_empty() && !icon_path.is_empty() {
                game.icon_path = icon_path;
            }
            if game.hero_image_path.is_empty() && !hero_path.is_empty() {
                game.hero_image_path = hero_path;
            }
        }

        if game.icon_path.is_empty() && trophy_source.has_steam_enrichment() {
            if let Some(png) = fetch_steam_game_icon(&app_id, &save_dir, &steam) {
                game.icon_path = png;
            }
        }

        if game.grid_path.is_empty() || game.header_path.is_empty() || game.logo_path.is_empty() {
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
        }

        if !game.achievements.is_empty()
            && game.achievements.iter().any(|a| a.global_percent == 0.0)
        {
            if let Some(pcts) = steam.fetch_global_achievements(&app_id) {
                for a in &mut game.achievements {
                    if let Some(&pct) = pcts.get(&a.name) {
                        a.global_percent = pct;
                    }
                }
            }
        }

        if game.release_date.is_empty() || game.metacritic_score < 0 || game.steam_review_score < 0
        {
            if let Some(cmd) = steam.fetch_steamcmd_info(&app_id) {
                if game.release_timestamp == 0 && cmd.release_timestamp > 0 {
                    game.release_timestamp = cmd.release_timestamp;
                }
                if game.metacritic_score < 0 && cmd.metacritic_score >= 0 {
                    game.metacritic_score = cmd.metacritic_score;
                }
                if game.steam_review_score < 0 && cmd.review_score >= 0 {
                    game.steam_review_score = cmd.review_score;
                    game.steam_review_count = cmd.review_percentage;
                }
                if game.name.starts_with("App ID:") && !cmd.name.is_empty() {
                    game.set_name(&cmd.name);
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
    }

    let _ = sender.send(AppMessage::EnrichedGame(game));
}

/// Fetch the game icon from Steam's local clienticon cache or CDN.
/// 1. Look up clienticon hash from cached steamcmd.net data, fall back to appinfo.vdf
/// 2. Try local steam/games/<hash>.ico
/// 3. Fall back to Steam CDN download
/// 4. Convert ICO to WebP and save to the game's data directory
fn fetch_steam_game_icon(
    app_id: &str,
    save_dir: &str,
    steam: &std::sync::Arc<ira_api::SteamDataClient>,
) -> Option<String> {
    let _s = tracing::info_span!("fetch_steam_game_icon", app_id = %app_id).entered();
    let clienticon = steam.cached_clienticon(app_id).or_else(|| {
        let app_id_num: u32 = app_id.parse().ok()?;
        ira_platforms::steam::get_clienticon(app_id_num)
    })?;
    if clienticon.is_empty() {
        return None;
    }

    let dest_webp = ira_parser::data_dir(save_dir, app_id).join("icon.webp");
    if dest_webp.is_file() {
        return Some(dest_webp.to_string_lossy().into_owned());
    }

    let _ = std::fs::create_dir_all(dest_webp.parent()?);

    let ico_in_games = ira_platforms::steam::steam_install_dir().map(|d| {
        d.join("steam")
            .join("games")
            .join(format!("{}.ico", clienticon))
    });

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
            match steam.download_bytes(&url) {
                Ok(b) => b,
                Err(_) => return None,
            }
        }
    };

    let webp = ira_parser::convert_bytes_to_lossless_webp(&ico_bytes)?;
    std::fs::write(&dest_webp, &webp).ok()?;
    Some(dest_webp.to_string_lossy().into_owned())
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
    ra_web_api_key: &'a str,
    db: &'a DbConn,
}

fn enrich_ra(params: EnrichRaParams) {
    let EnrichRaParams {
        app_id,
        trophy_source,
        platform_id,
        db_id,
        title,
        save_dir,
        sender,
        ra_username,
        ra_web_api_key,
        db,
    } = params;
    let _s = tracing::info_span!("enrich_ra", app_id = %app_id, db_id = db_id).entered();
    if ra_username.is_empty() || ra_web_api_key.is_empty() {
        eprintln!(
            "RA: skipping enrichment for {} — username_len={} web_api_key_len={}",
            app_id,
            ra_username.len(),
            ra_web_api_key.len()
        );
        return;
    }

    let entry = ira_db::find_by_db_id(db, db_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            GameEntry::for_reload(
                db_id,
                ira_models::GameKind::Retro,
                ira_models::TrophySource::from_string(trophy_source),
                "",
                app_id,
                platform_id,
            )
        });
    let mut game = match load_game(&entry, save_dir) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("RA: failed to load game {}: {}", app_id, e);
            return;
        }
    };
    if !title.is_empty() {
        game.set_name(title);
    }

    ira_platforms::retroachievements::enrich_ra_game(
        &mut game,
        save_dir,
        ra_username,
        ra_web_api_key,
    );

    let _ = sender.send(AppMessage::EnrichedGame(game));
}
