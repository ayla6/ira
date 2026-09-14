//! The mass matcher's ScreenScraper pass: console games are resolved
//! through their stored ROM md5 first — the exact-match search — and fall
//! back to one platform-narrowed title search with the display name. Every
//! decision is logged so misses explain themselves, and only a confirmed
//! miss (the source answered and does not know the game) tombstones the
//! game; a failed request retries on the next dialog opening.

use adw::prelude::*;
use std::collections::HashSet;
use std::sync::Arc;

use super::css::*;
use super::helpers::replace_row_actions;
use super::mass_match_batch::{run_batch, BatchHit, BatchItem, RowActions};
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
    let (steam, db, creds, missed) = {
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

    run_batch(
        queue,
        150,
        1000,
        {
            let steam = Arc::clone(&steam);
            let db = db.clone();
            move |item| resolve(&steam, &creds, &db, item)
        },
        {
            let state = state.clone();
            let rows = rows.to_vec();
            let dialog = dialog.clone();
            move |hit| apply_hit(&state, &rows, &dialog, hit)
        },
    );
}

/// Off-thread: the hash search answers authoritatively when the ROM's md5
/// is known to the source; otherwise one title search runs with the
/// display name, and its top candidates must plausibly *be* the game to
/// count — a wrong auto-match would stick forever.
fn resolve(
    steam: &SteamDataClient,
    creds: &ScraperCreds,
    db: &ira_db::DbConn,
    item: &BatchItem,
) -> Option<SsOutcome> {
    let entry = ira_db::find_by_db_id(db, item.db_id).ok().flatten()?;
    let platform_id = entry.platform_id.clone();
    screenscraper_system_id(&platform_id)?;

    // The exact hash search runs only on consoles where file digests are
    // the matching key — disc consoles go serial-first below, and their
    // multi-gigabyte images never get hashed at all. NDS rows keep an
    // RA-flavored rom_hash, so the plain content md5 wins when the scan
    // has filled it. romnom carries the real file name with extension,
    // exactly what ScreenScraper's rom index stores (ES-DE sends it the
    // same way); the stem alone loses the match.
    if ira_models::screenscraper_hashes_content(&platform_id) && !entry.hashes.md5.is_empty() {
        let romnom = std::path::Path::new(&entry.rom_path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| item.name.clone());
        let size = std::fs::metadata(&entry.rom_path).ok().map(|m| m.len());
        let Some(size) = size else {
            eprintln!("SS batch: '{romnom}' [{platform_id}] hash present but file missing");
            return Some(SsOutcome::Miss);
        };
        match steam.screenscraper_rom_lookup(
            creds,
            &romnom,
            &platform_id,
            Some((entry.hashes.md5.as_str(), size)),
        ) {
            Err(e) => {
                eprintln!("SS batch: '{romnom}' [{platform_id}] lookup failed: {e}");
                return Some(SsOutcome::Failed(e));
            }
            Ok(games) => {
                if let Some(game) = games.into_iter().next() {
                    eprintln!(
                        "SS batch: '{romnom}' [{platform_id}] hash hit -> ss id {} '{}'",
                        game.ss_id, game.name
                    );
                    return Some(SsOutcome::Hit(Box::new(game)));
                }
                eprintln!("SS batch: '{romnom}' [{platform_id}] no hash hit");
            }
        }
    }

    // No hash hit: a disc serial is the next-best exact identity — it
    // survives chd/rvz repacks that scramble every file digest. Only
    // serial-shaped ids qualify; RA ids and title ids are plain numbers
    // or too long.
    if ira_models::screenscraper_matches_by_serial(&platform_id) && looks_like_serial(&entry.game_id) {
        let serial = entry.game_id.clone();
        match steam.screenscraper_serial_lookup(creds, &serial, &platform_id) {
            Err(e) => {
                eprintln!("SS batch: serial '{serial}' lookup failed: {e}");
                return Some(SsOutcome::Failed(e));
            }
            Ok(games) => {
                if let Some(game) = games.into_iter().next() {
                    eprintln!(
                        "SS batch: serial '{serial}' [{platform_id}] hit -> ss id {} '{}'",
                        game.ss_id, game.name
                    );
                    return Some(SsOutcome::Hit(Box::new(game)));
                }
            }
        }
        eprintln!("SS batch: serial '{serial}' [{platform_id}] unknown to the source");
    }

    // No hash hit: one title search, preferring the ROM file name — the
    // library title is user-editable and drifts from the dump, while the
    // file name is what the scene shipped. Punctuation goes: the colon in
    // "13 Sentinels: Aegis Rim" poisons ScreenScraper's search (ES-DE
    // strips parentheses for the same reason).
    let stem = std::path::Path::new(&entry.rom_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let term = search_term(&[stem.as_str(), entry.title.as_str(), item.name.as_str()]
        .into_iter()
        .map(clean_rom_name)
        .find(|t| !t.trim().is_empty())
        .unwrap_or_default());
    if term.is_empty() {
        eprintln!("SS batch: [{platform_id}] no hash hit and no name to search");
        return Some(SsOutcome::Miss);
    }
    match steam.screenscraper_search(creds, &term, &platform_id) {
        Err(e) => {
            eprintln!("SS batch: '{term}' [{platform_id}] search failed: {e}");
            Some(SsOutcome::Failed(e))
        }
        Ok(candidates) => {
            let target = normalized_for_match(&term);
            let hit = candidates.into_iter().find(|game| {
                acceptable(&target, &normalized_for_match(&game.name))
            });
            match hit {
                Some(game) => {
                    eprintln!(
                        "SS batch: '{term}' [{platform_id}] no hash hit; name matched ss id {} '{}'",
                        game.ss_id, game.name
                    );
                    Some(SsOutcome::Hit(Box::new(game)))
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

/// Strip the dump tags a ROM stem carries — `(USA)`, `[!]`, `(Rev 1)` —
/// and the separators around them, so the title search sees a title.
fn clean_rom_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut depth = 0usize;
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let cleaned: String = out.replace('_', " ");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
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

/// A candidate counts as the game when the normalized names agree or one
/// is a prefix of the other — subtitle and region chopping tolerated,
/// missing or extra leading words not.
fn acceptable(target: &str, candidate: &str) -> bool {
    !target.is_empty()
        && !candidate.is_empty()
        && (candidate == target
            || candidate.starts_with(target)
            || target.starts_with(candidate))
}

/// The term the title search sends: dump tags gone (ES-DE's
/// removeParenthesis), then every punctuation run collapsed to one space —
/// the colon in "13 Sentinels: Aegis Rim" poisons ScreenScraper's search.
fn search_term(name: &str) -> String {
    clean_rom_name(name)
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
    use super::{acceptable, clean_rom_name, normalized_for_match, search_term};

    #[test]
    fn test_clean_rom_name_strips_dump_tags() {
        assert_eq!(
            clean_rom_name("Fire Emblem - Three Houses (USA) (En,Fr,De) [b]"),
            "Fire Emblem - Three Houses"
        );
        assert_eq!(clean_rom_name("Zelda_No_Densetsu [v1.0]"), "Zelda No Densetsu");
        // Unbalanced opening brackets still keep the text.
        assert_eq!(clean_rom_name("Half Life (Source"), "Half Life");
        assert_eq!(clean_rom_name("   "), "");
    }

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
    fn test_search_term_strips_punctuation_the_source_chokes_on() {
        assert_eq!(search_term("13 Sentinels: Aegis Rim"), "13 Sentinels Aegis Rim");
        assert_eq!(
            search_term("Ace Attorney - Justice for All (USA)"),
            "Ace Attorney Justice for All"
        );
        assert_eq!(search_term("Pokémon: Let's Go"), "Pokémon Let s Go");
    }

    #[test]
    fn test_acceptable_tolerates_subtitles_not_unrelated_games() {
        let target = normalized_for_match("Dragon Quest I & II");
        assert!(acceptable(&target, &normalized_for_match("Dragon Quest I & II")));
        assert!(acceptable(
            &target,
            &normalized_for_match("Dragon Quest I & II (Japan)")
        ));
        assert!(acceptable(
            &normalized_for_match("Final Fantasy VI Advance"),
            &normalized_for_match("Final Fantasy VI")
        ));
        assert!(!acceptable(
            &target,
            &normalized_for_match("Dragon Quest Monsters")
        ));
        // A leading brand word the target lacks is a different title, not
        // the game with decoration.
        assert!(!acceptable(
            &normalized_for_match("Ace Attorney Justice for All"),
            &normalized_for_match("Phoenix Wright Ace Attorney Justice for All")
        ));
        // Empty sides never match — a nameless candidate is not the game.
        assert!(!acceptable(&target, ""));
        assert!(!acceptable("", &target));
    }
}
