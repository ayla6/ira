use crate::config::Config;
use crate::db::DbConn;
use crate::parser::Game;
use crate::steam::SteamClient;
use crate::watcher::AchievementWatcher;
use crate::AppMessage;
use gtk4::glib;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub const SAVE_DIR: &str = "/data/Games/Saves/GSE";
pub const EAGER_IMAGE_BUDGET: usize = 18;

pub struct AppState {
    pub window: adw::ApplicationWindow,
    pub games: Vec<Game>,
    pub rows: Vec<SidebarRowWidgets>,
    pub game_list: gtk4::ListBox,
    pub sidebar_scroll: gtk4::ScrolledWindow,
    pub content_scroll: gtk4::ScrolledWindow,
    pub content_box: gtk4::Box,
    pub selected_id: String,
    pub cfg: Config,
    pub steam: Arc<SteamClient>,
    pub watcher: Option<AchievementWatcher>,
    pub db: DbConn,
    pub sender: Sender<AppMessage>,
    pub game_names: Arc<Mutex<HashMap<String, String>>>,
    pub content_unloaded: bool,
    pub restoring: bool,
    pub running_games: Arc<Mutex<HashMap<i64, std::process::Child>>>,
}

impl AppState {
    pub fn new(
        app: &adw::Application,
        games: Vec<Game>,
        cfg: Config,
        steam: Arc<SteamClient>,
        watcher: Option<AchievementWatcher>,
        db: DbConn,
        sender: Sender<AppMessage>,
        game_names: Arc<Mutex<HashMap<String, String>>>,
    ) -> Self {
        Self {
            window: adw::ApplicationWindow::new(app),
            games,
            rows: Vec::new(),
            game_list: gtk4::ListBox::new(),
            sidebar_scroll: gtk4::ScrolledWindow::new(),
            content_scroll: gtk4::ScrolledWindow::new(),
            content_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
            selected_id: String::new(),
            cfg,
            steam,
            watcher,
            db,
            sender,
            game_names,
            content_unloaded: false,
            restoring: false,
            running_games: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

pub type SharedState = Rc<RefCell<AppState>>;

/// Programmatically select a sidebar row without firing the `row-selected` handler.
pub fn select_row_silently(state: &SharedState, row: Option<&gtk4::ListBoxRow>) {
    state.borrow_mut().restoring = true;
    state.borrow().game_list.select_row(row);
    state.borrow_mut().restoring = false;
}

pub struct SidebarRowWidgets {
    pub row: gtk4::ListBoxRow,
    pub icon: gtk4::Image,
    pub title: gtk4::Label,
    pub subtitle: gtk4::Label,
}

pub struct ImageLoadBudget {
    remaining: usize,
    deferred: Vec<(gtk4::Image, String)>,
}

impl ImageLoadBudget {
    pub fn new(budget: usize) -> Self {
        Self {
            remaining: budget,
            deferred: Vec::new(),
        }
    }

    pub fn load(&mut self, img: &gtk4::Image, path: &str) {
        if path.is_empty() {
            return;
        }
        if self.remaining > 0 {
            self.remaining -= 1;
            crate::images::set_image(img, path);
        } else {
            self.deferred.push((img.clone(), path.to_string()));
        }
    }

    pub fn flush(self) {
        if self.deferred.is_empty() {
            return;
        }
        let reqs = self.deferred;
        let mut i = 0usize;
        glib::idle_add_local(move || {
            let end = (i + 12).min(reqs.len());
            for (img, path) in &reqs[i..end] {
                crate::images::set_image(img, path);
            }
            i = end;
            if i >= reqs.len() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}
