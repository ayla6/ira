use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::consoles::{all_consoles, ConsoleDef};
use crate::retroachievements::api::{RaClient, RaGameEntry};
use ira_config::Config;
use ira_models::{Game, GameDisc, GameKind, TrophySource};

use super::discovery_helpers::{group_multi_disc_roms, normalize_name, scan_roms};

struct ActiveConsole {
    def: &'static ConsoleDef,
    folder: String,
}

fn active_consoles(cfg: &Config) -> Vec<ActiveConsole> {
    all_consoles()
        .filter_map(|def| {
            if !def.uses_rom_folder() {
                return None;
            }
            let cc = cfg.console(def.id);
            let folder = cfg.rom_folder(def.id);
            (cc.enabled && !folder.as_os_str().is_empty()).then_some(ActiveConsole {
                def,
                folder: folder.to_string_lossy().into_owned(),
            })
        })
        .collect()
}

pub fn build_ra_games(
    db: &ira_db::DbConn,
    save_dir: &str,
    cfg: &Config,
    load_game: impl Fn(&ira_models::GameEntry, &str) -> Result<ira_models::Game, String> + Sync,
    progress: impl Fn(&str) + Sync,
) -> Vec<Game> {
    let consoles = active_consoles(cfg);
    let load_game = &load_game;
    progress("Checking ROM library caches…");

    let needs_fetch = consoles.iter().any(|c| {
        c.def.ra_console_id != 0
            && !crate::retroachievements::api::RaClient::console_cache_is_current(
                save_dir,
                c.def.ra_console_id,
            )
    });
    if needs_fetch {
        if let Some(ra_client) = RaClient::from_config(cfg) {
            for console in consoles.iter().filter(|c| c.def.ra_console_id != 0) {
                progress(&format!("Updating {} game list…", console.def.display_name));
                if let Err(e) = ra_client.fetch_console_games(save_dir, console.def.ra_console_id) {
                    eprintln!(
                        "RA: failed to fetch console games for {}: {}",
                        console.def.id, e
                    );
                }
            }
        }
    }

    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for console in &consoles {
            let db = db.clone();
            let progress = &progress;
            handles.push(s.spawn(move || {
                let _console_span =
                    tracing::info_span!("ra_console", console = console.def.id).entered();
                build_ra_games_for_console(
                    &db,
                    save_dir,
                    console,
                    cfg.unpack_roms,
                    load_game,
                    progress,
                )
            }));
        }

        let mut games = Vec::new();
        for h in handles {
            match h.join() {
                Ok(console_games) => games.extend(console_games),
                Err(_) => eprintln!("RA: console thread panicked"),
            }
        }
        games
    })
}

