use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::consoles::{all_consoles, ConsoleDef};
use crate::retroachievements::api::{RaClient, RaGameEntry};
use ira_config::Config;
use ira_models::{Game, GameDisc, TrophySource};

use super::discovery_helpers::{
    group_multi_disc_roms, normalize_name, scan_roms, DiscGroup, RaMatchIndex,
};

struct ActiveConsole {
    def: &'static ConsoleDef,
    /// This console's folder inside every configured ROM root,
    /// in root priority order.
    folders: Vec<std::path::PathBuf>,
    /// The configured emulator executable, used to locate per-emulator
    /// metadata caches (Eden's Switch game list cache).
    executable: String,
}

fn active_consoles(cfg: &Config) -> Vec<ActiveConsole> {
    all_consoles()
        .filter_map(|def| {
            if !def.uses_rom_folder() {
                return None;
            }
            let cc = cfg.console(def.id);
            if !cc.enabled {
                return None;
            }
            let folders = cfg.all_rom_roots();
            if folders.is_empty() {
                return None;
            }
            let folders = folders
                .iter()
                .map(|root| root.join(def.id))
                .collect::<Vec<_>>();
            Some(ActiveConsole {
                def,
                folders,
                executable: cc.executable.clone(),
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

    let scan_results = {
        let _s = tracing::info_span!("scan_roms").entered();
        progress(&format!("Scanning {} ROMs…", console.def.display_name));
        console
            .folders
            .iter()
            .map(|folder| scan_roms(&folder.to_string_lossy(), console.def.extensions))
            .collect::<Vec<_>>()
    };
    let scan_succeeded = scan_results.iter().any(Option::is_some);
    if !scan_succeeded {
        eprintln!(
            "RA: no readable ROM folder for {} (tried {:?})",
            console.def.id, console.folders
        );
    }
    let roms: Vec<(String, PathBuf)> = scan_results
        .into_iter()
        .flatten()
        .flat_map(|roms| roms.into_iter())
        .collect();

    let to_relative = |abs_path: &std::path::Path| -> String {
        for folder in &console.folders {
            if let Ok(rel) = abs_path.strip_prefix(folder) {
                return rel.to_string_lossy().into_owned();
            }
        }
        abs_path.to_string_lossy().into_owned()
    };

    let existing_entries = {
        let _s = tracing::info_span!("db_find_rom").entered();
        ira_db::find_all_rom_by_platform(db, console.def.id).unwrap_or_default()
    };
    let disc_paths = {
        let _s = tracing::info_span!("db_disc_paths").entered();
        ira_db::get_disc_paths_for_platform(db, console.def.id).unwrap_or_default()
    };
    let disc_owners = ira_db::get_disc_owners_for_platform(db, console.def.id).unwrap_or_default();

    let all_groups = group_multi_disc_roms(db, roms);
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
    let ra_index = RaMatchIndex::new(&ra_games);

    // Switch titles carry their native metadata in Eden's game list cache
    // instead of an RA list; resolve it once per scan.
    let eden_cache = (console.def.id == "switch")
        .then(|| crate::switch::SwitchCaches::load(&console.executable));
    let switch_metas =
        precompute_switch_metas(console, eden_cache.as_ref(), &new_roms, &to_relative);

    {
        let _s = tracing::info_span!("load_known_games", count = existing_by_path.len()).entered();
        for (rom_path_str, entry) in &existing_by_path {
            if grouped_paths.contains(rom_path_str) {
                continue;
            }
            let mut entry = entry.clone();

            if entry.trophy_source == ira_models::TrophySource::Empty
                && !entry.manual_unmatch
                && !ra_index.is_empty()
            {
                let rom_name = std::path::Path::new(&rom_path_str)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let rom_norm = normalize_name(&rom_name);
                if let Some(ra_id) = ra_index.find(&entry.rom_hash, &rom_norm) {
                    let new_game_id = ra_id.to_string();
                    if ira_db::find_by_game_id(db, &new_game_id, console.def.id)
                        .ok()
                        .flatten()
                        .is_none()
                    {
                        let ra_title = ra_index
                            .title_of(ra_id)
                            .map(str::to_string)
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
                kind: console.def.game_kind(),
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

    let mut hashed_now: HashSet<i64> = HashSet::new();
    if !new_roms.is_empty() {
        let _s = tracing::info_span!("process_new_roms", count = new_roms.len()).entered();

        let groups = group_multi_disc_roms(db, new_roms);
        let nds_infos = precompute_nds_infos(console, unpack_roms, &groups, &to_relative);
        for group in &groups {
            let (rom_name, rom_path, _disc_num) = &group.roms[0];
            let rom_path_str = to_relative(rom_path);

            let rom_norm = normalize_name(rom_name);
            let rom_hash = nds_infos
                .get(&rom_path_str)
                .map(|info| info.rom_hash.as_str())
                .unwrap_or_default();
            let matched_id = ra_index.find(rom_hash, &rom_norm);

            let serial = if matched_id.is_some() {
                group.serial.clone()
            } else {
                group
                    .serial
                    .clone()
                    .or_else(|| platform_serial(console.def.id, db, rom_path))
            };

            let (app_id, title, trophy_source) = match matched_id {
                Some(id) => {
                    let t = ra_index
                        .title_of(id)
                        .map(str::to_string)
                        .unwrap_or_else(|| rom_name.clone());
                    (id.to_string(), t, TrophySource::Ra)
                }
                None => {
                    // Switch: native title id and application title from
                    // the emulator caches or, with keys installed, the
                    // ROM's own control NACP; the file name stays the
                    // final fallback.
                    let meta = switch_metas.get(&rom_path_str);
                    let native_id = meta
                        .and_then(|m| (!m.title_id.is_empty()).then(|| m.title_id.clone()));
                    let native_title =
                        meta.and_then(|m| (!m.title.is_empty()).then(|| m.title.clone()));
                    (
                        native_id.or(serial.clone()).unwrap_or_else(|| rom_name.clone()),
                        native_title.unwrap_or_else(|| rom_name.clone()),
                        TrophySource::Empty,
                    )
                }
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
            let had_hash = existing_by_id
                .as_ref()
                .is_some_and(|e| !e.rom_hash.is_empty());
            // Resolved before the match below: `rom_path_str` is moved into
            // the built game in both branches.
            let nds_info = if had_hash {
                None
            } else {
                nds_infos.get(&rom_path_str)
            };
            let switch_meta = switch_metas.get(&rom_path_str);
            let game = match existing_by_id {
                Some(e) => {
                    if e.rom_path.is_empty() || group.roms.len() > 1 {
                        if let Err(e) = ira_db::set_rom_path(db, e.id, &rom_path_str) {
                            eprintln!("Failed to set ROM path: {}", e);
                        }
                    }
                    let mut g = load_game(&e, save_dir).unwrap_or_else(|_| Game {
                        app_id: e.game_id.clone(),
                        kind: console.def.game_kind(),
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
                        console.def.game_kind(),
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
                                kind: console.def.game_kind(),
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

            if let Some(info) = nds_info {
                if let Err(e) = ira_db::set_rom_hash(db, game.db_id, &info.rom_hash) {
                    eprintln!("Failed to store DS ROM hash: {e}");
                }
                write_nds_icon(save_dir, game.db_id, &info.icon);
                hashed_now.insert(game.db_id);
            }

            if let Some(meta) = switch_meta {
                write_switch_icon(save_dir, game.db_id, &meta.icon);
            }

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
            &console.folders,
            unpack_roms,
            &existing_entries,
            &games,
            &hashed_now,
        );
    }

    if console.def.id == "switch" {
        enrich_switch_roms(db, save_dir, console, eden_cache.as_ref(), &mut games);
    }

    games
}

/// Resolve a ROM path relative to the console folder against every
/// configured root, preferring a root where the file exists.
fn resolve_in_folders(folders: &[PathBuf], relative: &str) -> PathBuf {
    for folder in folders {
        let candidate = folder.join(relative);
        if candidate.exists() {
            return candidate;
        }
    }
    match folders.first() {
        Some(folder) => folder.join(relative),
        None => PathBuf::from(relative),
    }
}

/// Hashes the first disc of every new ROM group up front on NDS so the
/// scan's first pass can match by exact RA hash rather than title alone.
/// Keyed by the ROM path relative to the console folder; always empty for
/// other consoles or when ROM reading is disabled.
fn precompute_nds_infos(
    console: &ActiveConsole,
    unpack_roms: bool,
    groups: &[DiscGroup],
    to_relative: &dyn Fn(&std::path::Path) -> String,
) -> HashMap<String, crate::nds::DsRomInfo> {
    if console.def.id != "nds" || !unpack_roms {
        return HashMap::new();
    }
    use rayon::prelude::*;

    let targets: Vec<(String, PathBuf)> = groups
        .iter()
        .filter_map(|group| group.roms.first())
        .map(|(_, path, _)| (to_relative(path), path.clone()))
        .collect();
    let infos: Vec<Option<crate::nds::DsRomInfo>> = targets
        .par_iter()
        .map(|(_, abs)| crate::nds::read_rom_info(abs, unpack_roms))
        .collect();
    targets
        .into_iter()
        .zip(infos)
        .filter_map(|(target, info)| info.map(|i| (target.0, i)))
        .collect()
}

/// Extracts DS banner icons and RetroAchievements hashes for games that
/// don't have them yet. Reads stop once the hashed header ranges are in,
/// so containers only decompress a few megabytes; the reads run
/// concurrently while the cheap writes stay serial. Skipped entirely when
/// `unpack_roms` is off, so scans never touch ROM files.
fn enrich_nds_roms(
    db: &ira_db::DbConn,
    save_dir: &str,
    folders: &[PathBuf],
    unpack_roms: bool,
    existing: &[ira_models::GameEntry],
    games: &[Game],
    hashed_now: &HashSet<i64>,
) {
    if !unpack_roms {
        return;
    }
    use rayon::prelude::*;

    let already_hashed: HashSet<i64> = existing
        .iter()
        .filter(|entry| !entry.rom_hash.is_empty())
        .map(|entry| entry.id)
        .collect();
    let targets: Vec<(i64, PathBuf)> = games
        .iter()
        .filter(|game| {
            !game.rom_path.is_empty()
                && !already_hashed.contains(&game.db_id)
                && !hashed_now.contains(&game.db_id)
        })
        .map(|game| (game.db_id, resolve_in_folders(folders, &game.rom_path)))
        .collect();

    let infos: Vec<Option<crate::nds::DsRomInfo>> = targets
        .par_iter()
        .map(|(_, abs)| crate::nds::read_rom_info(abs, unpack_roms))
        .collect();
    for ((db_id, _), info) in targets.into_iter().zip(infos) {
        let Some(info) = info else {
            continue;
        };
        if let Err(e) = ira_db::set_rom_hash(db, db_id, &info.rom_hash) {
            eprintln!("Failed to store DS ROM hash: {e}");
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

/// Resolves cached and ROM-native metadata (control-NCA icon and NACP
/// title, see `switch::rom_meta_deep`) for every new Switch ROM up front;
/// keyed by the ROM path relative to the console folder. Always empty for
/// other consoles.
fn precompute_switch_metas(
    console: &ActiveConsole,
    cache: Option<&crate::switch::SwitchCaches>,
    new_roms: &[(String, PathBuf)],
    to_relative: &dyn Fn(&std::path::Path) -> String,
) -> HashMap<String, crate::switch::SwitchRomMeta> {
    if console.def.id != "switch" {
        return HashMap::new();
    }
    let Some(cache) = cache else {
        return HashMap::new();
    };
    use rayon::prelude::*;

    let metas: Vec<crate::switch::SwitchRomMeta> = new_roms
        .par_iter()
        .map(|(_, abs)| crate::switch::rom_meta_deep(abs, cache, &console.executable))
        .collect();
    new_roms
        .iter()
        .map(|(_, abs)| to_relative(abs))
        .zip(metas)
        .collect()
}

/// Saves a Switch title's native icon (an emulator-cached JPEG, the
/// ROM's decrypted control-NCA icon, or a homebrew NRO's embedded PNG)
/// into the game's switch data dir as lossless WebP unless one already
/// exists, so downloaded or user-chosen icons always win.
fn write_switch_icon(save_dir: &str, db_id: i64, icon: &crate::switch::SwitchIcon) {
    let data_dir = ira_parser::switch_data_dir(save_dir, db_id);
    if ira_parser::find_image_file(&data_dir, "icon").is_some() {
        return;
    }
    match icon {
        crate::switch::SwitchIcon::File(cached) => {
            if let Err(e) = std::fs::create_dir_all(&data_dir) {
                eprintln!("Failed to create data dir for game {db_id}: {e}");
                return;
            }
            if ira_parser::import_image_as_webp(cached, &data_dir, "icon").is_none() {
                eprintln!("Failed to import Switch icon for game {db_id}");
            }
        }
        crate::switch::SwitchIcon::Bytes(raw) => {
            if std::fs::create_dir_all(&data_dir).is_err() {
                return;
            }
            match ira_parser::encode_bytes_to_lossless_webp(raw) {
                Some(webp) => {
                    if std::fs::write(data_dir.join("icon.webp"), webp).is_err() {
                        eprintln!("Failed to write Switch icon for game {db_id}");
                    }
                }
                None => eprintln!("Failed to decode Switch icon for game {db_id}"),
            }
        }
        crate::switch::SwitchIcon::None => {}
    }
}

/// Backfills native Switch metadata onto games first scanned before the
/// Switch integration existed: the title id becomes the game id, the
/// native application title (emulator cache or the ROM's control NACP)
/// replaces the file-name-derived one, and the native icon (cache or
/// decrypted NCA) is imported. Games already carrying a title id are
/// left alone.
fn enrich_switch_roms(
    db: &ira_db::DbConn,
    save_dir: &str,
    console: &ActiveConsole,
    cache: Option<&crate::switch::SwitchCaches>,
    games: &mut [Game],
) {
    let Some(cache) = cache else {
        return;
    };
    for game in games.iter_mut() {
        if game.rom_path.is_empty() {
            continue;
        }
        let rom = resolve_in_folders(&console.folders, &game.rom_path);
        let meta = crate::switch::rom_meta_deep(&rom, cache, &console.executable);

        if !meta.title_id.is_empty() && !crate::switch::is_title_id(&game.app_id) {
            if let Err(e) = ira_db::update_game_ids(
                db,
                game.db_id,
                "",
                &meta.title_id,
                game.trophy_source,
                console.def.id,
            ) {
                eprintln!("Failed to set Switch title id for game {}: {e}", game.db_id);
            } else {
                game.app_id = meta.title_id.clone();
            }
        }

        let file_title = std::path::Path::new(&game.rom_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !meta.title.is_empty() && (game.name.is_empty() || game.name == file_title) {
            if let Err(e) = ira_db::update_game_title(db, game.db_id, &meta.title) {
                eprintln!("Failed to set Switch title for game {}: {e}", game.db_id);
            } else {
                game.set_name(&meta.title);
            }
        }

        write_switch_icon(save_dir, game.db_id, &meta.icon);
    }
}

fn rom_path_is_present(scan_succeeded: bool, seen_paths: &HashSet<String>, rom_path: &str) -> bool {
    !scan_succeeded || seen_paths.contains(rom_path)
}

/// Reads the serial a ROM carries in its own data, so identity does not
/// depend on the file name. DS ROMs expose a header game code; other
/// consoles use the cached disc-info reader.
fn platform_serial(
    console_id: &str,
    conn: &ira_db::DbConn,
    path: &std::path::Path,
) -> Option<String> {
    match console_id {
        "nds" => crate::nds::read_serial(path),
        _ => crate::rom_serial::read_serial_cached(conn, path),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::discovery_helpers::*;

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
        let groups = group_multi_disc_roms(&test_db(), roms);
        assert_eq!(groups.len(), 2);
        let ff7 = groups.iter().find(|g| g.roms.len() == 3).unwrap();
        assert_eq!(ff7.roms[0].2, Some(1));
        assert_eq!(ff7.roms[1].2, Some(2));
        assert_eq!(ff7.roms[2].2, Some(3));
    }

    #[test]
    fn test_group_single_rom() {
        let roms = vec![("Game".to_string(), PathBuf::from("/games/game.bin"))];
        let groups = group_multi_disc_roms(&test_db(), roms);
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

    fn test_db() -> ira_db::DbConn {
        let tmp = tempfile::tempdir().unwrap();
        let conn = ira_db::init_db(tmp.path().join("ira.db").to_str().unwrap());
        std::mem::forget(tmp);
        conn
    }

    /// A portable Eden cache with one title, and a `Game` row whose id and
    /// title still come from the ROM file name (pre-integration shape).
    fn switch_backfill_fixture()
    -> (tempfile::TempDir, ira_db::DbConn, ira_models::Game, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("eden.AppImage");
        std::fs::write(&exe, b"").unwrap();
        let cache_dir = tmp.path().join("user/cache/game_list");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::write(
            cache_dir.join("01007EF00011E000.appname.txt"),
            "The Legend of Zelda",
        )
        .unwrap();
        // PNG payload with a .jpeg name: imports decode by content.
        let probe = tmp.path().join("probe.png");
        ira_parser::save_rgba_png(&probe, 2, 2, &[7u8; 2 * 2 * 4]).unwrap();
        let png = std::fs::read(&probe).unwrap();
        std::fs::write(cache_dir.join("01007EF00011E000.jpeg"), png).unwrap();

        let rom_dir = tmp.path().join("roms/switch");
        std::fs::create_dir_all(&rom_dir).unwrap();
        let rom = rom_dir.join("The Legend of Zelda [01007EF00011E000].xci");
        std::fs::write(&rom, b"fake rom").unwrap();

        let db = test_db();
        let db_id = ira_db::add_game(
            &db,
            ira_models::GameKind::Switch,
            ira_models::TrophySource::Empty,
            "",
            "The Legend of Zelda [01007EF00011E000]",
            "switch",
            "The Legend of Zelda [01007EF00011E000]",
        )
        .unwrap();
        ira_db::set_rom_path(&db, db_id, "The Legend of Zelda [01007EF00011E000].xci").unwrap();

        let game = ira_models::Game {
            app_id: "The Legend of Zelda [01007EF00011E000]".into(),
            kind: ira_models::GameKind::Switch,
            trophy_source: ira_models::TrophySource::Empty,
            platform_id: "switch".into(),
            db_id,
            name: "The Legend of Zelda [01007EF00011E000]".into(),
            rom_path: "The Legend of Zelda [01007EF00011E000].xci".into(),
            ..Default::default()
        };
        (tmp, db, game, exe)
    }

    #[test]
    fn test_enrich_switch_roms_backfills_id_title_and_icon() {
        let (tmp, db, mut game, exe) = switch_backfill_fixture();
        let save_dir = tmp.path().join("save").to_str().unwrap().to_string();

        let console = super::ActiveConsole {
            def: ira_models::find_console("switch").unwrap(),
            folders: vec![tmp.path().join("roms/switch")],
            executable: exe.to_string_lossy().into_owned(),
        };
        let cache = crate::switch::SwitchCaches::load(&console.executable);
        super::enrich_switch_roms(&db, &save_dir, &console, Some(&cache), std::slice::from_mut(&mut game));

        // Game id and title now come from Eden's cache…
        assert_eq!(game.app_id, "01007ef00011e000");
        assert_eq!(game.name, "The Legend of Zelda");
        let entry = ira_db::find_by_db_id(&db, game.db_id).unwrap().unwrap();
        assert_eq!(entry.game_id, "01007ef00011e000");

        // …and the cached icon landed in the switch data dir.
        let data_dir = ira_parser::switch_data_dir(&save_dir, game.db_id);
        assert!(ira_parser::find_image_file(&data_dir, "icon").is_some());
    }

    #[test]
    fn test_enrich_switch_roms_keeps_title_ids_and_custom_titles() {
        let (tmp, db, mut game, exe) = switch_backfill_fixture();
        let save_dir = tmp.path().join("save").to_str().unwrap().to_string();

        // A game already carrying a title id and a custom title is untouched.
        ira_db::update_game_ids(
            &db,
            game.db_id,
            "",
            "01007ef00011e000",
            game.trophy_source,
            "switch",
        )
        .unwrap();
        game.app_id = "01007ef00011e000".into();
        game.set_name("My own name");

        let console = super::ActiveConsole {
            def: ira_models::find_console("switch").unwrap(),
            folders: vec![tmp.path().join("roms/switch")],
            executable: exe.to_string_lossy().into_owned(),
        };
        let cache = crate::switch::SwitchCaches::load(&console.executable);
        super::enrich_switch_roms(&db, &save_dir, &console, Some(&cache), std::slice::from_mut(&mut game));

        assert_eq!(game.name, "My own name");
        let entry = ira_db::find_by_db_id(&db, game.db_id).unwrap().unwrap();
        assert_eq!(entry.game_id, "01007ef00011e000");
    }
}
