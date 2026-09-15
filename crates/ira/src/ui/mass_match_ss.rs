//! The mass matcher's ScreenScraper pass: console games are resolved
//! through their stored ROM md5 first — the exact-match search — and fall
//! back to one platform-narrowed title search with the ROM file's clean
//! name, ES-DE style. No ladder of ever-shorter terms: two requests at
//! most, and only a confirmed miss (the source answered and does not know
//! the game) tombstones the game; a failed request retries on the next
//! dialog opening.

use adw::prelude::*;
use std::collections::HashSet;
use std::sync::Arc;

use super::css::*;
use super::helpers::replace_row_actions;
use super::mass_match_batch::{run_batch, BatchHit, BatchItem, RowActions};
use super::rom_name::{
    clean_rom_name, looks_like_title_id, region_hints, search_head, too_short_for_recherche,
};
use super::ss_match_dialog::{persist_ss_match, show_matched, show_unmatched};
use super::state::SharedState;
use super::steam_search_dialog::status_label;
use crate::Game;
use ira_api::screenscraper::ScrapedGame;
use ira_api::{ScraperCreds, SteamDataClient};
use ira_models::screenscraper_system_id;

/// The per-game outcome of the pass. The distinction decides the
/// tombstone: only `Miss` records one — `Failed` leaves the game untombed
/// so the next opening asks again.
enum SsOutcome {
    Hit(Box<ScrapedGame>),
    Miss,
    Failed(String),
}

/// The status box of a match-list row's ScreenScraper pass, added as its
/// own suffix so the Steam/SGDB/RA boxes stay independent. Rows the source
/// already missed skip straight to the manual search button instead of
/// burning the request again.
pub(super) fn attach_ss_actions(
    row: &adw::ActionRow,
    state: &SharedState,
    game: &Game,
    dialog: &adw::Dialog,
    missed: bool,
) -> gtk4::Box {
    let ss_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    ss_box.set_valign(gtk4::Align::Center);
    if missed {
        show_unmatched(
            &ss_box,
            state,
            game.db_id,
            &game.name,
            &game.platform_id,
            dialog,
        );
    } else {
        ss_box.append(&status_label(
            &crate::tr!("Searching ScreenScraper..."),
            CSS_DIM_LABEL,
        ));
    }
    row.add_suffix(&ss_box);
    ss_box
}

