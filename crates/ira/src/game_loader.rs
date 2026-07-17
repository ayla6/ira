use ira_db::DbConn;
use ira_models::{AchievementStatus, AppDetails, Game, GameEntry, MergedAchievement};
use std::collections::HashMap;

pub fn read_app_details(save_dir: &str, app_id: &str) -> Option<AppDetails> {
    let path = ira_parser::data_dir(save_dir, app_id).join("appdetails.json");
    let data = std::fs::read(&path).ok()?;
    serde_json::from_slice(&data).ok()
}

pub fn load_games(conn: &DbConn, save_dir: &str) -> Vec<Game> {
    let _span = tracing::info_span!("load_games").entered();
    let entries = match ira_db::load_all_games(conn) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to load games from DB: {}", e);
            return Vec::new();
        }
    };

    let mut games = Vec::new();
    for entry in entries {
        if entry.kind != ira_models::GameKind::Linux && entry.kind != ira_models::GameKind::Wine {
            continue;
        }
        let app_id = if !entry.steam_id.is_empty() { &entry.steam_id } else { &entry.game_id };
        let _s = tracing::info_span!("load_game", app_id).entered();
        match load_game(&entry, save_dir) {
            Ok(game) => games.push(game),
            Err(e) => eprintln!("Skipping game {} ({}): {}", if !entry.steam_id.is_empty() { &entry.steam_id } else { &entry.game_id }, entry.kind, e),
        }
    }
    games.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
    games
}

