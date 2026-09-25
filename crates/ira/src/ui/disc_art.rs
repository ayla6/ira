//! Per-disc artwork for the disc pickers and the Images page:
//! ScreenScraper's `support-2D` media (the photo of the CD/cartridge
//! each disc shipped on), cached as `disc{n}.webp` beside the game's
//! other images. Discs the service has no art for never arrive here —
//! the pickers keep their numbered icons as the fallback.

use super::state::SharedState;
use crate::Game;
use ira_api::ScraperCreds;
use std::collections::HashMap;
use std::sync::mpsc;

/// The data-dir file stem for one disc's art: `disc1`, `disc2`, …
pub(super) fn disc_file_base(disc_number: i32) -> String {
    format!("disc{disc_number}")
}

/// True for the settings-draft keys disc art stages under: `disc` plus
/// a positive disc number, so the Save pipeline can tell them apart
/// from asset-type keys.
pub(super) fn is_disc_draft_key(key: &str) -> bool {
    key.strip_prefix("disc").is_some_and(|rest| {
        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
    })
}

/// The ScreenScraper region the game's own ROMs point at, for ordering
/// names and art: disc serial prefixes and filename tags, `None` when
/// no path says anything about region.
pub(crate) fn rom_region(db: &ira_db::DbConn, db_id: i64) -> Option<&'static str> {
    let entry = ira_db::find_by_db_id(db, db_id).ok().flatten()?;
    let mut paths: Vec<String> = ira_db::get_discs(db, db_id)
        .unwrap_or_default()
        .into_iter()
        .map(|disc| disc.rom_path)
        .collect();
    if !entry.rom_path.is_empty() {
        paths.push(entry.rom_path);
    }
    ira_models::region_from_rom_paths(&paths)
}

/// The region order every disc-art path downloads with: the region
/// the game's own ROMs come from first, the service default filling
/// the rest.
fn region_order(db: &ira_db::DbConn, db_id: i64) -> Vec<String> {
    let mut regions = Vec::new();
    if let Some(region) = rom_region(db, db_id) {
        regions.push(region.to_string());
    }
    for fallback in ["us", "wor", "ss", "eu", "jp"] {
        if !regions.iter().any(|region| region == fallback) {
            regions.push(fallback.to_string());
        }
    }
    regions
}

/// The ScreenScraper game id the metadata match recorded — the only id
/// disc media can be queried by.
fn ss_match_id(db: &ira_db::DbConn, db_id: i64) -> String {
    ira_db::scraper_metadata_for_game(db, db_id)
        .ok()
        .flatten()
        .map(|metadata| metadata.ss_id)
        .unwrap_or_default()
}

/// Disc numbers with no art on disk yet — the shared "what's missing"
/// behind the picker, the drafts, and the automatic ensure.
fn missing_disc_numbers(discs: &[i32], present: &HashMap<i32, Vec<u8>>) -> Vec<i32> {
    discs
        .iter()
        .copied()
        .filter(|number| !present.contains_key(number))
        .collect()
}

/// The blocking download half behind every disc-art path: fetches the
/// missing discs' media with the region order, returning only what the
/// service actually had. Persists nothing — callers decide.
fn download_disc_media(
    steam: &ira_api::SteamDataClient,
    creds: &ScraperCreds,
    ss_id: &str,
    regions: &[String],
    missing: &[i32],
) -> HashMap<i32, Vec<u8>> {
    let mut downloaded = HashMap::new();
    if missing.is_empty() || ss_id.is_empty() {
        return downloaded;
    }
    match steam.screenscraper_disc_media(creds, ss_id, regions) {
        Ok(media) => {
            for entry in media {
                if missing.contains(&entry.disc) {
                    downloaded.insert(entry.disc, entry.png);
                }
            }
        }
        Err(e) => eprintln!("Disc art fetch failed: {e}"),
    }
    downloaded
}

