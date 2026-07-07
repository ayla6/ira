use crate::config::Config;
use crate::db::{DbConn, GameEntry};
use crate::parser::{load_game, Game, MergedAchievement, set_achievement_earned};
use crate::steam::SteamClient;
use crate::strings as S;
use crate::watcher::AchievementWatcher;
use crate::AppMessage;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use gtk4::glib;
use gtk4::pango;
use gtk4::prelude::*;
use adw::prelude::*;pub const SAVE_DIR: &str = "/data/Games/Saves/GSE";
const EAGER_IMAGE_BUDGET: usize = 18;

const APP_CSS: &str = "
.sidebar-row-title { min-width: 0; }
.global-bar trough { background-color: transparent; border: none; }
.global-bar progress { border: none; border-radius: 0; }
.hidden-game { opacity: 0.5; }
.popover-btn, .popover-btn > label { font-weight: normal; }
.play-btn-label { font-size: 1.15em; }

.success-label { color: @accent_color; font-weight: bold; }
.logo-pos-btn {
    min-width: 28px;
    min-height: 28px;
    padding: 2px;
    border-radius: 6px;
}
.logo-pos-selected {
    background-color: @accent_color;
    color: white;
}
";

extern "C" {
    pub(crate) fn malloc_trim(pad: usize) -> i32;
}

pub struct AppState {
    pub window: adw::ApplicationWindow,
    pub games: Vec<Game>,
    rows: Vec<SidebarRowWidgets>,
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

pub type SharedState = Rc<RefCell<AppState>>;

/// Programmatically select a sidebar row without firing the `row-selected` handler.
fn select_row_silently(state: &SharedState, row: Option<&gtk4::ListBoxRow>) {
    state.borrow_mut().restoring = true;
    state.borrow().game_list.select_row(row);
    state.borrow_mut().restoring = false;
}

struct SidebarRowWidgets {
    row: gtk4::ListBoxRow,
    icon: gtk4::Image,
    title: gtk4::Label,
    subtitle: gtk4::Label,
}

struct ImageLoadBudget {
    remaining: usize,
    deferred: Vec<(gtk4::Image, String)>,
}

impl ImageLoadBudget {
    fn new(budget: usize) -> Self {
        Self { remaining: budget, deferred: Vec::new() }
    }

    fn load(&mut self, img: &gtk4::Image, path: &str) {
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

    fn flush(self) {
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

pub fn build_ui(
    app: &adw::Application,
    games: Vec<Game>,
    cfg: Config,
    steam: Arc<SteamClient>,
    watcher: Option<AchievementWatcher>,
    db: DbConn,
    sender: Sender<AppMessage>,
    game_names: Arc<Mutex<HashMap<String, String>>>,
) -> SharedState {
    let state = Rc::new(RefCell::new(AppState {
        window: adw::ApplicationWindow::new(app),
        games,
        rows: Vec::new(),
        game_list: gtk4::ListBox::new(),
        sidebar_scroll: gtk4::ScrolledWindow::new(),
        content_scroll: gtk4::ScrolledWindow::new(),
        content_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
        selected_id: String::new(),
        cfg: cfg.clone(),
        steam: steam.clone(),
        watcher: watcher.clone(),
        db,
        sender,
        game_names,
        content_unloaded: false,
        restoring: false,
        running_games: Arc::new(Mutex::new(HashMap::new())),
    }));

    build_window(&state, app);
    let win = state.borrow().window.clone();
    win.present();
    state
}

fn build_window(state: &SharedState, app: &adw::Application) {
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some(S::APP_TITLE));
    window.set_default_size(1100, 720);

    let css = gtk4::CssProvider::new();
    css.load_from_string(APP_CSS);
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().expect("no default display"),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let split_view = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    split_view.set_position(260);
    split_view.set_shrink_start_child(false);
    split_view.set_resize_start_child(false);

    let sidebar_scroll = gtk4::ScrolledWindow::new();
    sidebar_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    sidebar_scroll.set_size_request(220, -1);
    sidebar_scroll.set_vexpand(true);

    let game_list = gtk4::ListBox::new();
    game_list.add_css_class("navigation-sidebar");
    sidebar_scroll.set_child(Some(&game_list));

    split_view.set_start_child(Some(&sidebar_scroll));

    let content_scroll = gtk4::ScrolledWindow::new();
    content_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content_scroll.set_child(Some(&content_box));
    split_view.set_end_child(Some(&content_scroll));

    let header_bar = adw::HeaderBar::new();

    // Hamburger menu with settings + grid scale
    let menu_btn = gtk4::MenuButton::new();
    menu_btn.set_icon_name("open-menu-symbolic");
    menu_btn.set_tooltip_text(Some(S::MENU));
    menu_btn.add_css_class("flat");
    header_bar.pack_end(&menu_btn);

    let popover = gtk4::Popover::new();
    popover.set_size_request(300, -1);
    let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    popover_box.set_margin_start(16);
    popover_box.set_margin_end(16);
    popover_box.set_margin_top(12);
    popover_box.set_margin_bottom(12);

    let settings_btn = gtk4::Button::new();
    let settings_label = gtk4::Label::new(Some(S::SETTINGS));
    settings_label.set_xalign(0.0);
    settings_btn.set_child(Some(&settings_label));
    settings_btn.add_css_class("flat");
    settings_btn.add_css_class("popover-btn");
    settings_btn.set_halign(gtk4::Align::Fill);
    settings_btn.set_margin_top(6);
    settings_btn.set_margin_bottom(6);
    popover_box.append(&settings_btn);

    let match_btn = gtk4::Button::new();
    let match_label = gtk4::Label::new(Some("Match unmatched games"));
    match_label.set_xalign(0.0);
    match_btn.set_child(Some(&match_label));
    match_btn.add_css_class("flat");
    match_btn.add_css_class("popover-btn");
    match_btn.set_halign(gtk4::Align::Fill);
    match_btn.set_margin_top(6);
    match_btn.set_margin_bottom(6);

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    popover_box.append(&match_btn);
    popover_box.append(&sep);

    let state_clone2 = state.clone();
    match_btn.connect_clicked(move |_| show_mass_match_dialog(&state_clone2));

    let hidden_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    hidden_row.set_hexpand(true);
    let hidden_label = gtk4::Label::new(Some(S::SHOW_HIDDEN_GAMES));
    hidden_label.set_xalign(0.0);
    hidden_label.set_hexpand(true);
    let hidden_switch = gtk4::Switch::new();
    hidden_switch.set_active(state.borrow().cfg.show_hidden_games);
    hidden_row.append(&hidden_label);
    hidden_row.append(&hidden_switch);
    popover_box.append(&hidden_row);

    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));

