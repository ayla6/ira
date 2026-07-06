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
.hero-gradient {
    background-image: linear-gradient(to bottom, rgba(0,0,0,0) 0%, rgba(0,0,0,0.85) 100%);
}
.hero-progress trough { background-color: transparent; border: none; border-radius: 0; }
.hero-progress progress { background-color: @accent_color; border: none; border-radius: 0; }
.hero-progress { min-height: 8px; border: none; }
.sidebar-row-title { min-width: 0; }
.global-bar trough { background-color: transparent; border: none; }
.global-bar progress { border: none; border-radius: 0; }
";

extern "C" {
    pub(crate) fn malloc_trim(pad: usize) -> i32;
}

pub struct AppState {
    pub window: adw::ApplicationWindow,
    pub games: Vec<Game>,
    rows: Vec<SidebarRowWidgets>,
    pub game_list: gtk4::ListBox,
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
}

pub type SharedState = Rc<RefCell<AppState>>;

/// Programmatically select a sidebar row without firing the `row-selected` handler.
fn select_row_silently(state: &SharedState, row: Option<&gtk4::ListBoxRow>) {
    state.borrow_mut().restoring = true;
    state.borrow().game_list.select_row(row);
    state.borrow_mut().restoring = false;
}

struct SidebarRowWidgets {
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
    settings_btn.set_halign(gtk4::Align::Fill);
    popover_box.append(&settings_btn);

    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));

    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some(S::ADD_GAME));
    add_btn.add_css_class("flat");
    header_bar.pack_start(&add_btn);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&split_view));
    window.set_content(Some(&toolbar_view));

    {
        let mut s = state.borrow_mut();
        s.window = window.clone();
        s.game_list = game_list.clone();
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
                let app_id = s.games[game_idx].app_id.clone();
                drop(s);
                switch_to_game(&state_clone, &app_id);
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
    let games: Vec<Game> = state.borrow().games.to_vec();
    let mut rows = Vec::with_capacity(games.len());
    for g in &games {
        rows.push(build_sidebar_row(&game_list, g, state));
    }
    state.borrow_mut().rows = rows;
}

fn build_sidebar_row(list: &gtk4::ListBox, game: &Game, state: &SharedState) -> SidebarRowWidgets {
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
    title_label.add_css_class("heading");
    title_label.add_css_class("sidebar-row-title");
    title_label.set_ellipsize(pango::EllipsizeMode::End);
    title_label.set_hexpand(true);
    title_label.set_tooltip_text(Some(&format!("{} ({})", game.name, game.app_id)));
    vbox.append(&title_label);

    let pct = if game.total_count > 0 {
        game.earned_count * 100 / game.total_count
    } else {
        0
    };
    let count_label = gtk4::Label::new(Some(&format!(
        "{}/{} · {}%",
        game.earned_count, game.total_count, pct
    )));
    count_label.set_xalign(0.0);
    count_label.add_css_class("dim-label");
    count_label.add_css_class("caption");
    count_label.set_ellipsize(pango::EllipsizeMode::End);
    vbox.append(&count_label);

    hbox.append(&vbox);
    row.set_child(Some(&hbox));
    list.append(&row);

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
        icon,
        title: title_label,
        subtitle: count_label,
    }
}

fn merge_game_enrichment(existing: &Game, updated: &mut Game) {
    if updated.name.starts_with("App ID:") && !existing.name.starts_with("App ID:") {
        updated.name = existing.name.clone();
    }
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
    }
}

