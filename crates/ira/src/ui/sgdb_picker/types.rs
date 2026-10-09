use super::super::state::{PendingImage, SgdbAssetsCacheEntry};
use ira_api::types::SgdbAsset;
use ira_api::SteamDataClient;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// One card's views plus its downloaded texture. Zoom rebuilds the
/// paintables from the stored texture at the new tile size — no
/// re-download — and arrivals size the first paintable from the live
/// zoom level. The thumbnail receiver lives here too: one shared
/// ticker polls every card instead of one timeout per card saturating
/// the main loop.
pub(super) struct CardViews {
    pub(super) grid: glib::WeakRef<gtk4::Picture>,
    pub(super) list: glib::WeakRef<gtk4::Picture>,
    pub(super) label: glib::WeakRef<gtk4::Label>,
    pub(super) texture: Option<gdk4::Texture>,
    pub(super) rx: RefCell<Option<std::sync::mpsc::Receiver<Option<Vec<u8>>>>>,
}

#[derive(Clone)]
pub(super) struct SgdbPickerCtx {
    pub(super) id: String,
    pub(super) save_dir: String,
    pub(super) steam: Arc<SteamDataClient>,
    pub(super) asset: String,
    pub(super) is_steam_id: bool,
    // Weak: ctx clones are owned by closures attached to the picker's own
    // widgets, and the settings-screen cache owns the window strongly, so a
    // strong ref here would make window -> closure -> window a cycle.
    pub(super) picker: glib::WeakRef<adw::Window>,
    pub(super) on_done: Rc<dyn Fn()>,
    pub(super) pending_copies: Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
    pub(super) dest_dir: Option<String>,
    // Live view state, so asset cards can react to clicks without closures
    // strongly owning the widgets they render into.
    pub(super) store: Rc<RefCell<Vec<SgdbAsset>>>,
    pub(super) rendered: Rc<Cell<usize>>,
    pub(super) zoom_level: Rc<Cell<i32>>,
    /// Pagination state: next page to fetch, exhaustion, one flight at a
    /// time, and whether the first page has landed yet.
    pub(super) has_more: Rc<Cell<bool>>,
    pub(super) current_page: Rc<Cell<u32>>,
    pub(super) loading_more: Rc<Cell<bool>>,
    pub(super) is_initial_load: Rc<Cell<bool>>,
    /// Uniform tile aspect for this picker's asset type (width over
    /// height): every card takes the type's shape like game-grid tiles.
    pub(super) tile_aspect: f64,
    /// Per-card views in render order.
    pub(super) pics: Rc<RefCell<Vec<CardViews>>>,
    pub(super) all_buttons: Rc<RefCell<Vec<glib::WeakRef<gtk4::Button>>>>,
    /// Bounded thumbnail workers, shared by every card this picker renders.
    pub(super) pool: ThumbPool,
    pub(super) flow: glib::WeakRef<gtk4::FlowBox>,
    pub(super) list_view: glib::WeakRef<gtk4::ListBox>,
    // Persists an author (name, steam64) to the config's filtered-users
    // list; None when the caller has no config to persist to.
    pub(super) filter_user: Option<Rc<dyn Fn(String, String)>>,
    // Removes an author from the filtered-users list; the eye button's
    // unfilter half. Paired with filter_user above.
    pub(super) unfilter_user: Option<Rc<dyn Fn(String)>>,
}

/// Geometry sync callback (zoom, width, height).
pub(super) type GeometrySync = Rc<dyn Fn(i32, i32, i32)>;

/// How browsers fetch a grid of images: around six parallel
/// per-origin requests, first-rendered first. A thread per card
/// stampedes past that on long grids — hundreds of threads over a
/// scrolling session — so one small pool drains a FIFO queue in render
/// order instead. The visible top cards render first, so they still win.
/// Workers exit when the last pool clone drops.
const THUMB_POOL_SIZE: usize = 6;

type ThumbJob = (String, std::sync::mpsc::Sender<Option<Vec<u8>>>);

#[derive(Clone)]
pub(super) struct ThumbPool {
    tx: std::sync::mpsc::Sender<ThumbJob>,
}

impl ThumbPool {
    pub(super) fn new(steam: &Arc<SteamDataClient>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<ThumbJob>();
        let rx = Arc::new(std::sync::Mutex::new(rx));
        for _ in 0..THUMB_POOL_SIZE {
            let rx = rx.clone();
            let steam = steam.clone();
            std::thread::spawn(move || {
                while let Ok((url, reply)) =
                    rx.lock().unwrap_or_else(|e| e.into_inner()).recv()
                {
                    let _s = tracing::info_span!("thumbnail_download", url = %url).entered();
                    let bytes = steam
                        .download_bytes(&url)
                        .ok()
                        .filter(|b| !b.is_empty());
                    let _ = reply.send(bytes);
                }
            });
        }
        Self { tx }
    }

    pub(super) fn submit(&self, url: String, reply: std::sync::mpsc::Sender<Option<Vec<u8>>>) {
        let _ = self.tx.send((url, reply));
    }
}

/// Settings-screen picker cache: one live window per SGDB query, kept
/// alive only while the settings screen is open.
pub(crate) type SgdbPickerCache = Rc<RefCell<HashMap<String, SgdbAssetsCacheEntry>>>;

pub(crate) struct ShowSgdbPickerParams<'a> {
    pub steam: &'a Arc<SteamDataClient>,
    pub id: &'a str,
    pub asset: &'a str,
    pub is_steam_id: bool,
    pub dimensions: &'a [&'a str],
    pub parent: &'a adw::Window,
    pub on_done: Rc<dyn Fn()>,
    pub pending_copies: Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
    pub sgdb_cache: Option<SgdbPickerCache>,
    pub save_dir: &'a str,
    pub dest_dir: Option<&'a str>,
    /// Persists an author (name, steam64) to the filtered-users config;
    /// enables the per-item filter buttons when set.
    pub filter_user: Option<Rc<dyn Fn(String, String)>>,
    /// Removes an author from the filtered-users config; the eye button's
    /// unfilter half. Paired with filter_user above.
    pub unfilter_user: Option<Rc<dyn Fn(String)>>,
    /// Live geometry sync (zoom, width, height): keeps the in-memory
    /// config current so a later settings save can't clobber what the
    /// close-request persists to disk.
    pub on_geometry: Option<GeometrySync>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thumb_pool_serves_queued_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let steam = Arc::new(SteamDataClient::new(
            String::new(),
            String::new(),
            dir.path().to_str().unwrap(),
        ));
        let pool = ThumbPool::new(&steam);
        // Empty urls fail fast: ten queued jobs must all still get a
        // reply, proving the queue drains instead of deadlocking.
        let rxs: Vec<_> = (0..10)
            .map(|_| {
                let (tx, rx) = std::sync::mpsc::channel::<Option<Vec<u8>>>();
                pool.submit(String::new(), tx);
                rx
            })
            .collect();
        for rx in rxs {
            let reply = rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("pooled download replies");
            assert_eq!(reply, None);
        }
    }
}