    let state_clone = state.clone();
    hidden_switch.connect_active_notify(move |sw| {
        let active = sw.is_active();
        state_clone.borrow_mut().cfg.show_hidden_games = active;
        let _ = state_clone.borrow().cfg.save();
        let s = state_clone.borrow();
        for (i, g) in s.games.iter().enumerate() {
            if i < s.rows.len() {
                s.rows[i].row.set_visible(!g.hidden || active);
            }
        }
    });

    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some(S::ADD_GAME));
    add_btn.add_css_class("flat");
    // Disabled for now — game adding will move to the Lutris-based flow.
    add_btn.set_sensitive(false);
    header_bar.pack_start(&add_btn);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&split_view));
    window.set_content(Some(&toolbar_view));

    {
        let mut s = state.borrow_mut();
        s.window = window.clone();
        s.game_list = game_list.clone();
        s.sidebar_scroll = sidebar_scroll.clone();
        s.content_scroll = content_scroll.clone();
        s.content_box = content_box.clone();
    }

    rebuild_sidebar(state);

    // Only build grid on initial startup, not when rebuilding from hide-to-background
    if !state.borrow().content_unloaded {
        // Select "All Games" by default
        let row = state.borrow().game_list.row_at_index(0);
        select_row_silently(state, row.as_ref());
    }

    let state_clone = state.clone();
    game_list.connect_row_selected(move |_list, row| {
        let s = state_clone.borrow();
        if s.restoring {
            return;
        }
        let Some(row) = row else { return; };
        let idx = row.index();
        if idx == 0 {
            // "All Games" — grid view removed; just deselect
            drop(s);
            state_clone.borrow_mut().selected_id.clear();
            clear_content(&state_clone);
        } else if idx >= 1 {
            // Game rows start at index 1
            let game_idx = (idx - 1) as usize;
            if game_idx < s.games.len() {
                let lutris_id = s.games[game_idx].lutris_id;
                drop(s);
                switch_to_game(&state_clone, lutris_id);
            }
        }
    });

    let state_clone = state.clone();
    add_btn.connect_clicked(move |_| {
        show_add_game_dialog(&state_clone);
    });

    let state_clone = state.clone();
    settings_btn.connect_clicked(move |_| {
        let (window, cfg, steam) = {
            let s = state_clone.borrow();
            (s.window.clone(), s.cfg.clone(), s.steam.clone())
        };
        show_settings_dialog(&window, cfg, steam, &state_clone);
    });

    let state_clone = state.clone();
    window.connect_close_request(move |_| {
        let close_to_bg = state_clone.borrow().cfg.close_to_background;
        if close_to_bg {
            show_close_choice_dialog(&state_clone);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
}

fn rebuild_sidebar(state: &SharedState) {
    let game_list = state.borrow().game_list.clone();
    let sidebar_scroll = state.borrow().sidebar_scroll.clone();
    // Save scroll position so the list doesn't jump to the top on rebuild
    let saved_scroll = sidebar_scroll.vadjustment().value();

    while let Some(child) = game_list.first_child() {
        game_list.remove(&child);
    }

    // "All Games" row at index 0
    let all_games_row = gtk4::ListBoxRow::new();
    all_games_row.add_css_class("all-games-row");
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    hbox.set_margin_top(8);
    hbox.set_margin_bottom(8);
    hbox.set_margin_start(10);
    hbox.set_margin_end(10);
    let icon = gtk4::Image::from_icon_name("view-grid-symbolic");
    icon.set_pixel_size(32);
    hbox.append(&icon);
    let label = gtk4::Label::new(Some(S::ALL_GAMES));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.add_css_class("heading");
    hbox.append(&label);
    all_games_row.set_child(Some(&hbox));
    game_list.append(&all_games_row);

    // Game rows starting at index 1
    let show_hidden = state.borrow().cfg.show_hidden_games;
    let games: Vec<Game> = state.borrow().games.to_vec();
    let mut rows = Vec::with_capacity(games.len());
    for g in &games {
        rows.push(build_sidebar_row(&game_list, g, state, show_hidden));
    }
    state.borrow_mut().rows = rows;

    // Restore scroll position after rebuild
    let adj = sidebar_scroll.vadjustment();
    let upper = adj.upper();
    let max = (upper - adj.page_size()).max(0.0);
    adj.set_value(saved_scroll.min(max));

    // Restore the previously selected row
    let selected_id = state.borrow().selected_id.clone();
    if selected_id.is_empty() {
        let row = game_list.row_at_index(0);
        select_row_silently(state, row.as_ref());
    } else if let Ok(lutris_id) = selected_id.parse::<i64>() {
        let idx = {
            let s = state.borrow();
            s.games.iter().position(|g| g.lutris_id == lutris_id)
        };
        if let Some(idx) = idx {
            let row = game_list.row_at_index((idx + 1) as i32);
            select_row_silently(state, row.as_ref());
        } else {
            let row = game_list.row_at_index(0);
            select_row_silently(state, row.as_ref());
        }
    }
}

fn build_sidebar_row(list: &gtk4::ListBox, game: &Game, state: &SharedState, show_hidden: bool) -> SidebarRowWidgets {
    let row = gtk4::ListBoxRow::new();

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    hbox.set_margin_top(8);
    hbox.set_margin_bottom(8);
    hbox.set_margin_start(10);
    hbox.set_margin_end(10);

    let icon = if game.icon_path.is_empty() {
        gtk4::Image::from_icon_name("application-x-executable")
    } else {
        crate::images::new_image_from_file(&game.icon_path)
    };
    icon.set_pixel_size(32);
    hbox.append(&icon);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_valign(gtk4::Align::Center);
    vbox.set_hexpand(true);

    let title_label = gtk4::Label::new(Some(&game.name));
    title_label.set_xalign(0.0);
    title_label.add_css_class("sidebar-row-title");
    title_label.set_ellipsize(pango::EllipsizeMode::End);
    title_label.set_hexpand(true);
    title_label.set_tooltip_text(Some(&format!("{} ({})", game.name, game.app_id)));
    vbox.append(&title_label);

    hbox.append(&vbox);
    row.set_child(Some(&hbox));
    list.append(&row);
    row.set_visible(!game.hidden || show_hidden);
    if game.hidden {
        row.add_css_class("hidden-game");
    }

    // Right-click context menu for game settings
    let state_clone = state.clone();
    let game_clone = game.clone();
    let row_clone = row.clone();
    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    right_click.connect_pressed(move |_, _, _, _| {
        show_game_context_menu(&state_clone, &game_clone, &row_clone);
    });
    row.add_controller(right_click);

    SidebarRowWidgets {
        row,
        icon,
        title: title_label,
        subtitle: gtk4::Label::new(None),
    }
}

fn merge_game_enrichment(existing: &Game, updated: &mut Game) {
    // A real (non-placeholder) name already in state — typically the DB title,
    // possibly user-edited — always wins. Only adopt the enriched name when we
    // don't have one yet (e.g. a freshly added game with no title).
    if !existing.name.is_empty() && !existing.name.starts_with("App ID:") {
        updated.name = existing.name.clone();
    }
    // Enrichment reloads the game from disk with a fresh GameEntry, which
    // doesn't know the Lutris metadata or hidden flag — never let it clobber
    // the real values set by build_game_list.
    updated.hidden = existing.hidden;
    updated.lutris_id = existing.lutris_id;
    updated.slug = existing.slug.clone();
    updated.playtime = existing.playtime;
    updated.lastplayed = existing.lastplayed;
    updated.lutris_name = existing.lutris_name.clone();
    updated.manual_unmatch = existing.manual_unmatch;
    if updated.icon_path.is_empty() {
        updated.icon_path = existing.icon_path.clone();
    }
    if updated.hero_image_path.is_empty() {
        updated.hero_image_path = existing.hero_image_path.clone();
    }
    if updated.grid_path.is_empty() {
        updated.grid_path = existing.grid_path.clone();
    }
    if updated.header_path.is_empty() {
        updated.header_path = existing.header_path.clone();
    }
    if updated.logo_path.is_empty() {
        updated.logo_path = existing.logo_path.clone();
    }
    // Enrichment sends empty/0 logo settings — preserve user's DB values
    if updated.logo_position.is_empty() {
        updated.logo_position = existing.logo_position.clone();
    }
    if updated.logo_size == 0 {
        updated.logo_size = existing.logo_size;
    }

    if !existing.achievements.is_empty() {
        let existing_pcts: HashMap<String, f64> = existing
            .achievements
            .iter()
            .map(|a| (a.name.clone(), a.global_percent))
            .collect();
        for a in &mut updated.achievements {
            if a.global_percent == 0.0 {
                if let Some(&pct) = existing_pcts.get(&a.name) {
                    a.global_percent = pct;
                }
            }
        }
    }
}

pub fn handle_app_message(state: &SharedState, msg: AppMessage) {
    match msg {
        AppMessage::EnrichedGame(game) | AppMessage::WatcherGameUpdated(game) => {
            apply_game_update(state, game);
        }
        AppMessage::NewGame(game) => {
            insert_or_update_game(state, game);
        }
        AppMessage::AddGameError(e) => {
            let window = state.borrow().window.clone();
            let dialog = adw::AlertDialog::new(
                Some(S::COULDNT_ADD_GAME),
                Some(&e),
            );
            dialog.add_response("ok", S::OK);
            dialog.set_default_response(Some("ok"));
            dialog.set_close_response("ok");
            dialog.present(Some(&window));
        }
        AppMessage::GameRemoved { app_id } => {
            let mut s = state.borrow_mut();
            s.games.retain(|g| g.app_id != app_id);
            s.game_names.lock().unwrap().remove(&app_id);
            drop(s);
            rebuild_sidebar(state);
        }
        AppMessage::GameStopped(lutris_id) => {
            state.borrow().running_games.lock().unwrap().remove(&lutris_id);
            let selected_id = state.borrow().selected_id.clone();
            if selected_id == lutris_id.to_string() {
                if let Some(game) = state.borrow().games.iter().find(|g| g.lutris_id == lutris_id).cloned() {
                    display_game(&game, state);
                }
            }
        }
    }
}

fn apply_game_update(state: &SharedState, mut updated: Game) {
    let app_id = updated.app_id.clone();

    let game_for_display = {
        let mut s = state.borrow_mut();
        let Some(i) = s.games.iter().position(|g| g.app_id == app_id) else {
            return;
        };

        let was_placeholder =
            s.games[i].name.is_empty() || s.games[i].name.starts_with("App ID:");
        merge_game_enrichment(&s.games[i], &mut updated);

        // Enrichment resolved a real name for a game that previously had none —
        // persist it so we don't re-fetch (and risk overwriting) on every start.
        if was_placeholder && !updated.name.is_empty() && !updated.name.starts_with("App ID:") {
            let db = s.db.clone();
            if let Err(e) = crate::db::update_game_title(&db, updated.db_id, &updated.name) {
                eprintln!("Failed to persist game title: {}", e);
            }
        }

        s.game_names.lock().unwrap().insert(app_id.clone(), updated.name.clone());

        if i < s.rows.len() {
            let rw = &s.rows[i];
            rw.title.set_text(&updated.name);
            rw.title.set_tooltip_text(Some(&format!("{} ({})", updated.name, updated.app_id)));
            if !updated.icon_path.is_empty() {
                crate::images::set_image(&rw.icon, &updated.icon_path);
            }
        }

        // Only the currently-displayed game needs a content rebuild.
        let needs_rebuild = s.selected_id == updated.lutris_id.to_string() && !s.content_unloaded;
        let game = if needs_rebuild { Some(updated.clone()) } else { None };
        s.games[i] = updated;
        game
    };

    if let Some(game) = game_for_display {
        display_game(&game, state);
    }
}

fn insert_or_update_game(state: &SharedState, game: Game) {
    let app_id = game.app_id.clone();
    let lutris_id = game.lutris_id;

    {
        let mut s = state.borrow_mut();
        if !app_id.is_empty() {
            s.game_names.lock().unwrap().insert(app_id.clone(), game.name.clone());
        }

        // Look up by lutris_id (the stable identity) first, then by app_id.
        let found = if lutris_id != 0 {
            s.games.iter().position(|g| g.lutris_id == lutris_id)
        } else {
            None
        };
        let found = found.or_else(|| {
            if !app_id.is_empty() {
                s.games.iter().position(|g| g.app_id == app_id)
            } else {
                None
            }
        });

        if let Some(i) = found {
            let mut g = game;
            // Preserve fields that the incoming game might not have correct
            g.hidden = s.games[i].hidden;
            g.lutris_name = s.games[i].lutris_name.clone();
            g.manual_unmatch = s.games[i].manual_unmatch;
            s.games[i] = g;
        } else {
            s.games.push(game);
            s.games.sort_by(|a, b| a.name.cmp(&b.name));
        }
    }

    if !state.borrow().content_unloaded {
        rebuild_sidebar(state);
        // If no game is selected, re-select "All Games"
        let selected = state.borrow().selected_id.clone();
        if selected.is_empty() {
            let row = state.borrow().game_list.row_at_index(0);
            select_row_silently(state, row.as_ref());
        }
    }
}

pub fn switch_to_game(state: &SharedState, lutris_id: i64) {
    state.borrow_mut().selected_id = lutris_id.to_string();

    // Select the corresponding sidebar row (index + 1 for "All games" at index 0)
    let game_list = state.borrow().game_list.clone();
    let idx = state.borrow().games.iter().position(|g| g.lutris_id == lutris_id);
    if let Some(idx) = idx {
        let row = game_list.row_at_index((idx + 1) as i32);
        select_row_silently(state, row.as_ref());
    }

    let game = state.borrow().games.iter().find(|g| g.lutris_id == lutris_id).cloned();
    if let Some(game) = game {
        display_game(&game, state);
    }
}

fn clear_content(state: &SharedState) {
    let content_box = state.borrow().content_box.clone();
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }
}

fn play_button(state: &SharedState, lutris_id: i64) -> gtk4::Button {
    let running_games = state.borrow().running_games.clone();
    let sender = state.borrow().sender.clone();

    let btn = gtk4::Button::new();
    btn.set_valign(gtk4::Align::Center);
    btn.set_size_request(130, 48);

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_valign(gtk4::Align::Center);
    hbox.set_halign(gtk4::Align::Center);

    let icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
    icon.set_pixel_size(20);
    hbox.append(&icon);

    let label = gtk4::Label::new(Some("Play"));
    label.add_css_class("play-btn-label");
    hbox.append(&label);

    btn.set_child(Some(&hbox));

    let is_running = running_games.lock().unwrap().contains_key(&lutris_id);
    if is_running {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text("Stop");
    } else {
        btn.add_css_class("suggested-action");
    }

    // Click: start / stop
    let icon_click = icon.clone();
    let label_click = label.clone();
    let rg = running_games.clone();
    let s = sender.clone();
    btn.connect_clicked(move |btn| {
        let uri = format!("lutris:rungameid/{}", lutris_id);
        let mut map = rg.lock().unwrap();
        if let Some(mut child) = map.remove(&lutris_id) {
            drop(map);
            // Send SIGTERM so Lutris can shut down the game gracefully
            let _ = std::process::Command::new("kill")
                .arg(child.id().to_string())
                .spawn();
            // Reap the child in background
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            icon_click.set_icon_name(Some("media-playback-start-symbolic"));
            label_click.set_text("Play");
            btn.add_css_class("suggested-action");
            s.send(AppMessage::GameStopped(lutris_id)).ok();
        } else {
            drop(map);
            match std::process::Command::new("lutris").arg(&uri).spawn() {
                Ok(child) => {
                    rg.lock().unwrap().insert(lutris_id, child);
                    icon_click.set_icon_name(Some("window-close-symbolic"));
                    label_click.set_text("Stop");
                    btn.remove_css_class("suggested-action");

                    // Monitor thread: poll for game exit
                    let rg_mon = rg.clone();
                    let s_mon = s.clone();
                    let id = lutris_id;
                    std::thread::spawn(move || {
                        loop {
                            std::thread::sleep(std::time::Duration::from_secs(2));
                            let mut map = rg_mon.lock().unwrap();
                            if let Some(child) = map.get_mut(&id) {
                                match child.try_wait() {
                                    Ok(Some(_)) => {
                                        map.remove(&id);
                                        drop(map);
                                        s_mon.send(AppMessage::GameStopped(id)).ok();
                                        return;
                                    }
                                    Ok(None) => {}
                                    Err(_) => {
                                        map.remove(&id);
                                        drop(map);
                                        s_mon.send(AppMessage::GameStopped(id)).ok();
                                        return;
                                    }
                                }
                            } else {
                                // Already removed (user stopped)
                                return;
                            }
                        }
                    });
                }
                Err(e) => {
                    eprintln!("Failed to launch {}: {}", uri, e);
                }
            }
        }
    });

    btn
}

/// Format playtime (hours, float) as e.g. "5h20min".
fn format_playtime(hours: f64) -> String {
    let total = (hours * 60.0).round() as u64;
    let h = total / 60;
    let m = total % 60;
    match (h, m) {
        (0, 0) => "0min".to_string(),
        (0, m) => format!("{}min", m),
        (h, 0) => format!("{}h", h),
        (h, m) => format!("{}h{:02}min", h, m),
    }
}

/// Format a unix timestamp as e.g. "Jul 6" or "Never".
fn format_lastplayed(ts: i64) -> String {
    if ts == 0 {
        return "Never".to_string();
    }
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%b %-d").to_string())
        .unwrap_or_else(|| "Never".to_string())
}

/// A stat label pair: small caption above a larger value.
fn stat_label(caption: &str, value: &str) -> gtk4::Box {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_valign(gtk4::Align::Center);
    vbox.set_size_request(110, -1);
    let cap = gtk4::Label::new(Some(caption));
    cap.set_xalign(0.0);
    cap.add_css_class("dim-label");
    cap.add_css_class("caption");
    vbox.append(&cap);
    let val = gtk4::Label::new(Some(value));
    val.set_xalign(0.0);
    val.add_css_class("heading");
    vbox.append(&val);
    vbox
}

/// The game header: icon + title on the first row, then a stats row with
/// Play | Last played | Play time | Trophies, with hero background and logo.
fn build_game_header(game: &Game, fraction: f64, state: &SharedState, content_width: i32) -> gtk4::Widget {
    // Build common widgets
    let title_row = {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
        let title_label = gtk4::Label::new(Some(&game.name));
        title_label.set_xalign(0.0);
        title_label.add_css_class("title-1");
        row.append(&title_label);
        row
    };

    let stats_row = {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
        row.set_valign(gtk4::Align::Center);
        row.append(&play_button(state, game.lutris_id));
        row.append(&stat_label("Last played", &format_lastplayed(game.lastplayed)));
        row.append(&stat_label("Play time", &format_playtime(game.playtime)));
        // Trophies count with inline progress bar (hidden when 0 achievements)
        if game.total_count > 0 {
            let tbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            tbox.set_valign(gtk4::Align::Center);
            let cap = gtk4::Label::new(Some("Trophies"));
            cap.set_xalign(0.0);
            cap.add_css_class("dim-label");
            cap.add_css_class("caption");
            tbox.append(&cap);
            let trow = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            trow.set_valign(gtk4::Align::Center);
            let val = gtk4::Label::new(Some(&format!("{}/{}", game.earned_count, game.total_count)));
            val.set_xalign(0.0);
            val.set_valign(gtk4::Align::Center);
            val.add_css_class("heading");
            trow.append(&val);
            let prog = gtk4::ProgressBar::new();
            prog.set_fraction(fraction);
            prog.set_valign(gtk4::Align::Center);
            prog.set_size_request(120, -1);
            trow.append(&prog);
            tbox.append(&trow);
            row.append(&tbox);
        }
        row
    };

    let has_hero = !game.hero_image_path.is_empty();
    if !has_hero {
        let header = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        header.set_margin_top(24);
        header.set_margin_bottom(8);
        header.set_margin_start(24);
        header.set_margin_end(24);
        header.append(&title_row);
        header.append(&stats_row);
        return header.upcast();
    }

    // Hero overlay: hero image + logo (logo painted via DrawingArea for exact positioning)
    let overlay = gtk4::Overlay::new();
    overlay.set_vexpand(false);
    overlay.set_hexpand(true);
    overlay.set_height_request(((content_width as f64) / 3.1).max(150.0) as i32);

    let hero = gtk4::Picture::for_filename(&game.hero_image_path);
    hero.set_halign(gtk4::Align::Fill);
    hero.set_valign(gtk4::Align::Fill);
    hero.set_hexpand(true);
    hero.set_content_fit(gtk4::ContentFit::Cover);
    overlay.set_child(Some(&hero));

    // Logo rendered via DrawingArea stacked on top of hero.
    // The DrawingArea fills the full overlay so we compute the logo position
    // exactly in draw-coordinates (Cairo) – no overlay child-allocation issues.
    if !game.logo_path.is_empty() {
        let logo_pct = game.logo_size.clamp(5, 100);
        let logo_pos = game.logo_position.clone();

        if let Ok(pixbuf) = gtk4::gdk_pixbuf::Pixbuf::from_file(&game.logo_path) {
            let pb_w = pixbuf.width() as f64;
            let pb_h = pixbuf.height() as f64;

            let logo_area = gtk4::DrawingArea::new();
            logo_area.set_halign(gtk4::Align::Fill);
            logo_area.set_valign(gtk4::Align::Fill);
            logo_area.set_hexpand(true);
            logo_area.set_vexpand(true);

            logo_area.set_draw_func(move |_area, cr, area_w, area_h| {
                let w = area_w as f64;
                let h = area_h as f64;
                if w <= 0.0 || h <= 0.0 {
                    return;
                }

                let (sw, sh) = logo_scaled_dims(w, h, pb_w, pb_h, logo_pct);
                let lw = sw as f64;
                let lh = sh as f64;

                let (halign, valign) = logo_position_align(&logo_pos);

                let x = match halign {
                    gtk4::Align::Start => 24.0,
                    gtk4::Align::Center => (w - lw) / 2.0,
                    gtk4::Align::End => w - lw - 24.0,
                    _ => 24.0,
                };
                let y = match valign {
                    gtk4::Align::Start => 24.0,
                    gtk4::Align::Center => (h - lh) / 2.0,
                    gtk4::Align::End => h - lh - 24.0,
                    _ => h - lh - 24.0,
                };

                let _ = cr.save();
                cr.translate(x, y);
                cr.scale(lw / pb_w, lh / pb_h);
                cr.set_source_pixbuf(&pixbuf, 0.0, 0.0);
                let _ = cr.paint();
                let _ = cr.restore();
            });

            overlay.add_overlay(&logo_area);
        }
    }

    // Tick callback: update hero height on resize
    overlay.add_tick_callback(move |o, _fc| {
        let w = o.allocated_width();
        if w > 0 {
            let target = ((w as f64) / 3.1).max(150.0) as i32;
            if o.height_request() != target {
                o.set_height_request(target);
            }
        }
        glib::ControlFlow::Continue
    });

    // Stats bar below hero (not overlaid)
    let stats_container = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
    stats_container.set_margin_start(24);
    stats_container.set_margin_end(24);
    stats_container.set_margin_top(12);
    stats_container.set_margin_bottom(12);
    stats_container.append(&stats_row);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.append(&overlay);
    outer.append(&stats_container);
    outer.upcast()
}

/// Compute logo scaled dimensions so both fit constraints:
/// max height = logo_pct% of hero height, max width = logo_pct/2 % of hero width.
fn logo_scaled_dims(hero_w: f64, hero_h: f64, src_w: f64, src_h: f64, logo_pct: i32) -> (i32, i32) {
    let max_h = hero_h * (logo_pct as f64 / 100.0);
    let max_w = hero_w * (logo_pct as f64 / 200.0);
    let scale = (max_w / src_w).min(max_h / src_h);
    let w = (src_w * scale).max(32.0) as i32;
    let h = (src_h * scale).max(32.0) as i32;
    (w, h)
}

fn logo_position_align(pos: &str) -> (gtk4::Align, gtk4::Align) {
    match pos {
        "bottom-center" => (gtk4::Align::Center, gtk4::Align::End),
        "bottom-right" => (gtk4::Align::End, gtk4::Align::End),
        "center-left" => (gtk4::Align::Start, gtk4::Align::Center),
        "center" => (gtk4::Align::Center, gtk4::Align::Center),
        "center-right" => (gtk4::Align::End, gtk4::Align::Center),
        "top-left" => (gtk4::Align::Start, gtk4::Align::Start),
        "top-center" => (gtk4::Align::Center, gtk4::Align::Start),
        "top-right" => (gtk4::Align::End, gtk4::Align::Start),
        _ => (gtk4::Align::Start, gtk4::Align::End), // bottom-left
    }
}

fn display_game(game: &Game, state: &SharedState) {
    let content_box = state.borrow().content_box.clone();
    let content_scroll = state.borrow().content_scroll.clone();

    // Reset scroll position BEFORE removing old content, so there's no visible jump
    content_scroll.vadjustment().set_value(0.0);

    // Remove old content — this drops widget references to the previous
    // game's textures, so clearing the cache actually frees the pixel data.
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    // Clear the texture cache so the previous game's decoded images are freed.
    // Sidebar icons stay alive (their gtk::Image widgets hold their own refs);
    // only the cache's extra references are dropped.
    crate::images::clear_texture_cache();

    let fraction = if game.total_count > 0 {
        game.earned_count as f64 / game.total_count as f64
    } else {
        0.0
    };

    let content_width = content_scroll.allocated_width().max(600);
    content_box.append(&build_game_header(game, fraction, state, content_width));

    if game.app_id.is_empty() {
        // Unmatched — no achievement source yet.
        let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        box_.set_margin_top(32);
        box_.set_margin_bottom(32);
        box_.set_halign(gtk4::Align::Center);
        let label = gtk4::Label::new(Some("This game isn't linked to a trophy source yet.\nUse \"Match unmatched games\" in the menu to find a match."));
        label.add_css_class("dim-label");
        label.set_wrap(true);
        label.set_justify(gtk4::Justification::Center);
        box_.append(&label);
        content_box.append(&box_);
        return;
    }

    let spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    spacer.set_margin_top(12);
    content_box.append(&spacer);

    let game_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    game_vbox.set_margin_start(16);
    game_vbox.set_margin_end(16);

    let view_stack = adw::ViewStack::new();

    let view_switcher = adw::ViewSwitcher::new();
    view_switcher.set_stack(Some(&view_stack));
    view_switcher.set_halign(gtk4::Align::Center);
    view_switcher.set_margin_top(12);
    view_switcher.set_margin_bottom(12);
    game_vbox.append(&view_switcher);

    let switcher_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    switcher_spacer.set_margin_bottom(12);
    game_vbox.append(&switcher_spacer);

    let progress_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    let global_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);

    let mut earned: Vec<&MergedAchievement> = Vec::new();
    let mut locked: Vec<&MergedAchievement> = Vec::new();
    let mut hidden: Vec<&MergedAchievement> = Vec::new();
    for ach in &game.achievements {
        if ach.earned {
            earned.push(ach);
        } else if ach.hidden {
            hidden.push(ach);
        } else {
            locked.push(ach);
        }
    }

    earned.sort_by(|a, b| b.earned_time.cmp(&a.earned_time));
    locked.sort_by(|a, b| a.display_name.cmp(&b.display_name));

    let app_id_for_reload = game.app_id.clone();
    let kind_for_reload = game.kind.clone();
    let platform_id_for_reload = game.platform_id.clone();
    let db_id_for_reload = game.db_id;
    let lutris_id_for_reload = game.lutris_id;
    let state_for_reload = state.clone();
    let reload = move || {
        let entry = GameEntry {
            id: db_id_for_reload,
            kind: kind_for_reload.clone(),
            steam_id: app_id_for_reload.clone(),
            platform_id: platform_id_for_reload.clone(),
            title: String::new(),
            lutris_db_id: if lutris_id_for_reload != 0 { Some(lutris_id_for_reload) } else { None },
            sgdb_id: None,
            hidden: false,
            logo_position: String::new(),
            logo_size: 0,
            ignored: Some(0),
            manual_unmatch: Some(0),
        };
        if let Ok(updated) = load_game(&entry, SAVE_DIR) {
            apply_game_update(&state_for_reload, updated);
        }
    };

    let mut budget = ImageLoadBudget::new(EAGER_IMAGE_BUDGET);

    if !earned.is_empty() {
        let earned_group = adw::PreferencesGroup::new();
        earned_group.set_title(&format!("Earned  ·  {}", earned.len()));
        for ach in &earned {
            earned_group.add(&create_achievement_row(ach, None, &mut budget));
        }
        progress_vbox.append(&earned_group);
    }

    if !locked.is_empty() || !hidden.is_empty() {
        let locked_group = adw::PreferencesGroup::new();
        locked_group.set_title(&format!("Locked  ·  {}", locked.len() + hidden.len()));
        for ach in &locked {
            let ach_clone = (*ach).clone();
            let reload_clone = reload.clone();
            let kind_clone = game.kind.clone();
            let app_id_clone = game.app_id.clone();
            let platform_id_clone = game.platform_id.clone();
            let state_clone = state.clone();
            locked_group.add(&create_achievement_row(
                ach,
                Some(Box::new(move || {
                    confirm_mark_unlocked(&state_clone, &kind_clone, &app_id_clone, &platform_id_clone, &ach_clone, reload_clone.clone());
                })),
                &mut budget,
            ));
        }
        if !hidden.is_empty() {
            let hidden_row = adw::ActionRow::new();
            hidden_row.set_title(&format!("... and {} hidden trophies", hidden.len()));
            hidden_row.set_subtitle("Earn them to reveal details");
            hidden_row.set_sensitive(false);
            locked_group.add(&hidden_row);
        }
        progress_vbox.append(&locked_group);
    }
    budget.flush();

    let global_built = std::cell::Cell::new(false);
    let app_id_for_global = game.app_id.clone();
    let state_for_global = state.clone();
    let global_vbox_weak = global_vbox.downgrade();
    view_stack.connect_notify_local(Some("visible-child-name"), move |stack, _| {
        if stack.visible_child_name() == Some("global".into()) && !global_built.get() {
            global_built.set(true);
            if let Some(global_vbox) = global_vbox_weak.upgrade() {
                // Re-borrow the game from state instead of cloning it eagerly;
                // only the (rare) case of actually opening the Global tab pays
                // anything, and build_global_tab only reads the game.
                let s = state_for_global.borrow();
                if let Some(game) = s.games.iter().find(|g| g.app_id == app_id_for_global) {
                    build_global_tab(game, &global_vbox);
                }
            }
        }
    });

    let progress_page = view_stack.add_titled(&progress_vbox, Some("progress"), S::MY_PROGRESS);
    progress_page.set_icon_name(Some("user-home-symbolic"));

    let global_page = view_stack.add_titled(&global_vbox, Some("global"), S::GLOBAL_STATS);
    global_page.set_icon_name(Some("dialog-information-symbolic"));

    view_stack.set_vhomogeneous(false);
    view_stack.set_margin_bottom(32);

    game_vbox.append(&view_stack);

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(860);
    clamp.set_tightening_threshold(860);
    clamp.set_margin_start(16);
    clamp.set_margin_end(16);
    clamp.set_child(Some(&game_vbox));

    content_box.append(&clamp);
}