fn apply_game_update(state: &SharedState, mut updated: Game) {
    let app_id = updated.app_id.clone();

    let game_for_display = {
        let mut s = state.borrow_mut();
        let Some(i) = s.games.iter().position(|g| g.app_id == app_id) else {
            return;
        };

        merge_game_enrichment(&s.games[i], &mut updated);

        s.game_names.lock().unwrap().insert(app_id.clone(), updated.name.clone());

        if i < s.rows.len() {
            let rw = &s.rows[i];
            rw.title.set_text(&updated.name);
            rw.title.set_tooltip_text(Some(&format!("{} ({})", updated.name, updated.app_id)));
            let pct = if updated.total_count > 0 {
                updated.earned_count * 100 / updated.total_count
            } else {
                0
            };
            rw.subtitle.set_text(&format!("{}/{} · {}%", updated.earned_count, updated.total_count, pct));
            if !updated.icon_path.is_empty() {
                crate::images::set_image(&rw.icon, &updated.icon_path);
            }
        }

        // Only the currently-displayed game needs a content rebuild.
        let needs_rebuild = s.selected_id == app_id && !s.content_unloaded;
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

    {
        let mut s = state.borrow_mut();
        s.game_names.lock().unwrap().insert(app_id.clone(), game.name.clone());

        if let Some(i) = s.games.iter().position(|g| g.app_id == app_id) {
            s.games[i] = game;
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

pub fn switch_to_game(state: &SharedState, app_id: &str) {
    state.borrow_mut().selected_id = app_id.to_string();

    // Select the corresponding sidebar row (index + 1 for "All games" at index 0)
    let game_list = state.borrow().game_list.clone();
    let idx = state.borrow().games.iter().position(|g| g.app_id == app_id);
    if let Some(idx) = idx {
        let row = game_list.row_at_index((idx + 1) as i32);
        select_row_silently(state, row.as_ref());
    }

    let game = state.borrow().games.iter().find(|g| g.app_id == app_id).cloned();
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

fn title_vbox(title: &str, subtitle: &str) -> gtk4::Box {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    vbox.set_valign(gtk4::Align::Center);

    let title_label = gtk4::Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.add_css_class("title-1");
    vbox.append(&title_label);

    let sub_label = gtk4::Label::new(Some(subtitle));
    sub_label.set_xalign(0.0);
    sub_label.add_css_class("dim-label");
    vbox.append(&sub_label);

    vbox
}

/// The hero banner (when a hero image exists) or the plain header box.
fn build_game_header(game: &Game, fraction: f64) -> gtk4::Widget {
    let subtitle = format!(
        "{} of {} achievements  ·  {:.0}% complete",
        game.earned_count, game.total_count, fraction * 100.0
    );

    if !game.hero_image_path.is_empty() {
        let overlay = gtk4::Overlay::new();

        let hero = gtk4::Picture::new();
        hero.set_content_fit(gtk4::ContentFit::Cover);
        hero.set_can_shrink(true);
        crate::images::set_picture(&hero, &game.hero_image_path);
        overlay.set_child(Some(&hero));

        let gradient = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        gradient.set_vexpand(true);
        gradient.set_hexpand(true);
        gradient.add_css_class("hero-gradient");
        overlay.add_overlay(&gradient);

        let info_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
        info_box.set_valign(gtk4::Align::End);
        info_box.set_margin_start(24);
        info_box.set_margin_end(24);
        info_box.set_margin_bottom(24);

        if !game.icon_path.is_empty() {
            let img = crate::images::new_image_from_file(&game.icon_path);
            img.set_pixel_size(56);
            info_box.append(&img);
        }

        info_box.append(&title_vbox(&game.name, &subtitle));
        overlay.add_overlay(&info_box);

        let progress = gtk4::ProgressBar::new();
        progress.set_fraction(fraction);
        progress.set_valign(gtk4::Align::End);
        progress.add_css_class("hero-progress");
        overlay.add_overlay(&progress);

        overlay.set_vexpand(false);
        overlay.set_hexpand(true);
        gradient.set_vexpand(false);

        let hero_wrap = gtk4::ScrolledWindow::new();
        hero_wrap.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Never);
        hero_wrap.set_propagate_natural_height(false);
        hero_wrap.set_propagate_natural_width(false);
        hero_wrap.set_size_request(-1, 280);
        hero_wrap.set_vexpand(false);
        hero_wrap.set_child(Some(&overlay));
        hero_wrap.upcast()
    } else {
        let header_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        header_box.set_margin_top(28);
        header_box.set_margin_bottom(8);
        header_box.set_margin_start(28);
        header_box.set_margin_end(28);

        let title_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
        if !game.icon_path.is_empty() {
            let img = crate::images::new_image_from_file(&game.icon_path);
            img.set_pixel_size(56);
            title_row.append(&img);
        }

        title_row.append(&title_vbox(&game.name, &subtitle));
        header_box.append(&title_row);

        let progress = gtk4::ProgressBar::new();
        progress.set_fraction(fraction);
        progress.set_margin_top(4);
        header_box.append(&progress);

        header_box.upcast()
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

    content_box.append(&build_game_header(game, fraction));

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
    let state_for_reload = state.clone();
    let reload = move || {
        let entry = GameEntry {
            id: db_id_for_reload,
            kind: kind_for_reload.clone(),
            steam_id: app_id_for_reload.clone(),
            platform_id: platform_id_for_reload.clone(),
            title: String::new(),
            lutris_id: None,
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
            hidden_row.set_title(&format!("... and {} hidden achievements", hidden.len()));
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

    let game = state.borrow().games.iter().find(|g| g.app_id == selected_id).cloned();
    if let Some(game) = game {
        display_game(&game, state);

        let game_list = state.borrow().game_list.clone();
        let idx = state.borrow().games.iter().position(|g| g.app_id == selected_id);
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
    let menu = gio::Menu::new();
    menu.append(Some(S::EDIT_GAME_SETTINGS), Some("game.edit"));
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

    row.insert_action_group("game", Some(&actions));

    popover.set_parent(row);
    popover.popup();
}

fn show_game_settings_dialog(state: &SharedState, game: &Game) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(
        Some(&format!("Edit Game: {}", game.name)),
        Some(S::EDIT_GAME_PROPS),
    );

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(4);

    let title_entry = gtk4::Entry::new();
    title_entry.set_placeholder_text(Some(S::GAME_TITLE));
    title_entry.set_text(&game.name);
    box_.append(&gtk4::Label::new(Some(S::TITLE)));
    box_.append(&title_entry);

    let lutris_entry = gtk4::Entry::new();
    lutris_entry.set_placeholder_text(Some("e.g. yakuza-like-a-dragon"));
    box_.append(&gtk4::Label::new(Some(S::LUTRIS_SLUG)));
    box_.append(&lutris_entry);

    dialog.set_extra_child(Some(&box_));

    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("save", S::SAVE);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");

    let state_clone = state.clone();
    let db_id = game.db_id;
    let app_id = game.app_id.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }
        let title = title_entry.text().to_string();
        let lutris_id = lutris_entry.text().to_string();
        let lutris_opt = if lutris_id.is_empty() { None } else { Some(lutris_id.as_str()) };
        if let Err(e) = crate::db::update_game(&state_clone.borrow().db, db_id, &title, lutris_opt) {
            eprintln!("Failed to update game: {}", e);
        }
        // Update game in state and rebuild
        if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.app_id == app_id) {
            g.name = title.clone();
        }
        state_clone.borrow().game_names.lock().unwrap().insert(app_id.clone(), title);
        rebuild_sidebar(&state_clone);
    });
    dialog.present(Some(&window));
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
            let _ = sender.send(AppMessage::NewGame(game));
            enrich_game_async(
                app_id.to_string(),
                kind.to_string(),
                platform_id.to_string(),
                entry.id,
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
            title: String::new(),
            lutris_id: None,
        };

        let meta_path = crate::parser::achievements_dir(SAVE_DIR, &app_id).join("achievements.json");
        if !meta_path.exists() {
            if let Err(e) = steam.generate_steam_settings(&app_id) {
                eprintln!("Could not generate achievements for {}: {}", app_id, e);
            }
        }

        let Ok(mut game) = load_game(&entry, SAVE_DIR) else {
            eprintln!("Failed reloading {}", app_id);
            return;
        };

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
            let (icon_path, hero_path) = steam.ensure_assets(&app_id, Some(&details), has_local_icon);
            if game.icon_path.is_empty() && !icon_path.is_empty() {
                game.icon_path = icon_path;
            }
            if !hero_path.is_empty() {
                game.hero_image_path = hero_path;
            }
        }

        // Download grid assets (vertical, header, logo)
        let (grid_path, header_path, logo_path) = steam.ensure_grids(&app_id);
        if !grid_path.is_empty() {
            game.grid_path = grid_path;
        }
        if !header_path.is_empty() {
            game.header_path = header_path;
        }
        if !logo_path.is_empty() {
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

        let _ = sender.send(AppMessage::EnrichedGame(game));
    });
}
