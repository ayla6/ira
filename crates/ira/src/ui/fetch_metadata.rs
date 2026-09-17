//! Mass metadata refetch on the sidebar strip, one source per job. The
//! ScreenScraper pass gives matched games whose stored record has holes
//! one exact fetch by their own id; the Steam pass gives PC games whose
//! stored metadata misses something Steam can give — release date,
//! studios, synopsis, age boards — a re-read of their SteamCMD entry and
//! store page. Both merge only what's missing, both report through the
//! same bottom-of-sidebar strip the image fetcher uses — so they keep
//! running, and stay visible, with dialogs or the whole window closed —
//! and the strip takes one job at a time. The ScreenScraper pass
//! additionally owns the quota gate, so it and the matching pass never
//! stack their requests.

use std::sync::Arc;

use super::mass_match_dialog::normalize_title;
use super::mass_match_ss::{RefetchOutcome, RefetchProgress};
use super::state::SharedState;

/// Start refetching missing metadata for every matched game with holes in
/// its record, revealing the sidebar strip. `false` when a ScreenScraper
/// job or a strip job is already running, or nothing has gaps.
pub fn start_metadata_refetch(state: &SharedState) -> bool {
    let (busy, has_creds) = {
        let s = state.borrow();
        (s.ss_job_busy.get(), !s.cfg.screenscraper_id.is_empty())
    };
    if busy || !has_creds {
        return false;
    }
    let queue = super::mass_match_ss::refetch_queue(state);
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
        crate::tr!("{} games updated"),
        crate::tr!("Metadata fetched"),
        |state| state.borrow().ss_job_busy.set(false),
    );
    true
}

/// Start the Steam-only refetch for every game whose stored metadata
/// misses something Steam can give: PC games by their platform's app
/// id, consoles by an exact title search over the store. The match is
/// metadata only — nothing Steam-ish lands on the game. No
/// ScreenScraper quota involved —
/// the strip's one-job rule is the only gate.
pub fn start_steam_refetch(state: &SharedState) -> bool {
    let queue = steam_refetch_queue(state);
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
        crate::tr!("{} games updated"),
        crate::tr!("Steam data fetched"),
        |_| {},
    );
    true
}

