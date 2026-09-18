//! The mass matcher's ScreenScraper pass: console games are resolved
//! through their stored ROM md5 first — the exact-match search — and fall
//! back to one platform-narrowed title search with the ROM file's clean
//! name, ES-DE style. No ladder of ever-shorter terms: two requests at
//! most, and only a confirmed miss (the source answered and does not know
//! the game) tombstones the game; a failed request retries on the next
//! dialog opening.

use adw::prelude::*;
use std::cell::Cell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::css::*;
use super::helpers::replace_row_actions;
use super::mass_match_batch::{run_batch, BatchHit, BatchItem, RowActions, BATCH_FINISHED};
use super::rom_name::{
    alt_search_term, clean_rom_name, looks_like_title_id, pick_search_name, region_hints,
    search_term,
};
use unicode_normalization::UnicodeNormalization;
use super::ss_match_dialog::{
    persist_ss_match, show_background_match, show_matched, show_unmatched,
};
use super::state::SharedState;
use super::steam_search_dialog::status_label;
use crate::Game;
use ira_api::screenscraper::ScrapedGame;
use ira_api::types::SteamCmdInfo;
use ira_api::{ScraperCreds, SteamDataClient};
use ira_models::screenscraper_system_id;

/// The per-game outcome of the pass. The distinction decides the
/// tombstone: only `Miss` records one — `Failed` leaves the game untombed
/// so the next opening asks again. PC matching shares it: a miss there
/// used to come back as nothing at all, leaving the row "searching"
/// forever and re-asking on every opening.
pub(super) enum SsOutcome {
    Hit(Box<ScrapedGame>),
    Miss,
    Failed(String),
}

/// Whether a stored record is missing any piece a refetch could bring.
fn metadata_has_gaps(meta: &ira_models::ScraperMetadata) -> bool {
    release_date_is_broken(&meta.release_date)
        || meta.players.is_empty()
        || meta.rating <= 0.0
        || meta.synopses.is_empty()
        || meta.classifications.is_empty()
        || meta.developers.is_empty()
        || meta.publishers.is_empty()
        || meta.genres.is_empty()
}

/// Matched games whose stored record has holes, in list order: the
/// refetch passes' queue. A matched game with no record at all is all
/// holes.
pub(crate) fn refetch_queue(state: &SharedState) -> Vec<i64> {
    let s = state.borrow();
    s.games
        .iter()
        .filter(|g| !g.screenscraper_id.is_empty())
        .filter(|g| match ira_db::scraper_metadata_for_game(&s.db, g.db_id) {
            Ok(None) => true,
            Ok(Some(meta)) => metadata_has_gaps(&meta),
            Err(_) => false,
        })
        .map(|g| g.db_id)
        .collect()
}

/// One entry's refetch outcome, for the passes' progress reporting.
pub(crate) enum RefetchOutcome {
    /// Something missing was found and merged.
    Filled,
    /// The entry answered but everything was already stored.
    Unchanged,
    /// The request or the store failed; the game keeps its holes.
    Failed(String),
}

/// One queue item's report from the refetch worker.
pub(crate) struct RefetchProgress {
    pub done: usize,
    pub total: usize,
    pub db_id: i64,
    pub current: String,
    pub outcome: RefetchOutcome,
}

/// One sequential worker over the refetch queue: an exact fetch per entry
/// by its own id, merged fill-only-gaps over what's stored. One request
/// per second keeps the pass well inside ScreenScraper's quota; the
/// worker stands down between items when `cancel` says so. Progress
/// arrives through the returned channel, read on the UI loop.
pub(crate) fn spawn_refetch_worker(
    queue: Vec<i64>,
    steam: Arc<SteamDataClient>,
    creds: ScraperCreds,
    db: ira_db::DbConn,
    cancel: Option<Arc<AtomicBool>>,
) -> async_channel::Receiver<RefetchProgress> {
    let total = queue.len();
    let (tx, rx) = super::helpers::ui_channel();
    std::thread::spawn(move || {
        for (done, db_id) in queue.into_iter().enumerate() {
            if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                break;
            }
            if done > 0 {
                std::thread::sleep(std::time::Duration::from_millis(1000));
            }
            let current = ira_db::find_by_db_id(&db, db_id)
                .ok()
                .flatten()
                .map(|entry| entry.title)
                .unwrap_or_default();
            let outcome = refetch_one(&steam, &creds, &db, db_id);
            let _ = tx.try_send(RefetchProgress {
                done: done + 1,
                total,
                db_id,
                current,
                outcome,
            });
        }
    });
    rx
}

/// One exact fetch by id, merged over what's stored.
pub(crate) fn refetch_one(
    steam: &SteamDataClient,
    creds: &ScraperCreds,
    db: &ira_db::DbConn,
    db_id: i64,
) -> RefetchOutcome {
    let entry = match ira_db::find_by_db_id(db, db_id) {
        Ok(Some(entry)) => entry,
        _ => return RefetchOutcome::Failed("game not found".to_string()),
    };
    let games = match steam.screenscraper_game(creds, &entry.screenscraper_id) {
        Ok(games) => games,
        Err(e) => return RefetchOutcome::Failed(e),
    };
    let Some(game) = games.into_iter().next() else {
        return RefetchOutcome::Failed("entry not found".to_string());
    };
    let timestamp = ira_db::scraper_release_timestamp(&game.release_date);
    let fresh = game.metadata(timestamp);
    match ira_db::merge_missing_scraper_metadata(db, db_id, &fresh) {
        Ok(true) => {
            eprintln!("SS refetch: filled gaps for game {db_id}");
            RefetchOutcome::Filled
        }
        Ok(false) => RefetchOutcome::Unchanged,
        Err(e) => RefetchOutcome::Failed(e),
    }
}

/// The status box of a match-list row's ScreenScraper pass, added as its
/// own suffix so the Steam/SGDB/RA boxes stay independent. Rows the source
/// already missed skip straight to the manual search button instead of
/// burning the request again.
pub(super) fn attach_ss_actions(
    row: &adw::ActionRow,
    state: &SharedState,
    game: &Game,
    dialog: &gtk4::Widget,
    missed: bool,
    vis: &super::mass_match_dialog::RowVis,
) -> gtk4::Box {
    let ss_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    ss_box.set_valign(gtk4::Align::Center);
    let console = ira_models::scraper_console_id(game.kind, &game.platform_id);
    if missed {
        // Already tombstoned by an earlier pass: terminal from the start.
        vis.mark_attempted(game.db_id);
        show_unmatched(
            &ss_box,
            state,
            game.db_id,
            &game.name,
            &console,
            dialog,
        );
    } else {
        ss_box.append(&status_label(
            &crate::tr!("Searching SS..."),
            CSS_DIM_LABEL,
        ));
    }
    row.add_suffix(&ss_box);
    ss_box
}

