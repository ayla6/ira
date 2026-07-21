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
        match load_game_fast(&entry, save_dir) {
            Ok(game) => {
                let variant_entries = build_variant_entries(conn, save_dir, &game);
                games.push(game);
                games.extend(variant_entries);
            }
            Err(e) => eprintln!("Skipping game {} ({}): {}", if !entry.steam_id.is_empty() { &entry.steam_id } else { &entry.game_id }, entry.kind, e),
        }
    }
    games.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
    games
}

fn build_game_base(entry: &GameEntry, save_dir: &str) -> Game {
    let app_id = if !entry.steam_id.is_empty() { &entry.steam_id } else { &entry.game_id };
    let kind = entry.kind;

    let mut game = Game {
        app_id: app_id.to_string(),
        kind,
        trophy_source: entry.trophy_source,
        platform_id: entry.platform_id.to_string(),
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
        variant_id: None,
        release_date: entry.release_date.clone(),
        release_timestamp: entry.release_timestamp,
        metacritic_score: entry.metacritic_score,
        steam_review_score: entry.steam_review_score,
        steam_review_count: entry.steam_review_count,
        ra_core: entry.ra_core.clone(),
        emulator_override: entry.emulator_override.clone(),
        rom_path: entry.rom_path.clone(),
    };

    if entry.title.is_empty() {
        if let Some(name) = ira_parser::read_app_name(save_dir, app_id) {
            game.name = name;
        }
    }

    let image_dir = ira_parser::entry_data_dir(save_dir, entry);

    if let Some(icon_path) = ira_parser::find_image_file(&image_dir, "icon") {
        game.icon_path = icon_path.to_string_lossy().into_owned();
    } else if entry.trophy_source == ira_models::TrophySource::Ra {
        let ra_icon = ira_parser::ra_icon_path(save_dir, app_id);
        if ra_icon.is_file() {
            game.icon_path = ra_icon.to_string_lossy().into_owned();
        }
    }

    ira_parser::populate_image_paths(&image_dir, &mut game);

    game
}

pub fn load_game_fast(entry: &GameEntry, save_dir: &str) -> Result<Game, String> {
    let app_id = if !entry.steam_id.is_empty() { &entry.steam_id } else { &entry.game_id };
    let _s = tracing::info_span!("load_game_fast", app_id).entered();
    let mut game = build_game_base(entry, save_dir);
    game.earned_count = entry.cached_earned_count as usize;
    game.total_count = entry.cached_total_count as usize;
    Ok(game)
}

/// Returns the max mtime of RA achievement files (game.json + unlocks.json).
/// Returns 0 if neither file exists. Used to skip background reloading
/// when achievement files haven't changed since the last cache write.
pub fn ra_achievement_mtime(save_dir: &str, game_id: &str) -> i64 {
    let ra_dir = std::path::Path::new(save_dir).join("data").join("ra").join(game_id);
    let mtime = |p: std::path::PathBuf| {
        p.metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };
    mtime(ra_dir.join("game.json")).max(mtime(ra_dir.join("unlocks.json")))
}

pub fn load_game(entry: &GameEntry, save_dir: &str) -> Result<Game, String> {
    let app_id = if !entry.steam_id.is_empty() { &entry.steam_id } else { &entry.game_id };
    let platform_id = &entry.platform_id;
    let _s = tracing::info_span!("load_game", app_id).entered();

    let mut game = build_game_base(entry, save_dir);

    let ach_dir = ira_parser::achievements_dir(save_dir, app_id);

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

/// Apply a specific variant's images to the base game.
/// Only applies if the variant has `custom_images=true` and `show_as_entry=false`.
/// Called when the user selects a variant on the base game's play button.
pub fn apply_variant_images_for(db: &DbConn, save_dir: &str, entry: &GameEntry, game: &mut Game, variant_id: i64) {
    let Ok(variants) = ira_db::get_variants(db, entry.id) else { return };
    let Some(var) = variants.iter().find(|v| v.id == variant_id) else { return };
    if !var.custom_images || var.show_as_entry { return }

    let image_dir = ira_parser::entry_data_dir(save_dir, entry);
    let var_dir = image_dir.join(format!("variant-{}", variant_id));
    if !var_dir.is_dir() { return }

    let mut var_game = Game::default();
    ira_parser::populate_image_paths(&var_dir, &mut var_game);
    if !var_game.icon_path.is_empty() { game.icon_path = var_game.icon_path; }
    if !var_game.hero_image_path.is_empty() { game.hero_image_path = var_game.hero_image_path; }
    if !var_game.grid_path.is_empty() { game.grid_path = var_game.grid_path; }
    if !var_game.header_path.is_empty() { game.header_path = var_game.header_path; }
    if !var_game.logo_path.is_empty() { game.logo_path = var_game.logo_path; }

    game.logo_position = var.logo_position.clone();
    game.logo_size = var.logo_size;
}

/// For each variant with `show_as_entry=true`, create a pseudo-Game entry
/// that appears in the grid as a separate game. The pseudo-game shares
/// achievements, playtime, etc. with the base game but has its own images.
pub fn build_variant_entries(db: &DbConn, save_dir: &str, game: &Game) -> Vec<Game> {
    let Ok(variants) = ira_db::get_variants(db, game.db_id) else { return Vec::new() };
    let image_dir = ira_parser::game_data_dir(save_dir, game);

    variants.iter()
        .filter(|v| v.show_as_entry)
        .map(|v| {
            let mut entry = game.clone();
            entry.variant_id = Some(v.id);
            entry.name = format!("{} - {}", game.name, v.name);
            entry.playtime = v.playtime;
            entry.last_played = v.last_played;
            entry.logo_position = v.logo_position.clone();
            entry.logo_size = v.logo_size;

            let var_dir = image_dir.join(format!("variant-{}", v.id));
            if var_dir.is_dir() {
                let mut var_game = Game::default();
                ira_parser::populate_image_paths(&var_dir, &mut var_game);
                if !var_game.icon_path.is_empty() { entry.icon_path = var_game.icon_path; }
                if !var_game.hero_image_path.is_empty() { entry.hero_image_path = var_game.hero_image_path; }
                if !var_game.grid_path.is_empty() { entry.grid_path = var_game.grid_path; }
                if !var_game.header_path.is_empty() { entry.header_path = var_game.header_path; }
                if !var_game.logo_path.is_empty() { entry.logo_path = var_game.logo_path; }
            }

            entry
        })
        .collect()
}
