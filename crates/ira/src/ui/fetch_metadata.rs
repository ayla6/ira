//! Mass metadata refetch on the sidebar strip, one source per job. The
//! ScreenScraper pass gives matched games whose stored record has holes
//! one exact fetch by their own id; the Steam pass gives games with a
//! store app id — PC games by their platform id, console games by a
//! link stored earlier — whose stored metadata misses something Steam
//! can give (release date, studios, synopsis, age boards) a re-read of
//! their SteamCMD entry and store page. Both merge only what's missing,
//! both report through the same bottom-of-sidebar strip the image
//! fetcher uses — so they keep running, and stay visible, with dialogs
//! or the whole window closed — and the strip takes one job at a time.
//! The ScreenScraper pass additionally owns the quota gate, so it and
//! the matching pass never stack their requests.

use std::sync::Arc;

use super::mass_match_ss::{RefetchOutcome, RefetchProgress};
use super::state::SharedState;

/// Start refetching missing metadata for every matched game with holes in
/// its record, revealing the sidebar strip. `false` when a ScreenScraper
/// job or a strip job is already running, or nothing has gaps.
pub fn start_metadata_refetch(state: &SharedState, force: bool) -> bool {
    let (busy, allowed) = {
        let s = state.borrow();
        (
            s.ss_job_busy.get(),
            s.cfg.screenscraper_enabled && !s.cfg.screenscraper_id.is_empty(),
        )
    };
    if busy || !allowed {
        return false;
    }
    let queue = super::mass_match_ss::refetch_queue(state, force);
    if queue.is_empty() {
        return false;
    }
    let Some(job) = super::fetch_images::begin_strip_job(
        state,
        &crate::tr!("Fetching ScreenScraper data…"),
        &crate::tr!("Fetching missing metadata…"),
    ) else {
        return false;
    };
    state.borrow().ss_job_busy.set(true);

    let (steam, creds, db) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            ira_api::ScraperCreds::from_account(
                s.cfg.screenscraper_id.clone(),
                s.cfg.screenscraper_password.clone(),
            ),
            s.db.clone(),
        )
    };
    let cancel = job.cancel_flag();
    let rx = super::mass_match_ss::spawn_refetch_worker(queue, steam, creds, db, Some(cancel));
    let state = state.clone();
    poll_refetch(
        state,
        job,
        rx,
        crate::tr!("{} of {} games updated"),
        crate::tr!("Metadata fetched"),
        |state| state.borrow().ss_job_busy.set(false),
    );
    true
}

/// Start the Steam-only refetch for every game with a store app id —
/// PC games by their platform's app id, console games by a link stored
/// earlier. The match is metadata only — nothing Steam-ish lands on the
/// game. No ScreenScraper quota involved — the strip's one-job rule is
/// the only gate.
pub fn start_steam_refetch(state: &SharedState, force: bool) -> bool {
    let queue = steam_refetch_queue(state, force);
    if queue.is_empty() {
        return false;
    }
    let Some(job) = super::fetch_images::begin_strip_job(
        state,
        &crate::tr!("Fetching Steam data…"),
        &crate::tr!("Fetching missing Steam data…"),
    ) else {
        return false;
    };
    let (steam, db) = {
        let s = state.borrow();
        (s.steam.clone(), s.db.clone())
    };
    let cancel = job.cancel_flag();
    let rx = spawn_steam_refetch_worker(queue, steam, db, Some(cancel));
    let state = state.clone();
    poll_refetch(
        state,
        job,
        rx,
        crate::tr!("{} of {} games updated"),
        crate::tr!("Steam data fetched"),
        |_| {},
    );
    true
}