/// Runs the ScreenScraper pass over every row that has an SS box. The
/// pass owns the quota gate (`ss_job_busy`): a dialog reopened while it
/// runs only relabels the new rows and starts nothing. One request per
/// second keeps the batch well inside the service's quota; progress
/// shows on the sidebar strip, so closing the dialog — or the whole
/// window — stops nothing.
pub(super) fn start_ss_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    rows: &[RowActions],
    dialog: &gtk4::Widget,
    vis: super::mass_match_dialog::RowVis,
) {
    let missed: HashSet<i64> = match ira_db::scraper_missed_ids(&state.borrow().db) {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => {
            eprintln!("ScreenScraper batch: could not read misses: {e}");
            HashSet::new()
        }
    };
    // The master switch: a disabled ScreenScraper starts no pass at all.
    // Its rows never get an answer from anywhere, so they go straight to
    // the manual search button — which says as much when used.
    if !state.borrow().cfg.screenscraper_enabled {
        for (i, g) in needs_matching.iter().enumerate() {
            let Some(ss_box) = rows.get(i).and_then(|r| r.ss.clone()) else {
                continue;
            };
            if missed.contains(&g.db_id) {
                continue;
            }
            show_unmatched(&ss_box, state, g.db_id, &g.name, &g.platform_id, dialog);
            vis.pass_done(i);
        }
        return;
    }
    if state.borrow().ss_job_busy.get() {
        // The background pass already owns the quota: rows still in
        // their "searching" phase get the in-flight note instead, with
        // their manual search button.
        for (i, g) in needs_matching.iter().enumerate() {
            let Some(ss_box) = rows.get(i).and_then(|r| r.ss.clone()) else {
                continue;
            };
            if missed.contains(&g.db_id) {
                continue;
            }
            show_background_match(&ss_box, state, g.db_id, &g.name, &g.platform_id, dialog);
        }
        return;
    }

    let (steam, db, creds, cfg) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            s.db.clone(),
            ScraperCreds::from_account(
                s.cfg.screenscraper_id.clone(),
                s.cfg.screenscraper_password.clone(),
            ),
            s.cfg.clone(),
        )
    };
    let queue: Vec<BatchItem> = needs_matching
        .iter()
        .enumerate()
        .filter(|(i, g)| rows.get(*i).is_some_and(|r| r.ss.is_some()) && !missed.contains(&g.db_id))
        .map(|(row_idx, g)| BatchItem {
            name: g.name.clone(),
            db_id: g.db_id,
            row_idx,
        })
        .collect();
    if queue.is_empty() {
        return;
    }
    eprintln!("ScreenScraper batch: {} game(s) to resolve", queue.len());

    // Quota management is obligatory per ScreenScraper — stand down for
    // the day when the scrape quota is spent — but reading the counters
    // is a blocking request that must never run on the GTK loop, or the
    // whole app freezes while the dialog opens. The verdict comes back
    // through a channel and the batch starts from the main loop; a quota
    // read that fails lets the pass run rather than silently skip it.
    let quota_steam = Arc::clone(&steam);
    let quota_creds = creds.clone();
    let (tx, rx) = super::helpers::ui_channel::<bool>();
    std::thread::spawn(move || {
        let allowed = match quota_steam.screenscraper_user_infos(&quota_creds) {
            Ok(infos) => {
                eprintln!(
                    "ScreenScraper batch: quota {}/{} requests today ({} not found, max {}), {}/min",
                    infos.requests_today,
                    infos.max_requests_per_day,
                    infos.requests_ko_today,
                    infos.max_requests_ko_per_day,
                    infos.max_requests_per_min
                );
                !infos.exhausted()
            }
            Err(e) => {
                eprintln!("ScreenScraper batch: quota unavailable: {e}");
                true
            }
        };
        let _ = tx.try_send(allowed);
    });
    let batch_state = std::rc::Rc::clone(state);
    let batch_rows = rows.to_vec();
    let batch_dialog = dialog.clone();
    super::helpers::once_channel(rx, move |allowed| {
        let (queue, steam, db, creds, cfg, state, rows, dialog) = (
            queue,
            steam,
            db,
            creds,
            cfg,
            batch_state,
            batch_rows,
            batch_dialog,
        );
            if allowed {
                state.borrow().ss_job_busy.set(true);
                // The strip shows the pass and carries its cancel
                // button; when another job owns the strip the pass
                // simply runs without visible progress.
                let (job, cancel) =
                    match super::fetch_images::begin_strip_job(
                        &state,
                        &crate::tr!("Matching games…"),
                        &crate::tr!("Matching unmatched games…"),
                    ) {
                        Some(job) => {
                            let cancel = job.cancel_flag();
                            (Some(job), Some(cancel))
                        }
                        None => (None, None),
                    };
                let total = queue.len();
                let done = Cell::new(0usize);
                let matched = Cell::new(0usize);
                run_batch(
                    queue,
                    0,
                    cancel,
                    {
                        let steam = Arc::clone(&steam);
                        move |item| resolve(&steam, &creds, &db, &cfg, item)
                    },
                    move |hit| {
                        if hit.row_idx == BATCH_FINISHED {
                            state.borrow().ss_job_busy.set(false);
                            if let Some(job) = &job {
                                job.finish(
                                    &state,
                                    &crate::tr!("{} games matched")
                                        .replacen("{}", &matched.get().to_string(), 1),
                                    &crate::tr!("Matching finished"),
                                );
                            }
                            return;
                        }
                        let name = hit.name.clone();
                        apply_hit(&state, &rows, &dialog, hit, &matched, &vis);
                        let finished = done.get() + 1;
                        done.set(finished);
                        if let Some(job) = &job {
                            job.progress(&state, finished, total, &name);
                        }
                    },
                );
            } else {
                // Quota spent: the queued rows never get an answer, so
                // they go straight to the manual search button — and
                // count as done for the hide toggle, since nothing will
                // ask for them again today.
                for item in &queue {
                    let Some(ss_box) = rows.get(item.row_idx).and_then(|r| r.ss.clone()) else {
                        continue;
                    };
                    let platform_id = state
                        .borrow()
                        .games
                        .iter()
                        .find(|g| g.db_id == item.db_id)
                        .map(|g| g.platform_id.clone())
                        .unwrap_or_default();
                    show_unmatched(
                        &ss_box, &state, item.db_id, &item.name, &platform_id, &dialog,
                    );
                    vis.pass_done(item.row_idx);
                }
                eprintln!("ScreenScraper batch: quota exhausted, standing down for today");
            }
    });
}

