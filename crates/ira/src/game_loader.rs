use ira_db::DbConn;
use ira_models::{AchievementStatus, AppDetails, Game, GameEntry, MergedAchievement};
use std::collections::HashMap;
use std::path::PathBuf;

pub fn read_app_details(save_dir: &str, app_id: &str) -> Option<AppDetails> {
    let dir = ira_parser::data_dir(save_dir, app_id);
    let path = dir.join("appdetails.json");
    let mut details = ira_api::steam::read_app_details_from_cache(&path)?;

    let dlc_config_path = dir.join("dlc_config.json");
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

/// Load every saved game from the database without probing external sources.
/// This is the base of every list build; enabled source scans refresh
/// source-specific data on top of it.
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

/// The store-facing id: the matched Steam id when one exists, otherwise the
/// platform-native id. Store-API callers resolve both through
/// `Game::steam_api_id`; this is the same choice in one place.
fn store_app_id(entry: &GameEntry) -> &str {
    if !entry.steam_id.is_empty() {
        &entry.steam_id
    } else {
        entry.external_id()
    }
}

/// Console-library kinds whose rows store the absolute install location
/// discovered at scan time (later rebuilds keep it for the context menu
/// and icon restore).
fn is_console_library_kind(kind: ira_models::GameKind) -> bool {
    matches!(
        kind,
        ira_models::GameKind::Ps4
            | ira_models::GameKind::Ps3
            | ira_models::GameKind::PsVita
            | ira_models::GameKind::WiiU
            | ira_models::GameKind::ThreeDS
    )
}
/// Whether a console-library row's game is gone: the owning source scan
/// marked it vanished, or its recorded install path no longer exists. The
/// row stays in the DB for its playtime and trophies, but loads must ignore
/// it entirely — this is independent of the user's hidden flag, which the
/// "show hidden games" setting governs.
fn console_game_vanished(entry: &GameEntry) -> bool {
    is_console_library_kind(entry.kind)
        && (entry.vanished
            || (!entry.rom_path.is_empty() && !std::path::Path::new(&entry.rom_path).exists()))
}

/// Shared implementation behind `load_saved_games`.
/// `skip` filters out entries this caller must not process and `describe`
/// renders one entry in the per-failure diagnostic.
fn load_selected_games(
    conn: &DbConn,
    save_dir: &str,
    db_error_context: &str,
    skip: impl Fn(&GameEntry) -> bool + Sync,
    describe: impl Fn(&GameEntry) -> String + Sync,
) -> Vec<Game> {
    let entries = match ira_db::load_all_games_unordered(conn) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{}: {}", db_error_context, e);
            return Vec::new();
        }
    };
    if entries.is_empty() {
        return Vec::new();
    }
    // One round-trip for every game's variants instead of one per game.
    let variant_map = load_variant_map(conn, &entries);
    let mut games = load_entries_parallel(&entries, save_dir, &variant_map, &skip, &describe);
    games.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
    games
}

/// Batch-load variants for every entry at once. A per-game failure used to
/// mean that game's variants silently vanished; a batch failure means the
/// same (empty map), so the output matches without a log line.
fn load_variant_map(
    conn: &DbConn,
    entries: &[GameEntry],
) -> HashMap<i64, Vec<ira_models::GameVariant>> {
    let ids: Vec<i64> = entries.iter().map(|entry| entry.id).collect();
    ira_db::get_variants_for_games(conn, &ids).unwrap_or_default()
}

/// Load and expand every entry. Filesystem work (directory listings,
/// thumbnail checks) dominates, so entries split across worker threads;
/// the final sort makes thread completion order irrelevant.
fn load_entries_parallel(
    entries: &[GameEntry],
    save_dir: &str,
    variant_map: &HashMap<i64, Vec<ira_models::GameVariant>>,
    skip: &(impl Fn(&GameEntry) -> bool + Sync),
    describe: &(impl Fn(&GameEntry) -> String + Sync),
) -> Vec<Game> {
    let threads = worker_thread_count(entries.len());
    if threads <= 1 {
        return load_chunk(entries, save_dir, variant_map, skip, describe);
    }
    let chunk_size = entries.len().div_ceil(threads);
    std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .chunks(chunk_size)
            .map(|chunk| s.spawn(move || load_chunk(chunk, save_dir, variant_map, skip, describe)))
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| match handle.join() {
                Ok(games) => games,
                Err(_) => {
                    eprintln!("Game load worker panicked; skipping its chunk");
                    Vec::new()
                }
            })
            .collect()
    })
}

/// Worker count for the load: sequential below 32 entries, otherwise one
/// worker per core capped at 8 so the outer source-scan threads keep room.
fn worker_thread_count(entry_count: usize) -> usize {
    if entry_count < 32 {
        return 1;
    }
    std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .min(entry_count)
}