fn build_global_tab(game: &Game, global_vbox: &gtk4::Box) {
    let mut all_ach: Vec<MergedAchievement> = game.achievements.clone();
    all_ach.sort_by(|a, b| b.global_percent.partial_cmp(&a.global_percent).unwrap_or(std::cmp::Ordering::Equal));

    let global_group = adw::PreferencesGroup::new();
    global_group.set_title(S::GLOBAL_UNLOCK_RATES);
    global_group.set_margin_bottom(24);

    let mut budget = ImageLoadBudget::new(EAGER_IMAGE_BUDGET);

    // Build first batch synchronously (fills the viewport so the tab feels instant)
    let first_batch = 30.min(all_ach.len());
    for ach in &all_ach[..first_batch] {
        add_global_row(&global_group, ach, &mut budget);
    }
    global_vbox.append(&global_group);
    budget.flush();

    // Build remaining rows in idle batches so the UI never freezes
    if all_ach.len() > first_batch {
        let remaining = all_ach[first_batch..].to_vec();
        let group = global_group.clone();
        let mut i = 0;
        glib::idle_add_local(move || {
            let end = (i + 20).min(remaining.len());
            let mut batch_budget = ImageLoadBudget::new(0);
            for ach in &remaining[i..end] {
                add_global_row(&group, ach, &mut batch_budget);
            }
            batch_budget.flush();
            i = end;
            if i >= remaining.len() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}

fn add_global_row(group: &adw::PreferencesGroup, ach: &MergedAchievement, budget: &mut ImageLoadBudget) {
    let (row, reveal) = create_global_stats_row(ach, budget);
    group.add(&row);
    if let Some(reveal) = reveal {
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            reveal();
        });
        row.add_controller(click);
    }
}

fn achievement_icon(ach: &MergedAchievement, budget: &mut ImageLoadBudget) -> gtk4::Image {
    let img = gtk4::Image::from_icon_name("changes-prevent-symbolic");
    img.set_pixel_size(48);
    img.set_valign(gtk4::Align::Start);
    if ach.earned {
        if !ach.icon_path.is_empty() {
            budget.load(&img, &ach.icon_path);
        } else {
            img.set_icon_name(Some("starred-symbolic"));
        }
    } else if !ach.icon_gray_path.is_empty() {
        budget.load(&img, &ach.icon_gray_path);
    }
    img
}

fn create_achievement_row(
    ach: &MergedAchievement,
    on_mark_unlocked: Option<Box<dyn Fn()>>,
    budget: &mut ImageLoadBudget,
) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_selectable(false);

    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let img = achievement_icon(ach, budget);
    content.append(&img);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_valign(gtk4::Align::Start);
    vbox.set_hexpand(true);

    let title = gtk4::Label::new(Some(&ach.display_name));
    title.set_xalign(0.0);
    title.set_valign(gtk4::Align::Start);
    vbox.append(&title);

    let desc = gtk4::Label::new(Some(&ach.description));
    desc.set_xalign(0.0);
    desc.set_valign(gtk4::Align::Start);
    desc.set_wrap(true);
    desc.set_wrap_mode(pango::WrapMode::WordChar);
    desc.add_css_class("dim-label");
    desc.add_css_class("caption");
    vbox.append(&desc);

    content.append(&vbox);

    if ach.earned {
        let time_label = gtk4::Label::new(Some(""));
        time_label.set_justify(gtk4::Justification::Right);
        time_label.set_valign(gtk4::Align::Start);
        time_label.add_css_class("dim-label");
        time_label.add_css_class("caption");
        if ach.earned_time > 0 {
            let t = chrono::DateTime::from_timestamp(ach.earned_time, 0)
                .map(|dt| {
                    dt.format("%b %e, %Y @ %l:%M %p")
                        .to_string()
                        .replace("  ", " ")
                })
                .unwrap_or_else(|| "Unknown".to_string());
            time_label.set_text(&t);
        } else {
            time_label.set_text("Marked manually");
        }
        content.append(&time_label);
    }

    row.set_child(Some(&content));

    if let Some(on_mark) = on_mark_unlocked {
        let click = gtk4::GestureClick::new();
        click.set_button(3);
        click.connect_pressed(move |_, _, _, _| {
            on_mark();
        });
        row.add_controller(click);
        row.set_tooltip_text(Some(S::RIGHT_CLICK_TO_MARK));
    }

    row
}