/// Off-thread: the hash search answers authoritatively when the ROM's md5
/// is known to the source; otherwise one title search runs with the ROM
/// file's clean name, and its top candidates must plausibly *be* the game
/// to count — a wrong auto-match would stick forever.
fn resolve(
    steam: &SteamDataClient,
    creds: &ScraperCreds,
    db: &ira_db::DbConn,
    cfg: &ira_config::Config,
    item: &BatchItem,
) -> Option<SsOutcome> {
    let mut entry = ira_db::find_by_db_id(db, item.db_id).ok().flatten()?;
    // PS3/PS4 games carry a native product code here, not a console.
    let platform_id = ira_models::scraper_console_id(entry.kind, &entry.platform_id);
    // PC games search ScreenScraper's own Windows/Linux systems and fall
    // back to a cross-platform lookup diffed against their Steam data.
    if entry.kind.is_pc() {
        let target = PcMatchTarget {
            kind: entry.kind,
            platform_id: &platform_id,
            title: &entry.title,
            display: &item.name,
            db_id: item.db_id,
        };
        return Some(run_pc_matching(steam, creds, db, &target));
    }
    screenscraper_system_id(&platform_id)?;
    // ROM paths are stored relative to the console's folder in the ROM
    // roots; every file access resolves through the config first.
    let abs = cfg
        .resolve_rom_path(&platform_id, &entry.rom_path)
        .unwrap_or_else(|| std::path::PathBuf::from(&entry.rom_path));

    // The exact hash search runs only on consoles where file digests are
    // the matching key — disc consoles go serial-first below, and their
    // multi-gigabyte images never get hashed at all. NDS rows keep an
    // RA-flavored rom_hash, so the plain content md5 wins when the scan
    // has filled it. romnom carries the real file name with extension,
    // exactly what ScreenScraper's rom index stores (ES-DE sends it the
    // same way); the stem alone loses the match.
    if ira_models::screenscraper_hashes_content(&platform_id) {
        let verbose = verbose_logging();
        let pick = rom_extension_pick(&platform_id);
        // Hash on demand — digest and inner-file size in one streaming
        // pass — and keep the pair stored so this costs nothing next
        // time. The size is the inner ROM's own, never the container's:
        // a wrong size reads as a miss, an absent one still matches on
        // the digest alone. A row whose digest is already stored from
        // the scan but whose size never landed gets only the size —
        // re-reading whole archives on every run was the batch's CPU
        // spike.
        if entry.hashes.md5.is_empty() {
            match ira_platforms::rom_hash::content_md5_and_size(&abs, &pick) {
                Some((hash, size)) => {
                    for (key, value) in [("md5", hash.clone()), ("size", size.to_string())] {
                        if let Err(e) = ira_db::set_hash_key(db, item.db_id, key, &value) {
                            eprintln!("SS batch: failed to store the {key}: {e}");
                        }
                    }
                    entry.hashes.md5 = hash;
                    entry.hashes.size = size as i64;
                }
                None => eprintln!("SS batch: could not hash {}", abs.display()),
            }
        } else if entry.hashes.size == 0 {
            match ira_platforms::archives::entry_size(&abs, &pick) {
                Some(size) => {
                    if let Err(e) =
                        ira_db::set_hash_key(db, item.db_id, "size", &size.to_string())
                    {
                        eprintln!("SS batch: failed to store the size: {e}");
                    }
                    entry.hashes.size = size as i64;
                }
                None => eprintln!("SS batch: could not size {}", abs.display()),
            }
        }
        if !entry.hashes.md5.is_empty() {
            let romnom = std::path::Path::new(&abs)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| item.name.clone());
            let lookup = if entry.hashes.size > 0 {
                steam.screenscraper_rom_lookup(
                    creds,
                    &romnom,
                    &platform_id,
                    Some((entry.hashes.md5.as_str(), entry.hashes.size as u64)),
                )
            } else {
                steam.screenscraper_rom_lookup(creds, &romnom, &platform_id, None)
            };
            match lookup {
                Err(e) => {
                    eprintln!("SS batch: '{romnom}' [{platform_id}] lookup failed: {e}");
                    return Some(SsOutcome::Failed(e));
                }
                Ok(games) => {
                    if let Some(game) = games.into_iter().next() {
                        if verbose {
                            eprintln!(
                                "SS batch: '{romnom}' [{platform_id}] hash hit -> ss id {} '{}'",
                                game.ss_id, game.name
                            );
                        }
                        return Some(SsOutcome::Hit(Box::new(game)));
                    }
                    if verbose {
                        eprintln!("SS batch: '{romnom}' [{platform_id}] no hash hit");
                    }
                }
            }
        }
    }

    // No hash hit: a disc serial read from the scan's cache is the
    // next-best exact identity — it survives chd/rvz repacks that
    // scramble every file digest. Normalized to ScreenScraper's dashed
    // uppercase form: raw PS2 serials arrive as SLES_520.05 from
    // SYSTEM.CNF. Only serial-shaped values qualify.
    let serial = ira_models::screenscraper_matches_by_serial(&platform_id)
        .then(|| ira_platforms::rom_serial::read_serial_cached(db, &abs))
        .flatten()
        .map(|s| normalize_serial(&s))
        .filter(|s| looks_like_serial(s));
    if let Some(serial) = serial {
        match steam.screenscraper_serial_lookup(creds, &serial, &platform_id) {
            Err(e) => {
                eprintln!("SS batch: serial '{serial}' lookup failed: {e}");
                return Some(SsOutcome::Failed(e));
            }
            Ok(games) => {
                if let Some(game) = games.into_iter().next() {
                    if verbose_logging() {
                        eprintln!(
                            "SS batch: serial '{serial}' [{platform_id}] hit -> ss id {} '{}'",
                            game.ss_id, game.name
                        );
                    }
                    return Some(SsOutcome::Hit(Box::new(game)));
                }
            }
        }
        eprintln!("SS batch: serial '{serial}' [{platform_id}] unknown to the source");
    }

    // No hash hit: one title search, preferring the ROM file name — the
    // library title is user-editable and drifts from the dump, while the
    // file name is what the scene shipped. The word search is a substring
    // match over ScreenScraper's own names, byte-sensitive about
    // separator spacing, and a series head buries the game among its
    // siblings — so the search goes out with the distinctive tail
    // (rom_name::search_term) and the acceptance comparison, which
    // flattens punctuation, matches the full name against the
    // candidates. A stem that is only a bare title id cannot be searched
    // at all, so the library title stands in.
    let stem = std::path::Path::new(&abs)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // The search base is the game's own title when the row's name comes
    // from a trusted source (the console header, an RA/SS match, the
    // user) or the console carries authoritative internal titles
    // (switch); every other console searches from the ROM file name,
    // with the title only taking over when the file lost its
    // punctuation.
    let trusted = entry.title_trusted || ira_models::title_from_trusted_source(&platform_id);
    let bases = [
        clean_rom_name(&stem),
        clean_rom_name(&entry.title),
        clean_rom_name(&item.name),
    ];
    let Some(full) = pick_search_name(trusted, &bases[0], &bases[1], &bases[2]) else {
        eprintln!("SS batch: [{platform_id}] no usable name to search");
        return Some(SsOutcome::Failed("no usable name".to_string()));
    };
    // Region tags never gate anything — plenty of dumps carry none — they
    // only matter when two candidates match equally well.
    let hints = region_hints(&stem);
    let term = search_term(&full);
    if term.is_empty() {
        // Not "the source does not know the game" — "we cannot ask yet"
        // (a title-id-named dump waiting on enrichment). Leave untombed
        // so the next opening retries with whatever name exists by then.
        eprintln!("SS batch: [{platform_id}] no usable search term");
        return Some(SsOutcome::Failed("no usable search term".to_string()));
    }
    match steam.screenscraper_search(creds, &term, &platform_id) {
        Err(e) => {
            eprintln!("SS batch: '{term}' [{platform_id}] search failed: {e}");
            Some(SsOutcome::Failed(e))
        }
        Ok(candidates) => {
            let verbose = verbose_logging();
            // A distinctive tail can still arrive empty — the source's
            // member settings and search index have their moods — so the
            // name's other side gets one widened try before the miss.
            let candidates = if candidates.is_empty() {
                match alt_search_term(&full) {
                    Some(alt) => match steam.screenscraper_search(creds, &alt, &platform_id) {
                        Err(e) => {
                            eprintln!("SS batch: '{alt}' [{platform_id}] search failed: {e}");
                            return Some(SsOutcome::Failed(e));
                        }
                        Ok(widened) => {
                            if verbose {
                                eprintln!("SS batch: '{term}' empty, widened to '{alt}'");
                            }
                            widened
                        }
                    },
                    None => candidates,
                }
            } else {
                candidates
            };
            if verbose {
                eprintln!(
                    "SS batch: '{term}' [{platform_id}] {} candidate(s)",
                    candidates.len()
                );
            }
            let target = normalized_for_match(&full);
            let hit = pick_candidate(&candidates, &target, &hints);
            match hit {
                Some(game) => {
                    if verbose {
                        eprintln!(
                            "SS batch: '{term}' [{platform_id}] name matched ss id {} '{}'",
                            game.ss_id, game.name
                        );
                    }
                    Some(SsOutcome::Hit(Box::new(game.clone())))
                }
                None => {
                    eprintln!(
                        "SS batch: '{term}' [{platform_id}] no acceptable candidate ({} arrived: {})",
                        candidates.len(),
                        candidate_names(&candidates)
                    );
                    Some(SsOutcome::Miss)
                }
            }
        }
    }
}

/// Success lines are development noise now that the pass works: they
/// only print when `IRA_SS_VERBOSE` is set. Misses and failures always
/// log, because they are what a bad matching run is diagnosed with.
fn verbose_logging() -> bool {
    std::env::var_os("IRA_SS_VERBOSE").is_some()
}