/// Load one slice of entries: filter, build the base game, expand variants.
fn load_chunk(
    chunk: &[GameEntry],
    save_dir: &str,
    variant_map: &HashMap<i64, Vec<ira_models::GameVariant>>,
    skip: &(impl Fn(&GameEntry) -> bool + Sync),
    describe: &(impl Fn(&GameEntry) -> String + Sync),
) -> Vec<Game> {
    let mut games = Vec::new();
    for entry in chunk {
        if skip(entry) {
            continue;
        }
        match load_game_fast(entry, save_dir) {
            Ok(game) => {
                let variants = build_variant_entries_from_slice(
                    variant_map
                        .get(&game.db_id)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    save_dir,
                    &game,
                );
                games.push(game);
                games.extend(variants);
            }
            Err(e) => {
                eprintln!("{}: {}", describe(entry), e);
            }
        }
    }
    games
}

fn build_game_base(entry: &GameEntry, save_dir: &str) -> Game {
    // The operational id is always the platform-native one; the Steam
    // match id lives in `steam_id` and never lands here. Store-API
    // callers resolve both through `Game::steam_api_id`.
    let app_id = entry.external_id();
    let store_id = store_app_id(entry);

    let mut game = entry.to_game();
    game.app_id = app_id.to_string();
    if entry.title.is_empty() {
        game.name = crate::tr!("App ID: {}").replacen("{}", app_id, 1);
    }

    // Retro and Switch paths stay relative to the console's ROM folder;
    // every console kind with an emulator library stores the absolute
    // location discovered at scan time and later rebuilds must keep it
    // (context menu, icon restore).
    if (matches!(
        entry.kind,
        ira_models::GameKind::Retro | ira_models::GameKind::Switch
    ) || is_console_library_kind(entry.kind))
        && !entry.rom_path.is_empty()
    {
        game.game_path = entry.rom_path.clone();
    }

    if entry.title.is_empty() {
        // App-name sidecars live beside the store-keyed data dir, which
        // prefers the Steam id — resolve it the same way.
        if let Some(name) = ira_parser::read_app_name(save_dir, store_id) {
            game.name = name;
        }
    }
    game.name_lower = game.name.to_lowercase();

    let image_dir = ira_parser::entry_data_dir(save_dir, entry);
    // populate_image_paths sets every asset path including the icon (small
    // variant preferred), so no separate icon lookup is needed.
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
    let app_id = store_app_id(entry);
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
    let app_id = store_app_id(entry);
    let _s = tracing::info_span!("load_game", app_id).entered();

    let mut game = build_game_base(entry, save_dir);

    if entry.kind == ira_models::GameKind::Ps3 {
        game.achievements = ira_platforms::ps3::load_ps3_trophies(app_id);
        recount(&mut game);
        return Ok(game);
    }

    if entry.kind == ira_models::GameKind::Ps4 {
        game.achievements = ira_platforms::ps4::load_ps4_trophies(app_id);
        recount(&mut game);
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
        recount(&mut game);
        return Ok(game);
    }

    let meta_path = ach_dir.join("achievements.json");
    let has_meta = meta_path.is_file();

    let status_map = if is_steam_native {
        steam_native_status_map(app_id, &steam_native_data)
    } else {
        let status_path = ira_parser::unlock_status_path(
            save_dir,
            entry.trophy_source,
            app_id,
            &entry.native_id,
        );
        ira_parser::load_status_map(&status_path)
    };

    if !has_meta && !steam_native_data.achievements.is_empty() {
        push_steam_native_achievements(&mut game, &ach_dir, &steam_native_data);
    } else if has_meta {
        push_meta_achievements(&mut game, &ach_dir, &meta_path, &status_map, app_id)?;
    } else {
        push_bare_status_achievements(&mut game, &status_map);
    }

    recount(&mut game);

    if is_steam_native && steam_native_data.n_total > 0 {
        game.total_count = steam_native_data.n_total;
        game.earned_count = steam_native_data.n_achieved;
    }

    Ok(game)
}

/// Recompute the earned/total counters from the loaded achievement list.
fn recount(game: &mut Game) {
    game.total_count = game.achievements.len();
    game.earned_count = game.achievements.iter().filter(|a| a.earned).count();
}

/// Merge live Steam user stats with the cached achievement list: stats win
/// on earned state, the cache fills in unlock times the stats lack.
fn steam_native_status_map(
    app_id: &str,
    data: &ira_platforms::steam::SteamAchievementData,
) -> HashMap<String, AchievementStatus> {
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
    for ach in &data.achievements {
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
}

/// Achievements known only from the Steam cache (no `achievements.json`
/// metadata yet): ids and display data come from the cache verbatim.
fn push_steam_native_achievements(
    game: &mut Game,
    ach_dir: &std::path::Path,
    data: &ira_platforms::steam::SteamAchievementData,
) {
    let icons_dir = ach_dir.join("achievement_images");
    for ach in &data.achievements {
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
}

/// Achievements with on-disk metadata: display data from
/// `achievements.json`, earned state from the unlock status map.
fn push_meta_achievements(
    game: &mut Game,
    ach_dir: &std::path::Path,
    meta_path: &std::path::Path,
    status_map: &HashMap<String, AchievementStatus>,
    app_id: &str,
) -> Result<(), String> {
    let meta_data =
        std::fs::read(meta_path).map_err(|e| format!("read achievements.json: {}", e))?;
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
            icon_path: ira_parser::find_icon_path(ach_dir, &meta.icon),
            icon_gray_path: ira_parser::find_icon_path(ach_dir, &icon_gray),
            global_percent: 0.0,
            trophy_type: '\0',
        });
    }
    Ok(())
}