fn create_global_stats_row(
    ach: &MergedAchievement,
    budget: &mut ImageLoadBudget,
) -> (gtk4::ListBoxRow, Option<Box<dyn Fn() + 'static>>) {
    let row = gtk4::ListBoxRow::new();
    row.set_selectable(false);

    let grid = gtk4::Grid::new();

    let progress = gtk4::ProgressBar::new();
    progress.set_fraction(ach.global_percent / 100.0);
    progress.set_valign(gtk4::Align::Fill);
    progress.set_halign(gtk4::Align::Fill);
    progress.set_hexpand(true);
    progress.set_vexpand(true);
    progress.set_opacity(0.18);
    progress.add_css_class("global-bar");

    let is_hidden_spoiler = ach.hidden && !ach.earned;

    // Build a content stack so the entire row (icon + text + percentage)
    // animates together when revealing a hidden achievement.
    let content_stack = gtk4::Stack::new();
    content_stack.set_hexpand(true);
    content_stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
    content_stack.set_transition_duration(350);

    let make_content = |img: gtk4::Image, title: &str, desc: &str| -> gtk4::Box {
        let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.append(&img);

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        vbox.set_valign(gtk4::Align::Start);
        vbox.set_hexpand(true);
        let title_label = gtk4::Label::new(Some(title));
        title_label.set_xalign(0.0);
        vbox.append(&title_label);
        let desc_label = gtk4::Label::new(Some(desc));
        desc_label.set_wrap(true);
        desc_label.set_wrap_mode(pango::WrapMode::WordChar);
        desc_label.set_xalign(0.0);
        desc_label.add_css_class("dim-label");
        desc_label.add_css_class("caption");
        vbox.append(&desc_label);
        content.append(&vbox);

        let pct_label = gtk4::Label::new(Some(&format!("{:.1}%", ach.global_percent)));
        pct_label.set_valign(gtk4::Align::Start);
        pct_label.add_css_class("heading");
        content.append(&pct_label);
        content
    };

    let pct_str = format!("{:.1}%", ach.global_percent);

    let reveal = if is_hidden_spoiler {
        // Spoiler view: gray icon (or placeholder if none) + "Hidden Achievement"
        let spoiler_img = gtk4::Image::from_icon_name("changes-prevent-symbolic");
        spoiler_img.set_pixel_size(48);
        spoiler_img.set_valign(gtk4::Align::Start);
        if !ach.icon_gray_path.is_empty() {
            budget.load(&spoiler_img, &ach.icon_gray_path);
        }

        let spoiler_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        spoiler_content.set_margin_top(8);
        spoiler_content.set_margin_bottom(8);
        spoiler_content.set_margin_start(12);
        spoiler_content.set_margin_end(12);
        spoiler_content.append(&spoiler_img);

        let vbox_spoiler = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        vbox_spoiler.set_valign(gtk4::Align::Start);
        vbox_spoiler.set_hexpand(true);
        let title_spoiler = gtk4::Label::new(Some(S::HIDDEN_ACHIEVEMENT));
        title_spoiler.set_xalign(0.0);
        vbox_spoiler.append(&title_spoiler);
        let desc_spoiler = gtk4::Label::new(Some(S::CLICK_TO_REVEAL));
        desc_spoiler.set_xalign(0.0);
        desc_spoiler.add_css_class("dim-label");
        desc_spoiler.add_css_class("caption");
        vbox_spoiler.append(&desc_spoiler);
        spoiler_content.append(&vbox_spoiler);

        let pct_label = gtk4::Label::new(Some(&pct_str));
        pct_label.set_valign(gtk4::Align::Start);
        pct_label.add_css_class("heading");
        spoiler_content.append(&pct_label);

        content_stack.add_named(&spoiler_content, Some("spoiler"));

        // Real view: gray icon (or placeholder if none) + title + description
        let real_img = gtk4::Image::from_icon_name("changes-prevent-symbolic");
        real_img.set_pixel_size(48);
        real_img.set_valign(gtk4::Align::Start);
        if !ach.icon_gray_path.is_empty() {
            budget.load(&real_img, &ach.icon_gray_path);
        }

        let real_content = make_content(real_img, &ach.display_name, &ach.description);
        content_stack.add_named(&real_content, Some("real"));

        content_stack.set_visible_child_name("spoiler");
        row.set_selectable(true);
        row.set_activatable(true);

        grid.attach(&progress, 0, 0, 1, 1);
        grid.attach(&content_stack, 0, 0, 1, 1);
        row.set_child(Some(&grid));

        let stack_weak = content_stack.downgrade();
        let row_weak = row.downgrade();
        Some(Box::new(move || {
            let Some(row) = row_weak.upgrade() else { return };
            let Some(stack) = stack_weak.upgrade() else { return };
            stack.set_visible_child_name("real");
            row.set_activatable(false);
            row.set_selectable(false);
        }) as Box<dyn Fn()>)
    } else {
        let img = achievement_icon(ach, budget);

        let content = make_content(img, &ach.display_name, &ach.description);
        content_stack.add_named(&content, Some("real"));
        content_stack.set_visible_child_name("real");

        grid.attach(&progress, 0, 0, 1, 1);
        grid.attach(&content_stack, 0, 0, 1, 1);
        row.set_child(Some(&grid));

        None
    };

    (row, reveal)
}

fn open_folder(path: &str) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// A two-response (cancel / confirm) alert dialog. `on_confirm` runs only when
/// the user picks the confirm response.
fn confirm_dialog(
    parent: &adw::ApplicationWindow,
    title: &str,
    body: &str,
    confirm_label: &str,
    appearance: adw::ResponseAppearance,
    on_confirm: impl Fn() + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(body));
    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("confirm", confirm_label);
    dialog.set_response_appearance("confirm", appearance);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, resp| {
        if resp == "confirm" {
            on_confirm();
        }
    });
    dialog.present(Some(parent));
}

fn show_close_choice_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(
        Some(S::CLOSE_VIEWER),
        Some(S::CLOSE_VIEWER_BODY),
    );
    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("background", S::HIDE_TO_BACKGROUND);
    dialog.add_response("quit", S::QUIT);
    dialog.set_response_appearance("background", adw::ResponseAppearance::Suggested);
    dialog.set_response_appearance("quit", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("background"));
    dialog.set_close_response("cancel");

    let state_clone = state.clone();
    dialog.connect_response(None, move |_, resp| {
        match resp {
            "background" => hide_to_background(&state_clone),
            "quit" => {
                let app = state_clone.borrow().window.application()
                    .expect("no application")
                    .downcast::<adw::Application>()
                    .expect("not an adw Application");
                app.quit();
            }
            _ => {}
        }
    });
    dialog.present(Some(&window));
}

pub fn hide_to_background(state: &SharedState) {
    // Remove all content widgets from the old window
    teardown_content(state);

    // Get the app before destroying the window
    let app = state.borrow().window.application()
        .expect("no application")
        .downcast::<adw::Application>()
        .expect("not an adw Application");

    // Destroy the old window entirely — this frees all rendering buffers,
    // GPU resources, and widget GObjects that just removing children can't.
    state.borrow().window.destroy();

    // Build a fresh hidden window (minimal memory footprint)
    build_window(state, &app);
    let win = state.borrow().window.clone();
    win.set_visible(false);

    // Free decoded image textures
    crate::images::clear_texture_cache();

    // Return freed heap pages to the OS
    unsafe { malloc_trim(0); }

    // GTK releases some resources asynchronously — trim again after a delay
    glib::timeout_add_local(std::time::Duration::from_millis(300), || {
        unsafe { malloc_trim(0); }
        glib::ControlFlow::Break
    });
}

fn teardown_content(state: &SharedState) {
    let (content_box, game_list) = {
        let s = state.borrow();
        (s.content_box.clone(), s.game_list.clone())
    };

    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }
    while let Some(child) = game_list.first_child() {
        game_list.remove(&child);
    }

    let mut s = state.borrow_mut();
    s.rows.clear();
    s.content_unloaded = true;
}

pub fn restore_content(state: &SharedState) {
    if !state.borrow().content_unloaded {
        return;
    }
    let selected_id = state.borrow().selected_id.clone();
    state.borrow_mut().content_unloaded = false;
    rebuild_sidebar(state);

    if selected_id.is_empty() {
        clear_content(state);
        // Re-select "All Games"
        let row = state.borrow().game_list.row_at_index(0);
        select_row_silently(state, row.as_ref());
        return;
    }

    let lutris_id: i64 = selected_id.parse().unwrap_or(0);
    let game = state.borrow().games.iter().find(|g| g.lutris_id == lutris_id).cloned();
    if let Some(game) = game {
        display_game(&game, state);

        let game_list = state.borrow().game_list.clone();
        let idx = state.borrow().games.iter().position(|g| g.lutris_id == lutris_id);
        if let Some(idx) = idx {
            let row = game_list.row_at_index((idx + 1) as i32);
            select_row_silently(state, row.as_ref());
        }
    } else {
        state.borrow_mut().selected_id.clear();
        clear_content(state);
        let row = state.borrow().game_list.row_at_index(0);
        select_row_silently(state, row.as_ref());
    }
}