/// Games whose stored metadata misses something Steam can give, and
/// which the pass can actually read: a stored Steam link (a console
/// game linked to the store before) or a PC game whose platform id is
/// the store app id. Everything else would only get a hopeful title
/// search whose exact-hit policy almost never lands — the strip would
/// cycle the whole library naming games nothing ever happens to.
/// Includes games with no record at all, and the epoch dates an old
/// diff bug wrote. A forced pass drops the gap filter — every readable
/// game is re-read, which is how stored records pick up fields that
/// only the store page provides.
fn steam_refetch_queue(state: &SharedState, force: bool) -> Vec<i64> {
    let s = state.borrow();
    s.games
        .iter()
        .filter(|g| {
            !g.steam_link_id.is_empty()
                || (g.kind.is_pc() && g.platform_id.parse::<u32>().is_ok())
        })
        .filter(|g| {
            if force {
                return true;
            }
            match ira_db::scraper_metadata_for_game(&s.db, g.db_id) {
                Ok(None) => true,
                Ok(Some(meta)) => steam_gaps(&meta),
                Err(_) => false,
            }
        })
        .map(|g| g.db_id)
        .collect()
}

/// The holes Steam's own sources can fill: the release date, the
/// studios, the synopsis, the age boards.
fn steam_gaps(meta: &ira_models::ScraperMetadata) -> bool {
    super::mass_match_ss::release_date_is_broken(&meta.release_date)
        || (meta.developers.is_empty() && meta.publishers.is_empty())
        || meta.synopses.is_empty()
        || meta.classifications.is_empty()
}

/// Fetch the genre table whole, upsert it into the lookup cache under
/// the rename latch, and stamp the fetched-at marker. Returns the
/// entry count for the strip's summary. Families stay harvest-only:
/// their tables are library-dependent, and the whole source list is
/// mostly series no game in the library belongs to.
pub(crate) fn fetch_and_warm_genres(
    steam: &ira_api::SteamDataClient,
    db: &ira_db::DbConn,
    creds: &ira_api::ScraperCreds,
) -> Result<usize, String> {
    let entries: Vec<(i64, String)> = steam
        .genres_list(creds)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    row.id.parse::<i64>().ok().map(|id| (id, row.name.clone()))
                })
                .collect()
        })?;
    ira_db::warm_entity_cache(db, ira_db::KIND_GENRE, &entries)?;
    ira_db::set_entity_fetched(db, ira_db::KIND_GENRE, chrono::Utc::now().timestamp())?;
    Ok(entries.len())
}

