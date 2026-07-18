use ira_config::Config;
use super::game_selection_model::GameSelectionModel;
use ira_db::DbConn;
use ira_api::SteamDataClient;
use ira_watcher::AchievementWatcher;
use crate::AppSender;
use crate::Game;
use ira_models::{Group, GroupSelection};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct SettingsData {
    pub window: adw::Window,
    pub stack: gtk4::Stack,
    pub db_id: i64,
    pub pending_copies: Rc<RefCell<HashMap<String, String>>>,
    pub ra_container: Option<gtk4::Box>,
}

pub struct AppState {
    pub window: adw::ApplicationWindow,
    pub games: Vec<Game>,
    pub sidebar_store: gio::ListStore,
    pub sidebar_selection: GameSelectionModel,
    pub sidebar_view: gtk4::ListView,
    pub sidebar_scroll: gtk4::ScrolledWindow,
    pub content_scroll: gtk4::ScrolledWindow,
    pub content_box: gtk4::Box,
    pub grid_header: gtk4::Box,
    pub selected_id: String,
    pub displayed_db_id: i64,
    pub cfg: Config,
    pub steam: Arc<SteamDataClient>,
    pub watcher: Option<AchievementWatcher>,
    pub shadps4_watcher: Option<ira_platforms::ps4::ShadPS4Watcher>,
    pub db: DbConn,
    pub sender: AppSender,
    pub game_names: Arc<Mutex<HashMap<String, String>>>,
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
    pub search_entry: gtk4::SearchEntry,
    pub sort_label: gtk4::Label,
    pub collapsed_collections: HashSet<i64>,
    pub multi_selected_ids: HashSet<i64>,
}

pub type SharedState = Rc<RefCell<AppState>>;

extern "C" {
    pub fn malloc_trim(pad: usize) -> i32;
}
