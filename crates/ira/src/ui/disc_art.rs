//! Per-disc artwork for the disc pickers and the Images page:
//! ScreenScraper's `support-2D` media (the photo of the CD/cartridge
//! each disc shipped on), cached as `disc{n}.webp` beside the game's
//! other images. Discs the service has no art for never arrive here —
//! the pickers keep their numbered icons as the fallback.

use super::state::SharedState;
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
        let ss_id = ira_db::scraper_metadata_for_game(&s.db, db_id)
            .ok()
            .flatten()
            .map(|metadata| metadata.ss_id)
            .unwrap_or_default();
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
    // The region the game's own ROMs come from orders the download:
    // its art is tried first, the service default fills the rest.
    let mut regions = Vec::new();
    if let Some(region) = rom_region(&db, db_id) {
        regions.push(region.to_string());
    }
    for fallback in ["us", "wor", "ss", "eu", "jp"] {
        if !regions.iter().any(|region| region == fallback) {
            regions.push(fallback.to_string());
        }
    }
    DiscFetchCtx {
        steam,
        creds,
        ss_id,
        regions,
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
        let missing: Vec<i32> = ctx
            .discs
            .iter()
            .copied()
            .filter(|number| !map.contains_key(number))
            .collect();
        if !missing.is_empty() && !ctx.ss_id.is_empty() {
            match ctx
                .steam
                .screenscraper_disc_media(&ctx.creds, &ctx.ss_id, &ctx.regions)
            {
                Ok(media) => {
                    for entry in media {
                        if !missing.contains(&entry.disc) {
                            continue;
                        }
                        if let Some(ref dir) = ctx.data_dir {
                            persist_disc_art(dir, entry.disc, &entry.png);
                        }
                        map.insert(entry.disc, entry.png);
                    }
                }
                Err(e) => eprintln!("Disc art fetch failed: {e}"),
            }
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
    let missing: Vec<i32> = ctx
        .discs
        .iter()
        .copied()
        .filter(|number| !map.contains_key(number))
        .collect();

    let (tx, rx) = mpsc::channel::<HashMap<i32, Vec<u8>>>();
    if missing.is_empty() || ctx.ss_id.is_empty() {
        let _ = tx.send(map);
    } else {
        std::thread::spawn(move || {
            match ctx
                .steam
                .screenscraper_disc_media(&ctx.creds, &ctx.ss_id, &ctx.regions)
            {
                Ok(media) => {
                    for entry in media {
                        if missing.contains(&entry.disc) {
                            map.insert(entry.disc, entry.png);
                        }
                    }
                }
                Err(e) => eprintln!("Disc art fetch failed: {e}"),
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
    let ctx = fetch_context(state, db_id);
    if ctx.discs.len() <= 1 || ctx.ss_id.is_empty() {
        return;
    }
    let data_dir = match ctx.data_dir {
        Some(dir) => dir,
        None => return,
    };
    let map = present_art(&Some(data_dir.clone()), &ctx.discs);
    let missing: Vec<i32> = ctx
        .discs
        .iter()
        .copied()
        .filter(|number| !map.contains_key(number))
        .collect();
    if missing.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        match ctx
            .steam
            .screenscraper_disc_media(&ctx.creds, &ctx.ss_id, &ctx.regions)
        {
            Ok(media) => {
                let mut count = 0;
                for entry in media {
                    if !missing.contains(&entry.disc) {
                        continue;
                    }
                    persist_disc_art(&data_dir, entry.disc, &entry.png);
                    count += 1;
                }
                eprintln!("Disc art autodownloaded {count} tiles for game {db_id}");
            }
            Err(e) => eprintln!("Disc art autodownload failed: {e}"),
        }
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
}