fn show_settings_dialog(
    parent: &adw::ApplicationWindow,
    cfg: Config,
    steam: Arc<SteamClient>,
    state: &SharedState,
) {
    let dialog = adw::Window::new();
    dialog.set_title(Some(S::SETTINGS));
    dialog.set_default_size(450, 360);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(parent));

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&adw::HeaderBar::new());

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let group = adw::PreferencesGroup::new();
    group.set_title(S::API_KEYS);
    group.set_margin_top(16);
    group.set_margin_bottom(16);
    group.set_margin_start(16);
    group.set_margin_end(16);

    let steam_entry = adw::EntryRow::new();
    steam_entry.set_title(S::STEAM_WEB_API_KEY);
    steam_entry.set_text(&cfg.steam_api_key);
    steam_entry.set_input_purpose(gtk4::InputPurpose::Password);
    group.add(&steam_entry);

    let sgdb_entry = adw::EntryRow::new();
    sgdb_entry.set_title(S::STEAMGRIDDB_KEY);
    sgdb_entry.set_text(&cfg.steam_griddb_api_key);
    sgdb_entry.set_input_purpose(gtk4::InputPurpose::Password);
    group.add(&sgdb_entry);

    let notif_group = adw::PreferencesGroup::new();
    notif_group.set_title(S::LIVE_UPDATES);
    notif_group.set_margin_top(16);
    notif_group.set_margin_start(16);
    notif_group.set_margin_end(16);

    let notif_row = adw::SwitchRow::new();
    notif_row.set_title(S::NOTIFY_ON_UNLOCKS);
    notif_row.set_subtitle(S::NOTIFY_SUBTITLE);
    notif_row.set_active(cfg.notifications_enabled);
    notif_group.add(&notif_row);

    let bg_row = adw::SwitchRow::new();
    bg_row.set_title(S::CLOSE_TO_BG_TITLE);
    bg_row.set_subtitle(S::CLOSE_TO_BG_SUBTITLE);
    bg_row.set_active(cfg.close_to_background);
    notif_group.add(&bg_row);

    let save_btn = gtk4::Button::with_label(S::SAVE);
    save_btn.add_css_class("suggested-action");
    save_btn.set_margin_top(8);
    save_btn.set_margin_bottom(16);
    save_btn.set_margin_start(16);
    save_btn.set_margin_end(16);

    let state_clone = state.clone();
    let dialog_clone = dialog.clone();
    let steam_clone = steam.clone();
    save_btn.connect_clicked(move |_| {
        let mut s = state_clone.borrow_mut();
        s.cfg.steam_api_key = steam_entry.text().to_string();
        s.cfg.steam_griddb_api_key = sgdb_entry.text().to_string();
        s.cfg.notifications_enabled = notif_row.is_active();
        s.cfg.close_to_background = bg_row.is_active();

        steam_clone.update_keys(&s.cfg.steam_api_key, &s.cfg.steam_griddb_api_key);

        let cfg = s.cfg.clone();
        drop(s);

        if let Err(e) = cfg.save() {
            eprintln!("Failed to save config: {}", e);
        }
        dialog_clone.destroy();
    });

    box_.append(&group);
    box_.append(&notif_group);
    box_.append(&save_btn);
    toolbar_view.set_content(Some(&box_));
    dialog.set_content(Some(&toolbar_view));
    dialog.present();
}

fn show_game_context_menu(state: &SharedState, game: &Game, row: &gtk4::ListBoxRow) {
    // Read the current hidden state from state — the `game` arg is captured when
    // the sidebar row was built and may be stale after a hide/unhide.
    let current_hidden = state
        .borrow()
        .games
        .iter()
        .find(|g| g.lutris_id == game.lutris_id)
        .map(|g| g.hidden)
        .unwrap_or(game.hidden);

    let menu = gio::Menu::new();
    menu.append(Some(S::EDIT_GAME_SETTINGS), Some("game.edit"));
    // Open folders submenu
    let folders_menu = gio::Menu::new();
    if game.kind == "steam" || game.kind == "sgdb" {
        let subdir = if game.kind == "sgdb" { "steamgriddb" } else { "steam" };
        folders_menu.append(Some("Image data"), Some("game.open_images"));
    }
    if game.kind == "steam" {
        folders_menu.append(Some("Achievement status"), Some("game.open_steam_status"));
    } else if game.kind == "gog" {
        folders_menu.append(Some("Achievement status"), Some("game.open_gog_status"));
    }
    if folders_menu.n_items() > 0 {
        menu.append_submenu(Some("Open folder"), &folders_menu);
    }
    menu.append(Some(if current_hidden { S::UNHIDE_GAME } else { S::HIDE_GAME }), Some("game.hide"));
    menu.append(Some(S::REMOVE_GAME), Some("game.remove"));
    let popover = gtk4::PopoverMenu::from_model(Some(&menu));
    popover.set_halign(gtk4::Align::Start);

    let state_clone = state.clone();
    let game_clone = game.clone();
    let actions = gio::SimpleActionGroup::new();

    let edit_action = gio::SimpleAction::new("edit", None);
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    edit_action.connect_activate(move |_, _| {
        show_game_settings_dialog(&sc, &gc);
    });
    actions.add_action(&edit_action);

    let hide_action = gio::SimpleAction::new("hide", None);
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    let row_clone = row.clone();
    hide_action.connect_activate(move |_, _| {
        let new_hidden = !current_hidden;
        // Look up current db_id from state (the captured gc may be stale after enrichment)
        let lutris_id = gc.lutris_id;
        {
            let s = sc.borrow();
            if let Some(g) = s.games.iter().find(|g| g.lutris_id == lutris_id) {
                if g.db_id != 0 {
                    if let Err(e) = crate::db::set_game_hidden(&s.db, g.db_id, new_hidden) {
                        eprintln!("Failed to set hidden: {}", e);
                    }
                } else if lutris_id != 0 {
                    // Unmatched game — persist via lutris_meta table
                    if let Err(e) = crate::db::set_lutris_hidden(&s.db, lutris_id, new_hidden) {
                        eprintln!("Failed to set lutris hidden: {}", e);
                    }
                }
            }
        }
        if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
            g.hidden = new_hidden;
        }
        // Save scroll position — set_visible on a row can cause the ListBox to relayout
        let scroll = sc.borrow().sidebar_scroll.clone();
        let saved_scroll = scroll.vadjustment().value();
        if new_hidden {
            row_clone.add_css_class("hidden-game");
        } else {
            row_clone.remove_css_class("hidden-game");
        }
        let show_hidden = sc.borrow().cfg.show_hidden_games;
        row_clone.set_visible(!new_hidden || show_hidden);
        // Restore scroll position after visibility change
        let adj = scroll.vadjustment();
        let max = (adj.upper() - adj.page_size()).max(0.0);
        adj.set_value(saved_scroll.min(max));
    });
    actions.add_action(&hide_action);

    let remove_action = gio::SimpleAction::new("remove", None);
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    remove_action.connect_activate(move |_, _| {
        let window = sc.borrow().window.clone();
        let sc2 = sc.clone();
        let app_id = gc.app_id.clone();
        let db_id = gc.db_id;
        confirm_dialog(
            &window,
            S::REMOVE_GAME_QUESTION,
            &format!("Remove \u{201C}{}\u{201D} from the database? This won't delete any save files.", gc.name),
            S::REMOVE_GAME,
            adw::ResponseAppearance::Destructive,
            move || {
                if let Err(e) = crate::db::remove_game(&sc2.borrow().db, db_id) {
                    eprintln!("Failed to remove game from DB: {}", e);
                }
                let _ = sc2.borrow().sender.send(AppMessage::GameRemoved { app_id: app_id.clone() });
            },
        );
    });
    actions.add_action(&remove_action);

    // Open image data folder
    if game_clone.kind == "steam" || game_clone.kind == "sgdb" {
        let open_images = gio::SimpleAction::new("open_images", None);
        let gc = game_clone.clone();
        open_images.connect_activate(move |_, _| {
            let subdir = if gc.kind == "sgdb" { "steamgriddb" } else { "steam" };
            let path = format!("{}/data/{}/{}", SAVE_DIR, subdir, gc.app_id);
            open_folder(&path);
        });
        actions.add_action(&open_images);
    }

    // Open Steam achievement status folder
    if game_clone.kind == "steam" {
        let open_status = gio::SimpleAction::new("open_steam_status", None);
        let gc = game_clone.clone();
        open_status.connect_activate(move |_, _| {
            let path = format!("{}/steam/{}", SAVE_DIR, gc.app_id);
            open_folder(&path);
        });
        actions.add_action(&open_status);
    }

    // Open GOG achievement status folder
    if game_clone.kind == "gog" {
        let open_gog = gio::SimpleAction::new("open_gog_status", None);
        let gc = game_clone.clone();
        open_gog.connect_activate(move |_, _| {
            let path = format!("{}/gog/{}/{}", SAVE_DIR, crate::parser::GALAXY_ID, gc.platform_id);
            open_folder(&path);
        });
        actions.add_action(&open_gog);
    }

    row.insert_action_group("game", Some(&actions));

    popover.set_parent(row);
    popover.popup();
}

fn show_game_settings_dialog(state: &SharedState, game: &Game) {
    let parent = state.borrow().window.clone();
    let win = adw::Window::new();
    win.set_default_width(500);
    win.set_default_height(500);
    win.set_transient_for(Some(&parent));
    win.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(&game.name))));
    outer.append(&header_bar);

    let notebook = gtk4::Notebook::new();

    // --- Settings tab ---
    let settings_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    settings_page.set_margin_start(16);
    settings_page.set_margin_end(16);
    settings_page.set_margin_top(16);
    settings_page.set_margin_bottom(16);

    let title_entry = gtk4::Entry::new();
    title_entry.set_placeholder_text(Some(S::GAME_TITLE));
    title_entry.set_text(&game.name);
    let title_label = gtk4::Label::new(Some(S::TITLE));
    title_label.set_halign(gtk4::Align::Start);
    settings_page.append(&title_label);
    settings_page.append(&title_entry);

    // Unmatch button (only for matched games)
    if game.lutris_id != 0 && !game.app_id.is_empty() {
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        settings_page.append(&sep);

        let unmatch_btn = gtk4::Button::with_label("Unmatch game");
        unmatch_btn.add_css_class("destructive-action");
        unmatch_btn.set_halign(gtk4::Align::Start);
        let sc = state.clone();
        let gc = game.clone();
        let win_clone = win.clone();
        unmatch_btn.connect_clicked(move |_| {
            let parent = sc.borrow().window.clone();
            let sc2 = sc.clone();
            let lutris_id = gc.lutris_id;
            let name = gc.name.clone();
            let win2 = win_clone.clone();
            confirm_dialog(
                &parent,
                "Unmatch game?",
                &format!("Unmatch \u{201C}{}\u{201D} from its trophy source? The game will remain in the list but without trophies.", name),
                "Unmatch",
                adw::ResponseAppearance::Destructive,
                move || {
                    if let Err(e) = crate::db::unmatch_game(&sc2.borrow().db, lutris_id) {
                        eprintln!("Failed to unmatch game: {}", e);
                    }
                    if let Some(g) = sc2.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                        g.app_id.clear();
                        g.kind.clear();
                        g.platform_id.clear();
                        g.achievements.clear();
                        g.earned_count = 0;
                        g.total_count = 0;
                        g.icon_path.clear();
                        g.hero_image_path.clear();
                        g.grid_path.clear();
                        g.header_path.clear();
                        g.logo_path.clear();
                        g.manual_unmatch = true;
                        if !g.lutris_name.is_empty() {
                            g.name = g.lutris_name.clone();
                        }
                    }
                    rebuild_sidebar(&sc2);
                    win2.close();
                },
            );
        });
        settings_page.append(&unmatch_btn);
    }

    let settings_label = gtk4::Label::new(Some("Settings"));
    notebook.append_page(&settings_page, Some(&settings_label));

    // --- Logo tab (only if game has a logo) ---
    let logo_positions = ["bottom-left", "bottom-center", "bottom-right", "center-left", "center", "center-right", "top-left", "top-center", "top-right"];
    let logo_controls = if !game.logo_path.is_empty() {
        let logo_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        logo_page.set_margin_start(16);
        logo_page.set_margin_end(16);
        logo_page.set_margin_top(16);
        logo_page.set_margin_bottom(16);

        let selected_pos: Rc<RefCell<String>> = Rc::new(RefCell::new(game.logo_position.clone()));

        let pos_grid = gtk4::Grid::new();
        pos_grid.set_column_spacing(3);
        pos_grid.set_row_spacing(3);
        pos_grid.set_halign(gtk4::Align::Center);

        let mut all_btns: Vec<gtk4::Button> = Vec::new();
        for (i, &pos) in logo_positions.iter().enumerate() {
            let btn = gtk4::Button::new();
            btn.set_size_request(32, 32);
            btn.add_css_class("logo-pos-btn");
            if pos == game.logo_position {
                btn.add_css_class("logo-pos-selected");
            }
            let row = i / 3;
            let col = i % 3;
            pos_grid.attach(&btn, col as i32, row as i32, 1, 1);
            all_btns.push(btn);
        }

        let btns: Rc<Vec<gtk4::Button>> = Rc::new(all_btns);
        for (i, &pos) in logo_positions.iter().enumerate() {
            let btns_c = btns.clone();
            let selected_pos_c = selected_pos.clone();
            let pos_owned = pos.to_string();
            btns[i].connect_clicked(move |btn| {
                for b in btns_c.iter() {
                    b.remove_css_class("logo-pos-selected");
                }
                btn.add_css_class("logo-pos-selected");
                *selected_pos_c.borrow_mut() = pos_owned.clone();
            });
        }

        let pos_row = adw::ActionRow::new();
        pos_row.set_title("Position");
        pos_row.add_suffix(&pos_grid);
        logo_page.append(&pos_row);

        let size_pct = game.logo_size.clamp(5, 100);
        let size_adj = gtk4::Adjustment::new(size_pct as f64, 5.0, 100.0, 5.0, 10.0, 0.0);
        let size_spin = gtk4::SpinButton::new(Some(&size_adj), 1.0, 0);
        size_spin.set_numeric(true);
        let size_row = adw::ActionRow::new();
        size_row.set_title("Size (% of hero)");
        size_row.add_suffix(&size_spin);
        logo_page.append(&size_row);

        let logo_label = gtk4::Label::new(Some("Logo"));
        notebook.append_page(&logo_page, Some(&logo_label));
        Some((selected_pos, size_adj))
    } else {
        None
    };

    // --- Images tab (only for matched games) ---
    if !game.app_id.is_empty() {
        let images_page = build_image_manager_content(state, game, &win);
        let images_label = gtk4::Label::new(Some("Images"));
        notebook.append_page(&images_page, Some(&images_label));
    }

    outer.append(&notebook);

    // Save / Close buttons
    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(16);
    btn_row.set_margin_end(16);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(12);

    let cancel_btn = gtk4::Button::with_label(S::CANCEL);
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());

    let save_btn = gtk4::Button::with_label(S::SAVE);
    save_btn.add_css_class("suggested-action");
    let state_clone = state.clone();
    let db_id = game.db_id;
    let lutris_id = game.lutris_id;
    let app_id = game.app_id.clone();
    let title_entry_c = title_entry.clone();
    let logo_controls_c = logo_controls.clone();
    let win_s = win.clone();
    save_btn.connect_clicked(move |_| {
        let title = title_entry_c.text().to_string();
        if let Err(e) = crate::db::update_game_title(&state_clone.borrow().db, db_id, &title) {
            eprintln!("Failed to update game: {}", e);
        }

        // Save logo settings if the game has a logo
        if let Some((ref selected_pos, ref size_adj)) = logo_controls_c {
            let pos = selected_pos.borrow().clone();
            let size = size_adj.value() as i32;
            if db_id != 0 {
                if let Err(e) = crate::db::set_logo_settings(&state_clone.borrow().db, db_id, &pos, size) {
                    eprintln!("Failed to update logo settings: {}", e);
                }
            }
            // Update in-memory game
            if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                g.logo_position = pos;
                g.logo_size = size;
            }
        }

        // Update title in game
        if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
            g.name = title.clone();
        }
        if !app_id.is_empty() {
            state_clone.borrow().game_names.lock().unwrap().insert(app_id.clone(), title);
        }
        rebuild_sidebar(&state_clone);

        // Re-display the current game to apply logo changes live
        let selected = state_clone.borrow().selected_id.clone();
        if selected == lutris_id.to_string() {
            if let Some(g) = state_clone.borrow().games.iter().find(|g| g.lutris_id == lutris_id).cloned() {
                display_game(&g, &state_clone);
            }
        }

        win_s.close();
    });

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    outer.append(&btn_row);

    win.set_content(Some(&outer));
    win.present();
}