/// Blocking disc-art ensure for the automatic passes (fetch-all,
/// enrichment, post-match): missing per-disc art downloads straight to
/// the data dir — no drafts, no main-loop round-trip. Returns true when
/// at least one disc image landed. Single-disc games, games without a
/// ScreenScraper match, and fully present sets have nothing to do.
pub(crate) fn ensure_game_discs(
    steam: &ira_api::SteamDataClient,
    db: &ira_db::DbConn,
    cfg: &ira_config::Config,
    save_dir: &str,
    game: &Game,
) -> bool {
    let discs: Vec<i32> = ira_db::get_discs(db, game.db_id)
        .unwrap_or_default()
        .into_iter()
        .map(|disc| disc.disc_number)
        .collect();
    if discs.len() <= 1 {
        return false;
    }
    let dir = ira_parser::game_data_dir(save_dir, game);
    let missing = missing_disc_numbers(&discs, &present_art(&Some(dir.clone()), &discs));
    if missing.is_empty() {
        return false;
    }
    let ss_id = ss_match_id(db, game.db_id);
    if ss_id.is_empty() {
        return false;
    }
    let creds = ScraperCreds::from_account(
        cfg.screenscraper_id.clone(),
        cfg.screenscraper_password.clone(),
    );
    let downloaded = download_disc_media(steam, &creds, &ss_id, &region_order(db, game.db_id), &missing);
    if downloaded.is_empty() {
        return false;
    }
    for (disc, png) in downloaded {
        persist_disc_art(&dir, disc, &png);
    }
    true
}

/// Everything a disc-art fetch needs, read off the main loop up front
/// so the network thread never borrows state.
struct DiscFetchCtx {
    steam: std::sync::Arc<ira_api::SteamDataClient>,
    creds: ScraperCreds,
    ss_id: String,
    regions: Vec<String>,
    data_dir: Option<std::path::PathBuf>,
    discs: Vec<i32>,
}

fn fetch_context(state: &SharedState, db_id: i64) -> DiscFetchCtx {
    let (steam, creds, ss_id, save_dir, game, db) = {
        let s = state.borrow();
        let ss_id = ss_match_id(&s.db, db_id);
        let game = s.games.iter().find(|g| g.db_id == db_id).cloned();
        (
            s.steam.clone(),
            ScraperCreds::from_account(
                s.cfg.screenscraper_id.clone(),
                s.cfg.screenscraper_password.clone(),
            ),
            ss_id,
            s.save_dir.clone(),
            game,
            s.db.clone(),
        )
    };
    let discs = ira_db::get_discs(&db, db_id)
        .unwrap_or_default()
        .into_iter()
        .map(|disc| disc.disc_number)
        .collect();
    let data_dir = game
        .as_ref()
        .map(|game| ira_parser::game_data_dir(&save_dir, game));
    DiscFetchCtx {
        steam,
        creds,
        ss_id,
        regions: region_order(&db, db_id),
        data_dir,
        discs,
    }
}

/// The art already on disk, keyed by disc number.
fn present_art(data_dir: &Option<std::path::PathBuf>, discs: &[i32]) -> HashMap<i32, Vec<u8>> {
    let mut present = HashMap::new();
    let Some(dir) = data_dir else {
        return present;
    };
    for number in discs {
        let base = disc_file_base(*number);
        if let Some(path) = ira_parser::find_image_file(dir, &base) {
            if let Ok(bytes) = std::fs::read(&path) {
                present.insert(*number, bytes);
            }
        }
    }
    present
}

/// Persist one downloaded disc image as `disc{n}.webp` in the game's
/// data dir, best-effort: a failed write only costs the next download.
/// Transparent margins are trimmed first so every disc tiles at a
/// regular size no matter how the service framed its photo.
fn persist_disc_art(dir: &std::path::Path, disc_number: i32, png: &[u8]) {
    let bytes = ira_parser::trim_transparent_margins(png).unwrap_or_else(|| png.to_vec());
    let path = dir.join(format!("{}.png", disc_file_base(disc_number)));
    if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, &bytes)) {
        eprintln!("Failed to save disc image {}: {e}", path.display());
        return;
    }
    ira_parser::convert_to_lossless_webp(&path);
}

