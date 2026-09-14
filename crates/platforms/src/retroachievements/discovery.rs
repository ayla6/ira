use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::consoles::{all_consoles, ConsoleDef};
use crate::retroachievements::api::{is_main_ra_entry, RaClient, RaGameEntry};
use ira_config::Config;
use ira_models::{normalize_name, Game, GameDisc, TrophySource};

use super::discovery_helpers::{
    group_multi_disc_roms, rom_display_title, scan_roms, DiscGroup, RaMatchIndex,
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

    // The RA toggle governs talking to retroachievements.org; the ROM scan
    // itself runs offline regardless.
    let needs_fetch = cfg.ra_enabled
        && consoles.iter().any(|c| {
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
                    cfg.ra_enabled,
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
    ra_enabled: bool,
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
        ira_db::find_all_rom_by_platform(db, console.def.id).unwrap_or_else(|e| {
            eprintln!("Failed to list roms for {}: {}", console.def.id, e);
            Vec::new()
        })
    };
    let disc_paths = {
        let _s = tracing::info_span!("db_disc_paths").entered();
        ira_db::get_disc_paths_for_platform(db, console.def.id).unwrap_or_else(|e| {
            eprintln!("Failed to list discs for {}: {}", console.def.id, e);
            HashSet::new()
        })
    };
    let disc_owners = ira_db::get_disc_owners_for_platform(db, console.def.id)
        .unwrap_or_else(|e| {
            eprintln!("Failed to list disc owners for {}: {}", console.def.id, e);
            HashMap::new()
        });

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

    // Rows whose ROM vanished keep their content hash, so a file that
    // comes back under a new name or path can be pinned to its old row.
    let existing_by_hash: HashMap<&str, &ira_models::GameEntry> = existing_entries
        .iter()
        .filter(|e| !e.hashes.md5.is_empty())
        .map(|e| (e.hashes.md5.as_str(), e))
        .collect();

    // With RA disabled the match index stays empty: ROMs are discovered
    // offline and get matched whenever the integration is turned back on.
    let needs_ra_cache = ra_enabled
        && (!new_roms.is_empty()
            || existing_by_path
                .values()
                .any(|e| e.trophy_source == ira_models::TrophySource::Empty && !e.manual_unmatch));
    let ra_games: Vec<RaGameEntry> = if console.def.ra_console_id != 0 && needs_ra_cache {
        let _cs = tracing::info_span!("read_console_games_cache").entered();
        match crate::retroachievements::read_console_games_cache(
            save_dir,
            console.def.ra_console_id,
        ) {
            Some(g) => g.into_iter().filter(is_main_ra_entry).collect(),
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
            rehash_archived_rom(db, console, rom_path_str, &mut entry, &ra_index, unpack_roms);

            if entry.trophy_source == ira_models::TrophySource::Empty
                && !entry.manual_unmatch
                && !ra_index.is_empty()
            {
                let rom_name = std::path::Path::new(&rom_path_str)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let rom_norm = normalize_name(&rom_name);
                if let Some(ra_id) = ra_index.find(entry.hashes.ra_key(), &rom_norm) {
                    let new_game_id = ra_id.to_string();
                    let already_matched =
                        ira_db::find_by_game_id(db, &new_game_id, console.def.id)
                            .unwrap_or_else(|e| {
                                eprintln!(
                                    "Failed to look up game_id {new_game_id}: {e}"
                                );
                                None
                            });
                    if already_matched.is_none() {
                        let ra_title = ra_index
                            .title_of(ra_id)
                            .map(str::to_string)
                            .unwrap_or_else(|| rom_display_title(&rom_name));
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
        let hashed: HashSet<&str> = existing_by_path
            .iter()
            .filter(|(_, entry)| !entry.hashes.md5.is_empty())
            .map(|(path, _)| path.as_str())
            .collect();
        let rom_hashes = compute_rom_hashes(
            &nds_infos,
            &groups,
            &to_relative,
            &hashed,
            console.def.extensions,
            unpack_roms,
        );
        for group in &groups {
            let (rom_name, rom_path, _disc_num) = &group.roms[0];
            let rom_path_str = to_relative(rom_path);

            let rom_norm = normalize_name(rom_name);
            let rom_hash = rom_hashes.get(&rom_path_str).map(|h| h.ra_key());
            let matched_id = rom_hash
                .and_then(|hash| ra_index.find(hash, &rom_norm));

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
                        .unwrap_or_else(|| rom_display_title(rom_name));
                    (id.to_string(), t, TrophySource::Ra)
                }
                None => {
                    // Switch: native title id and application title from
                    // the emulator caches or, with keys installed, the
                    // ROM's own control NACP; the cleaned file name stays
                    // the final fallback.
                    let meta = switch_metas.get(&rom_path_str);
                    let native_id = meta
                        .and_then(|m| (!m.title_id.is_empty()).then(|| m.title_id.clone()));
                    let native_title =
                        meta.and_then(|m| (!m.title.is_empty()).then(|| m.title.clone()));
                    (
                        native_id.or(serial.clone()).unwrap_or_else(|| rom_name.clone()),
                        native_title.unwrap_or_else(|| rom_display_title(rom_name)),
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

            // A matching content hash pins the file to the row it lived on
            // before its old file vanished — the reattachment path for a
            // ROM that returns renamed or moved.
            let id_from_hash = rom_hash
                .and_then(|hash| existing_by_hash.get(hash))
                .map(|entry| entry.id);
            if let Some(id) = id_from_hash {
                candidate_ids.push(id);
                candidate_ids.sort_unstable();
                candidate_ids.dedup();
            }

            let canonical_id = id_from_key
                .or(id_from_hash)
                .or_else(|| candidate_ids.first().copied());
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

            let existing_by_id = canonical_id.and_then(|id| {
                ira_db::find_by_db_id(db, id).unwrap_or_else(|e| {
                    eprintln!("Failed to look up game {id}: {e}");
                    None
                })
            });
            let existing_hashes = existing_by_id
                .as_ref()
                .map(|e| e.hashes.clone())
                .unwrap_or_default();
            // Resolved before the match below: `rom_path_str` is moved into
            // the built game in both branches.
            let pending_hash = rom_hashes
                .get(&rom_path_str)
                .filter(|hashes| *hashes != &existing_hashes);
            let nds_info = if existing_hashes.is_empty() {
                nds_infos.get(&rom_path_str)
            } else {
                None
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

            if let Some(hashes) = pending_hash {
                for (key, value) in [("md5", &hashes.md5), ("ra_md5", &hashes.ra_md5)] {
                    if let Err(e) = ira_db::set_hash_key(db, game.db_id, key, value) {
                        eprintln!("Failed to store {key} hash: {e}");
                    }
                }
            }
            if let Some(info) = nds_info {
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

    fill_content_hashes(db, console, unpack_roms);

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

/// Content hash for every new ROM group's first disc, keyed by the path
/// relative to the console folder. NDS RA hashes (already computed) win;
/// everything else is a full MD5 of the ROM data — through the decompressed
/// entry for `.zip`/`.7z`/`.zst` containers, which are skipped while
/// `unpack_roms` is off rather than stored under a useless container
/// digest. A file is read at most once: rows in `hashed` already carry
/// their digest (multi-disc groups re-enter this path on every scan), and
/// disc images are never hashed at all — no RA console hashes a whole disc
/// image, and their serial, probed from specific sectors, is the identity
/// that reattaches them. The hash is what later scans use to reattach a
/// cartridge ROM that reappears under a new name, and what RA matches on.
fn compute_rom_hashes(
    nds_infos: &HashMap<String, crate::nds::DsRomInfo>,
    groups: &[DiscGroup],
    to_relative: &dyn Fn(&std::path::Path) -> String,
    hashed: &HashSet<&str>,
    extensions: &[&str],
    unpack_roms: bool,
) -> HashMap<String, ira_models::RomHashes> {
    if groups.is_empty() {
        return HashMap::new();
    }
    use rayon::prelude::*;

    let mut hashes: HashMap<String, ira_models::RomHashes> = nds_infos
        .iter()
        .map(|(path, info)| {
            (path.clone(), ira_models::RomHashes {
                ra_md5: info.rom_hash.clone(),
                ..Default::default()
            })
        })
        .collect();
    let targets: Vec<(String, PathBuf)> = groups
        .iter()
        .filter_map(|group| group.roms.first())
        .map(|(_, path, _)| (to_relative(path), path.clone()))
        .filter(|(relative, path)| {
            !hashes.contains_key(relative)
                && !hashed.contains(relative.as_str())
                && !crate::rom_serial::is_disc_extension(path)
                && (unpack_roms || !is_archive_path(path))
        })
        .collect();
    let pick = |name: &str| has_rom_extension(name, extensions);
    let digests: Vec<Option<String>> = targets
        .par_iter()
        .map(|(_, abs)| crate::rom_hash::content_md5(abs, &pick))
        .collect();
    for ((relative, _), digest) in targets.into_iter().zip(digests) {
        if let Some(digest) = digest {
            hashes.insert(
                relative,
                ira_models::RomHashes {
                    md5: digest,
                    ..Default::default()
                },
            );
        }
    }
    hashes
}

/// Whether `name` carries one of the console's ROM extensions — the test
/// that picks the ROM entry out of an archive.
fn has_rom_extension(name: &str, extensions: &[&str]) -> bool {
    std::path::Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
        .unwrap_or(false)
}

fn is_archive_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(crate::archives::is_archive_extension)
}

/// Files bigger than this never get a content hash: md5-ing the whole
/// library of DVD-sized PS2/Switch/Wii images costs hours of I/O for a
/// match key those platforms barely use — their SS matching runs through
/// the title search instead. Archive files are statted by their container
/// size, a lower bound of the inner ROM.
const CONTENT_HASH_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The plain full-file md5 the ScreenScraper matcher keys on, computed
/// once per ROM and stored beside the RA-flavored `rom_hash` (whose NDS
/// variant hashes only the ranges RetroAchievements identifies). No
/// account or match gating: matched, unmatched and manual rows all get
/// one, so the pass costs nothing after its first full sweep.
fn fill_content_hashes(
    db: &ira_db::DbConn,
    console: &ActiveConsole,
    unpack_roms: bool,
) {
    let rows = match ira_db::find_all_rom_by_platform(db, console.def.id) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("Content hash pass: could not list rows: {e}");
            return;
        }
    };
    for entry in rows {
        if !entry.hashes.md5.is_empty() {
            continue;
        }
        let is_archive = is_archive_path(std::path::Path::new(&entry.rom_path));
        if is_archive && !unpack_roms {
            continue;
        }
        let abs = resolve_in_folders(&console.folders, &entry.rom_path);
        if std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0) > CONTENT_HASH_MAX_BYTES {
            continue;
        }
        let pick = |name: &str| has_rom_extension(name, console.def.extensions);
        let Some(hash) = crate::rom_hash::content_md5(&abs, &pick) else {
            continue;
        };
        if let Err(e) = ira_db::set_hash_key(db, entry.id, "md5", &hash) {
            eprintln!("Content hash pass: failed to store: {e}");
        }
    }
}

/// Archive rows with no hash yet — the container's digest was cleared
/// because no RA game lists it, or the archive was first scanned while
/// `unpack_roms` was off — are hashed once through the decompressed entry.
/// Only unmatched rows are worth the read, and a stored hash is never
/// recomputed, so the pass costs nothing after that one scan.
fn rehash_archived_rom(
    db: &ira_db::DbConn,
    console: &ActiveConsole,
    rom_path_str: &str,
    entry: &mut ira_models::GameEntry,
    ra_index: &RaMatchIndex,
    unpack_roms: bool,
) {
    if !unpack_roms
        || ra_index.is_empty()
        || entry.manual_unmatch
        || entry.trophy_source != TrophySource::Empty
        || !entry.hashes.md5.is_empty()
        || !is_archive_path(std::path::Path::new(rom_path_str))
    {
        return;
    }
    let abs = resolve_in_folders(&console.folders, rom_path_str);
    let pick = |name: &str| has_rom_extension(name, console.def.extensions);
    let Some(hash) = crate::rom_hash::content_md5(&abs, &pick) else {
        return;
    };
    if let Err(e) = ira_db::set_hash_key(db, entry.id, "md5", &hash) {
        eprintln!("Failed to store rehashed ROM hash: {}", e);
        return;
    }
    entry.hashes.md5 = hash;
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
        .filter(|entry| !entry.hashes.is_empty())
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
        if let Err(e) = ira_db::set_hash_key(db, db_id, "ra_md5", &info.rom_hash) {
            eprintln!("Failed to store DS ROM hash: {e}");
        }
        write_nds_icon(save_dir, db_id, &info.icon);
    }
}

/// Saves the banner icon into the game's retro data dir unless one already
/// exists, so downloaded or user-chosen icons always win.
fn write_nds_icon(save_dir: &str, db_id: i64, icon_rgba: &[u8]) {
    let data_dir = ira_parser::retro_data_dir(save_dir, db_id);
    if ira_parser::find_image_file(
            &data_dir,
            ira_models::AssetType::Icon.file_base(),
        ).is_some() {
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
    if ira_parser::find_image_file(
            &data_dir,
            ira_models::AssetType::Icon.file_base(),
        ).is_some() {
        return;
    }
    match icon {
        crate::switch::SwitchIcon::File(cached) => {
            if let Err(e) = std::fs::create_dir_all(&data_dir) {
                eprintln!("Failed to create data dir for game {db_id}: {e}");
                return;
            }
            if ira_parser::import_image_as_webp(
                cached,
                &data_dir,
                ira_models::AssetType::Icon.file_base(),
            ).is_none() {
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
/// decrypted NCA) is imported. Games that already have all three are
/// skipped without opening the ROM.
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
        let file_title = std::path::Path::new(&game.rom_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Opening an XCI/NSP is the expensive part; a game that already has
        // its title id, a native title and an icon has nothing to gain.
        let needs_id = !crate::switch::is_title_id(&game.app_id);
        let needs_title = game.name.is_empty() || game.name == file_title;
        let needs_icon = ira_parser::find_image_file(
            &ira_parser::switch_data_dir(save_dir, game.db_id),
            ira_models::AssetType::Icon.file_base(),
        )
        .is_none();
        if !(needs_id || needs_title || needs_icon) {
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
    fn test_rom_display_title_strips_region_language_and_version_tags() {
        assert_eq!(
            rom_display_title("Super Mario Bros. (USA) (En,Ja)"),
            "Super Mario Bros."
        );
        assert_eq!(
            rom_display_title("Tokyo Mirage Sessions #FE Encore [0100A9400C9C2000][v0][Base]"),
            "Tokyo Mirage Sessions #FE Encore"
        );
        assert_eq!(rom_display_title("Chrono Trigger [T-En_v1.1] (Rev 2)"), "Chrono Trigger");
    }

    #[test]
    fn test_rom_display_title_collapses_tag_leftovers() {
        // Tags removed mid-string must not leave double spaces behind.
        assert_eq!(rom_display_title("Game (USA) Extra"), "Game Extra");
        assert_eq!(rom_display_title("Game - (Australia)"), "Game -");
    }

    #[test]
    fn test_rom_display_title_all_tags_falls_back_to_input() {
        let all_tags = "(USA) [En]";
        assert_eq!(rom_display_title(all_tags), all_tags);
        assert_eq!(rom_display_title("Bastion"), "Bastion");
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

    fn load_entry_stub(
        entry: &ira_models::GameEntry,
        _save_dir: &str,
    ) -> Result<ira_models::Game, String> {
        Ok(ira_models::Game {
            db_id: entry.id,
            kind: entry.kind,
            platform_id: entry.platform_id.clone(),
            name: entry.title.clone(),
            ..Default::default()
        })
    }

    fn gba_console(rom_dir: &std::path::Path) -> super::ActiveConsole {
        super::ActiveConsole {
            def: ira_models::find_console("gba").unwrap(),
            folders: vec![rom_dir.to_path_buf()],
            executable: String::new(),
        }
    }

    #[test]
    fn test_fill_content_hashes_hashes_rows_without_guards() {
        let tmp = tempfile::tempdir().unwrap();
        let rom_dir = tmp.path().join("roms/gba");
        std::fs::create_dir_all(&rom_dir).unwrap();
        let rom = rom_dir.join("Filled (USA).gba");
        std::fs::write(&rom, b"content to hash").unwrap();

        let db = test_db();
        let db_id = ira_db::add_game(
            &db,
            ira_models::GameKind::Retro,
            ira_models::TrophySource::Ra,
            "",
            "",
            "gba",
            "Filled",
        )
        .unwrap();
        ira_db::set_rom_path(&db, db_id, "Filled (USA).gba").unwrap();

        super::fill_content_hashes(&db, &gba_console(&rom_dir), true);

        let entry = ira_db::find_by_db_id(&db, db_id).unwrap().unwrap();
        // Even an RA-matched row gets the plain content hash: it is
        // ScreenScraper's key, independent of the trophy match.
        assert_eq!(
            entry.hashes.md5,
            crate::rom_hash::file_md5(&rom).unwrap()
        );
    }

    #[test]
    fn test_scan_reattaches_returned_rom_by_content_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let rom_dir = tmp.path().join("roms/gba");
        std::fs::create_dir_all(&rom_dir).unwrap();
        let rom = rom_dir.join("Same Game (USA).gba");
        std::fs::write(&rom, b"identical rom bytes").unwrap();

        let db = test_db();
        // A row whose file vanished: no path, but its content hash stayed.
        let db_id = ira_db::add_game(
            &db,
            ira_models::GameKind::Retro,
            ira_models::TrophySource::Empty,
            "",
            "",
            "gba",
            "Old title",
        )
        .unwrap();
        let hash = crate::rom_hash::file_md5(&rom).unwrap();
        ira_db::set_hash_key(&db, db_id, "md5", &hash).unwrap();

        let save_dir = tmp.path().join("save").to_string_lossy().into_owned();
        let games = super::build_ra_games_for_console(
            &db,
            &save_dir,
            &gba_console(&rom_dir),
            false,
            false,
            &load_entry_stub,
            &|_| {},
        );

        // The returned file was pinned to its old row — no duplicate, and
        // the metadata survived the vanish.
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].db_id, db_id);
        assert_eq!(games[0].rom_path, "Same Game (USA).gba");
        let entry = ira_db::find_by_db_id(&db, db_id).unwrap().unwrap();
        assert_eq!(entry.rom_path, "Same Game (USA).gba");
        assert_eq!(entry.title, "Old title");
        assert_eq!(entry.hashes.md5, hash);
        assert_eq!(
            ira_db::find_all_rom_by_platform(&db, "gba").unwrap().len(),
            1
        );
    }

    #[test]
    fn test_scan_keeps_row_when_rom_file_disappears() {
        let tmp = tempfile::tempdir().unwrap();
        let rom_dir = tmp.path().join("roms/gba");
        std::fs::create_dir_all(&rom_dir).unwrap();

        let db = test_db();
        let db_id = ira_db::add_game(
            &db,
            ira_models::GameKind::Retro,
            ira_models::TrophySource::Empty,
            "",
            "42",
            "gba",
            "Gone game",
        )
        .unwrap();
        ira_db::set_rom_path(&db, db_id, "Gone.gba").unwrap();

        let save_dir = tmp.path().join("save").to_string_lossy().into_owned();
        let games = super::build_ra_games_for_console(
            &db,
            &save_dir,
            &gba_console(&rom_dir),
            false,
            false,
            &load_entry_stub,
            &|_| {},
        );

        // The scan finds nothing, so the game leaves the list — but the
        // row itself (title, ids, play history) stays for a later reattach.
        assert!(games.is_empty());
        let entry = ira_db::find_by_db_id(&db, db_id).unwrap().unwrap();
        assert!(entry.rom_path.is_empty());
        assert_eq!(entry.title, "Gone game");
    }

    #[test]
    fn test_scan_hashes_unhashed_archived_rom_and_matches_by_content_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let rom_dir = tmp.path().join("roms/gba");
        std::fs::create_dir_all(&rom_dir).unwrap();
        let archive = rom_dir.join("Some Game (USA).zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file(
                "Some Game (USA).gba",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            std::io::Write::write_all(&mut zip, b"gba rom bytes").unwrap();
            zip.finish().unwrap();
        }
        let plain = tmp.path().join("plain.gba");
        std::fs::write(&plain, b"gba rom bytes").unwrap();
        let content_hash = crate::rom_hash::file_md5(&plain).unwrap();

        let console = gba_console(&rom_dir);
        let save_dir = tmp.path().join("save").to_string_lossy().into_owned();
        let cache = crate::retroachievements::paths::console_games_path(
            &save_dir,
            console.def.ra_console_id,
        );
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        // The RA title shares nothing with the file name: only the hash can match.
        std::fs::write(
            &cache,
            format!(
                r#"[{{"ID":77,"Title":"Totally Different Title","ImageIcon":"","ImageUrl":"","NumAchievements":3,"Points":10,"Hashes":["{content_hash}"]}}]"#
            ),
        )
        .unwrap();

        let db = test_db();
        // The row was scanned while the archive could not be hashed (the
        // container digest was cleared, unpacking was off at the time) and
        // never matched: it carries no hash at all.
        let db_id = ira_db::add_game(
            &db,
            ira_models::GameKind::Retro,
            ira_models::TrophySource::Empty,
            "",
            "",
            "gba",
            "Some Game",
        )
        .unwrap();
        ira_db::set_rom_path(&db, db_id, "Some Game (USA).zip").unwrap();

        let games = super::build_ra_games_for_console(
            &db,
            &save_dir,
            &console,
            true,
            true,
            &load_entry_stub,
            &|_| {},
        );

        assert_eq!(games.len(), 1);
        let entry = ira_db::find_by_db_id(&db, db_id).unwrap().unwrap();
        assert_eq!(entry.hashes.md5, content_hash);
        assert_eq!(entry.game_id, "77");
        assert_eq!(entry.trophy_source, ira_models::TrophySource::Ra);
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