fn build_image_manager_content(state: &SharedState, game: &Game, parent_win: &adw::Window) -> gtk4::Box {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);

    let is_steam = game.kind == "steam" || game.kind == "gog";
    let id = game.app_id.clone();
    let data_subdir = if game.kind == "sgdb" { "steamgriddb" } else { "steam" };

    for (label, file, asset, thumb_w, thumb_h) in [
        ("Icon", "icon.png", "icon", 48, 48),
        ("Hero", "library_hero.jpg", "hero", 96, 32),
        ("Grid", "library_600x900.jpg", "grid", 32, 48),
        ("Header", "header.jpg", "header", 96, 45),
        ("Logo", "logo.png", "logo", 96, 32),
    ] {
        let section = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        let lbl = gtk4::Label::new(Some(label));
        lbl.set_halign(gtk4::Align::Start);
        lbl.add_css_class("heading");
        section.append(&lbl);

        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_hexpand(true);

        let img_path = format!("{}/data/{}/{}/{}", SAVE_DIR, data_subdir, id, file);
        let preview = gtk4::Picture::for_filename(&img_path);
        preview.set_content_fit(gtk4::ContentFit::ScaleDown);
        preview.set_size_request(thumb_w, thumb_h);
        let preview_wrapper = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        preview_wrapper.set_size_request(thumb_w, thumb_h);
        if std::path::Path::new(&img_path).exists() {
            preview_wrapper.append(&preview);
        } else {
            let ph = gtk4::Label::new(Some("—"));
            ph.add_css_class("dim-label");
            preview_wrapper.append(&ph);
        }
        row.append(&preview_wrapper);

        let btns = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        btns.set_hexpand(true);
        btns.set_halign(gtk4::Align::End);

        let refresh_images: std::rc::Rc<dyn Fn()> = std::rc::Rc::new({
            let preview_wrapper = preview_wrapper.clone();
            let img_path = img_path.clone();
            let state_clone = state.clone();
            let game_clone = game.clone();
            move || {
                while let Some(child) = preview_wrapper.first_child() {
                    preview_wrapper.remove(&child);
                }
                if std::path::Path::new(&img_path).exists() {
                    let p = gtk4::Picture::for_filename(&img_path);
                    p.set_content_fit(gtk4::ContentFit::ScaleDown);
                    preview_wrapper.append(&p);
                } else {
                    let ph = gtk4::Label::new(Some("—"));
                    ph.add_css_class("dim-label");
                    preview_wrapper.append(&ph);
                }
                let s = state_clone.borrow();
                if let Ok(Some(entry)) = crate::db::find_by_lutris_id(&s.db, game_clone.lutris_id) {
                    drop(s);
                    if let Ok(updated) = crate::parser::load_game(&entry, SAVE_DIR) {
                        apply_game_update(&state_clone, updated);
                    }
                }
            }
        });

        // Browse
        let browse_btn = gtk4::Button::with_label("Browse…");
        let dest_path = img_path.clone();
        let state_browse = state.clone();
        let refresh = refresh_images.clone();
        browse_btn.connect_clicked(move |_| {
            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Images"));
            filter.add_mime_type("image/png");
            filter.add_mime_type("image/jpeg");
            filter.add_mime_type("image/webp");
            filter.add_mime_type("image/x-icon");
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Select image");
            dialog.set_default_filter(Some(&filter));
            let dest = dest_path.clone();
            let refresh_c = refresh.clone();
            dialog.open(Some(&state_browse.borrow().window), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let is_ico = path.extension().and_then(|e| e.to_str()) == Some("ico");
                        if is_ico {
                            let ico_dest = std::path::Path::new(&dest).with_extension("ico");
                            if std::fs::copy(&path, &ico_dest).is_ok() {
                                if let Ok(png) = crate::parser::convert_ico_to_png(&ico_dest) {
                                    let _ = std::fs::remove_file(&ico_dest);
                                }
                            }
                        } else if let Err(e) = std::fs::copy(&path, &dest) {
                            eprintln!("Failed to copy image: {}", e);
                        }
                        refresh_c();
                    }
                }
            });
        });
        btns.append(&browse_btn);

        // Steam
        if is_steam && asset != "icon" {
            let btn = gtk4::Button::with_label("Steam");
            let steam = state.borrow().steam.clone();
            let id_c = id.clone();
            let asset_c = asset.to_string();
            let refresh = refresh_images.clone();
            btn.connect_clicked(move |_| {
                let _ = steam.force_download_steam(&id_c, &asset_c);
                refresh();
            });
            btns.append(&btn);
        }

        // SGDB
        let btn = gtk4::Button::with_label("SGDB…");
        let steam = state.borrow().steam.clone();
        let id_c = id.clone();
        let asset_c = asset.to_string();
        let is_steam_c = is_steam;
        let parent = parent_win.clone();
        let refresh = refresh_images.clone();
        btn.connect_clicked(move |_| {
            show_sgdb_picker(&steam, &id_c, &asset_c, is_steam_c, &parent, refresh.clone());
        });
        btns.append(&btn);

        row.append(&btns);
        section.append(&row);
        content.append(&section);
    }

    let note = gtk4::Label::new(Some("Re-select the game after changing images to see updates."));
    note.add_css_class("dim-label");
    note.add_css_class("caption");
    content.append(&note);

    content
}

fn show_sgdb_picker(steam: &Arc<crate::steam::SteamClient>, id: &str, asset: &str, is_steam_id: bool, parent: &adw::Window, on_done: std::rc::Rc<dyn Fn()>) {
    let picker = adw::Window::new();
    picker.set_default_width(500);
    picker.set_default_height(450);
    picker.set_transient_for(Some(parent));
    picker.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(&format!("Pick {}", asset)))));
    outer.append(&header_bar);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_top(8);
    list.set_margin_bottom(8);

    let loading = gtk4::Label::new(Some("Loading…"));
    loading.add_css_class("dim-label");
    list.append(&loading);

    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win = picker.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    picker.set_content(Some(&outer));
    picker.present();

    // Fetch SGDB assets in background
    let (tx, rx) = std::sync::mpsc::channel::<Vec<crate::steam::SgdbAsset>>();
    let rx = std::cell::RefCell::new(rx);
    let steam_c = steam.clone();
    let id_c = id.to_string();
    let asset_c = asset.to_string();
    std::thread::spawn(move || {
        let results = steam_c.list_sgdb_assets(&id_c, &asset_c, is_steam_id);
        let _ = tx.send(results);
    });

    let list_clone = list.clone();
    let steam_clone = steam.clone();
    let id_clone = id.to_string();
    let asset_clone = asset.to_string();
    let picker_clone = picker.clone();
    let on_done = on_done.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok(assets) = rx.borrow_mut().try_recv() {
            while let Some(child) = list_clone.first_child() {
                list_clone.remove(&child);
            }

            if assets.is_empty() {
                let none = gtk4::Label::new(Some("No images found on SteamGridDB"));
                none.add_css_class("dim-label");
                list_clone.append(&none);
                return glib::ControlFlow::Break;
            }

            for a in assets {
                let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                row.set_margin_top(4);
                row.set_margin_bottom(4);

                // Thumbnail — download to temp and show
                let thumb_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
                let thumb_pic = gtk4::Picture::new();
                thumb_pic.set_content_fit(gtk4::ContentFit::ScaleDown);
                thumb_pic.set_size_request(48, 48);
                thumb_box.append(&thumb_pic);
                row.append(&thumb_box);

                // Download thumbnail in background
                let url_clone = a.url.clone();
                let thumb_pic_clone = thumb_pic.clone();
                let steam_thumb = steam_clone.clone();
                let thumb_dir = format!("{}/data/.thumbnails", SAVE_DIR);
                let _ = std::fs::create_dir_all(&thumb_dir);
                let thumb_name = format!("{}/{}", thumb_dir, url_clone.rsplit('/').next().unwrap_or("thumb"));
                let (tx_thumb, rx_thumb) = std::sync::mpsc::channel::<Option<String>>();
                let rx_thumb = std::cell::RefCell::new(rx_thumb);
                std::thread::spawn(move || {
                    let final_path = if std::path::Path::new(&thumb_name).exists() {
                        Some(thumb_name.clone())
                    } else if steam_thumb.download_file(&url_clone, std::path::Path::new(&thumb_name)).is_ok() {
                        // Convert ICO to PNG for display
                        if std::path::Path::new(&thumb_name).extension().and_then(|e| e.to_str()) == Some("ico") {
                            if let Ok(img) = image::open(&thumb_name) {
                                let png_path = std::path::Path::new(&thumb_name).with_extension("png");
                                if img.save(&png_path).is_ok() {
                                    let _ = std::fs::remove_file(&thumb_name);
                                    Some(png_path.to_string_lossy().into_owned())
                                } else {
                                    Some(thumb_name.clone())
                                }
                            } else {
                                Some(thumb_name.clone())
                            }
                        } else {
                            Some(thumb_name.clone())
                        }
                    } else {
                        None
                    };
                    let _ = tx_thumb.send(final_path);
                });
                let tp = thumb_pic_clone.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    if let Ok(path) = rx_thumb.borrow_mut().try_recv() {
                        if let Some(p) = path {
                            tp.set_filename(Some(&p));
                        }
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                });

                // Info label
                let mut info = if a.width > 0 && a.height > 0 {
                    format!("{}×{}", a.width, a.height)
                } else {
                    "ICO".to_string()
                };
                if !a.style.is_empty() {
                    info = format!("{} · {}", info, a.style);
                }
                if !a.author.is_empty() {
                    info = format!("{} · by {}", info, a.author);
                }
                let label = gtk4::Label::new(Some(&info));
                label.set_xalign(0.0);
                label.set_hexpand(true);
                label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                row.append(&label);

                // Download button
                let dl_btn = gtk4::Button::with_label("Download");
                dl_btn.add_css_class("suggested-action");
                let url = a.url.clone();
                let data_subdir = if is_steam_id { "steam" } else { "steamgriddb" };
                let dest_dir = format!("{}/data/{}/{}", SAVE_DIR, data_subdir, id_clone);
                let file_name = match asset_clone.as_str() {
                    "icon" => {
                        // Use mime to determine extension, fallback to URL
                        let ext = if a.mime.contains("icon") || a.mime.contains("x-icon") {
                            "ico"
                        } else if a.mime.contains("png") {
                            "png"
                        } else if a.mime.contains("jpeg") || a.mime.contains("jpg") {
                            "jpg"
                        } else if a.mime.contains("webp") {
                            "webp"
                        } else {
                            std::path::Path::new(&url).extension().and_then(|e| e.to_str()).unwrap_or("png")
                        };
                        format!("icon.{}", ext)
                    }
                    "hero" => "library_hero.jpg".to_string(),
                    "grid" => "library_600x900.jpg".to_string(),
                    "logo" => "logo.png".to_string(),
                    _ => continue,
                };
                let dest = format!("{}/{}", dest_dir, file_name);
                let steam_c = steam_clone.clone();
                let picker_c = picker_clone.clone();
                let on_done_c = on_done.clone();
                dl_btn.connect_clicked(move |_| {
                    let _ = std::fs::create_dir_all(&dest_dir);
                    // Remove old icon files first
                    for old_ext in ["png", "ico", "jpg", "webp"] {
                        let old = format!("{}/icon.{}", dest_dir, old_ext);
                        let _ = std::fs::remove_file(&old);
                    }
                    if steam_c.download_file(&url, std::path::Path::new(&dest)).is_ok() {
                        if file_name.ends_with(".ico") {
                            match crate::parser::convert_ico_to_png(std::path::Path::new(&dest)) {
                                Ok(png_path) => eprintln!("Converted ICO to {}", png_path.display()),
                                Err(e) => {
                                    eprintln!("ICO conversion failed: {}, trying direct load", e);
                                    // If conversion fails, try renaming to .png
                                    // (some "ICO" files are actually PNG)
                                    let png_dest = std::path::Path::new(&dest).with_extension("png");
                                    let _ = std::fs::rename(&dest, &png_dest);
                                }
                            }
                        }
                        on_done_c();
                        picker_c.close();
                    } else {
                        eprintln!("Download failed for {}", url);
                    }
                });
                row.append(&dl_btn);

                list_clone.append(&row);
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn show_add_game_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();
    let dialog = gtk4::FileDialog::new();
    dialog.set_title(S::SELECT_GAME_FOLDER);

    let state_clone = state.clone();
    dialog.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let folder = path.to_string_lossy().into_owned();

        if let Some(app_id) = crate::gamesetup::detect_app_id(&folder) {
            finish_add_game(&state_clone, &folder, &app_id);
        } else if crate::gamesetup::is_gog_game(&folder) {
            if let Some((_info_dir, product_id, game_name)) = crate::gamesetup::find_gog_info(&folder) {
                prompt_for_steam_id_gog(&state_clone, &folder, &product_id, &game_name);
            } else {
                prompt_for_app_id(&state_clone, &folder);
            }
        } else {
            prompt_for_app_id(&state_clone, &folder);
        }
    });
}