/// Unlock status with neither metadata nor a Steam cache entry: bare
/// name-only achievements so progress is still visible.
fn push_bare_status_achievements(
    game: &mut Game,
    status_map: &HashMap<String, AchievementStatus>,
) {
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

    if let Some(pos) = var.logo_position {
        game.logo_position = pos;
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
    build_variant_entries_from_slice(&variants, save_dir, game)
}

/// Pure expansion shared by the single-game reload and the bulk loader: the
/// same rows in the same order produce the same pseudo-games.
fn build_variant_entries_from_slice(
    variants: &[ira_models::GameVariant],
    save_dir: &str,
    game: &Game,
) -> Vec<Game> {
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
            if let Some(pos) = v.logo_position {
                entry.logo_position = pos;
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
                game.steam_api_id(),
                &game.native_id,
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

    fn saved_game_fixture(db: &ira_db::DbConn, native_id: &str, title: &str) -> GameEntry {
        let id = ira_db::add_game(
            db,
            ira_db::NewGame {
                kind: GameKind::Steam,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: native_id,
                trophy_id: "",
                native_id,
                platform_id: GameKind::Steam.as_str(),
                title,
            },
        )
        .unwrap();
        ira_db::find_by_db_id(db, id).unwrap().unwrap()
    }

    fn add_show_as_entry_variant(db: &ira_db::DbConn, game_id: i64, name: &str) {
        let variant = ira_models::GameVariant {
            game_id,
            name: name.to_string(),
            show_as_entry: true,
            ..Default::default()
        };
        ira_db::add_variant(db, &variant).unwrap();
    }

    /// The batched variant expansion must match the single-game query path
    /// exactly: same count, same names, same variant ids in the same order.
    #[test]
    fn test_variant_slice_matches_db_path() {
        let tmp = tempfile::tempdir().unwrap();
        let db = ira_db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let save_dir = tmp.path().join("save").to_string_lossy().into_owned();
        let entry = saved_game_fixture(&db, "100", "Base Game");
        add_show_as_entry_variant(&db, entry.id, "Beta");
        let hidden = ira_models::GameVariant {
            game_id: entry.id,
            name: "Hidden".to_string(),
            ..Default::default()
        };
        ira_db::add_variant(&db, &hidden).unwrap();

        let game = load_game_fast(&entry, &save_dir).unwrap();
        let from_db = build_variant_entries(&db, &save_dir, &game);
        let all = ira_db::get_variants(&db, entry.id).unwrap();
        let from_slice = build_variant_entries_from_slice(&all, &save_dir, &game);

        assert_eq!(from_db.len(), 1);
        assert_eq!(from_slice.len(), from_db.len());
        assert_eq!(from_slice[0].variant_id, from_db[0].variant_id);
        assert_eq!(from_slice[0].name, from_db[0].name);
        assert_eq!(from_slice[0].db_id, from_db[0].db_id);
    }

    /// The parallel bulk load must equal a sequential per-game load: same
    /// games in the same sorted order, variants expanded identically.
    #[test]
    fn test_load_saved_games_matches_sequential_load() {
        let tmp = tempfile::tempdir().unwrap();
        let db = ira_db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let save_dir = tmp.path().join("save").to_string_lossy().into_owned();
        let mut ids = Vec::new();
        for i in 0..40 {
            let entry = saved_game_fixture(&db, &format!("app{i}"), &format!("Game {i:03}"));
            ids.push(entry.id);
        }
        add_show_as_entry_variant(&db, ids[0], "Extra");

        let parallel = load_saved_games(&db, &save_dir);
        let entries = ira_db::load_all_games_unordered(&db).unwrap();
        let mut sequential = Vec::new();
        for entry in &entries {
            let game = load_game_fast(entry, &save_dir).unwrap();
            let variants = build_variant_entries(&db, &save_dir, &game);
            sequential.push(game);
            sequential.extend(variants);
        }
        sequential.sort_by(|a: &Game, b: &Game| a.sort_key().cmp(b.sort_key()));

        let key = |g: &Game| (g.db_id, g.variant_id, g.name.clone());
        let mut parallel_keys: Vec<_> = parallel.iter().map(key).collect();
        let mut sequential_keys: Vec<_> = sequential.iter().map(key).collect();
        parallel_keys.sort();
        sequential_keys.sort();
        assert_eq!(parallel_keys, sequential_keys);
        // 40 bases plus the one show-as-entry variant.
        assert_eq!(parallel.len(), 41);
    }

    #[test]
    fn test_worker_thread_count_stays_sequential_when_small() {
        assert_eq!(worker_thread_count(0), 1);
        assert_eq!(worker_thread_count(31), 1);
        let many = worker_thread_count(10_000);
        assert!((1..=8).contains(&many));
    }
}