/// One sequential worker over the Steam refetch queue: the SteamCMD
/// entry and the store page get re-read, merged fill-only-gaps over
/// what's stored. A short pace keeps steamcmd.net happy; the worker
/// stands down between items when `cancel` says so.
fn spawn_steam_refetch_worker(
    queue: Vec<i64>,
    steam: std::sync::Arc<ira_api::SteamDataClient>,
    db: ira_db::DbConn,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> async_channel::Receiver<RefetchProgress> {
    use std::sync::atomic::Ordering;

    let total = queue.len();
    let (tx, rx) = super::helpers::ui_channel();
    std::thread::spawn(move || {
        for (done, db_id) in queue.into_iter().enumerate() {
            if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                break;
            }
            if done > 0 {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            let current = ira_db::find_by_db_id(&db, db_id)
                .ok()
                .flatten()
                .map(|entry| entry.title)
                .unwrap_or_default();
            let outcome = steam_refetch_one(&steam, &db, db_id);
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

/// One game's Steam re-read, merged over what's stored. The app id is
/// the stored Steam link, or the platform id for a PC game whose
/// platform is the store. SteamCMD's timestamp wins for the release
/// date, the store page's parsed date fills in when it has none — and
/// an epoch date an old bug wrote counts as missing, so refetches
/// repair it.
pub(crate) fn steam_refetch_one(
    steam: &ira_api::SteamDataClient,
    db: &ira_db::DbConn,
    db_id: i64,
) -> RefetchOutcome {
    let Some(entry) = ira_db::find_by_db_id(db, db_id).ok().flatten() else {
        return RefetchOutcome::Failed("game not found".to_string());
    };
    // The match is metadata only — no app id, no Steam enrichment ever
    // lands on the game.
    let app_id = if !entry.steam_link_id.is_empty() {
        Some(entry.steam_link_id.clone())
    } else {
        entry
            .platform_id
            .parse::<u32>()
            .ok()
            .map(|id| id.to_string())
    };
    let Some(app_id) = app_id else {
        return RefetchOutcome::Unchanged;
    };
    let info = steam.fetch_steamcmd_info(&app_id);
    let extras = steam.fetch_store_extras(&app_id);
    if info.is_none() && extras.is_none() {
        // Most often a stored id Steam does not know — a link typed or
        // picked by hand, or a platform id that is not a store app id
        // after all.
        eprintln!(
            "Steam refetch: '{}': steam answered nothing for app {app_id}",
            entry.title
        );
        return RefetchOutcome::Failed("steam answered nothing".to_string());
    }
    let mut meta = ira_db::scraper_metadata_for_game(db, db_id)
        .ok()
        .flatten()
        .unwrap_or_default();
    let mut changed = false;
    if super::mass_match_ss::release_date_is_broken(&meta.release_date) {
        let date = info
            .as_ref()
            .filter(|info| info.release_timestamp > 0)
            .map(|info| super::mass_match_ss::steam_release_date(info.release_timestamp))
            .filter(|date| !date.is_empty())
            .or_else(|| {
                extras
                    .as_ref()
                    .map(|extras| extras.release_date.clone())
                    .filter(|date| !date.is_empty())
            });
        if let Some(date) = date {
            meta.release_timestamp = ira_db::scraper_release_timestamp(&date);
            meta.release_date = date;
            changed = true;
        }
    }
    if meta.developers.is_empty() && meta.publishers.is_empty() {
        if let Some(info) = &info {
            super::enrichment::fill_steam_companies(
                db,
                info,
                &mut meta.developers,
                &mut meta.publishers,
            );
            changed |= !(meta.developers.is_empty() && meta.publishers.is_empty());
        }
    }
    if let Some(extras) = &extras {
        if meta.synopses.is_empty() && !extras.synopsis.is_empty() {
            meta.synopses.push(("en".to_string(), extras.synopsis.clone()));
            changed = true;
        }
        if meta.classifications.is_empty() && !extras.ratings.is_empty() {
            meta.classifications.extend(extras.ratings.iter().map(|(kind, value)| {
                ira_models::ScraperClassification {
                    kind: kind.clone(),
                    value: value.clone(),
                }
            }));
            changed = true;
        }
    }
    if !changed {
        return RefetchOutcome::Unchanged;
    }
    match ira_db::store_scraper_metadata(db, db_id, &meta) {
        Ok(()) => {
            eprintln!("Steam refetch: filled gaps for game {db_id}");
            RefetchOutcome::Filled
        }
        Err(e) => RefetchOutcome::Failed(e),
    }
}

/// Start the refetch over every source at once: Steam first (fast),
/// then the paced ScreenScraper pass. One strip job covers both, so
/// "everything" really is one click.
pub fn start_full_refetch(state: &SharedState, force: bool) -> bool {
    let (busy, allowed) = {
        let s = state.borrow();
        (
            s.ss_job_busy.get(),
            s.cfg.screenscraper_enabled && !s.cfg.screenscraper_id.is_empty(),
        )
    };
    if busy || !allowed {
        return false;
    }
    let ss_queue = super::mass_match_ss::refetch_queue(state, force);
    let steam_queue = steam_refetch_queue(state, force);
    // The genre table rides along when never fetched: "everything"
    // includes the index the genre pickers search. Families stay
    // harvest-only — their table is library-dependent.
    let genres_wanted =
        matches!(ira_db::entity_fetched_at(&state.borrow().db, ira_db::KIND_GENRE), Ok(None));
    if ss_queue.is_empty() && steam_queue.is_empty() && !genres_wanted {
        return false;
    }
    let Some(job) = super::fetch_images::begin_strip_job(
        state,
        &crate::tr!("Fetching metadata…"),
        &crate::tr!("Fetching missing metadata…"),
    ) else {
        return false;
    };
    state.borrow().ss_job_busy.set(true);

    let (steam, creds, db) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            ira_api::ScraperCreds::from_account(
                s.cfg.screenscraper_id.clone(),
                s.cfg.screenscraper_password.clone(),
            ),
            s.db.clone(),
        )
    };
    let cancel = job.cancel_flag();
    let jobs: Vec<(Source, i64)> = steam_queue
        .into_iter()
        .map(|db_id| (Source::Steam, db_id))
        .chain(ss_queue.into_iter().map(|db_id| (Source::Ss, db_id)))
        .chain(genres_wanted.then_some((Source::GenresTable, 0)))
        .collect();
    let rx = spawn_full_refetch_worker(jobs, steam, creds, db, Some(cancel));
    let state = state.clone();
    poll_refetch(
        state,
        job,
        rx,
        crate::tr!("{} of {} games updated"),
        crate::tr!("Metadata fetched"),
        |state| state.borrow().ss_job_busy.set(false),
    );
    true
}

/// Which source a combined-queue item comes from — it decides the pace:
/// Steam answers are cheap, ScreenScraper ones ride the quota.
#[derive(Clone, Copy)]
enum Source {
    Steam,
    Ss,
    GenresTable,
}

fn spawn_full_refetch_worker(
    jobs: Vec<(Source, i64)>,
    steam: std::sync::Arc<ira_api::SteamDataClient>,
    creds: ira_api::ScraperCreds,
    db: ira_db::DbConn,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> async_channel::Receiver<RefetchProgress> {
    use std::sync::atomic::Ordering;

    let total = jobs.len();
    let (tx, rx) = super::helpers::ui_channel();
    std::thread::spawn(move || {
        for (index, (source, db_id)) in jobs.into_iter().enumerate() {
            if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                break;
            }
            if index > 0 {
                let pace = match source {
                    Source::Steam | Source::GenresTable => 250,
                    Source::Ss => 1000,
                };
                std::thread::sleep(std::time::Duration::from_millis(pace));
            }
            let current = match source {
                Source::GenresTable => crate::tr!("the genre table").to_string(),
                _ => ira_db::find_by_db_id(&db, db_id)
                    .ok()
                    .flatten()
                    .map(|entry| entry.title)
                    .unwrap_or_default(),
            };
            let outcome = match source {
                Source::Steam => steam_refetch_one(&steam, &db, db_id),
                Source::Ss => super::mass_match_ss::refetch_one(&steam, &creds, &db, db_id),
                Source::GenresTable => match fetch_and_warm_genres(&steam, &db, &creds) {
                    Ok(count) => {
                        eprintln!("Refetch: genre table warmed with {count} entries");
                        RefetchOutcome::Filled
                    }
                    Err(e) => RefetchOutcome::Failed(e),
                },
            };
            let _ = tx.try_send(RefetchProgress {
                done: index + 1,
                total,
                db_id,
                current,
                outcome,
            });
        }
    });
    rx
}

/// Drive a refetch job's channel on the UI loop: strip progress per
/// report, a section refresh on every filled game, and the finish
/// summary when the worker disconnects. `on_finish` releases whatever
/// gate the caller held.
fn poll_refetch(
    state: SharedState,
    job: super::fetch_images::StripJob,
    rx: async_channel::Receiver<RefetchProgress>,
    short_done: String,
    status_done: String,
    on_finish: impl Fn(&SharedState) + 'static,
) {
    // The receiver wakes the main loop on every message; the worker's
    // disconnect (done, cancelled, or failed) is the finish signal.
    glib::spawn_future_local(async move {
        let mut filled = 0usize;
        let mut processed = 0usize;
        loop {
            match rx.recv().await {
                Ok(progress) => {
                    processed += 1;
                    match &progress.outcome {
                        RefetchOutcome::Filled => {
                            filled += 1;
                            // Only repaints when the refilled game's settings
                            // window is the one open right now.
                            super::edit_game_scraper::refresh_scraper_section(
                                &state,
                                progress.db_id,
                            );
                        }
                        // A failed game explains itself in the terminal —
                        // the summary alone would read as a misreport
                        // ("why did my Steam games not update?").
                        RefetchOutcome::Failed(e) => {
                            eprintln!("Refetch: '{}': {e}", progress.current);
                        }
                        RefetchOutcome::Unchanged => {}
                    }
                    job.progress(&state, progress.done, progress.total, &progress.current);
                }
                Err(_) => {
                    on_finish(&state);
                    job.finish(
                        &state,
                        &short_done
                            .replacen("{}", &filled.to_string(), 1)
                            .replacen("{}", &processed.to_string(), 1),
                        &status_done,
                    );
                    return;
                }
            }
        }
    });
}