/// Runs the ScreenScraper pass over every row that has an SS box. One
/// request per second keeps the batch well inside the service's quota.
pub(super) fn start_ss_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    rows: &[RowActions],
    dialog: &adw::Dialog,
) {
    let (steam, db, creds, cfg, missed) = {
        let s = state.borrow();
        let missed: HashSet<i64> = match ira_db::scraper_missed_ids(&s.db) {
            Ok(ids) => ids.into_iter().collect(),
            Err(e) => {
                eprintln!("ScreenScraper batch: could not read misses: {e}");
                HashSet::new()
            }
        };
        (
            s.steam.clone(),
            s.db.clone(),
            ScraperCreds::from_account(
                s.cfg.screenscraper_id.clone(),
                s.cfg.screenscraper_password.clone(),
            ),
            s.cfg.clone(),
            missed,
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
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
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
        let _ = tx.send(allowed);
    });
    let rx = std::cell::RefCell::new(rx);
    let mut start = Some((
        queue,
        Arc::clone(&steam),
        db.clone(),
        creds.clone(),
        cfg.clone(),
        state.clone(),
        rows.to_vec(),
        dialog.clone(),
    ));
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        let Ok(allowed) = rx.borrow_mut().try_recv() else {
            return glib::ControlFlow::Continue;
        };
        if let Some((queue, steam, db, creds, cfg, state, rows, dialog)) = start.take() {
            if allowed {
                run_batch(
                    queue,
                    150,
                    1000,
                    {
                        let steam = Arc::clone(&steam);
                        move |item| resolve(&steam, &creds, &db, &cfg, item)
                    },
                    move |hit| apply_hit(&state, &rows, &dialog, hit),
                );
            }
        }
        glib::ControlFlow::Break
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
    let platform_id = entry.platform_id.clone();
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
        // Hash on demand — digest and inner-file size in one streaming
        // pass — and keep the pair stored so this costs nothing next
        // time. The size is the inner ROM's own, never the container's:
        // a wrong size reads as a miss, an absent one still matches on
        // the digest alone.
        if entry.hashes.md5.is_empty() || entry.hashes.size == 0 {
            let pick = rom_extension_pick(&platform_id);
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
    // match over ScreenScraper's own names, whose separator spacing
    // disagrees with the scene (and between systems), so the search head
    // — main title, or two words when the name has no separator — goes
    // out and the acceptance comparison, which flattens punctuation,
    // matches the full name against the candidates. A stem that is only
    // a bare title id cannot be searched at all, so the library title
    // stands in.
    let stem = std::path::Path::new(&abs)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let full = [stem.as_str(), entry.title.as_str(), item.name.as_str()]
        .into_iter()
        .map(clean_rom_name)
        .find(|t| !t.is_empty() && !looks_like_title_id(t))
        .unwrap_or_default();
    // Region tags never gate anything — plenty of dumps carry none — they
    // only matter when two candidates match equally well.
    let hints = region_hints(&stem);
    let term = search_head(&full);
    if term.is_empty() || too_short_for_recherche(&term) {
        eprintln!("SS batch: [{platform_id}] no usable search term");
        return Some(SsOutcome::Miss);
    }
    match steam.screenscraper_search(creds, &term, &platform_id) {
        Err(e) => {
            eprintln!("SS batch: '{term}' [{platform_id}] search failed: {e}");
            Some(SsOutcome::Failed(e))
        }
        Ok(candidates) => {
            let verbose = verbose_logging();
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
                        "SS batch: '{term}' [{platform_id}] no hash hit and no acceptable candidate"
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

/// Lowercase letters-and-digits only, so `13 Sentinels: Aegis Rim` and
/// `13 Sentinels - Aegis Rim (US)` compare equal.
fn normalized_for_match(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// How tightly a candidate name fits the target: 3 the names are equal,
/// 2 one is the other plus decoration in front or behind — subtitle and
/// region chopping tolerated ("(Japan)", "Advance") — 1 the weaker
/// suffix shape, a brand the region's name carries in front of the title
/// ("Simple 2000 Series Vol. 50 : The Daibijin" is "The Daibijin" with
/// a prefix in front). A number leading the extension is a sequel, not
/// decoration: "Advance Wars" is not "Advance Wars 2". `None` is not the
/// game at all.
fn match_rank(target: &str, candidate: &str) -> Option<u8> {
    if target.is_empty() || candidate.is_empty() {
        return None;
    }
    if candidate == target {
        Some(3)
    } else if prefix_extension_ok(target, candidate) || prefix_extension_ok(candidate, target) {
        Some(2)
    } else if suffix_extension_ok(target, candidate) || suffix_extension_ok(candidate, target) {
        Some(1)
    } else {
        None
    }
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

/// UI loop: persist a hit's metadata and repaint the row's SS box; only a
/// confirmed miss tombstones the game, and any non-hit grows the manual
/// search button.
fn apply_hit(
    state: &SharedState,
    rows: &[RowActions],
    dialog: &adw::Dialog,
    hit: BatchHit<SsOutcome>,
) {
    let Some(ss_box) = rows.get(hit.row_idx).and_then(|r| r.ss.clone()) else {
        return;
    };
    let platform_id = state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == hit.db_id)
        .map(|g| g.platform_id.clone())
        .unwrap_or_default();
    match hit.matched {
        Some(SsOutcome::Hit(game)) => {
            persist_ss_match(state, hit.db_id, &game);
            show_matched(&ss_box);
        }
        Some(SsOutcome::Miss) => {
            if let Err(e) = ira_db::tombstone_scraper_miss(&state.borrow().db, hit.db_id) {
                eprintln!("ScreenScraper batch: failed to record the miss: {e}");
            }
            replace_row_actions(&ss_box, |ab| {
                show_unmatched(ab, state, hit.db_id, &hit.name, &platform_id, dialog);
            });
        }
        Some(SsOutcome::Failed(e)) => {
            eprintln!("ScreenScraper batch: '{}': request failed, leaving untombed: {e}", hit.name);
            replace_row_actions(&ss_box, |ab| {
                show_unmatched(ab, state, hit.db_id, &hit.name, &platform_id, dialog);
            });
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{match_rank, normalized_for_match, pick_candidate};
    use ira_api::screenscraper::ScrapedGame;

    #[test]
    fn test_normalized_for_match_ignores_punctuation_and_case() {
        assert_eq!(
            normalized_for_match("13 Sentinels: Aegis Rim"),
            normalized_for_match("13 Sentinels   Aegis Rim!!")
        );
        assert_eq!(normalized_for_match("  pokémon! "), "pok mon");
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
            Some(3)
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
