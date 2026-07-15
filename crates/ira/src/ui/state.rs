use ira_config::Config;
use ira_db::DbConn;
use ira_api::SteamClient;
use ira_watcher::AchievementWatcher;
use crate::AppSender;
use crate::Game;
use ira_models::{Group, GroupSelection};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub window: adw::ApplicationWindow,
    pub games: Vec<Game>,
    pub rows: HashMap<i64, Vec<super::sidebar::SidebarRowWidgets>>,
    pub game_list: gtk4::ListBox,
    pub sidebar_scroll: gtk4::ScrolledWindow,
    pub content_scroll: gtk4::ScrolledWindow,
    pub content_box: gtk4::Box,
    pub selected_id: String,
    pub cfg: Config,
    pub steam: Arc<SteamClient>,
    pub watcher: Option<AchievementWatcher>,
    pub lutris_watcher: Option<ira_platforms::lutris_watcher::LutrisWatcher>,
    pub shadps4_watcher: Option<ira_platforms::ps4::ShadPS4Watcher>,
    pub db: DbConn,
    pub sender: AppSender,
    pub game_names: Arc<Mutex<HashMap<String, String>>>,
    pub content_unloaded: bool,
    pub restoring: bool,
    pub running_games: Arc<Mutex<HashMap<i64, i32>>>,
    pub grid_refresh_pending: bool,
    pub sidebar_rebuild_pending: bool,
    pub view_generation: u32,
    pub settings_data: Option<(adw::Window, gtk4::Stack, i64)>,
    pub save_dir: String,
    pub search_query: String,
    pub selected_group: GroupSelection,
    pub groups: Vec<Group>,
    pub search_entry: gtk4::SearchEntry,
    pub sort_label: gtk4::Label,
    pub collapsed_collections: HashSet<i64>,
}

pub type SharedState = Rc<RefCell<AppState>>;

extern "C" {
    pub fn malloc_trim(pad: usize) -> i32;
}