pub fn load_game(entry: &GameEntry, save_dir: &str) -> Result<Game, String> {
    let app_id = if !entry.steam_id.is_empty() { &entry.steam_id } else { &entry.game_id };
    let kind = entry.kind;
    let platform_id = &entry.platform_id;

    let mut game = Game {
        app_id: app_id.to_string(),
        kind,
        trophy_source: entry.trophy_source,
        platform_id: platform_id.to_string(),
        db_id: entry.id,
        name: if entry.title.is_empty() {
            format!("App ID: {}", app_id)
        } else {
            entry.title.clone()
        },
        icon_path: String::new(),
        hero_image_path: String::new(),
        grid_path: String::new(),
        header_path: String::new(),
        logo_path: String::new(),
        achievements: Vec::new(),
        earned_count: 0,
        total_count: 0,
        hidden: entry.hidden,
        slug: String::new(),
        playtime: entry.playtime,
        last_played: entry.last_played,
        logo_position: entry.logo_position.clone(),
        logo_size: entry.logo_size,
        manual_unmatch: entry.manual_unmatch,
        sort_title: entry.sort_title.clone(),
        game_path: String::new(),
        sgdb_id: entry.sgdb_id.clone().unwrap_or_default(),
        shadps4_version: entry.shadps4_version.clone(),
        release_date: entry.release_date.clone(),
        release_timestamp: entry.release_timestamp,
        metacritic_score: entry.metacritic_score,
        steam_review_score: entry.steam_review_score,
        steam_review_count: entry.steam_review_count,
        ra_core: entry.ra_core.clone(),
        emulator_override: entry.emulator_override.clone(),
        rom_path: entry.rom_path.clone(),
    };

    let ach_dir = ira_parser::achievements_dir(save_dir, app_id);

    if entry.title.is_empty() {
        if let Some(name) = ira_parser::read_app_name(save_dir, app_id) {
            game.name = name;
        }
    }

    let image_dir = if kind == ira_models::GameKind::Retro {
        ira_parser::retro_data_dir(save_dir, entry.id)
    } else if entry.trophy_source.has_steam_enrichment() {
        ira_parser::data_dir(save_dir, app_id)
    } else if entry.sgdb_id.as_deref().filter(|s| !s.is_empty()).is_some() {
        ira_parser::sgdb_data_dir(save_dir, entry.sgdb_id.as_deref().unwrap())
    } else {
        ira_parser::data_dir(save_dir, app_id)
    };

    {
        let _s = tracing::info_span!("find_images").entered();
        if let Some(icon_path) = ira_parser::find_image_file(&image_dir, "icon") {
            game.icon_path = icon_path.to_string_lossy().into_owned();
        } else {
            let ra_icon = ira_parser::ra_icon_path(save_dir, app_id);
            if ra_icon.is_file() {
                game.icon_path = ra_icon.to_string_lossy().into_owned();
            }
            let icon_ico = image_dir.join("icon.ico");
            if icon_ico.is_file() {
                let webp = icon_ico.with_extension("webp");
                ira_parser::convert_to_lossless_webp(&icon_ico);
                if webp.is_file() {
                    game.icon_path = webp.to_string_lossy().into_owned();
                } else {
                    game.icon_path = icon_ico.to_string_lossy().into_owned();
                }
            } else {
                for ext in ["jpg", "webp"] {
                    let p = image_dir.join(format!("icon.{}", ext));
                    if p.is_file() {
                        game.icon_path = p.to_string_lossy().into_owned();
                        break;
                    }
                }
            }
        }

        ira_parser::populate_image_paths(&image_dir, &mut game);
    }

    let is_steam_native = entry.trophy_source == ira_models::TrophySource::SteamNative;
    let steam_native_data = if is_steam_native {
        ira_platforms::steam::read_steam_achievements_full(app_id, save_dir)
    } else {
        ira_platforms::steam::SteamAchievementData { achievements: Vec::new(), n_total: 0, n_achieved: 0 }
    };

    if entry.trophy_source == ira_models::TrophySource::Ra {
        let _s = tracing::info_span!("load_ra_achievements_from_cache").entered();
        game.achievements = ira_platforms::retroachievements::load_ra_achievements_from_cache(save_dir, app_id);
        game.total_count = game.achievements.len();
        game.earned_count = game.achievements.iter().filter(|a| a.earned).count();
        return Ok(game);
    }

    let meta_path = ach_dir.join("achievements.json");
    let has_meta = meta_path.is_file();

    let status_map = if is_steam_native {
        let mut map: HashMap<String, AchievementStatus> = ira_platforms::steam::read_user_stats(app_id)
            .into_iter()
            .map(|(name, (earned, earned_time))| (name, AchievementStatus { earned, earned_time }))
            .collect();
        for ach in &steam_native_data.achievements {
            map.entry(ach.id.clone())
                .and_modify(|s| { if s.earned_time == 0 { s.earned_time = ach.earned_time; } })
                .or_insert(AchievementStatus { earned: ach.earned, earned_time: ach.earned_time });
        }
        map
    } else {
        let status_path = ira_parser::unlock_status_path(save_dir, entry.trophy_source, app_id, platform_id);
        ira_parser::load_status_map(&status_path)
    };

    if !has_meta && !steam_native_data.achievements.is_empty() {
        let icons_dir = ach_dir.join("achievement_images");
        for ach in &steam_native_data.achievements {
            let icon_path = if !ach.icon_url.is_empty() {
                let icon_file = icons_dir.join(format!("{}.jpg", ach.id));
                if icon_file.is_file() {
                    icon_file.to_string_lossy().into_owned()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            game.achievements.push(MergedAchievement {
                name: ach.id.clone(),
                display_name: ach.display_name.clone(),
                description: ach.description.clone(),
                hidden: ach.hidden,
                earned: ach.earned,
                earned_time: ach.earned_time,
                icon_path,
                icon_gray_path: String::new(),
                global_percent: ach.global_percent,
                trophy_type: '\0',
            });
        }
    } else if has_meta {
        let meta_data = std::fs::read(&meta_path).map_err(|e| format!("read achievements.json: {}", e))?;
        let meta_list: Vec<ira_models::achievement::AchievementMeta> =
            serde_json::from_slice(&meta_data).map_err(|e| {
                eprintln!("Meta load error for {}", app_id);
                format!("parse achievements.json: {}", e)
            })?;

        for meta in meta_list {
            let status = status_map.get(&meta.name).cloned().unwrap_or_default();
            let hidden = ira_models::achievement::parse_hidden(&meta.hidden);
            let icon_gray = if meta.icon_gray.is_empty() {
                meta.icon_gray_alt.clone()
            } else {
                meta.icon_gray.clone()
            };

            game.achievements.push(MergedAchievement {
                name: meta.name.clone(),
                display_name: meta.display_name.val.clone(),
                description: meta.description.val.clone(),
                hidden,
                earned: status.earned,
                earned_time: status.earned_time,
                icon_path: ira_parser::find_icon_path(&ach_dir, &meta.icon),
                icon_gray_path: ira_parser::find_icon_path(&ach_dir, &icon_gray),
                global_percent: 0.0,
                trophy_type: '\0',
            });
        }
    } else {
        let mut keys: Vec<_> = status_map.keys().cloned().collect();
        keys.sort();
        for name in keys {
            let status = &status_map[&name];
            game.achievements.push(MergedAchievement {
                name: name.clone(),
                display_name: name.clone(),
                description: "No description available.".into(),
                hidden: false,
                earned: status.earned,
                earned_time: status.earned_time,
                icon_path: String::new(),
                icon_gray_path: String::new(),
                global_percent: 0.0,
                trophy_type: '\0',
            });
        }
    }

    game.total_count = game.achievements.len();
    game.earned_count = game.achievements.iter().filter(|a| a.earned).count();

    if is_steam_native && steam_native_data.n_total > 0 {
        game.total_count = steam_native_data.n_total;
        game.earned_count = steam_native_data.n_achieved;
    }

    Ok(game)
}
