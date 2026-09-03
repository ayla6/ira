use ira_db::DbConn;
use ira_models::{AchievementStatus, AppDetails, AssetType, Game, GameEntry, MergedAchievement};
use std::collections::HashMap;
use std::path::PathBuf;

pub fn read_app_details(save_dir: &str, app_id: &str) -> Option<AppDetails> {
    let path = ira_parser::data_dir(save_dir, app_id).join("appdetails.json");
    let mut details = ira_api::steam::read_app_details_from_cache(&path)?;

    let dlc_config_path = ira_parser::data_dir(save_dir, app_id).join("dlc_config.json");
    if let Ok(data) = std::fs::read(&dlc_config_path) {
        if let Ok(saved) = serde_json::from_slice::<AppDetails>(&data) {
            for (id, saved_dlc) in &saved.dlcs {
                if let Some(dlc) = details.dlcs.get_mut(id) {
                    dlc.enabled = saved_dlc.enabled;
                }
            }
        }
    }

    Some(details)
}

pub fn load_games(conn: &DbConn, save_dir: &str) -> Vec<Game> {
    let _span = tracing::info_span!("load_games").entered();
    load_selected_games(
        conn,
        save_dir,
        "Failed to load games from DB",
        |entry| {
            entry.kind != ira_models::GameKind::Linux && entry.kind != ira_models::GameKind::Wine
        },
        |entry| {
            let id = if !entry.steam_id.is_empty() {
                &entry.steam_id
            } else {
                &entry.game_id
            };
            format!("Skipping game {} ({})", id, entry.kind)
        },
    )
}

/// Load every saved game from the database without probing external sources.
/// Use this during startup; explicit rescans can refresh source-specific data.
pub fn load_saved_games(conn: &DbConn, save_dir: &str) -> Vec<Game> {
    let _span = tracing::info_span!("load_saved_games").entered();
    load_selected_games(
        conn,
        save_dir,
        "Failed to load saved games from DB",
        // ROM-library entries only show once their source scan found their
        // file (or, for Switch, the emulator library that owns them), and
        // vanished console installs never show at all.
        |entry| {
            (matches!(
                entry.kind,
                ira_models::GameKind::Retro | ira_models::GameKind::Switch
            ) && entry.rom_path.is_empty())
                || console_game_vanished(entry)
        },
        |entry| format!("Skipping saved game {}", entry.id),
    )
}

/// Whether a console-library row's game is gone: the owning source scan
/// marked it vanished, or its recorded install path no longer exists. The
/// row stays in the DB for its playtime and trophies, but loads must ignore
/// it entirely — this is independent of the user's hidden flag, which the
/// "show hidden games" setting governs.
fn console_game_vanished(entry: &GameEntry) -> bool {
    matches!(
        entry.kind,
        ira_models::GameKind::Ps4
            | ira_models::GameKind::Ps3
            | ira_models::GameKind::PsVita
            | ira_models::GameKind::WiiU
            | ira_models::GameKind::ThreeDS
    ) && (entry.vanished
        || (!entry.rom_path.is_empty() && !std::path::Path::new(&entry.rom_path).exists()))
}

/// Shared implementation behind `load_games` and `load_saved_games`.
/// `skip` filters out entries this caller must not process and `describe`
/// renders one entry in the per-failure diagnostic.
fn load_selected_games(
    conn: &DbConn,
    save_dir: &str,
    db_error_context: &str,
    skip: impl Fn(&GameEntry) -> bool,
    describe: impl Fn(&GameEntry) -> String,
) -> Vec<Game> {
    let entries = match ira_db::load_all_games(conn) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{}: {}", db_error_context, e);
            return Vec::new();
        }
    };

    let mut games: Vec<Game> = entries
        .iter()
        .filter(|entry| !skip(entry))
        .flat_map(|entry| match load_game_fast(entry, save_dir) {
            Ok(game) => {
                let variant_entries = build_variant_entries(conn, save_dir, &game);
                std::iter::once(game)
                    .chain(variant_entries)
                    .collect::<Vec<_>>()
            }
            Err(e) => {
                eprintln!("{}: {}", describe(entry), e);
                Vec::new()
            }
        })
        .collect();
    games.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
    games
}

