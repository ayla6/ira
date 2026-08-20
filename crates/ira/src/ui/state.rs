use super::game_selection_model::GameSelectionModel;
use crate::AppSender;
use crate::Game;
use ira_api::types::SgdbAsset;
use ira_api::SteamDataClient;
use ira_config::Config;
use ira_db::DbConn;
use ira_input::ControllerRegistry;
use ira_models::{Group, GroupSelection};
use ira_watcher::AchievementWatcher;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub enum PendingImage {
    Path(String),
    Bytes(Vec<u8>),
}

/// Fetched SGDB asset list for one asset type, kept alive only while the
/// per-game settings screen is open so reopening the picker is instant. The
/// picker window itself is also kept alive (hidden, not destroyed) so its
/// loaded thumbnails survive closing and reopening.
#[derive(Clone)]
pub struct SgdbAssetsCacheEntry {
    pub assets: Vec<SgdbAsset>,
    pub has_more: bool,
    pub next_page: u32,
    pub picker: glib::WeakRef<adw::Window>,
}

#[derive(Clone)]
pub struct SettingsData {
    pub window: adw::Window,
    pub stack: gtk4::Stack,
    pub db_id: i64,
    pub pending_copies: Rc<RefCell<HashMap<String, PendingImage>>>,
    pub sgdb_cache: Rc<RefCell<HashMap<String, SgdbAssetsCacheEntry>>>,
    pub ra_container: Option<gtk4::Box>,
}

pub struct AppState {
    pub source_id: Cell<Option<u32>>,
    pub window: adw::ApplicationWindow,
    pub games: Vec<Game>,
    pub sidebar_store: gio::ListStore,
    pub sidebar_selection: GameSelectionModel,
    pub sidebar_view: gtk4::ListView,
    pub sidebar_scroll: gtk4::ScrolledWindow,
    pub content_scroll: gtk4::ScrolledWindow,
    pub content_box: gtk4::Box,
    pub grid_header: gtk4::Box,
    pub loading_status: Option<gtk4::Label>,
    pub loading_progress: Option<gtk4::ProgressBar>,
    pub grid_item_height: Cell<i32>,
    pub selected_id: String,
    pub displayed_db_id: i64,
    pub displayed_variant_id: Option<i64>,
    pub displayed_content_dirty: bool,
    pub cfg: Config,
    pub steam: Arc<SteamDataClient>,
    pub watcher: Option<AchievementWatcher>,
    pub shadps4_watcher: Option<ira_platforms::ps4::ShadPS4Watcher>,
    pub rpcs3_watcher: Option<ira_platforms::ps3::Rpcs3Watcher>,
    pub db: DbConn,
    pub sender: AppSender,
    pub game_names: Arc<Mutex<HashMap<String, String>>>,
    pub controller_registry: Arc<ControllerRegistry>,
    pub content_unloaded: bool,
    pub restoring: bool,
    pub running_games: Arc<Mutex<HashMap<i64, i32>>>,
    pub grid_store: gio::ListStore,
    pub sidebar_rebuild_pending: bool,
    pub view_generation: u32,
    pub settings_data: Option<SettingsData>,
    pub save_dir: String,
    pub search_query: String,
    pub selected_group: GroupSelection,
    pub groups: Vec<Group>,
    pub group_members: HashMap<i64, HashSet<i64>>,
    pub search_entry: gtk4::SearchEntry,
    pub sort_label: gtk4::Label,
    pub collapsed_collections: HashSet<i64>,
    pub multi_selected_ids: HashSet<String>,
}

pub type SharedState = Rc<RefCell<AppState>>;

extern "C" {
    pub fn malloc_trim(pad: usize) -> i32;
}