/// Games whose stored metadata misses something Steam can give: games
/// with a store app id on their platform are refetched by it; everyone
/// else — consoles, and manually added PC games that never had one — is
/// found by an exact title search over the store. Includes games with
/// no record at all, and the epoch dates an old diff bug wrote.
fn steam_refetch_queue(state: &SharedState) -> Vec<i64> {
    let s = state.borrow();
    s.games
        .iter()
        .filter(|g| {
            let by_app_id = g.kind.is_pc() && g.platform_id.parse::<u32>().is_ok();
            if by_app_id {
                return true;
            }
            // Title-searched games respect manual unmatch, and the
            // title must be one.
            !g.manual_unmatch
                && (g.kind.is_pc()
                    || g.kind.is_console_emulator()
                    || g.kind == ira_models::GameKind::Retro)
                && !g.name.trim().is_empty()
        })
        .filter(|g| match ira_db::scraper_metadata_for_game(&s.db, g.db_id) {
            Ok(None) => true,
            Ok(Some(meta)) => steam_gaps(&meta),
            Err(_) => false,
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

/// The store's app id for a title, when one of the answers IS the
/// title — normalized on both sides, nothing looser. A near miss is a
/// different game more often than not ("Catherine" is not "Catherine
/// Classic"… except when it is — exactness is the policy).
fn exact_title_hit(results: &[(String, String)], title: &str) -> Option<u32> {
    let norm = normalize_title(title);
    results
        .iter()
        .find(|(_, name)| normalize_title(name) == norm)
        .and_then(|(id, _)| id.parse().ok())
}

/// One sequential worker over the Steam refetch queue: the store title
/// search finds a console game's app id (PC games already carry it),
/// then the SteamCMD entry and the store page get re-read, merged
/// fill-only-gaps over what's stored. A short pace keeps steamcmd.net
/// happy; the worker stands down between items when `cancel` says so.
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

/// One game's Steam re-read, merged over what's stored. Games with a
/// store app id on their platform go straight to it; everyone else is
/// found by an exact title search over the store. SteamCMD's timestamp
/// wins for the release date, the store page's parsed date fills in
/// when it has none — and an epoch date an old bug wrote counts as
/// missing, so refetches repair it.
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
    let app_id = entry
        .platform_id
        .parse::<u32>()
        .ok()
        .or_else(|| exact_title_hit(&steam.search_steam_store(&entry.title), &entry.title));
    let Some(app_id) = app_id else {
        return RefetchOutcome::Unchanged;
    };
    let app_id = app_id.to_string();
    let info = steam.fetch_steamcmd_info(&app_id);
    let extras = steam.fetch_store_extras(&app_id);
    if info.is_none() && extras.is_none() {
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
pub fn start_full_refetch(state: &SharedState) -> bool {
    let (busy, has_creds) = {
        let s = state.borrow();
        (s.ss_job_busy.get(), !s.cfg.screenscraper_id.is_empty())
    };
    if busy || !has_creds {
        return false;
    }
    let ss_queue = super::mass_match_ss::refetch_queue(state);
    let steam_queue = steam_refetch_queue(state);
    if ss_queue.is_empty() && steam_queue.is_empty() {
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
        .collect();
    let rx = spawn_full_refetch_worker(jobs, steam, creds, db, Some(cancel));
    let state = state.clone();
    poll_refetch(
        state,
        job,
        rx,
        crate::tr!("{} games updated"),
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
                    Source::Steam => 250,
                    Source::Ss => 1000,
                };
                std::thread::sleep(std::time::Duration::from_millis(pace));
            }
            let current = ira_db::find_by_db_id(&db, db_id)
                .ok()
                .flatten()
                .map(|entry| entry.title)
                .unwrap_or_default();
            let outcome = match source {
                Source::Steam => steam_refetch_one(&steam, &db, db_id),
                Source::Ss => super::mass_match_ss::refetch_one(&steam, &creds, &db, db_id),
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
        loop {
            match rx.recv().await {
                Ok(progress) => {
                    if matches!(progress.outcome, RefetchOutcome::Filled) {
                        filled += 1;
                        // Only repaints when the refilled game's settings
                        // window is the one open right now.
                        super::edit_game_scraper::refresh_scraper_section(
                            &state,
                            progress.db_id,
                        );
                    }
                    job.progress(&state, progress.done, progress.total, &progress.current);
                }
                Err(_) => {
                    on_finish(&state);
                    job.finish(
                        &state,
                        &short_done.replacen("{}", &filled.to_string(), 1),
                        &status_done,
                    );
                    return;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::exact_title_hit;

    #[test]
    fn test_exact_title_hit_folds_spelling_and_rejects_near_misses() {
        let results = vec![
            ("620".to_string(), "Portal 2".to_string()),
            ("123".to_string(), "Catherine Classic".to_string()),
            ("456".to_string(), "Persona 4: Dancing All Night".to_string()),
        ];
        // Punctuation and case fold on both sides.
        assert_eq!(exact_title_hit(&results, "portal 2"), Some(620));
        assert_eq!(
            exact_title_hit(&results, "Persona 4 Dancing All Night"),
            Some(456)
        );
        // A near miss is a different game more often than not.
        assert_eq!(exact_title_hit(&results, "Catherine"), None);
        assert_eq!(exact_title_hit(&results, "Portal"), None);
        // Store ids that are not numbers never match.
        let odd = vec![("app_1".to_string(), "Portal 2".to_string())];
        assert_eq!(exact_title_hit(&odd, "Portal 2"), None);
    }
}
