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
    /// height): every card takes the type's shape like game-grid tiles,
    /// instead of everything squeezing into squares.
    pub(super) tile_aspect: f64,
    /// Per-card views in render order; zoom resizes them in place.
    pub(super) pics: Rc<RefCell<Vec<CardViews>>>,
    pub(super) all_buttons: Rc<RefCell<Vec<glib::WeakRef<gtk4::Button>>>>,
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