fn build_ra_games_for_console(
    db: &ira_db::DbConn,
    save_dir: &str,
    console: &ActiveConsole,
    unpack_roms: bool,
    load_game: &dyn Fn(&ira_models::GameEntry, &str) -> Result<ira_models::Game, String>,
    progress: &dyn Fn(&str),
) -> Vec<Game> {
    let mut games = Vec::new();

    let scan_result = {
        let _s = tracing::info_span!("scan_roms").entered();
        progress(&format!("Scanning {} ROMs…", console.def.display_name));
        scan_roms(&console.folder, console.def.extensions)
    };
    let scan_succeeded = scan_result.is_some();
    if !scan_succeeded {
        eprintln!(
            "RA: ROM folder is missing or unreadable for {}: {}",
            console.def.id, console.folder
        );
    }
    let roms = scan_result.unwrap_or_default();

    let to_relative = |abs_path: &std::path::Path| -> String {
        abs_path
            .strip_prefix(&console.folder)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| abs_path.to_string_lossy().into_owned())
    };

    let existing_entries = {
        let _s = tracing::info_span!("db_find_retro").entered();
        ira_db::find_all_retro_by_platform(db, console.def.id).unwrap_or_default()
    };
    let disc_paths = {
        let _s = tracing::info_span!("db_disc_paths").entered();
        ira_db::get_disc_paths_for_platform(db, console.def.id).unwrap_or_default()
    };
    let disc_owners = ira_db::get_disc_owners_for_platform(db, console.def.id).unwrap_or_default();

    let all_groups = group_multi_disc_roms(roms);
    let grouped_paths: HashSet<String> = all_groups
        .iter()
        .filter(|group| group.roms.len() > 1)
        .flat_map(|group| group.roms.iter())
        .map(|(_, path, _)| to_relative(path))
        .collect();

    let known_paths: HashSet<String> = existing_entries
        .iter()
        .filter(|entry| !entry.rom_path.is_empty())
        .map(|entry| entry.rom_path.clone())
        .chain(disc_paths)
        .collect();

    let mut seen_paths: HashSet<String> = HashSet::new();
    let mut new_roms: Vec<(String, PathBuf)> = Vec::new();
    for group in &all_groups {
        for (name, path, _) in &group.roms {
            let relative = to_relative(path);
            if grouped_paths.contains(&relative) || !known_paths.contains(&relative) {
                new_roms.push((name.clone(), path.clone()));
            }
            seen_paths.insert(relative);
        }
    }

    if scan_succeeded {
        for entry in &existing_entries {
            if !entry.rom_path.is_empty() && !seen_paths.contains(&entry.rom_path) {
                if let Err(e) = ira_db::set_rom_path(db, entry.id, "") {
                    eprintln!("Failed to clear stale ROM path: {}", e);
                }
                if let Err(e) = ira_db::delete_discs(db, entry.id) {
                    eprintln!("Failed to delete stale discs: {}", e);
                }
            }
        }
    }

    let existing_by_path: HashMap<String, ira_models::GameEntry> = existing_entries
        .iter()
        .filter(|e| {
            !e.rom_path.is_empty() && rom_path_is_present(scan_succeeded, &seen_paths, &e.rom_path)
        })
        .map(|e| (e.rom_path.clone(), e.clone()))
        .collect();

    let needs_ra_cache = !new_roms.is_empty()
        || existing_by_path
            .values()
            .any(|e| e.trophy_source == ira_models::TrophySource::Empty && !e.manual_unmatch);
    let ra_games: Vec<RaGameEntry> = if console.def.ra_console_id != 0 && needs_ra_cache {
        let _cs = tracing::info_span!("read_console_games_cache").entered();
        match crate::retroachievements::read_console_games_cache(
            save_dir,
            console.def.ra_console_id,
        ) {
            Some(g) => g
                .into_iter()
                .filter(|g| !g.title.contains('~') && !g.title.contains("[Subset"))
                .collect(),
            None => {
                if !new_roms.is_empty() {
                    eprintln!(
                        "RA: no cached game list for {}, new ROMs won't be matched",
                        console.def.id
                    );
                }
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let ra_title_map: HashMap<String, u32> = ra_games
        .iter()
        .map(|g| (normalize_name(&g.title), g.id))
        .collect();

    {
        let _s = tracing::info_span!("load_known_games", count = existing_by_path.len()).entered();
        for (rom_path_str, entry) in &existing_by_path {
            if grouped_paths.contains(rom_path_str) {
                continue;
            }
            let mut entry = entry.clone();

            if entry.trophy_source == ira_models::TrophySource::Empty
                && !entry.manual_unmatch
                && !ra_title_map.is_empty()
            {
                let rom_name = std::path::Path::new(&rom_path_str)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let rom_norm = normalize_name(&rom_name);
                if let Some(&ra_id) = ra_title_map.get(&rom_norm) {
                    let new_game_id = ra_id.to_string();
                    if ira_db::find_by_game_id(db, &new_game_id, console.def.id)
                        .ok()
                        .flatten()
                        .is_none()
                    {
                        let ra_title = ra_games
                            .iter()
                            .find(|g| g.id == ra_id)
                            .map(|g| g.title.clone())
                            .unwrap_or_else(|| rom_name.clone());
                        if let Err(e) = ira_db::update_game_ids(
                            db,
                            entry.id,
                            "",
                            &new_game_id,
                            ira_models::TrophySource::Ra,
                            console.def.id,
                        ) {
                            eprintln!("Failed to update game IDs for RA match: {}", e);
                        }
                        entry.game_id = new_game_id;
                        entry.trophy_source = ira_models::TrophySource::Ra;
                        if entry.title.is_empty() {
                            entry.title = ra_title;
                        }
                    }
                }
            }

            let _gs = tracing::info_span!("load_game", app_id = &entry.steam_id).entered();
            let mut g = load_game(&entry, save_dir).unwrap_or_else(|_| Game {
                app_id: if !entry.steam_id.is_empty() {
                    entry.steam_id.clone()
                } else {
                    entry.game_id.clone()
                },
                kind: GameKind::Retro,
                trophy_source: entry.trophy_source,
                platform_id: entry.platform_id.clone(),
                db_id: entry.id,
                name: if entry.title.is_empty() {
                    rom_path_str.clone()
                } else {
                    entry.title.clone()
                },
                ..Default::default()
            });
            if g.name_lower.is_empty() {
                g.name_lower = g.name.to_lowercase();
            }
            g.game_path = rom_path_str.clone();
            g.rom_path = rom_path_str.clone();
            games.push(g);
        }
    }

    if !new_roms.is_empty() {
        let _s = tracing::info_span!("process_new_roms", count = new_roms.len()).entered();

        let groups = group_multi_disc_roms(new_roms);
        for group in &groups {
            let (rom_name, rom_path, _disc_num) = &group.roms[0];
            let rom_path_str = to_relative(rom_path);

            let matched_id = if !ra_title_map.is_empty() {
                let rom_norm = normalize_name(rom_name);
                if rom_norm.is_empty() {
                    None
                } else {
                    ra_title_map.get(&rom_norm).copied()
                }
            } else {
                None
            };

            let serial = if matched_id.is_some() {
                group.serial.clone()
            } else {
                group
                    .serial
                    .clone()
                    .or_else(|| platform_serial(console.def.id, rom_path))
            };

            let (app_id, title, trophy_source) = match matched_id {
                Some(id) => {
                    let t = ra_games
                        .iter()
                        .find(|g| g.id == id)
                        .map(|g| g.title.clone())
                        .unwrap_or_else(|| rom_name.clone());
                    (id.to_string(), t, TrophySource::Ra)
                }
                None => (
                    serial.clone().unwrap_or_else(|| rom_name.clone()),
                    rom_name.clone(),
                    TrophySource::Empty,
                ),
            };

            let mut candidate_ids: Vec<i64> = group
                .roms
                .iter()
                .filter_map(|(_, path, _)| {
                    let path = to_relative(path);
                    existing_by_path
                        .get(&path)
                        .map(|entry| entry.id)
                        .or_else(|| disc_owners.get(&path).copied())
                })
                .collect();
            candidate_ids.sort_unstable();
            candidate_ids.dedup();

            let id_from_key = ira_db::find_by_game_id(db, &app_id, console.def.id)
                .ok()
                .flatten()
                .map(|entry| entry.id);
            if let Some(id) = id_from_key {
                candidate_ids.push(id);
                candidate_ids.sort_unstable();
                candidate_ids.dedup();
            }

            let canonical_id = id_from_key.or_else(|| candidate_ids.first().copied());
            if let Some(canonical_id) = canonical_id {
                let duplicate_ids: Vec<i64> = candidate_ids
                    .iter()
                    .copied()
                    .filter(|id| *id != canonical_id)
                    .collect();
                if !duplicate_ids.is_empty() {
                    if let Err(e) = ira_db::merge_duplicate_games(db, canonical_id, &duplicate_ids)
                    {
                        eprintln!("Failed to merge duplicate retro games: {e}");
                    }
                }
            }

            let existing_by_id =
                canonical_id.and_then(|id| ira_db::find_by_db_id(db, id).ok().flatten());
            let game = match existing_by_id {
                Some(e) => {
                    if e.rom_path.is_empty() || group.roms.len() > 1 {
                        if let Err(e) = ira_db::set_rom_path(db, e.id, &rom_path_str) {
                            eprintln!("Failed to set ROM path: {}", e);
                        }
                    }
                    let mut g = load_game(&e, save_dir).unwrap_or_else(|_| Game {
                        app_id: e.game_id.clone(),
                        kind: GameKind::Retro,
                        trophy_source: e.trophy_source,
                        platform_id: e.platform_id.clone(),
                        db_id: e.id,
                        name: if e.title.is_empty() {
                            title.clone()
                        } else {
                            e.title.clone()
                        },
                        ..Default::default()
                    });
                    if g.name_lower.is_empty() {
                        g.name_lower = g.name.to_lowercase();
                    }
                    g.game_path = rom_path_str.clone();
                    g.rom_path = rom_path_str;
                    g
                }
                None => {
                    match ira_db::add_game(
                        db,
                        GameKind::Retro,
                        trophy_source,
                        "",
                        &app_id,
                        console.def.id,
                        &title,
                    ) {
                        Ok(id) => {
                            if let Err(e) = ira_db::set_rom_path(db, id, &rom_path_str) {
                                eprintln!("Failed to set ROM path: {}", e);
                            }
                            Game {
                                app_id: app_id.clone(),
                                kind: GameKind::Retro,
                                trophy_source,
                                platform_id: console.def.id.to_string(),
                                db_id: id,
                                name: title.clone(),
                                name_lower: title.to_lowercase(),
                                game_path: rom_path_str.clone(),
                                rom_path: rom_path_str,
                                ..Default::default()
                            }
                        }
                        Err(e) => {
                            eprintln!("RA: failed to add {} to DB: {}", rom_name, e);
                            continue;
                        }
                    }
                }
            };

            if let Err(e) = ira_db::delete_discs(db, game.db_id) {
                eprintln!("Failed to delete discs: {}", e);
            }
            if group.roms.len() > 1 {
                for (i, (_, disc_path, disc_num)) in group.roms.iter().enumerate() {
                    let disc_num = disc_num.unwrap_or((i + 1) as i32);
                    let disc_path_str = to_relative(disc_path);
                    let label = format!("Disc {}", disc_num);
                    if let Err(e) = ira_db::add_disc(
                        db,
                        &GameDisc {
                            id: 0,
                            game_id: game.db_id,
                            disc_number: disc_num,
                            rom_path: disc_path_str,
                            label,
                        },
                    ) {
                        eprintln!("Failed to add disc: {}", e);
                    }
                }
            }

            games.push(game);
        }
    }

    if console.def.id == "nds" {
        enrich_nds_roms(
            db,
            save_dir,
            &console.folder,
            unpack_roms,
            &existing_entries,
            &games,
        );
    }

    games
}

/// Extracts DS banner icons and identification hashes (No-Intro CRC32,
/// RetroAchievements hash) for games that don't have them yet. Reading a
/// ROM streams — and for containers, decompresses — its whole image, so
/// the reads run concurrently while the cheap writes stay serial.
/// Archives are handled by `read_rom_info`, which skips them unless
/// `unpack_roms` is on.
fn enrich_nds_roms(
    db: &ira_db::DbConn,
    save_dir: &str,
    folder: &str,
    unpack_roms: bool,
    existing: &[ira_models::GameEntry],
    games: &[Game],
) {
    use rayon::prelude::*;

    let already_hashed: HashSet<i64> = existing
        .iter()
        .filter(|entry| !entry.rom_hash.is_empty())
        .map(|entry| entry.id)
        .collect();
    let targets: Vec<(i64, PathBuf)> = games
        .iter()
        .filter(|game| !game.rom_path.is_empty() && !already_hashed.contains(&game.db_id))
        .map(|game| {
            (
                game.db_id,
                std::path::Path::new(folder).join(&game.rom_path),
            )
        })
        .collect();

    let infos: Vec<Option<crate::nds::DsRomInfo>> = targets
        .par_iter()
        .map(|(_, abs)| crate::nds::read_rom_info(abs, unpack_roms))
        .collect();
    for ((db_id, _), info) in targets.into_iter().zip(infos) {
        let Some(info) = info else {
            continue;
        };
        if let Err(e) = ira_db::set_rom_hashes(db, db_id, &info.rom_crc32, &info.rom_hash) {
            eprintln!("Failed to store DS ROM hashes: {e}");
        }
        write_nds_icon(save_dir, db_id, &info.icon);
    }
}

/// Saves the banner icon into the game's retro data dir unless one already
/// exists, so downloaded or user-chosen icons always win.
fn write_nds_icon(save_dir: &str, db_id: i64, icon_rgba: &[u8]) {
    let data_dir = ira_parser::retro_data_dir(save_dir, db_id);
    if ira_parser::find_image_file(&data_dir, "icon").is_some() {
        return;
    }
    if std::fs::create_dir_all(&data_dir).is_err() {
        return;
    }
    let png = data_dir.join("icon.png");
    if let Err(e) = ira_parser::save_rgba_png(&png, 32, 32, icon_rgba) {
        eprintln!("Failed to write DS icon for game {db_id}: {e}");
        return;
    }
    ira_parser::convert_to_lossless_webp(&png);
}

fn rom_path_is_present(scan_succeeded: bool, seen_paths: &HashSet<String>, rom_path: &str) -> bool {
    !scan_succeeded || seen_paths.contains(rom_path)
}

/// Reads the serial a ROM carries in its own data, so identity does not
/// depend on the file name. DS ROMs expose a header game code; other
/// consoles use the disc-info helper.
fn platform_serial(console_id: &str, path: &std::path::Path) -> Option<String> {
    match console_id {
        "nds" => crate::nds::read_serial(path),
        _ => crate::rom_serial::read_serial(path),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::discovery_helpers::*;
    use crate::retroachievements::api::RaGameEntry;

    #[test]
    fn test_normalize_name_basic() {
        assert_eq!(
            normalize_name("Final Fantasy VII (USA)"),
            "final fantasy vii"
        );
    }

    #[test]
    fn test_normalize_name_underscores() {
        assert_eq!(normalize_name("Final_Fantasy.VII"), "final fantasy vii");
    }

    #[test]
    fn test_normalize_name_version_tags() {
        assert_eq!(normalize_name("Chrono Trigger [!]"), "chrono trigger");
    }

    #[test]
    fn test_normalize_name_empty() {
        assert_eq!(normalize_name(""), "");
    }

    #[test]
    fn test_match_rom_exact() {
        let games = vec![RaGameEntry {
            id: 1,
            title: "Final Fantasy VII".to_string(),
            image_icon: String::new(),
            image_url: String::new(),
            num_achievements: 0,
            points: 0,
        }];
        assert_eq!(match_rom_to_game("Final Fantasy VII", &games), Some(1));
    }

    #[test]
    fn test_match_rom_normalized() {
        let games = vec![RaGameEntry {
            id: 42,
            title: "Chrono Trigger".to_string(),
            image_icon: String::new(),
            image_url: String::new(),
            num_achievements: 0,
            points: 0,
        }];
        assert_eq!(match_rom_to_game("Chrono_Trigger", &games), Some(42));
    }

    #[test]
    fn test_match_rom_no_match() {
        let games = vec![RaGameEntry {
            id: 1,
            title: "Final Fantasy VII".to_string(),
            image_icon: String::new(),
            image_url: String::new(),
            num_achievements: 0,
            points: 0,
        }];
        assert_eq!(match_rom_to_game("Completely Different Game", &games), None);
    }

    #[test]
    fn test_strip_disc_pattern_paren() {
        assert_eq!(
            strip_disc_pattern("Final Fantasy VII (Disc 1)"),
            Some(("Final Fantasy VII".to_string(), 1))
        );
        assert_eq!(
            strip_disc_pattern("Final Fantasy VII (Disc 2)"),
            Some(("Final Fantasy VII".to_string(), 2))
        );
    }

    #[test]
    fn test_strip_disc_pattern_bracket() {
        assert_eq!(
            strip_disc_pattern("Metal Gear Solid [Disc 1]"),
            Some(("Metal Gear Solid".to_string(), 1))
        );
        assert_eq!(
            strip_disc_pattern("Game [CD 2]"),
            Some(("Game".to_string(), 2))
        );
    }

    #[test]
    fn test_strip_disc_pattern_disk_variant() {
        assert_eq!(
            strip_disc_pattern("Game (Disk 1)"),
            Some(("Game".to_string(), 1))
        );
        assert_eq!(
            strip_disc_pattern("Game (Disk 3)"),
            Some(("Game".to_string(), 3))
        );
    }

    #[test]
    fn test_strip_disc_pattern_no_match() {
        assert_eq!(strip_disc_pattern("Final Fantasy VII"), None);
        assert_eq!(strip_disc_pattern("Game (USA)"), None);
        assert_eq!(strip_disc_pattern("Game [!]"), None);
    }

    #[test]
    fn test_strip_disc_pattern_preserves_base() {
        let (base, disc) = strip_disc_pattern("Resident Evil 2 (Disc 1) (USA)").unwrap();
        assert_eq!(disc, 1);
        assert!(base.contains("Resident Evil 2"));
    }

    #[test]
    fn test_group_multi_disc_by_pattern() {
        let roms = vec![
            (
                "Final Fantasy VII (Disc 1)".to_string(),
                PathBuf::from("/games/ff7_d1.bin"),
            ),
            (
                "Final Fantasy VII (Disc 2)".to_string(),
                PathBuf::from("/games/ff7_d2.bin"),
            ),
            (
                "Final Fantasy VII (Disc 3)".to_string(),
                PathBuf::from("/games/ff7_d3.bin"),
            ),
            ("Chrono Trigger".to_string(), PathBuf::from("/games/ct.bin")),
        ];
        let groups = group_multi_disc_roms(roms);
        assert_eq!(groups.len(), 2);
        let ff7 = groups.iter().find(|g| g.roms.len() == 3).unwrap();
        assert_eq!(ff7.roms[0].2, Some(1));
        assert_eq!(ff7.roms[1].2, Some(2));
        assert_eq!(ff7.roms[2].2, Some(3));
    }

    #[test]
    fn test_group_single_rom() {
        let roms = vec![("Game".to_string(), PathBuf::from("/games/game.bin"))];
        let groups = group_multi_disc_roms(roms);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].roms.len(), 1);
    }

    #[test]
    fn test_rom_path_is_present_after_successful_scan() {
        let seen_paths = std::collections::HashSet::from(["game.iso".to_string()]);
        assert!(super::rom_path_is_present(true, &seen_paths, "game.iso"));
        assert!(!super::rom_path_is_present(true, &seen_paths, "moved.iso"));
        assert!(super::rom_path_is_present(false, &seen_paths, "moved.iso"));
    }
}
