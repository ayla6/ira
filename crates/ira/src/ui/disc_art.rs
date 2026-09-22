//! Per-disc artwork for the disc picker: ScreenScraper's `support-2D`
//! media (the photo of the CD/cartridge each disc shipped on), fetched
//! once per game and cached on disk by the api crate. Discs the service
//! has no art for never arrive here — the pickers keep their numbered
//! icons as the fallback.

use super::state::SharedState;
use ira_api::ScraperCreds;
use std::collections::HashMap;
use std::sync::mpsc;

/// Fetch the disc art of one game and hand the decoded textures to
/// `ready` on the main loop, keyed by disc number. Runs the network work
/// on a background thread; without a ScreenScraper match or credentials
/// the map stays empty and the callback still fires.
pub(super) fn fetch_disc_art<F>(state: &SharedState, db_id: i64, ready: F)
where
    F: FnOnce(HashMap<i32, gdk4::Texture>) + 'static,
{
    let (steam, creds, ss_id) = {
        let s = state.borrow();
        let ss_id = ira_db::scraper_metadata_for_game(&s.db, db_id)
            .ok()
            .flatten()
            .map(|metadata| metadata.ss_id)
            .unwrap_or_default();
        (
            s.steam.clone(),
            ScraperCreds::from_account(
                s.cfg.screenscraper_id.clone(),
                s.cfg.screenscraper_password.clone(),
            ),
            ss_id,
        )
    };
    let (tx, rx) = mpsc::channel::<HashMap<i32, Vec<u8>>>();
    if ss_id.is_empty() {
        let _ = tx.send(HashMap::new());
    } else {
        std::thread::spawn(move || {
            let bytes = steam
                .screenscraper_disc_media(&creds, &ss_id)
                .map(|media| {
                    media
                        .into_iter()
                        .map(|entry| (entry.disc, entry.png))
                        .collect()
                });
            let map = match bytes {
                Ok(map) => map,
                Err(e) => {
                    eprintln!("Disc art fetch failed: {e}");
                    HashMap::new()
                }
            };
            let _ = tx.send(map);
        });
    }
    // The PNG decode runs here rather than on the fetch thread so only
    // the main loop touches gdk textures.
    let ready = std::rc::Rc::new(std::cell::RefCell::new(Some(ready)));
    glib::source::idle_add_local(move || match rx.try_recv() {
        Ok(map) => {
            let textures: HashMap<i32, gdk4::Texture> = map
                .into_iter()
                .filter_map(|(disc, png)| {
                    let bytes = glib::Bytes::from_owned(png);
                    gdk4::Texture::from_bytes(&bytes)
                        .map(|texture| (disc, texture))
                        .ok()
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