fn prompt_for_steam_id(state: &SharedState, title: &str, body: &str, on_add: impl Fn(&str) + 'static) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(Some(title), Some(body));

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("e.g. 1687950"));
    entry.set_input_purpose(gtk4::InputPurpose::Digits);
    entry.set_margin_top(8);
    entry.set_margin_bottom(8);
    entry.set_margin_start(8);
    entry.set_margin_end(8);
    dialog.set_extra_child(Some(&entry));

    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("add", S::ADD_GAME_BTN);
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response != "add" {
            return;
        }
        on_add(&entry.text());
    });
    dialog.present(Some(&window));
}

fn prompt_for_app_id(state: &SharedState, folder: &str) {
    let folder = folder.to_string();
    let state_clone = state.clone();
    prompt_for_steam_id(state, S::ENTER_STEAM_ID, S::ENTER_STEAM_ID_BODY, move |app_id| {
        finish_add_game(&state_clone, &folder, app_id);
    });
}

fn prompt_for_steam_id_gog(state: &SharedState, galaxy_folder: &str, product_id: &str, game_name: &str) {
    let galaxy_folder = galaxy_folder.to_string();
    let product_id = product_id.to_string();
    let game_name = game_name.to_string();
    let state_clone = state.clone();
    let body = format!(
        "{}: {}\n{}: {}\n\n{}",
        S::DETECTED_GOG_GAME, game_name,
        S::GOG_PRODUCT_ID, product_id,
        S::ENTER_STEAM_ID_GOG
    );
    prompt_for_steam_id(state, S::ADD_GOG_GAME, &body, move |steam_app_id| {
        finish_add_gog_game(&state_clone, &galaxy_folder, &product_id, &game_name, steam_app_id);
    });
}

/// Shared tail of the add-game flows: look the freshly-added game up in the DB,
/// load it, register it with the watcher, announce it, and kick off enrichment.
fn finalize_added_game(
    app_id: &str,
    kind: &str,
    platform_id: &str,
    steam: Arc<SteamClient>,
    watcher: Option<AchievementWatcher>,
    sender: Sender<AppMessage>,
    db: DbConn,
) {
    let entry = match crate::db::find_by_steam_id(&db, app_id) {
        Ok(Some(e)) => e,
        _ => {
            eprintln!("Failed to find game in DB after adding: {}", app_id);
            return;
        }
    };
    match load_game(&entry, SAVE_DIR) {
        Ok(game) => {
            if let Some(ref watcher) = watcher {
                watcher.watch(&entry, &game.achievements);
            }
            let name = game.name.clone();
            let _ = sender.send(AppMessage::NewGame(game));
            enrich_game_async(
                app_id.to_string(),
                kind.to_string(),
                platform_id.to_string(),
                entry.id,
                0,
                name,
                steam,
                watcher,
                sender,
            );
        }
        Err(e) => eprintln!("Failed to load newly added game: {}", e),
    }
}

fn finish_add_game(state: &SharedState, folder: &str, app_id: &str) {
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    let folder = folder.to_string();
    let app_id = app_id.to_string();
    std::thread::spawn(move || {
        match crate::gamesetup::add_game_from_folder(&folder, &app_id, &steam, &db, SAVE_DIR) {
            Ok(_) => finalize_added_game(&app_id, "steam", &app_id, steam, watcher, sender, db),
            Err(e) => {
                eprintln!("Add game failed: {}", e);
                let _ = sender.send(AppMessage::AddGameError(e));
            }
        }
    });
}

fn finish_add_gog_game(state: &SharedState, galaxy_folder: &str, product_id: &str, game_name: &str, steam_app_id: &str) {
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    let galaxy_folder = galaxy_folder.to_string();
    let product_id = product_id.to_string();
    let game_name = game_name.to_string();
    let steam_app_id = steam_app_id.to_string();

    std::thread::spawn(move || {
        match crate::gamesetup::add_gog_game_from_folder(
            &galaxy_folder, &product_id, &game_name, &steam_app_id, &steam, &db, SAVE_DIR,
        ) {
            Ok(_) => finalize_added_game(&steam_app_id, "gog", &product_id, steam, watcher, sender, db),
            Err(e) => {
                eprintln!("GOG add game failed: {}", e);
                let _ = sender.send(AppMessage::AddGameError(e));
            }
        }
    });
}

/// Link an unmatched Lutris game to a Steam achievement source: record the
/// matching in the DB, generate achievement definitions, then announce + enrich.
fn match_game_to_steam(state: &SharedState, lutris_id: i64, steam_app_id: String, lutris_name: String) {
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::db::upsert_matching(&db, lutris_id, &steam_app_id, "steam", &steam_app_id) {
            eprintln!("match_game_to_steam: upsert_matching failed: {}", e);
            return;
        }
        if let Err(e) = steam.generate_steam_settings(&steam_app_id) {
            eprintln!("match_game_to_steam: generate_steam_settings failed: {}", e);
        }
        match crate::db::find_by_lutris_id(&db, lutris_id) {
            Ok(Some(entry)) => {
                match crate::parser::load_game(&entry, SAVE_DIR) {
                    Ok(mut game) => {
                        if game.name.is_empty() || game.name.starts_with("App ID:") {
                            game.name = lutris_name.clone();
                        }
                        game.lutris_id = lutris_id;
                        let name = game.name.clone();
                        if let Some(ref watcher) = watcher {
                            watcher.watch(&entry, &game.achievements);
                        }
                        let _ = sender.send(AppMessage::NewGame(game));
                        enrich_game_async(
                            steam_app_id.clone(),
                            "steam".to_string(),
                            steam_app_id.clone(),
                            entry.id,
                            lutris_id,
                            name,
                            steam,
                            watcher,
                            sender,
                        );
                    }
                    Err(e) => eprintln!("match_game_to_steam: load_game failed: {}", e),
                }
            }
            Ok(None) => eprintln!("match_game_to_steam: find_by_lutris_id returned None for lutris_id={}", lutris_id),
            Err(e) => eprintln!("match_game_to_steam: find_by_lutris_id error: {}", e),
        }
    });
}

/// Link an unmatched Lutris game to SteamGridDB (for images only, no achievements).
fn match_game_to_sgdb(state: &SharedState, lutris_id: i64, sgdb_id: String, lutris_name: String) {
    let steam = state.borrow().steam.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::db::upsert_matching(&db, lutris_id, &sgdb_id, "sgdb", &sgdb_id) {
            eprintln!("match_game_to_sgdb: upsert_matching failed: {}", e);
            return;
        }
        // Download images from SGDB
        let (icon, hero, grid, logo) = steam.ensure_sgdb_assets(&sgdb_id);

        // Build a game entry and send it to the UI
        if let Ok(Some(entry)) = crate::db::find_by_lutris_id(&db, lutris_id) {
            let mut game = crate::parser::Game {
                app_id: sgdb_id.clone(),
                kind: "sgdb".to_string(),
                platform_id: sgdb_id.clone(),
                db_id: entry.id,
                name: if entry.title.is_empty() { lutris_name.clone() } else { entry.title.clone() },
                icon_path: icon,
                hero_image_path: hero,
                grid_path: grid,
                header_path: String::new(),
                logo_path: logo,
                achievements: Vec::new(),
                earned_count: 0,
                total_count: 0,
                hidden: entry.hidden,
                lutris_id,
                slug: String::new(),
                playtime: 0.0,
                lastplayed: 0,
                logo_position: entry.logo_position.clone(),
                logo_size: entry.logo_size,
                lutris_name: lutris_name.clone(),
                manual_unmatch: false,
            };
            let _ = sender.send(AppMessage::NewGame(game));
        }
    });
}

fn confirm_mark_unlocked(state: &SharedState, kind: &str, app_id: &str, platform_id: &str, ach: &MergedAchievement, reload: impl Fn() + 'static) {
    let window = state.borrow().window.clone();
    let ach_name = ach.name.clone();
    let kind = kind.to_string();
    let app_id = app_id.to_string();
    let platform_id = platform_id.to_string();
    confirm_dialog(
        &window,
        S::MARK_UNLOCKED,
        &format!(
            "This will mark \u{201C}{}\u{201D} as earned without a real unlock time. \
             Use this only if you already unlocked it previously (e.g. before using this tool).",
            ach.display_name
        ),
        S::MARK_AS_UNLOCKED,
        adw::ResponseAppearance::Destructive,
        move || {
            if let Err(e) = set_achievement_earned(SAVE_DIR, &kind, &app_id, &platform_id, &ach_name, true) {
                eprintln!("Failed to mark achievement as unlocked: {}", e);
                return;
            }
            reload();
        },
    );
}

pub fn enrich_game_async(
    app_id: String,
    kind: String,
    platform_id: String,
    db_id: i64,
    lutris_id: i64,
    title: String,
    steam: Arc<SteamClient>,
    watcher: Option<AchievementWatcher>,
    sender: Sender<AppMessage>,
) {
    std::thread::spawn(move || {
        let entry = GameEntry {
            id: db_id,
            kind: kind.clone(),
            steam_id: app_id.clone(),
            platform_id: platform_id.clone(),
            title,
            lutris_db_id: if lutris_id != 0 { Some(lutris_id) } else { None },
            sgdb_id: None,
            hidden: false,
            logo_position: String::new(),
            logo_size: 0,
            ignored: Some(0),
            manual_unmatch: Some(0),
        };

        let Ok(mut game) = load_game(&entry, SAVE_DIR) else {
            eprintln!("Failed reloading {}", app_id);
            return;
        };

        if kind != "sgdb" {
            let meta_path = crate::parser::achievements_dir(SAVE_DIR, &app_id).join("achievements.json");
            if !meta_path.exists() {
                if let Err(e) = steam.generate_steam_settings(&app_id) {
                    eprintln!("Could not generate achievements for {}: {}", app_id, e);
                }
            }

            if game.name.starts_with("App ID:") {
                if let Some(name) = steam.fetch_nemirtingas_game_name(&app_id) {
                    game.name = name;
                }
            }

            if let Some(details) = steam.fetch_game_details(&app_id) {
                if game.name.starts_with("App ID:") && !details.name.is_empty() {
                    game.name = details.name.clone();
                }
                let has_local_icon = !game.icon_path.is_empty();
                let (icon_path, hero_path) = steam.ensure_assets(&app_id, has_local_icon);
                if game.icon_path.is_empty() && !icon_path.is_empty() {
                    game.icon_path = icon_path;
                }
                if game.hero_image_path.is_empty() && !hero_path.is_empty() {
                    game.hero_image_path = hero_path;
                }
            }

            // Download grid assets (vertical, header, logo) — only fill what's missing.
            let (grid_path, header_path, logo_path) = steam.ensure_grids(&app_id);
            if game.grid_path.is_empty() && !grid_path.is_empty() {
                game.grid_path = grid_path;
            }
            if game.header_path.is_empty() && !header_path.is_empty() {
                game.header_path = header_path;
            }
            if game.logo_path.is_empty() && !logo_path.is_empty() {
                game.logo_path = logo_path;
            }

            if let Some(pcts) = steam.fetch_global_achievements(&app_id) {
                for a in &mut game.achievements {
                    if let Some(&pct) = pcts.get(&a.name) {
                        a.global_percent = pct;
                    }
                }
            }

            if let Some(ref watcher) = watcher {
                watcher.watch(&entry, &game.achievements);
            }
        }

        let _ = sender.send(AppMessage::EnrichedGame(game));
    });
}