fn build_game_base(entry: &GameEntry, save_dir: &str) -> Game {
    let app_id = if !entry.steam_id.is_empty() {
        &entry.steam_id
    } else {
        &entry.game_id
    };
    let kind = entry.kind;

    let mut game = Game {
        app_id: app_id.to_string(),
        kind,
        trophy_source: entry.trophy_source,
        platform_id: entry.platform_id.to_string(),
        db_id: entry.id,
        name: if entry.title.is_empty() {
            crate::tr!("App ID: {}").replacen("{}", app_id, 1)
        } else {
            entry.title.clone()
        },
        name_lower: String::new(),
        icon_path: String::new(),
        hero_image_path: String::new(),
        grid_path: String::new(),
        header_path: String::new(),
        logo_path: String::new(),
        square_path: String::new(),
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
        game_folder: entry.game_folder.clone(),
    };

    // Retro and Switch paths stay relative to the console's ROM folder;
    // every console kind with an emulator library stores the absolute
    // location discovered at scan time and later rebuilds must keep it
    // (context menu, icon restore).
    if matches!(
        entry.kind,
        ira_models::GameKind::Retro
            | ira_models::GameKind::Switch
            | ira_models::GameKind::ThreeDS
            | ira_models::GameKind::WiiU
            | ira_models::GameKind::PsVita
            | ira_models::GameKind::Ps4
            | ira_models::GameKind::Ps3
    ) && !entry.rom_path.is_empty()
    {
        game.game_path = entry.rom_path.clone();
    }

    if entry.title.is_empty() {
        if let Some(name) = ira_parser::read_app_name(save_dir, app_id) {
            game.name = name;
        }
    }
    game.name_lower = game.name.to_lowercase();

    let image_dir = ira_parser::entry_data_dir(save_dir, entry);

    if let Some(icon_path) = ira_parser::find_image_file(&image_dir, AssetType::Icon.file_base()) {
        game.icon_path = icon_path.to_string_lossy().into_owned();
    }

    ira_parser::populate_image_paths(&image_dir, &mut game);

    game
}

/// True when `name` is still the synthesized "App ID: …" placeholder shown
/// for DB rows whose real title was never learned. Mirrors how
/// `build_game_base` builds that name from the localized template, so it
/// stays correct whatever the current language does to the prefix.
pub fn is_placeholder_name(name: &str) -> bool {
    matches_placeholder(name, &crate::tr!("App ID: {}"))
}

/// Structural inverse of `<template>.replacen("{}", value, 1)`: the name
/// must be `<prefix><non-empty value><suffix>` around the template's `{}`
/// slot.
fn matches_placeholder(name: &str, template: &str) -> bool {
    match template.split_once("{}") {
        Some((pre, post)) => {
            name.len() > pre.len() + post.len() && name.starts_with(pre) && name.ends_with(post)
        }
        // Degenerate template without a slot: the generator's replacen is
        // then an identity, so only the bare template counts.
        None => name == template,
    }
}

pub fn load_game_fast(entry: &GameEntry, save_dir: &str) -> Result<Game, String> {
    let app_id = if !entry.steam_id.is_empty() {
        &entry.steam_id
    } else {
        &entry.game_id
    };
    let _s = tracing::info_span!("load_game_fast", app_id).entered();
    let mut game = build_game_base(entry, save_dir);
    game.earned_count = entry.cached_earned_count as usize;
    game.total_count = entry.cached_total_count as usize;
    Ok(game)
}

/// Returns the mtime of the RA `web_progress.json` cache.
/// Returns 0 if the file doesn't exist. Used to skip background reloading
/// when achievement files haven't changed since the last cache write.
pub fn ra_achievement_mtime(save_dir: &str, game_id: &str) -> i64 {
    let ra_dir = std::path::Path::new(save_dir)
        .join("data")
        .join("ra")
        .join(game_id);
    let mtime = |p: std::path::PathBuf| {
        p.metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };
    mtime(ra_dir.join("web_progress.json"))
}