/// Fetch the disc art of one game and hand the decoded textures to
/// `ready` on the main loop, keyed by disc number. Everything heavy
/// (disk reads, the network, the photo decode and downscale) runs on a
/// background thread, which hands over raw pixels; the main loop only
/// uploads them into textures — no second decode. Textures cap at
/// `max_px` a side: a size request is a minimum, so a bigger texture's
/// natural size would grow the tile past it. Art already on disk is
/// read straight away; only missing discs hit the network (with the
/// default region order), and what arrives is persisted before
/// decoding. Without a ScreenScraper match or credentials the map
/// holds whatever the disk had, and the callback still fires.
pub(super) fn fetch_disc_art<F>(state: &SharedState, db_id: i64, max_px: u32, ready: F)
where
    F: FnOnce(HashMap<i32, gdk4::Texture>) + 'static,
{
    let ctx = fetch_context(state, db_id);

    let (tx, rx) = mpsc::channel::<HashMap<i32, (u32, u32, Vec<u8>)>>();
    std::thread::spawn(move || {
        let mut map = present_art(&ctx.data_dir, &ctx.discs);
        let missing = missing_disc_numbers(&ctx.discs, &map);
        for (disc, png) in
            download_disc_media(&ctx.steam, &ctx.creds, &ctx.ss_id, &ctx.regions, &missing)
        {
            if let Some(ref dir) = ctx.data_dir {
                persist_disc_art(dir, disc, &png);
            }
            map.insert(disc, png);
        }
        let mut pixels = HashMap::new();
        for (disc, bytes) in map {
            if let Some((width, height, rgba)) =
                ira_parser::decode_rgba_preview(&bytes, max_px)
            {
                pixels.insert(disc, (width, height, rgba));
            }
        }
        let _ = tx.send(pixels);
    });
    // Pixel uploads stay on the main loop; undecodable files simply
    // never arrive and their tiles keep the fallback icon.
    let ready = std::rc::Rc::new(std::cell::RefCell::new(Some(ready)));
    glib::source::idle_add_local(move || match rx.try_recv() {
        Ok(map) => {
            let textures: HashMap<i32, gdk4::Texture> = map
                .into_iter()
                .map(|(disc, (width, height, rgba))| {
                    let pixbuf = gdk_pixbuf::Pixbuf::from_bytes(
                        &glib::Bytes::from_owned(rgba),
                        gdk_pixbuf::Colorspace::Rgb,
                        true,
                        8,
                        width as i32,
                        height as i32,
                        width as i32 * 4,
                    );
                    (disc, gdk4::Texture::for_pixbuf(&pixbuf))
                })
                .collect();
            if let Some(ready) = ready.borrow_mut().take() {
                ready(textures);
            }
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

/// Stage the disc art of one game for the settings drafts instead of
/// persisting it: missing discs download with the ROM-derived region
/// preference and land in `ready` as raw bytes, saved only if the
/// dialog itself is saved. Disk art still short-circuits the network.
pub(super) fn fetch_disc_art_staged<F>(state: &SharedState, db_id: i64, ready: F)
where
    F: FnOnce(HashMap<i32, Vec<u8>>) + 'static,
{
    let ctx = fetch_context(state, db_id);
    let mut map = present_art(&ctx.data_dir, &ctx.discs);
    let missing = missing_disc_numbers(&ctx.discs, &map);

    let (tx, rx) = mpsc::channel::<HashMap<i32, Vec<u8>>>();
    if missing.is_empty() || ctx.ss_id.is_empty() {
        let _ = tx.send(map);
    } else {
        std::thread::spawn(move || {
            for (disc, png) in
                download_disc_media(&ctx.steam, &ctx.creds, &ctx.ss_id, &ctx.regions, &missing)
            {
                map.insert(disc, png);
            }
            let _ = tx.send(map);
        });
    }
    let ready = std::rc::Rc::new(std::cell::RefCell::new(Some(ready)));
    glib::source::idle_add_local(move || match rx.try_recv() {
        Ok(map) => {
            if let Some(ready) = ready.borrow_mut().take() {
                ready(map);
            }
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

/// Download missing disc art in the background after a ScreenScraper
/// match lands: single-disc games, matched games with no id, and games
/// whose art is all present cost nothing. No textures, no main loop —
/// safe to call from any thread.
pub(super) fn autodownload_disc_art(state: &SharedState, db_id: i64) {
    let (steam, db, cfg, save_dir, game) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            s.db.clone(),
            s.cfg.clone(),
            s.save_dir.clone(),
            s.games.iter().find(|g| g.db_id == db_id).cloned(),
        )
    };
    let Some(game) = game else {
        return;
    };
    std::thread::spawn(move || {
        ensure_game_discs(&steam, &db, &cfg, &save_dir, &game);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disc_file_base_numbers_from_one() {
        assert_eq!(disc_file_base(1), "disc1");
        assert_eq!(disc_file_base(3), "disc3");
    }

    #[test]
    fn test_missing_disc_numbers_skips_present() {
        let discs = vec![1, 2, 3];
        let present = HashMap::from([(1, vec![0u8]), (3, vec![0u8])]);
        assert_eq!(missing_disc_numbers(&discs, &present), vec![2]);
        assert!(missing_disc_numbers(&discs, &HashMap::new()).len() == 3);
        assert!(missing_disc_numbers(&[], &present).is_empty());
    }
}