fn normalize_title(s: &str) -> String {
    let lower = s.to_lowercase();
    let alnum: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = alnum.split_whitespace().collect();
    let suffixes = ["the", "final", "cut", "edition", "complete", "definitive", "remastered", "hd"];
    let mut end = words.len();
    while end > 0 && suffixes.contains(&words[end - 1]) {
        end -= 1;
    }
    words[..end].join(" ")
}

fn show_mass_match_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();

    // Collect unmatched games + build a title→steam_id map from data dir
    let (unmatched, title_map) = {
        let s = state.borrow();
        let games = s.games.clone();
        let unmatched: Vec<Game> = games.into_iter().filter(|g| g.app_id.is_empty() && !g.manual_unmatch).collect();
        let data_dir = std::path::Path::new(SAVE_DIR).join("data").join("steam");
        // (normalized_title, app_id, original_name)
        let mut map: Vec<(String, String, String)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&data_dir) {
            for entry in entries.flatten() {
                let app_id = match entry.file_name().to_str() {
                    Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                    _ => continue,
                };
                if let Some(name) = crate::parser::read_app_name(SAVE_DIR, &app_id) {
                    map.push((normalize_title(&name), app_id, name));
                }
            }
        }
        (unmatched, map)
    };

    if unmatched.is_empty() {
        let d = adw::AlertDialog::new(Some("No unmatched games"), Some("Every game already has a trophy source linked."));
        d.add_response("ok", "OK");
        d.present(Some(&window));
        return;
    }

    // Build dialog window
    let dialog = adw::Window::new();
    dialog.set_default_width(600);
    dialog.set_default_height(500);
    dialog.set_transient_for(Some(&window));
    dialog.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some("Match unmatched games"))));
    outer.append(&header_bar);

    let header = gtk4::Label::new(Some(&format!("{} unmatched game(s)", unmatched.len())));
    header.set_margin_top(16);
    header.set_margin_bottom(8);
    header.set_margin_start(16);
    header.set_margin_end(16);
    header.set_xalign(0.0);
    header.add_css_class("heading");
    outer.append(&header);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);

    // Each row: (action_box) — we update the action_box as results come in
    let mut row_action_boxes: Vec<gtk4::Box> = Vec::new();

    for game in &unmatched {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.set_margin_top(6);
        row.set_margin_bottom(6);

        let name_label = gtk4::Label::new(Some(&game.name));
        name_label.set_xalign(0.0);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        row.append(&name_label);

        let action_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let searching = gtk4::Label::new(Some("Searching..."));
        searching.add_css_class("dim-label");
        action_box.append(&searching);
        row.append(&action_box);

        list.append(&row);
        row_action_boxes.push(action_box);
    }

    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_top(8);
    close_btn.set_margin_bottom(12);
    close_btn.set_margin_start(16);
    close_btn.set_margin_end(16);
    let win = dialog.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    dialog.set_content(Some(&outer));
    dialog.present();

    // Channel for background → UI communication
    // (row_index, Option<(steam_id, matched_name)>, game_name, lutris_id)
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Option<(String, String)>, String, i64)>();
    let rx = std::cell::RefCell::new(rx);
    let remaining = std::cell::Cell::new(unmatched.len());

    // Background thread: auto-match each game
    let steam = state.borrow().steam.clone();
    let games_for_thread: Vec<(String, i64)> = unmatched.iter().map(|g| (g.name.clone(), g.lutris_id)).collect();
    std::thread::spawn(move || {
        for (i, (game_name, lutris_id)) in games_for_thread.iter().enumerate() {
            let norm = normalize_title(game_name);

            // 1. Try appdetails match
            let matched: Option<(String, String)> = if norm.is_empty() {
                None
            } else {
                title_map
                    .iter()
                    .find(|(t, _, _)| t == &norm)
                    .map(|(_, id, name)| (id.clone(), name.clone()))
            };

            // 2. If not found, search Steam Store
            let final_match = if matched.is_some() {
                matched
            } else {
                let results = steam.search_steam_store(game_name);
                if results.is_empty() {
                    None
                } else {
                    // Only match if normalized title matches exactly
                    results
                        .iter()
                        .find(|(_, name)| normalize_title(name) == norm)
                        .map(|(id, name)| (id.clone(), name.clone()))
                }
            };

            let _ = tx.send((i, final_match, game_name.clone(), *lutris_id));
        }
    });

    // Poll for results
    let state_rx = state.clone();
    let steam_rx = state.borrow().steam.clone();
    let row_boxes = row_action_boxes.clone();
    let remaining = std::cell::Cell::new(unmatched.len());
    let parent_dialog = dialog.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        // Non-blocking: process at most one result per tick
        if let Ok((row_idx, matched, game_name, lutris_id)) = rx.borrow_mut().try_recv() {
            if row_idx < row_boxes.len() {
                let action_box = &row_boxes[row_idx];
                // Clear action_box
                while let Some(child) = action_box.first_child() {
                    action_box.remove(&child);
                }

                if let Some((sid, matched_name)) = matched {
                    // Auto-match
                    match_game_to_steam(&state_rx, lutris_id, sid.clone(), game_name.clone());

                    let label = gtk4::Label::new(Some(&format!("Matched: {} ({})", matched_name, sid)));
                    label.add_css_class("success-label");
                    action_box.append(&label);

                    // Undo button
                    let undo_btn = gtk4::Button::with_label("Undo");
                    let sc = state_rx.clone();
                    let lid = lutris_id;
                    undo_btn.connect_clicked(move |_| {
                        let _ = crate::db::unmatch_game(&sc.borrow().db, lid);
                        if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lid) {
                            g.app_id.clear();
                            g.kind.clear();
                            g.achievements.clear();
                            g.manual_unmatch = true;
                        }
                        rebuild_sidebar(&sc);
                    });
                    action_box.append(&undo_btn);
                } else {
                    let label = gtk4::Label::new(Some("Not found"));
                    label.add_css_class("dim-label");
                    action_box.append(&label);

                    // Callback: update this row when a match is made
                    let ab = action_box.clone();
                    let on_match: std::rc::Rc<dyn Fn(&str, &str)> = std::rc::Rc::new(move |sid, name| {
                        while let Some(child) = ab.first_child() {
                            ab.remove(&child);
                        }
                        let text = if name.is_empty() {
                            format!("Matched: {}", sid)
                        } else {
                            format!("Matched: {} ({})", name, sid)
                        };
                        let l = gtk4::Label::new(Some(&text));
                        l.add_css_class("success-label");
                        ab.append(&l);
                    });

                    // Enter ID button
                    let id_btn = gtk4::Button::with_label("Enter ID");
                    let sc = state_rx.clone();
                    let name = game_name.clone();
                    let lid = lutris_id;
                    let cb = on_match.clone();
                    id_btn.connect_clicked(move |_| {
                        let sc2 = sc.clone();
                        let name2 = name.clone();
                        let cb2 = cb.clone();
                        let body = format!("Enter the Steam app ID for \u{201C}{}\u{201D}:", name);
                        prompt_for_steam_id(&sc, "Match to Steam", &body, move |app_id| {
                            match_game_to_steam(&sc2, lid, app_id.to_string(), name2.clone());
                            cb2(&app_id, "");
                        });
                    });
                    action_box.append(&id_btn);

                    // Search Steam button
                    let steam_btn = gtk4::Button::with_label("Search Steam");
                    let sc2 = state_rx.clone();
                    let name2 = game_name.clone();
                    let steam2 = steam_rx.clone();
                    let cb2 = on_match.clone();
                    let pd = parent_dialog.clone();
                    steam_btn.connect_clicked(move |_| {
                        show_search_results_dialog(&sc2, steam2.clone(), "Steam", &name2, lutris_id, SearchSource::Steam, cb2.clone(), pd.upcast_ref());
                    });
                    action_box.append(&steam_btn);

                    // Search SGDB button
                    let sgdb_btn = gtk4::Button::with_label("Search SGDB");
                    let sc3 = state_rx.clone();
                    let name3 = game_name.clone();
                    let steam3 = steam_rx.clone();
                    let cb3 = on_match.clone();
                    let pd = parent_dialog.clone();
                    sgdb_btn.connect_clicked(move |_| {
                        show_search_results_dialog(&sc3, steam3.clone(), "SteamGridDB", &name3, lutris_id, SearchSource::SGDB, cb3.clone(), pd.upcast_ref());
                    });
                    action_box.append(&sgdb_btn);
                }

                let left = remaining.get();
                if left <= 1 {
                    return glib::ControlFlow::Break;
                }
                remaining.set(left - 1);
            }
        }
        glib::ControlFlow::Continue
    });
}

#[derive(Clone, Copy)]
enum SearchSource {
    Steam,
    SGDB,
}

fn show_search_results_dialog(
    state: &SharedState,
    steam: Arc<crate::steam::SteamClient>,
    source_name: &str,
    game_name: &str,
    lutris_id: i64,
    source: SearchSource,
    on_match: std::rc::Rc<dyn Fn(&str, &str)>,
    parent: &gtk4::Window,
) {
    let dialog = adw::Window::new();
    dialog.set_default_width(450);
    dialog.set_default_height(400);
    dialog.set_transient_for(Some(parent));
    dialog.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header_bar = adw::HeaderBar::new();
    let title_label = gtk4::Label::new(Some(&format!("Search {}", source_name)));
    header_bar.set_title_widget(Some(&title_label));
    outer.append(&header_bar);

    let entry = gtk4::Entry::new();
    entry.set_text(game_name);
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    entry.set_margin_top(12);
    entry.set_margin_bottom(8);
    outer.append(&entry);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let results_list = gtk4::ListBox::new();
    results_list.set_selection_mode(gtk4::SelectionMode::None);
    results_list.set_margin_start(12);
    results_list.set_margin_end(12);
    results_list.set_margin_bottom(8);

    // Initial "Searching..." placeholder
    let placeholder = gtk4::Label::new(Some("Searching..."));
    placeholder.add_css_class("dim-label");
    results_list.append(&placeholder);

    scrolled.set_child(Some(&results_list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win = dialog.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    dialog.set_content(Some(&outer));

    // Search handler (connected BEFORE emit_activate so auto-search works)
    let entry_clone = entry.clone();
    let results_clone = results_list.clone();
    let state_clone = state.clone();
    let steam_clone = steam.clone();
    let name_clone = game_name.to_string();
    let dialog_clone = dialog.clone();
    let on_match_clone = on_match.clone();
    entry.connect_activate(move |_| {
        let term = entry_clone.text().to_string();
        if term.is_empty() {
            return;
        }

        // Clear results
        while let Some(child) = results_clone.first_child() {
            results_clone.remove(&child);
        }
        let searching = gtk4::Label::new(Some("Searching..."));
        searching.add_css_class("dim-label");
        results_clone.append(&searching);

        // Channel for background → UI
        let (tx, rx) = std::sync::mpsc::channel::<Vec<(String, String)>>();
        let rx = std::cell::RefCell::new(rx);

        // Background thread: only captures Send data
        let steam = steam_clone.clone();
        let src = source;
        std::thread::spawn(move || {
            let search_results = match src {
                SearchSource::Steam => steam.search_steam_store(&term),
                SearchSource::SGDB => steam.search_sgdb(&term),
            };
            let _ = tx.send(search_results);
        });

        // UI thread: poll for results
        let sc = state_clone.clone();
        let results = results_clone.clone();
        let name = name_clone.clone();
        let cb = on_match_clone.clone();
        let dlg = dialog_clone.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if let Ok(search_results) = rx.borrow_mut().try_recv() {
                // Clear results
                while let Some(child) = results.first_child() {
                    results.remove(&child);
                }

                if search_results.is_empty() {
                    let none = gtk4::Label::new(Some("No results found"));
                    none.add_css_class("dim-label");
                    results.append(&none);
                    return glib::ControlFlow::Break;
                }

                for (app_id, result_name) in search_results {
                    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    row.set_margin_top(4);
                    row.set_margin_bottom(4);

                    let label = gtk4::Label::new(Some(&format!("{} ({})", result_name, app_id)));
                    label.set_xalign(0.0);
                    label.set_hexpand(true);
                    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                    row.append(&label);

                    let match_btn = gtk4::Button::with_label("Match");
                    match_btn.add_css_class("suggested-action");
                    let sc2 = sc.clone();
                    let name2 = name.clone();
                    let sid = app_id.clone();
                    let matched_name = result_name.clone();
                    let lid = lutris_id;
                    let dialog_clone = dlg.clone();
                    let callback = cb.clone();
                    let src_type = source;
                    match_btn.connect_clicked(move |_| {
                        match src_type {
                            SearchSource::Steam => match_game_to_steam(&sc2, lid, sid.clone(), name2.clone()),
                            SearchSource::SGDB => match_game_to_sgdb(&sc2, lid, sid.clone(), name2.clone()),
                        }
                        callback(&sid, &matched_name);
                        dialog_clone.close();
                    });
                    row.append(&match_btn);

                    results.append(&row);
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    });

    dialog.present();
    entry.emit_activate();
}
