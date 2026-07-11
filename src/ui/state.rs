use crate::config::Config;
use crate::db::DbConn;
use crate::api::SteamClient;
use crate::watcher::AchievementWatcher;
use crate::AppSender;
use crate::Game;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub window: adw::ApplicationWindow,
    pub games: Vec<Game>,
    pub rows: Vec<super::sidebar::SidebarRowWidgets>,
    pub game_list: gtk4::ListBox,
    pub sidebar_scroll: gtk4::ScrolledWindow,
    pub content_scroll: gtk4::ScrolledWindow,
    pub content_box: gtk4::Box,
    pub selected_id: String,
    pub cfg: Config,
    pub steam: Arc<SteamClient>,
    pub watcher: Option<AchievementWatcher>,
    pub lutris_watcher: Option<crate::platforms::lutris_watcher::LutrisWatcher>,
    pub shadps4_watcher: Option<crate::platforms::ps4::ShadPS4Watcher>,
    pub db: DbConn,
    pub sender: AppSender,
    pub game_names: Arc<Mutex<HashMap<String, String>>>,
    pub content_unloaded: bool,
    pub restoring: bool,
    pub running_games: Arc<Mutex<HashMap<i64, i32>>>,
    pub grid_refresh_pending: bool,
    pub view_generation: u32,
    pub settings_data: Option<(adw::Window, gtk4::Stack, i64)>,
    pub save_dir: String,
}

pub type SharedState = Rc<RefCell<AppState>>;

extern "C" {
    pub fn malloc_trim(pad: usize) -> i32;
}