pub fn load_game(entry: &GameEntry, save_dir: &str) -> Result<Game, String> {
    let app_id = if !entry.steam_id.is_empty() {
        &entry.steam_id
    } else {
        &entry.game_id
    };
    let platform_id = &entry.platform_id;
    let _s = tracing::info_span!("load_game", app_id).entered();

    let mut game = build_game_base(entry, save_dir);

    if entry.kind == ira_models::GameKind::Ps3 {
        game.achievements = ira_platforms::ps3::load_ps3_trophies(app_id);
        game.total_count = game.achievements.len();
        game.earned_count = game.achievements.iter().filter(|a| a.earned).count();
        return Ok(game);
    }

    let ach_dir = ira_parser::achievements_dir(save_dir, app_id);

    let is_steam_native = entry.trophy_source == ira_models::TrophySource::SteamNative;
    let steam_native_data = if is_steam_native {
        ira_platforms::steam::read_steam_achievements_full(app_id, save_dir)
    } else {
        ira_platforms::steam::SteamAchievementData {
            achievements: Vec::new(),
            n_total: 0,
            n_achieved: 0,
        }
    };

    if entry.trophy_source == ira_models::TrophySource::Ra {
        let _s = tracing::info_span!("load_ra_achievements_from_cache").entered();
        game.achievements =
            ira_platforms::retroachievements::load_ra_achievements_from_cache(save_dir, app_id);
        game.total_count = game.achievements.len();
        game.earned_count = game.achievements.iter().filter(|a| a.earned).count();
        return Ok(game);
    }

    let meta_path = ach_dir.join("achievements.json");
    let has_meta = meta_path.is_file();

    let status_map = if is_steam_native {
        let mut map: HashMap<String, AchievementStatus> =
            ira_platforms::steam::read_user_stats(app_id)
                .into_iter()
                .map(|(name, (earned, earned_time))| {
                    (
                        name,
                        AchievementStatus {
                            earned,
                            earned_time,
                        },
                    )
                })
                .collect();
        for ach in &steam_native_data.achievements {
            map.entry(ach.id.clone())
                .and_modify(|s| {
                    if s.earned_time == 0 {
                        s.earned_time = ach.earned_time;
                    }
                })
                .or_insert(AchievementStatus {
                    earned: ach.earned,
                    earned_time: ach.earned_time,
                });
        }
        map
    } else {
        let status_path =
            ira_parser::unlock_status_path(save_dir, entry.trophy_source, app_id, platform_id);
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
        let meta_data =
            std::fs::read(&meta_path).map_err(|e| format!("read achievements.json: {}", e))?;
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
                description: crate::tr!("No description available."),
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

/// Copy every non-empty asset path from `from` over `to`.
fn apply_non_empty_images(from: &Game, to: &mut Game) {
    if !from.icon_path.is_empty() {
        to.icon_path = from.icon_path.clone();
    }
    if !from.hero_image_path.is_empty() {
        to.hero_image_path = from.hero_image_path.clone();
    }
    if !from.square_path.is_empty() {
        to.square_path = from.square_path.clone();
    }
    if !from.grid_path.is_empty() {
        to.grid_path = from.grid_path.clone();
    }
    if !from.header_path.is_empty() {
        to.header_path = from.header_path.clone();
    }
    if !from.logo_path.is_empty() {
        to.logo_path = from.logo_path.clone();
    }
}

/// Apply a specific variant's images to the base game.
/// Only applies if the variant has `custom_images=true` and `show_as_entry=false`.
/// Called when the user selects a variant on the base game's play button.
pub fn apply_variant_images_for(
    db: &DbConn,
    save_dir: &str,
    entry: &GameEntry,
    game: &mut Game,
    variant_id: i64,
) {
    let Ok(variants) = ira_db::get_variants(db, entry.id) else {
        return;
    };
    let Some(var) = variants.iter().find(|v| v.id == variant_id) else {
        return;
    };
    if !var.custom_images || var.show_as_entry {
        return;
    }

    let image_dir = ira_parser::entry_data_dir(save_dir, entry);
    let var_dir = image_dir.join(format!("variant-{}", variant_id));
    if !var_dir.is_dir() {
        return;
    }

    let mut var_game = Game::default();
    ira_parser::populate_image_paths(&var_dir, &mut var_game);
    apply_non_empty_images(&var_game, game);

    if !var.logo_position.is_empty() {
        game.logo_position = var.logo_position.clone();
    }
    if var.logo_size != 0 {
        game.logo_size = var.logo_size;
    }
}

/// For each variant with `show_as_entry=true`, create a pseudo-Game entry
/// that appears in the grid as a separate game. The pseudo-game shares
/// achievements, playtime, etc. with the base game but has its own images.
pub fn build_variant_entries(db: &DbConn, save_dir: &str, game: &Game) -> Vec<Game> {
    let Ok(variants) = ira_db::get_variants(db, game.db_id) else {
        return Vec::new();
    };
    let image_dir = ira_parser::game_data_dir(save_dir, game);

    variants
        .iter()
        .filter(|v| v.show_as_entry)
        .map(|v| {
            let mut entry = game.clone();
            entry.variant_id = Some(v.id);
            entry.set_name(
                crate::tr!("{} - {}")
                    .replacen("{}", &game.name, 1)
                    .replacen("{}", &v.name, 1),
            );
            entry.playtime = v.playtime;
            entry.last_played = v.last_played;
            if !v.logo_position.is_empty() {
                entry.logo_position = v.logo_position.clone();
            }
            if v.logo_size != 0 {
                entry.logo_size = v.logo_size;
            }

            let var_dir = image_dir.join(format!("variant-{}", v.id));
            if var_dir.is_dir() {
                let mut var_game = Game::default();
                ira_parser::populate_image_paths(&var_dir, &mut var_game);
                apply_non_empty_images(&var_game, &mut entry);
            }

            entry
        })
        .collect()
}

/// Compute the file to watch for live achievement updates for a given game.
/// Returns None for game types that don't have a watchable achievement file
/// (e.g. SteamNative reads from Steam's API, RetroAchievements uses a cache).
pub fn achievement_watch_file(game: &Game, save_dir: &str) -> Option<PathBuf> {
    match game.trophy_source {
        ira_models::TrophySource::Gse | ira_models::TrophySource::Nge => {
            Some(ira_parser::unlock_status_path(
                save_dir,
                game.trophy_source,
                &game.app_id,
                &game.platform_id,
            ))
        }
        _ if game.kind == ira_models::GameKind::Ps3 => {
            Some(ira_platforms::ps3::tropusr_path(&game.app_id))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ira_models::{Game, GameEntry, GameKind};

    fn entry_with_path(kind: GameKind, rom_path: &str) -> GameEntry {
        let mut entry = GameEntry::from_game(&Game::default());
        entry.kind = kind;
        entry.rom_path = rom_path.to_string();
        entry
    }

    /// Console games rebuilt from the database row must keep their stored
    /// location: the context menu and native-icon restore rely on it.
    #[test]
    fn test_build_game_base_restores_console_rom_path() {
        for kind in [
            GameKind::Retro,
            GameKind::Switch,
            GameKind::ThreeDS,
            GameKind::WiiU,
            GameKind::PsVita,
            GameKind::Ps4,
            GameKind::Ps3,
        ] {
            let entry = entry_with_path(kind, "/games/PQ.zcci");
            let game = build_game_base(&entry, "/tmp/ira-test-save");
            assert_eq!(game.game_path, "/games/PQ.zcci");
        }
        let entry = entry_with_path(GameKind::Steam, "/games/x");
        assert!(build_game_base(&entry, "/tmp/ira-test-save")
            .game_path
            .is_empty());
    }

    #[test]
    fn test_console_game_vanished_when_marked_or_file_missing() {
        // A live install path and no verdict from the scan: present.
        let existing = entry_with_path(GameKind::WiiU, "/tmp");
        assert!(!console_game_vanished(&existing));

        let mut marked = entry_with_path(GameKind::Ps4, "");
        marked.vanished = true;
        assert!(console_game_vanished(&marked));

        let deleted = entry_with_path(GameKind::Ps4, "/games/Gone-Catherine");
        assert!(!std::path::Path::new("/games/Gone-Catherine").exists());
        assert!(console_game_vanished(&deleted));
    }

    #[test]
    fn test_console_game_vanished_ignores_other_kinds_and_present_files() {
        // The hidden-games setting never resurrects these — but the check
        // must not swallow non-console kinds either.
        for kind in [GameKind::Steam, GameKind::Retro, GameKind::Switch] {
            let mut entry = entry_with_path(kind, "/definitely/missing");
            entry.vanished = true;
            assert!(!console_game_vanished(&entry), "{kind:?}");
        }
        // Empty path without a vanished verdict means "unknown", not gone.
        let legacy = entry_with_path(GameKind::Ps4, "");
        assert!(!console_game_vanished(&legacy));
    }

    /// Generated placeholder names are recognized through the same
    /// localized template that produced them (gettext stays untranslated
    /// in unit tests, so this exercises the English path).
    #[test]
    fn test_is_placeholder_name_matches_generated_name() {
        let generated = crate::tr!("App ID: {}").replacen("{}", "1234567", 1);
        assert!(is_placeholder_name(&generated));
        assert!(is_placeholder_name("App ID: 42"));
        assert!(!is_placeholder_name("Pillars of Eternity"));
        // An empty id is never generated, so a bare prefix is not one.
        assert!(!is_placeholder_name("App ID: "));
    }

    /// Locale injection needs an installed gettext catalog and is not
    /// available in unit tests; translation robustness is therefore pinned
    /// on the structural check with synthetic templates: even when a
    /// translation moves the `{}` slot around, the captured prefix/suffix
    /// still recognizes exactly what the generator would produce.
    #[test]
    fn test_matches_placeholder_handles_translated_templates() {
        assert!(matches_placeholder("42 – App-ID", "{} – App-ID"));
        assert!(matches_placeholder("Aplicación 99", "Aplicación {}"));
        assert!(!matches_placeholder("Real Game", "{} – App-ID"));
        assert!(!matches_placeholder("", "{}"));
        assert!(matches_placeholder(
            &"{} – App-ID".replacen("{}", "7", 1),
            "{} – App-ID"
        ));
    }
}