/// A miss log names what arrived — "1 candidate(s)" says nothing when
/// the question is why the one answer was no good.
fn candidate_names(candidates: &[ScrapedGame]) -> String {
    candidates
        .iter()
        .take(3)
        .map(|game| game.name.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Off-thread PC branch. ScreenScraper barely covers desktop platforms,
/// so the search widens in circles. First the game's own system —
/// Windows for Steam and Wine games, Linux for native ones; a hit there
/// is simply the game. Then the whole ScreenScraper at once: a candidate
/// on any console whose developers and publishers agree with what Steam
/// reports is the same game, and it takes Steam's release date, because
/// the console entry's date is that port's, not the PC one's. Games with
/// no Steam id behind them — manual additions like a decomp port named
/// after its original — have nothing to diff against and lean entirely
/// on ScreenScraper: the ranked pick over every system is the answer,
/// down to the oldest release for a name shared by remakes.
/// The game identity the PC matching runs against.
pub(super) struct PcMatchTarget<'a> {
    pub kind: ira_models::GameKind,
    pub platform_id: &'a str,
    pub title: &'a str,
    pub display: &'a str,
    pub db_id: i64,
}

pub(super) fn run_pc_matching(
    steam: &SteamDataClient,
    creds: &ScraperCreds,
    db: &ira_db::DbConn,
    target: &PcMatchTarget,
) -> SsOutcome {
    let Some(system) = ira_models::screenscraper_pc_system_id(target.kind) else {
        return SsOutcome::Failed("no pc system".to_string());
    };
    // PC titles come from Steam or the user, never from dump tags: the
    // title leads and the executable stem is last resort.
    let full = [clean_rom_name(target.title), clean_rom_name(target.display)]
        .into_iter()
        .find(|t| !t.is_empty() && !looks_like_title_id(t))
        .unwrap_or_default();
    let term = search_term(&full);
    if term.is_empty() {
        eprintln!("SS batch: [pc] no usable search term");
        return SsOutcome::Failed("no usable search term".to_string());
    }
    let normalized = normalized_for_match(&full);
    let verbose = verbose_logging();
    let app_id = target.platform_id.parse::<u32>().ok();
    // The Steam diff. Companies decide identity — dates and titles are
    // shared by ports and remakes, but the developer is the studio.
    let steam_info = app_id
        .and_then(|app_id| steam.fetch_steamcmd_info(&app_id.to_string()))
        .filter(|info| !info.developer.trim().is_empty() || !info.publisher.trim().is_empty());
    let extras_for = |game: &ScrapedGame| {
        if game.synopses.is_empty()
            || game.classifications.is_empty()
            || release_date_is_broken(&game.release_date)
        {
            app_id.and_then(|id| steam.fetch_store_extras(&id.to_string()))
        } else {
            None
        }
    };
    // Every confirmed pick is stored, bare or not: the entry is usually
    // the game's console version, and recording it is what makes the
    // match stick instead of re-asking on every opening.
    let finish = |game: ScrapedGame| -> SsOutcome {
        let extras = extras_for(&game);
        SsOutcome::Hit(Box::new(finish_pc_pick(
            db,
            game,
            steam_info.as_ref(),
            extras.as_ref(),
        )))
    };

    // The game's own system first — a hit there is simply the game.
    match steam.screenscraper_search_in(creds, &term, Some(system)) {
        Err(e) => {
            eprintln!("SS batch: '{term}' [pc] search failed: {e}");
            return SsOutcome::Failed(e);
        }
        Ok(candidates) => {
            if let Some(game) = pick_candidate(&candidates, &normalized, &[]) {
                if verbose {
                    eprintln!("SS batch: '{term}' [pc] matched ss id {} '{}'", game.ss_id, game.name);
                }
                return finish(game.clone());
            }
            if verbose {
                eprintln!("SS batch: '{term}' [pc] nothing on the pc system");
            }
        }
    }

    // Widening: every system at once.
    let candidates = match steam.screenscraper_search_in(creds, &term, None) {
        Err(e) => {
            eprintln!("SS batch: '{term}' [pc] wide search failed: {e}");
            return SsOutcome::Failed(e);
        }
        Ok(candidates) => candidates,
    };

    if let Some(info) = steam_info.as_ref() {
        if let Some(mut game) = verified_pick(&candidates, &normalized, Some(info)) {
            let steam_date = steam_release_date(info.release_timestamp);
            if !steam_date.is_empty() {
                game.release_date = steam_date;
            }
            if verbose {
                eprintln!(
                    "SS batch: '{term}' [pc] Steam-diffed to ss id {} '{}'",
                    game.ss_id, game.name
                );
            }
            return finish(game);
        }
    } else {
        // No Steam data to diff with: ScreenScraper's ranked pick over
        // every system is the answer.
        if let Some(game) = pick_candidate(&candidates, &normalized, &[]) {
            if verbose {
                eprintln!("SS batch: '{term}' [pc] matched ss id {} '{}'", game.ss_id, game.name);
            }
            return finish(game.clone());
        }
    }

    // One widening try with the full name before the miss: the two-word
    // head loses the distinctive words ("Doki Doki" drowns "Doki Doki
    // Literature Club" among every other game with doki doki in the
    // name) and the source's answer table caps at thirty.
    if term != full {
        match steam.screenscraper_search_in(creds, &full, None) {
            Err(e) => {
                eprintln!("SS batch: '{full}' [pc] wide search failed: {e}");
                return SsOutcome::Failed(e);
            }
            Ok(widened) => {
                if let Some(info) = steam_info.as_ref() {
                    if let Some(mut game) = verified_pick(&widened, &normalized, Some(info)) {
                        let steam_date = steam_release_date(info.release_timestamp);
                        if !steam_date.is_empty() {
                            game.release_date = steam_date;
                        }
                        if verbose {
                            eprintln!(
                                "SS batch: '{full}' [pc] Steam-diffed to ss id {} '{}'",
                                game.ss_id, game.name
                            );
                        }
                        return finish(game);
                    }
                } else if let Some(game) = pick_candidate(&widened, &normalized, &[]) {
                    if verbose {
                        eprintln!("SS batch: '{full}' [pc] matched ss id {} '{}'", game.ss_id, game.name);
                    }
                    return finish(game.clone());
                }
            }
        }
    }
    eprintln!(
        "SS batch: '{term}' [pc] no acceptable candidate ({} arrived: {})",
        candidates.len(),
        candidate_names(&candidates)
    );
    // No match does not mean no metadata: the store synopsis and the
    // cache's companies are still on offer.
    if let Some(app_id) = app_id {
        super::enrichment::garnish_pc_ss_metadata(db, steam, target.db_id, app_id);
    }
    SsOutcome::Miss
}

/// Store a PC pick. Company-less entries get their developers and
/// publishers from our cache when the Steam names are known there —
/// with the proper ScreenScraper ids — and a missing synopsis comes
/// from the Steam store page. The pick is recorded even when it stays
/// bare afterwards: it is the game's console version most times, and
/// the stored id is what keeps the matcher from asking again.
fn finish_pc_pick(
    db: &ira_db::DbConn,
    mut game: ScrapedGame,
    info: Option<&SteamCmdInfo>,
    extras: Option<&ira_api::steam::StoreExtras>,
) -> ScrapedGame {
    if game.developers.is_empty() && game.publishers.is_empty() {
        if let Some(info) = info {
            super::enrichment::fill_steam_companies(
                db,
                info,
                &mut game.developers,
                &mut game.publishers,
            );
        }
    }
    if release_date_is_broken(&game.release_date) {
        if let Some(extras) = extras {
            if !extras.release_date.is_empty() {
                game.release_date = extras.release_date.clone();
            }
        }
    }
    if game.synopses.is_empty() {
        if let Some(extras) = extras {
            if !extras.synopsis.is_empty() {
                game.synopses.push(("en".to_string(), extras.synopsis.clone()));
            }
        }
    }
    if game.classifications.is_empty() {
        if let Some(extras) = extras {
            for (board, value) in &extras.ratings {
                game.classifications
                    .push(ira_models::ScraperClassification {
                        kind: board.clone(),
                        value: value.clone(),
                    });
            }
        }
    }
    game
}

/// Whether a candidate carries company data at all.
fn has_companies(game: &ScrapedGame) -> bool {
    !game.developers.is_empty() || !game.publishers.is_empty()
}

/// Whether any of a candidate's names fits the target by any accepted
/// match shape — the title evidence for candidates with no companies.
fn title_fits(target: &str, game: &ScrapedGame) -> bool {
    game.names
        .iter()
        .any(|(_, name)| match_rank(target, &normalized_for_match(name)).is_some())
}

/// The candidates the Steam diff confirms: companies agree with Steam's,
/// or — when the candidate carries no companies, so nothing can agree or
/// contradict — the title fits by any match shape. ScreenScraper's
/// Higurashi chapters are exactly that case: no developer on record and
/// no exact name (the store's bundle word "Hou" sits mid-title), but
/// every Steam-title word accounted for. The best-ranked of the winners
/// takes it, so an exact title still outranks a contained one.
fn verified_pick(
    set: &[ScrapedGame],
    target: &str,
    info: Option<&SteamCmdInfo>,
) -> Option<ScrapedGame> {
    let agreeing: Vec<ScrapedGame> = set
        .iter()
        .filter(|game| match info {
            Some(info) => {
                companies_overlap(info, game)
                    || (!has_companies(game) && title_fits(target, game))
            }
            None => false,
        })
        .cloned()
        .collect();
    pick_candidate(&agreeing, target, &[]).cloned()
}

/// Whether a ScreenScraper candidate's developer or publisher agrees with
/// the Steam ones: one company name in common, up to the legal suffixes
/// and word order. Accents, case and punctuation fold away, so "ATLUS"
/// meets "Atlus"; an empty answer on either side never agrees.
fn companies_overlap(info: &SteamCmdInfo, game: &ScrapedGame) -> bool {
    let steam: std::collections::HashSet<Vec<String>> =
        [info.developer.as_str(), info.publisher.as_str()]
            .into_iter()
            .flat_map(|field| field.split(','))
            .map(ira_models::company_tokens)
            .filter(|tokens| !tokens.is_empty())
            .collect();
    if steam.is_empty() {
        return false;
    }
    game.developers
        .iter()
        .chain(game.publishers.iter())
        .any(|entity| steam.contains(&ira_models::company_tokens(&entity.name)))
}

/// Steam's release timestamp as the `YYYY-MM-DD` string the metadata
/// stores — the PC release, when a diffed console entry carries the
/// port's date instead. A missing timestamp gives no date: writing the
/// epoch here once poisoned a batch of entries with 1970-01-01.
pub(crate) fn steam_release_date(timestamp: i64) -> String {
    if timestamp <= 0 {
        return String::new();
    }
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Whether a stored release date counts as none: either empty or the
/// epoch string the old Steam-diff bug wrote. Both are holes the Steam
/// refetch is allowed to fill.
pub(crate) fn release_date_is_broken(date: &str) -> bool {
    date.is_empty() || date == "1970-01-01"
}

/// Disc serials — `SLES-52005`, `SLPS-01204`, `RLJE52` — are short
/// uppercase codes mixing letters and digits. Plain numbers (RA ids),
/// 16-hex title ids and 4-letter NDS gamecodes don't qualify.
fn looks_like_serial(game_id: &str) -> bool {
    (6..=12).contains(&game_id.len())
        && game_id.chars().any(|c| c.is_ascii_alphabetic())
        && game_id.chars().any(|c| c.is_ascii_digit())
        && game_id
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

/// The digest pass's archive-entry filter: the inner file carrying the
/// console's own extension.
fn rom_extension_pick(platform_id: &str) -> impl Fn(&str) -> bool {
    let extensions: Vec<String> = ira_models::find_console(platform_id)
        .map(|console| {
            console
                .extensions
                .iter()
                .map(|ext| format!(".{ext}"))
                .collect()
        })
        .unwrap_or_default();
    move |name: &str| {
        let name = name.to_lowercase();
        extensions.iter().any(|ext| name.ends_with(ext))
    }
}

/// Disc serials normalized to ScreenScraper's dashed uppercase form —
/// the raw PS2 serial arrives as SLES_520.05 from SYSTEM.CNF.
fn normalize_serial(serial: &str) -> String {
    serial.to_uppercase().replace('_', "-").replace('.', "")
}

/// Lowercase letters-and-digits only, accents folded away (NFD plus the
/// combining marks stripped) — ScreenScraper's names keep their accents
/// ("Pokémon") while scene files write them plain ("Pokemon"), and the
/// two must compare equal.
fn normalized_for_match(name: &str) -> String {
    name.to_lowercase()
        .nfd()
        .filter(|c| !matches!(u32::from(*c), 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x20D0..=0x20FF))
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// How tightly a candidate name fits the target, from strongest down:
/// the names equal; the same words in a different order — ScreenScraper
/// titles lead with their brand where the dump leads with the game
/// ("Emio – The Smiling Man: Famicom Detective Club" against "Famicom
/// Detective Club - Emio The Smiling Man"); one being the other plus
/// decoration in front or behind, subtitle and region chopping tolerated
/// ("(Japan)", "Advance"); next weakest, a brand the region's name
/// carries in front of the title ("Simple 2000 Series Vol. 50 : The
/// Daibijin" is "The Daibijin" with a prefix in front); and weakest of
/// all, one name's every word inside the other's — the store's bundle
/// word sits mid-title where no prefix or suffix rule can see it
/// ("Higurashi When They Cry Hou - Ch.3 Tatarigoroshi" against
/// ScreenScraper's "Higurashi - When They Cry - Ch.3 - Tatarigoroshi").
/// A number leading the extension is a sequel, not decoration: "Advance
/// Wars" is not "Advance Wars 2". `None` is not the game at all.
fn match_rank(target: &str, candidate: &str) -> Option<u8> {
    if target.is_empty() || candidate.is_empty() {
        return None;
    }
    if candidate == target {
        return Some(4);
    }
    if same_words(target, candidate) {
        return Some(3);
    }
    if prefix_extension_ok(target, candidate) || prefix_extension_ok(candidate, target) {
        return Some(2);
    }
    if suffix_extension_ok(target, candidate) || suffix_extension_ok(candidate, target) {
        return Some(1);
    }
    if containment_ok(target, candidate) || containment_ok(candidate, target) {
        return Some(0);
    }
    None
}

/// Whether two names hold exactly the same words, order aside.
fn same_words(a: &str, b: &str) -> bool {
    let mut left: Vec<&str> = a.split_whitespace().collect();
    let mut right: Vec<&str> = b.split_whitespace().collect();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

/// The best candidate from a search answer, in order: the one whose
/// names fit the target most tightly (exact over prefix over suffix),
/// then the dump's region tags, then the earliest release date, and the
/// source's own probability order (the order the answer came in) last.
/// Earlier list position never outranks a better shape: a brand-suffixed
/// lookalike listed above the real game loses.
///
/// The date rung is for the plain-named dump whose candidates all carry
/// subtitles — three same-shape series entries and nothing to tell them
/// apart. The earliest is the plausible one: the first entry of a series
/// is the game whose dump ships with no subtitle to lose, while its
/// sequels' dumps carry theirs. It stays below region because release
/// names shuffle across regions ("Pokemon Stadium" in the US is Japan's
/// "Pokemon Stadium 2", and Japan had its own first one) — when the
/// dump's region and the entry's regional names disagree, that evidence
/// is stronger than chronology.
/// A candidate's full rank: match shape, the dump's region agreeing with
/// the matched name's, and the release date it is compared by.
type Rank = (u8, bool, Option<String>);

fn pick_candidate<'a>(
    candidates: &'a [ScrapedGame],
    target: &str,
    region_hints: &[&'static str],
) -> Option<&'a ScrapedGame> {
    let mut best: Option<(Rank, usize)> = None;
    for (index, game) in candidates.iter().enumerate() {
        let mut shape: Option<(u8, bool)> = None;
        for (region, name) in &game.names {
            if let Some(strength) = match_rank(target, &normalized_for_match(name)) {
                let this = (strength, region_hints.contains(&region.as_str()));
                if shape.is_none_or(|s| this > s) {
                    shape = Some(this);
                }
            }
        }
        if let Some(shape) = shape {
            let rank = (shape.0, shape.1, release_key(game, region_hints));
            if best.as_ref().is_none_or(|(b, _)| rank_better(&rank, b)) {
                best = Some((rank, index));
            }
        }
    }
    best.map(|(_, index)| &candidates[index])
}

/// Whether rank `a` beats `b`: stronger shape, then the dump's region,
/// then the earlier release — an absent date loses to a present one, so
/// a dated entry is always preferred over an undatable one.
fn rank_better(a: &Rank, b: &Rank) -> bool {
    if a.0 != b.0 {
        return a.0 > b.0;
    }
    if a.1 != b.1 {
        return a.1;
    }
    match (&a.2, &b.2) {
        (Some(x), Some(y)) => x < y,
        (Some(_), None) => true,
        _ => false,
    }
}

/// The release date a candidate is ranked by: the dump's region's date
/// when the tags say one, else the display date, else the earliest of
/// the rest.
fn release_key(game: &ScrapedGame, hints: &[&'static str]) -> Option<String> {
    game.release_dates
        .iter()
        .find(|(region, date)| !date.is_empty() && hints.contains(&region.as_str()))
        .map(|(_, date)| date.clone())
        .or_else(|| (!game.release_date.is_empty()).then(|| game.release_date.clone()))
        .or_else(|| {
            game.release_dates
                .iter()
                .filter(|(_, date)| !date.is_empty())
                .map(|(_, date)| date.as_str())
                .min()
                .map(str::to_string)
        })
}

fn prefix_extension_ok(short: &str, long: &str) -> bool {
    if !long.starts_with(short) || long.len() <= short.len() {
        return false;
    }
    // The extension must start at a word boundary, and the first new word
    // must not be a bare number — "(Japan)" and "Advance" are decoration,
    // the "2" in a sequel is not.
    match long[short.len()..].trim_start().chars().next() {
        None => true,
        Some(c) => {
            let at_word_boundary = long.as_bytes()[short.len()] == b' ';
            !c.is_ascii_digit() && at_word_boundary
        }
    }
}

/// The suffix mirror of the prefix rule, for names that carry a brand in
/// front of the title. The short side must hold at least two words — a
/// one-word suffix ("Mario" inside "Super Mario") is just a word, and
/// matching on it would swallow neighbors. Sequels are safe by
/// direction: their number sits at the end, so the base title is never
/// their suffix.
fn suffix_extension_ok(short: &str, long: &str) -> bool {
    if !long.ends_with(short)
        || long.len() <= short.len()
        || !short.as_bytes().contains(&b' ')
    {
        return false;
    }
    long.as_bytes()[long.len() - short.len() - 1] == b' '
}

/// The loosest shape: the shorter name's every word appears in the
/// longer one, position aside. This is for titles that carry an extra
/// word mid-name — the bundle's "Hou" in "Higurashi When They Cry Hou -
/// Ch.3 Tatarigoroshi", which ScreenScraper's "Higurashi - When They
/// Cry - Ch.3 - Tatarigoroshi" matches under no prefix or suffix rule.
/// The shorter side needs at least two words ("Mario" inside "Super
/// Mario Bros. Wonder" is a word, not a title), and the words only the
/// longer name holds must not be bare numbers — the sequel guard again,
/// and the same check pins the right chapter: a "Ch.3" title can never
/// be contained in a "Ch.4" name, because the 3 and the 4 are words.
fn containment_ok(short: &str, long: &str) -> bool {
    let short_words: Vec<&str> = short.split_whitespace().collect();
    let long_words: Vec<&str> = long.split_whitespace().collect();
    if short_words.len() < 2 || short_words.len() >= long_words.len() {
        return false;
    }
    if !short_words.iter().all(|word| long_words.contains(word)) {
        return false;
    }
    !long_words
        .iter()
        .filter(|word| !short_words.contains(word))
        .any(|word| word.chars().all(|c| c.is_ascii_digit()))
}

/// UI loop: persist a hit's result, and repaint the row's SS box when
/// the dialog still shows it. Persistence runs regardless — the pass
/// keeps working with the dialog or the whole window closed — and only a
/// confirmed miss tombstones the game.
fn apply_hit(
    state: &SharedState,
    rows: &[RowActions],
    dialog: &gtk4::Widget,
    hit: BatchHit<SsOutcome>,
    matched: &Cell<usize>,
    vis: &super::mass_match_dialog::RowVis,
) {
    let platform_id = state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == hit.db_id)
        .map(|g| ira_models::scraper_console_id(g.kind, &g.platform_id))
        .unwrap_or_default();
    match hit.matched {
        Some(SsOutcome::Hit(game)) => {
            persist_ss_match(state, hit.db_id, &game);
            matched.set(matched.get() + 1);
            // The settings window's scraper draft must learn the new
            // match before its next Save, or it would write the stale
            // empty id back over the fresh one.
            super::edit_game_scraper::refresh_scraper_section(state, hit.db_id);
            if let Some(ss_box) = rows.get(hit.row_idx).and_then(|r| r.ss.clone()) {
                show_matched(&ss_box);
            }
            vis.pass_done(hit.row_idx);
        }
        Some(SsOutcome::Miss) => {
            if let Err(e) = ira_db::tombstone_scraper_miss(&state.borrow().db, hit.db_id) {
                eprintln!("ScreenScraper batch: failed to record the miss: {e}");
            }
            if let Some(ss_box) = rows.get(hit.row_idx).and_then(|r| r.ss.clone()) {
                replace_row_actions(&ss_box, |ab| {
                    show_unmatched(ab, state, hit.db_id, &hit.name, &platform_id, dialog);
                });
            }
            vis.pass_done(hit.row_idx);
        }
        Some(SsOutcome::Failed(e)) => {
            eprintln!(
                "ScreenScraper batch: '{}': request failed, leaving untombed: {e}",
                hit.name
            );
            if let Some(ss_box) = rows.get(hit.row_idx).and_then(|r| r.ss.clone()) {
                replace_row_actions(&ss_box, |ab| {
                    show_unmatched(ab, state, hit.db_id, &hit.name, &platform_id, dialog);
                });
            }
            vis.pass_done(hit.row_idx);
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{match_rank, normalized_for_match, pick_candidate};
    use ira_api::screenscraper::ScrapedGame;
    use ira_api::types::SteamCmdInfo;

    #[test]
    fn test_normalized_for_match_ignores_punctuation_and_case() {
        assert_eq!(
            normalized_for_match("13 Sentinels: Aegis Rim"),
            normalized_for_match("13 Sentinels   Aegis Rim!!")
        );
        // Accents fold to their base letter, never to deletion — the
        // source writes "Pokémon" where scene files write "Pokemon".
        assert_eq!(normalized_for_match("  pokémon! "), "pokemon");
        assert_eq!(
            normalized_for_match("Pokémon XD: Gale of Darkness"),
            normalized_for_match("Pokemon XD - Gale of Darkness")
        );
        // En and em dashes fold like any punctuation — the source writes
        // "A — B" where dumps write "A - B".
        assert_eq!(
            normalized_for_match("Slay the Princess \u{2014} The Pristine Cut"),
            normalized_for_match("Slay the Princess - The Pristine Cut")
        );
    }

    #[test]
    fn test_looks_like_serial_gates_disc_ids_only() {
        assert!(super::looks_like_serial("SLES-52005"));
        assert!(super::looks_like_serial("SLPS_012.04"));
        assert!(super::looks_like_serial("RLJE52"));
        // Plain numbers are RA ids; 16 chars are switch title ids; four
        // letters are NDS gamecodes.
        assert!(!super::looks_like_serial("22069"));
        assert!(!super::looks_like_serial("0100A9400C9C2000"));
        assert!(!super::looks_like_serial("YG3E"));
        assert!(!super::looks_like_serial(""));
    }

    #[test]
    fn test_match_rank_tolerates_subtitles_rejects_sequels() {
        let target = normalized_for_match("Dragon Quest I & II");
        assert_eq!(
            match_rank(&target, &normalized_for_match("Dragon Quest I & II")),
            Some(4)
        );
        assert!(match_rank(
            &target,
            &normalized_for_match("Dragon Quest I & II (Japan)")
        )
        .is_some());
        assert!(match_rank(
            &normalized_for_match("Final Fantasy VI Advance"),
            &normalized_for_match("Final Fantasy VI")
        )
        .is_some());
        // A number leading the extension is a sequel, not decoration.
        assert!(match_rank(
            &normalized_for_match("Advance Wars"),
            &normalized_for_match("Advance Wars 2: Black Hole Rising")
        )
        .is_none());
        assert!(match_rank(&target, &normalized_for_match("Dragon Quest Monsters")).is_none());
        // A brand word in front of the target is the same game under its
        // full name: the search only returned the candidate because some
        // region's name matched, and sequel guards above keep the
        // lookalikes out.
        assert!(match_rank(
            &normalized_for_match("Ace Attorney Justice for All"),
            &normalized_for_match("Phoenix Wright Ace Attorney Justice for All")
        )
        .is_some());
        // Empty sides never match — a nameless candidate is not the game.
        assert!(match_rank(&target, "").is_none());
        assert!(match_rank("", &target).is_none());
    }

    #[test]
    fn test_match_rank_picks_the_game_from_main_title_results() {
        // The search sends only the main title; the answer table holds
        // the whole series, and the full name — punctuation flattened —
        // must select the right row (live jeuRecherche answer for
        // "Ace Combat 04" on ps2).
        let full = normalized_for_match("Ace Combat 04 - Shattered Skies");
        let candidates = [
            "Ace Combat 5 : The Unsung War",
            "Ace Combat Zero : The Belkan War",
            "Ace Combat 04 : Shattered Skies",
        ];
        let hit = candidates
            .iter()
            .find(|c| match_rank(&full, &normalized_for_match(c)).is_some());
        assert_eq!(*hit.unwrap(), "Ace Combat 04 : Shattered Skies");
    }

    #[test]
    fn test_match_rank_tolerates_brand_prefixed_japanese_names() {
        // A Japan-region dump named after the subtitle matches the
        // entry whose Japanese name carries a brand in front (live
        // Demolition Girl answer).
        let target = normalized_for_match("The Daibijin");
        let jp_name = normalized_for_match("Simple 2000 Series Vol. 50 : The Daibijin");
        assert!(match_rank(&target, &jp_name).is_some());
        // The English name of the same entry matches a file named the
        // English way.
        assert!(match_rank(
            &normalized_for_match("Demolition Girl"),
            &normalized_for_match("Demolition Girl")
        )
        .is_some());
        // One-word suffixes never match: "Mario" is not "Super Mario".
        assert!(match_rank(
            &normalized_for_match("Mario"),
            &normalized_for_match("Super Mario")
        )
        .is_none());
        // The base title is not a sequel's suffix.
        assert!(match_rank(
            &normalized_for_match("Advance Wars"),
            &normalized_for_match("Advance Wars 2 : Black Hole Rising")
        )
        .is_none());
    }

    fn scraped(ss_id: &str, names: &[(&str, &str)]) -> ScrapedGame {
        ScrapedGame {
            ss_id: ss_id.to_string(),
            names: names
                .iter()
                .map(|(r, n)| (r.to_string(), n.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    fn scraped_dated(ss_id: &str, names: &[(&str, &str)], date: &str) -> ScrapedGame {
        ScrapedGame {
            release_date: date.to_string(),
            release_dates: names
                .iter()
                .map(|(r, _)| (r.to_string(), date.to_string()))
                .collect(),
            ..scraped(ss_id, names)
        }
    }

    #[test]
    fn test_match_rank_containment_accepts_mid_title_extras() {
        // The store's bundle word sits mid-title ("Hou"); ScreenScraper
        // spells the chapters with dashes and dots around them. No
        // prefix or suffix rule sees past the middle word; containment
        // does (live jeuRecherche answer for "Ch 3 Tatarigoroshi").
        assert_eq!(
            match_rank(
                &normalized_for_match("Higurashi When They Cry Hou - Ch.3 Tatarigoroshi"),
                &normalized_for_match("Higurashi - When They Cry - Ch.3 - Tatarigoroshi"),
            ),
            Some(0)
        );
        // The chapter number is a word on both sides, so a wrong chapter
        // never contains.
        assert!(match_rank(
            &normalized_for_match("Higurashi When They Cry Hou - Ch.3 Tatarigoroshi"),
            &normalized_for_match("Higurashi - When They Cry - Ch.4 - Tatarigoroshi"),
        )
        .is_none());
        // Sibling titles that differ by a real word are not the game.
        assert!(match_rank(
            &normalized_for_match("Super Smash Bros. Melee"),
            &normalized_for_match("Super Smash Bros. Brawl"),
        )
        .is_none());
        // The sequel guard holds: the bare number extra rejects.
        assert!(match_rank(
            &normalized_for_match("Advance Wars"),
            &normalized_for_match("Advance Wars 2"),
        )
        .is_none());
        // A one-word name inside a longer one is a word, not a title.
        assert!(match_rank(
            &normalized_for_match("Mario"),
            &normalized_for_match("Super Mario Bros. Wonder"),
        )
        .is_none());
    }

    #[test]
    fn test_match_rank_accepts_reordered_words() {
        // ScreenScraper titles lead with their brand where the dump leads
        // with the game (live switch answer for "Emio": "Famicom
        // Detective Club - Emio The Smiling Man").
        let target = normalized_for_match("Emio – The Smiling Man: Famicom Detective Club");
        assert_eq!(
            match_rank(
                &target,
                &normalized_for_match("Famicom Detective Club - Emio The Smiling Man")
            ),
            Some(3)
        );
        // A sibling entry with different words stays out.
        assert!(match_rank(
            &target,
            &normalized_for_match("Famicom Detective Club - The Missing Heir")
        )
        .is_none());
        // Reorder is weaker than exact, stronger than prefix decoration.
        let target = normalized_for_match("Dragon Quest I & II");
        assert_eq!(
            match_rank(&target, &normalized_for_match("Dragon Quest I & II (Japan)")),
            Some(2)
        );
    }

    #[test]
    fn test_pick_candidate_ranks_shapes_over_list_order() {
        // The exact match sits last in the source's answer; a weaker
        // suffix-shaped candidate first. The exact one must win.
        let target = normalized_for_match("Ace Attorney Justice for All");
        let candidates = [
            scraped(
                "1",
                &[("us", "Phoenix Wright Ace Attorney Justice for All")],
            ),
            scraped("2", &[("us", "Ace Attorney Justice for All")]),
        ];
        let picked = pick_candidate(&candidates, &target, &[]);
        assert_eq!(picked.unwrap().ss_id, "2");
    }

    #[test]
    fn test_pick_candidate_breaks_prefix_ties_on_the_earliest_release() {
        // The plain-named dump against three subtitled series entries:
        // the earliest is the plausible one, whatever order the answer
        // carried them in.
        let target = normalized_for_match("Super Whatever");
        let candidates = [
            scraped_dated("1", &[("us", "Super Whatever: Mystical Awesome")], "2003-05-01"),
            scraped_dated("2", &[("us", "Super Whatever: Making Dreams")], "2001-11-11"),
            scraped_dated("3", &[("us", "Super Whatever: Great Danger")], "2004-01-01"),
        ];
        let picked = pick_candidate(&candidates, &target, &[]);
        assert_eq!(picked.unwrap().ss_id, "2");
        // An entry with no date at all loses to a dated one.
        let candidates = [
            scraped("1", &[("us", "Super Whatever: Making Dreams")]),
            scraped_dated("2", &[("us", "Super Whatever: Great Danger")], "2004-01-01"),
        ];
        let picked = pick_candidate(&candidates, &target, &[]);
        assert_eq!(picked.unwrap().ss_id, "2");
    }

    #[test]
    fn test_pick_candidate_region_outranks_the_release_date() {
        // Names shuffle across regions — a US dump of "Pokemon Stadium"
        // is Japan's Stadium 2, and Japan had its own older Stadium — so
        // the dump's region decides before chronology does.
        let target = normalized_for_match("Same Name");
        let candidates = [
            scraped_dated("1", &[("eu", "Same Name")], "1999-01-01"),
            scraped_dated("2", &[("us", "Same Name")], "2002-01-01"),
        ];
        let picked = pick_candidate(&candidates, &target, &["us"]);
        assert_eq!(picked.unwrap().ss_id, "2");
    }

    #[test]
    fn test_companies_overlap_folds_names_and_never_trusts_empty() {
        let info = SteamCmdInfo {
            developer: "ATLUS".into(),
            publisher: "Sega".into(),
            ..Default::default()
        };
        // Case folding meets the source's own spelling.
        let atlus = ScrapedGame {
            developers: vec![entity("P-Studio")],
            publishers: vec![entity("Atlus")],
            ..Default::default()
        };
        assert!(super::companies_overlap(&info, &atlus));
        // Publishers alone can carry the agreement.
        let sega = ScrapedGame {
            publishers: vec![entity("SEGA")],
            ..Default::default()
        };
        assert!(super::companies_overlap(&info, &sega));
        // No agreement, no match.
        let other = ScrapedGame {
            developers: vec![entity("Nintendo")],
            publishers: vec![entity("Nintendo")],
            ..Default::default()
        };
        assert!(!super::companies_overlap(&info, &other));
        // A candidate with no companies at all cannot agree.
        assert!(!super::companies_overlap(&info, &ScrapedGame::default()));
        // Steam knowing nothing about a game never agrees either.
        let empty = SteamCmdInfo::default();
        assert!(!super::companies_overlap(&empty, &atlus));
        // Corporate suffixes drop: the store's "Naughty Dog, LLC" is the
        // source's "Naughty Dog", word order included.
        let dog = SteamCmdInfo {
            developer: "Naughty Dog, LLC".into(),
            publisher: "Sony Interactive Entertainment".into(),
            ..Default::default()
        };
        let nd = ScrapedGame {
            publishers: vec![entity("Naughty Dog")],
            ..Default::default()
        };
        assert!(super::companies_overlap(&dog, &nd));
        // A name that is only filler agrees with nothing.
        let filler = SteamCmdInfo {
            developer: "LLC".into(),
            publisher: "Studios".into(),
            ..Default::default()
        };
        assert!(!super::companies_overlap(&filler, &nd));
    }

    fn entity(name: &str) -> ira_models::ScraperEntity {
        ira_models::ScraperEntity {
            id: "1".into(),
            name: name.into(),
        }
    }

    #[test]
    fn test_verified_pick_accepts_exact_titles_without_companies() {
        // ScreenScraper leaves many entries company-less; an exact-title
        // candidate with no companies verifies where nothing contradicts
        // it (the live "Metaphor: ReFantazio" answer).
        let info = SteamCmdInfo {
            developer: "ATLUS".into(),
            publisher: "Sega".into(),
            ..Default::default()
        };
        let target = normalized_for_match("Metaphor: ReFantazio");
        let candidates = [scraped("359000", &[("us", "Metaphor: ReFantazio")])];
        assert!(super::verified_pick(&candidates, &target, Some(&info)).is_some());
        // An exact title WITH disagreeing companies stays out.
        let contradicting = ScrapedGame {
            developers: vec![entity("Nintendo")],
            publishers: vec![entity("Nintendo")],
            ..scraped("359001", &[("us", "Metaphor: ReFantazio")])
        };
        assert!(
            super::verified_pick(
                std::slice::from_ref(&contradicting),
                &target,
                Some(&info)
            )
            .is_none()
        );
        // No Steam info verifies nothing.
        assert!(super::verified_pick(&candidates, &target, None).is_none());
    }

    #[test]
    fn test_verified_pick_accepts_companyless_containment_titles() {
        // The live Higurashi chapter case: Steam names it with the
        // bundle's "Hou" mid-title, and ScreenScraper's entry carries no
        // companies and no exact name — the contained title verifies.
        let info = SteamCmdInfo {
            developer: "07th Expansion".into(),
            publisher: "MangaGamer".into(),
            ..Default::default()
        };
        let target =
            normalized_for_match("Higurashi When They Cry Hou - Ch.3 Tatarigoroshi");
        let candidates = [scraped(
            "306342",
            &[("ss", "Higurashi - When They Cry - Ch.3 - Tatarigoroshi")],
        )];
        assert!(super::verified_pick(&candidates, &target, Some(&info)).is_some());
        // A different chapter stays out: the chapter numbers are words,
        // and Meakashi is not Tatarigoroshi.
        let wrong_chapter = scraped(
            "306346",
            &[("ss", "Higurashi - When They Cry - Ch.5 - Meakashi")],
        );
        assert!(super::verified_pick(
            std::slice::from_ref(&wrong_chapter),
            &target,
            Some(&info)
        )
        .is_none());
        // A company-less candidate whose title fits by nothing at all
        // does not verify either.
        let unrelated = scraped("1", &[("us", "Totally Different Thing")]);
        assert!(super::verified_pick(
            std::slice::from_ref(&unrelated),
            &target,
            Some(&info)
        )
        .is_none());
    }

    #[test]
    fn test_finish_pc_pick_fills_companies_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let conn = ira_db::init_db(dir.path().join("ira.db").to_str().unwrap());
        std::mem::forget(dir);
        conn.get()
            .unwrap()
            .execute(
                "INSERT INTO scraper_companies (id, name) VALUES ('54774', 'Team Salvato')",
                [],
            )
            .unwrap();

        // A company-less entry gets the cached company with its proper
        // ScreenScraper id, and the Steam synopsis fills in.
        let game = ScrapedGame {
            ss_id: "70000".into(),
            name: "Doki Doki Literature Club".into(),
            release_date: "2017-09-22".into(),
            ..Default::default()
        };
        let info = SteamCmdInfo {
            developer: "Team Salvato".into(),
            publisher: "Team Salvato".into(),
            ..Default::default()
        };
        let extras = ira_api::steam::StoreExtras {
            synopsis: "A poetry horror.".into(),
            ratings: vec![],
            release_date: String::new(),
        };
        let finished = super::finish_pc_pick(&conn, game.clone(), Some(&info), Some(&extras));
        assert_eq!(
            finished
                .developers
                .iter()
                .map(|d| (d.id.as_str(), d.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("54774", "Team Salvato")]
        );
        assert_eq!(
            finished.synopses.first().map(|(l, t)| (l.as_str(), t.as_str())),
            Some(("en", "A poetry horror."))
        );
        // A company the cache does not know becomes a local under a
        // negative id, spelled the way Steam spelled it.
        let unknown = SteamCmdInfo {
            developer: "Whoever".into(),
            publisher: "Whoever".into(),
            ..Default::default()
        };
        let game = ScrapedGame {
            ss_id: "70002".into(),
            name: "Named".into(),
            release_date: "2020-01-01".into(),
            ..Default::default()
        };
        let finished = super::finish_pc_pick(&conn, game, Some(&unknown), None);
        assert_eq!(finished.developers.len(), 1);
        assert!(finished.developers[0].id.parse::<i64>().unwrap() < 0);
        assert_eq!(finished.developers[0].name, "Whoever");
        // A bare pick is still recorded: it is the game's console
        // version most times, and the stored id keeps the matcher from
        // asking again. The parser's no-rating sentinel is -1.
        let bare = ScrapedGame {
            ss_id: "70001".into(),
            name: "Some Bare Entry".into(),
            rating: -1.0,
            ..Default::default()
        };
        assert_eq!(super::finish_pc_pick(&conn, bare, None, None).ss_id, "70001");
    }

    #[test]
    fn test_finish_pc_pick_repairs_broken_release_dates_from_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let conn = ira_db::init_db(dir.path().join("ira.db").to_str().unwrap());
        std::mem::forget(dir);
        let extras = ira_api::steam::StoreExtras {
            synopsis: String::new(),
            ratings: vec![],
            release_date: "2015-09-15".into(),
        };
        // An empty date fills from the store page.
        let undated = ScrapedGame {
            ss_id: "70003".into(),
            name: "Undated".into(),
            ..Default::default()
        };
        let finished = super::finish_pc_pick(&conn, undated, None, Some(&extras));
        assert_eq!(finished.release_date, "2015-09-15");
        // The epoch string the old diff bug wrote counts as no date and
        // is replaced the same way.
        let poisoned = ScrapedGame {
            ss_id: "70004".into(),
            name: "Poisoned".into(),
            release_date: "1970-01-01".into(),
            ..Default::default()
        };
        let finished = super::finish_pc_pick(&conn, poisoned, None, Some(&extras));
        assert_eq!(finished.release_date, "2015-09-15");
        // A real date is never overwritten.
        let dated = ScrapedGame {
            ss_id: "70005".into(),
            name: "Dated".into(),
            release_date: "2001-06-21".into(),
            ..Default::default()
        };
        let finished = super::finish_pc_pick(&conn, dated, None, Some(&extras));
        assert_eq!(finished.release_date, "2001-06-21");
    }

    #[test]
    fn test_steam_release_date_formats_the_pc_release() {
        assert_eq!(super::steam_release_date(86_400), "1970-01-02");
        assert_eq!(super::steam_release_date(1_609_459_200), "2021-01-01");
        // A missing timestamp gives no date — the epoch string poisoned
        // a batch of stored entries once.
        assert_eq!(super::steam_release_date(0), "");
        assert_eq!(super::steam_release_date(-5), "");
    }

    #[test]
    fn test_pick_candidate_breaks_ties_on_the_dumps_region() {
        // Two same-shape candidates: one is the Japanese release, the
        // dump says (Japan) — that one wins without excluding the other.
        let target = normalized_for_match("The Daibijin");
        let candidates = [
            scraped("1", &[("eu", "The Daibijin")]),
            scraped("2", &[("jp", "The Daibijin")]),
        ];
        let picked = pick_candidate(&candidates, &target, &["jp"]);
        assert_eq!(picked.unwrap().ss_id, "2");
        // Without region hints the source's own order stands.
        let picked = pick_candidate(&candidates, &target, &[]);
        assert_eq!(picked.unwrap().ss_id, "1");
        // A region hint never rescues a non-match.
        let candidates = [scraped("1", &[("jp", "Completely Different Game")])];
        assert!(pick_candidate(&candidates, &target, &["jp"]).is_none());
    }
}
