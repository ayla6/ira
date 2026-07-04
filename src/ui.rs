use crate::config::Config;
use crate::parser::{load_game, Game, MergedAchievement, set_achievement_earned};
use crate::steam::SteamClient;
use crate::watcher::AchievementWatcher;
use crate::AppMessage;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use gtk4::glib;
use gtk4::pango;
use gtk4::prelude::*;
use adw::prelude::*;

pub const SAVE_DIR: &str = "/data/Games/Saves/GSE Saves";
const EAGER_IMAGE_BUDGET: usize = 18;

pub struct AppState {
    pub window: adw::ApplicationWindow,
    pub games: Vec<Game>,
    rows: Vec<SidebarRowWidgets>,
    pub game_list: gtk4::ListBox,
    pub content_scroll: gtk4::ScrolledWindow,
    pub content_box: gtk4::Box,
    pub empty_state: adw::StatusPage,
    pub selected_id: String,
    pub cfg: Config,
    pub steam: Arc<SteamClient>,
    pub watcher: Option<AchievementWatcher>,
    pub scroll_positions: HashMap<String, f64>,
    pub sender: Sender<AppMessage>,
    pub game_names: Arc<Mutex<HashMap<String, String>>>,
    pub content_unloaded: bool,
    pub restoring: bool,
}

pub type SharedState = Rc<RefCell<AppState>>;

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
        empty_state: adw::StatusPage::new(),
        selected_id: String::new(),
        cfg: cfg.clone(),
        steam: steam.clone(),
        watcher: watcher.clone(),
        scroll_positions: HashMap::new(),
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
    window.set_title(Some("Achievement Viewer"));
    window.set_default_size(1100, 720);

    let css = gtk4::CssProvider::new();
    css.load_from_bytes(&gtk4::glib::Bytes::from(
        b".hero-gradient {
            background-image: linear-gradient(to bottom, rgba(0,0,0,0) 0%, rgba(0,0,0,0.85) 100%);
        }
        .hero-progress trough { background-color: transparent; border: none; border-radius: 0; }
        .hero-progress progress { background-color: @accent_color; border: none; border-radius: 0; }
        .hero-progress { min-height: 8px; border: none; }
        .sidebar-row-title { min-width: 0; }
        .global-bar trough { background-color: transparent; border: none; }
        .global-bar progress { border: none; border-radius: 0; }
        .grayscale-icon { filter: grayscale(100%); }",
    ));
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

    let settings_btn = gtk4::Button::from_icon_name("preferences-system-symbolic");
    settings_btn.set_tooltip_text(Some("Settings"));
    settings_btn.add_css_class("flat");
    header_bar.pack_end(&settings_btn);

    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some("Add a game by pointing at its install folder"));
    add_btn.add_css_class("flat");
    header_bar.pack_start(&add_btn);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&split_view));
    window.set_content(Some(&toolbar_view));

    let empty_state = adw::StatusPage::new();
    empty_state.set_title("No Game Selected");
    empty_state.set_description(Some("Select a game from the sidebar to view achievements."));
    empty_state.set_icon_name(Some("applications-games-symbolic"));
    content_box.append(&empty_state);

    {
        let mut s = state.borrow_mut();
        s.window = window.clone();
        s.game_list = game_list.clone();
        s.content_scroll = content_scroll.clone();
        s.content_box = content_box.clone();
        s.empty_state = empty_state.clone();
    }

    rebuild_sidebar(state);

    let state_clone = state.clone();
    game_list.connect_row_selected(move |_list, row| {
        let s = state_clone.borrow();
        if s.restoring {
            return;
        }
        let Some(row) = row else { return; };
        let idx = row.index();
        if idx >= 0 && (idx as usize) < s.games.len() {
            let app_id = s.games[idx as usize].app_id.clone();
            drop(s);
            switch_to_game(&state_clone, &app_id);
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

    let games: Vec<Game> = state.borrow().games.iter().cloned().collect();
    let mut rows = Vec::with_capacity(games.len());
    for g in &games {
        rows.push(build_sidebar_row(&game_list, g));
    }
    state.borrow_mut().rows = rows;
}

fn build_sidebar_row(list: &gtk4::ListBox, game: &Game) -> SidebarRowWidgets {
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
        AppMessage::WatcherNewGameDir { app_id, game_dir } => {
            let steam = state.borrow().steam.clone();
            let watcher = state.borrow().watcher.clone();
            let sender = state.borrow().sender.clone();
            enrich_game_async(app_id, game_dir, steam, watcher, sender, true);
        }
        AppMessage::AddGameError(e) => {
            let window = state.borrow().window.clone();
            let dialog = adw::AlertDialog::new(
                Some("Couldn't Add Game"),
                Some(&e),
            );
            dialog.add_response("ok", "OK");
            dialog.set_default_response(Some("ok"));
            dialog.set_close_response("ok");
            dialog.present(Some(&window));
        }
    }
}

fn apply_game_update(state: &SharedState, mut updated: Game) {
    let app_id = updated.app_id.clone();

    let needs_rebuild = {
        let mut s = state.borrow_mut();
        let Some(i) = s.games.iter().position(|g| g.app_id == app_id) else {
            return;
        };

        merge_game_enrichment(&s.games[i], &mut updated);
        s.games[i] = updated.clone();

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

        s.selected_id == app_id && !s.content_unloaded
    };

    if needs_rebuild {
        let game = state.borrow().games.iter().find(|g| g.app_id == app_id).cloned();
        if let Some(game) = game {
            display_game(&game, state);
            let scroll_val = state.borrow().content_scroll.vadjustment().value();
            let content_scroll = state.borrow().content_scroll.clone();
            glib::idle_add_local_once(move || {
                content_scroll.vadjustment().set_value(scroll_val);
            });
        }
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
    }
}

pub fn switch_to_game(state: &SharedState, app_id: &str) {
    {
        let mut s = state.borrow_mut();
        if !s.selected_id.is_empty() && s.selected_id != app_id {
            let prev_id = s.selected_id.clone();
            let v = s.content_scroll.vadjustment().value();
            s.scroll_positions.insert(prev_id, v);
        }
        s.selected_id = app_id.to_string();
    }

    let game = state.borrow().games.iter().find(|g| g.app_id == app_id).cloned();
    if let Some(game) = game {
        display_game(&game, state);
        let target = state.borrow().scroll_positions.get(app_id).copied().unwrap_or(0.0);
        let content_scroll = state.borrow().content_scroll.clone();
        glib::idle_add_local_once(move || {
            content_scroll.vadjustment().set_value(target);
        });
    }
}

fn display_game(game: &Game, state: &SharedState) {
    let content_box = state.borrow().content_box.clone();

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

        let title_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        title_vbox.set_valign(gtk4::Align::Center);

        let title_label = gtk4::Label::new(Some(&game.name));
        title_label.set_xalign(0.0);
        title_label.add_css_class("title-1");
        title_vbox.append(&title_label);

        let sub_label = gtk4::Label::new(Some(&format!(
            "{} of {} achievements  ·  {:.0}% complete",
            game.earned_count, game.total_count, fraction * 100.0
        )));
        sub_label.set_xalign(0.0);
        sub_label.add_css_class("dim-label");
        title_vbox.append(&sub_label);

        info_box.append(&title_vbox);
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
        content_box.append(&hero_wrap);
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

        let title_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        title_vbox.set_valign(gtk4::Align::Center);

        let title_label = gtk4::Label::new(Some(&game.name));
        title_label.set_xalign(0.0);
        title_label.add_css_class("title-1");
        title_vbox.append(&title_label);

        let sub_label = gtk4::Label::new(Some(&format!(
            "{} of {} achievements  ·  {:.0}% complete",
            game.earned_count, game.total_count, fraction * 100.0
        )));
        sub_label.set_xalign(0.0);
        sub_label.add_css_class("dim-label");
        title_vbox.append(&sub_label);

        title_row.append(&title_vbox);
        header_box.append(&title_row);

        let progress = gtk4::ProgressBar::new();
        progress.set_fraction(fraction);
        progress.set_margin_top(4);
        header_box.append(&progress);

        content_box.append(&header_box);
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

    let game_dir = format!("{}/{}", SAVE_DIR, game.app_id);
    let app_id_for_reload = game.app_id.clone();
    let game_dir_for_reload = game_dir.clone();
    let state_for_reload = state.clone();
    let reload = move || {
        if let Ok(updated) = load_game(&app_id_for_reload, Path::new(&game_dir_for_reload)) {
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
            let game_dir_clone = game_dir.clone();
            let state_clone = state.clone();
            locked_group.add(&create_achievement_row(
                ach,
                Some(Box::new(move || {
                    confirm_mark_unlocked(&state_clone, &game_dir_clone, &ach_clone, reload_clone.clone());
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
    let game_for_global = game.clone();
    let global_vbox_weak = global_vbox.downgrade();
    view_stack.connect_notify_local(Some("visible-child-name"), move |stack, _| {
        if stack.visible_child_name() == Some("global".into()) && !global_built.get() {
            global_built.set(true);
            if let Some(global_vbox) = global_vbox_weak.upgrade() {
                build_global_tab(&game_for_global, &global_vbox);
            }
        }
    });

    let progress_page = view_stack.add_titled(&progress_vbox, Some("progress"), "My Progress");
    progress_page.set_icon_name(Some("user-home-symbolic"));

    let global_page = view_stack.add_titled(&global_vbox, Some("global"), "Global Stats");
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
    global_group.set_title("Global Unlock Rates");
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
        row.set_tooltip_text(Some("Right-click to mark as already unlocked"));
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

    let img = gtk4::Image::from_icon_name("changes-prevent-symbolic");
    img.set_pixel_size(48);
    img.set_valign(gtk4::Align::Start);
    if is_hidden_spoiler && ach.icon_gray_path.is_empty() {
    } else if ach.earned {
        if !ach.icon_path.is_empty() {
            budget.load(&img, &ach.icon_path);
        } else {
            img.set_icon_name(Some("starred-symbolic"));
        }
    } else if !ach.icon_gray_path.is_empty() {
        budget.load(&img, &ach.icon_gray_path);
    }

    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&img);

    let text_stack = gtk4::Stack::new();
    text_stack.set_hexpand(true);
    text_stack.set_valign(gtk4::Align::Start);
    text_stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
    text_stack.set_transition_duration(350);

    let vbox_spoiler = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox_spoiler.set_valign(gtk4::Align::Start);
    let title_spoiler = gtk4::Label::new(Some("Hidden Achievement"));
    title_spoiler.set_xalign(0.0);
    vbox_spoiler.append(&title_spoiler);
    let desc_spoiler = gtk4::Label::new(Some("Click to reveal spoiler"));
    desc_spoiler.set_xalign(0.0);
    desc_spoiler.add_css_class("dim-label");
    desc_spoiler.add_css_class("caption");
    vbox_spoiler.append(&desc_spoiler);
    text_stack.add_named(&vbox_spoiler, Some("spoiler"));

    let vbox_real = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox_real.set_valign(gtk4::Align::Start);
    let title_real = gtk4::Label::new(Some(&ach.display_name));
    title_real.set_xalign(0.0);
    vbox_real.append(&title_real);
    let desc_real = gtk4::Label::new(Some(&ach.description));
    desc_real.set_wrap(true);
    desc_real.set_wrap_mode(pango::WrapMode::WordChar);
    desc_real.set_xalign(0.0);
    desc_real.add_css_class("dim-label");
    desc_real.add_css_class("caption");
    vbox_real.append(&desc_real);
    text_stack.add_named(&vbox_real, Some("real"));

    content.append(&text_stack);

    let pct_label = gtk4::Label::new(Some(&format!("{:.1}%", ach.global_percent)));
    pct_label.set_valign(gtk4::Align::Start);
    pct_label.add_css_class("heading");
    content.append(&pct_label);

    grid.attach(&progress, 0, 0, 1, 1);
    grid.attach(&content, 0, 0, 1, 1);
    row.set_child(Some(&grid));

    let reveal = if is_hidden_spoiler {
        text_stack.set_visible_child_name("spoiler");
        row.set_selectable(true);
        row.set_activatable(true);

        let img_weak = img.downgrade();
        let text_stack_weak = text_stack.downgrade();
        let row_weak = row.downgrade();
        let icon_path = ach.icon_path.clone();
        let icon_gray_path = ach.icon_gray_path.clone();

        Some(Box::new(move || {
            let Some(row) = row_weak.upgrade() else { return };
            let Some(img) = img_weak.upgrade() else { return };
            let Some(text_stack) = text_stack_weak.upgrade() else { return };
            text_stack.set_visible_child_name("real");
            if !icon_path.is_empty() {
                crate::images::set_image(&img, &icon_path);
                img.add_css_class("grayscale-icon");
            } else if !icon_gray_path.is_empty() {
                crate::images::set_image(&img, &icon_gray_path);
            }
            row.set_activatable(false);
            row.set_selectable(false);
        }) as Box<dyn Fn()>)
    } else {
        text_stack.set_visible_child_name("real");
        None
    };

    (row, reveal)
}

fn show_close_choice_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(
        Some("Close Achievement Viewer"),
        Some("Keep the watcher running in the background, or quit completely?"),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("background", "Hide to Background");
    dialog.add_response("quit", "Quit");
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
    // Save scroll position for the current game
    {
        let mut s = state.borrow_mut();
        if !s.selected_id.is_empty() {
            let id = s.selected_id.clone();
            let v = s.content_scroll.vadjustment().value();
            s.scroll_positions.insert(id, v);
        }
    }

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
    extern "C" { fn malloc_trim(pad: usize) -> i32; }
    unsafe { malloc_trim(0); }

    // GTK releases some resources asynchronously — trim again after a delay
    glib::timeout_add_local(std::time::Duration::from_millis(300), || {
        extern "C" { fn malloc_trim(pad: usize) -> i32; }
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
        let empty_state = state.borrow().empty_state.clone();
        state.borrow().content_box.append(&empty_state);
        return;
    }

    let game = state.borrow().games.iter().find(|g| g.app_id == selected_id).cloned();
    if let Some(game) = game {
        display_game(&game, state);
        let target = state.borrow().scroll_positions.get(&selected_id).copied().unwrap_or(0.0);
        let content_scroll = state.borrow().content_scroll.clone();
        glib::idle_add_local_once(move || {
            content_scroll.vadjustment().set_value(target);
        });

        let game_list = state.borrow().game_list.clone();
        let idx = state.borrow().games.iter().position(|g| g.app_id == selected_id);
        if let Some(idx) = idx {
            state.borrow_mut().restoring = true;
            let row = game_list.row_at_index(idx as i32);
            if let Some(row) = row {
                game_list.select_row(Some(&row));
            }
            state.borrow_mut().restoring = false;
        }
    } else {
        state.borrow_mut().selected_id.clear();
        let empty_state = state.borrow().empty_state.clone();
        state.borrow().content_box.append(&empty_state);
    }
}

fn show_settings_dialog(
    parent: &adw::ApplicationWindow,
    cfg: Config,
    steam: Arc<SteamClient>,
    state: &SharedState,
) {
    let dialog = adw::Window::new();
    dialog.set_title(Some("Settings"));
    dialog.set_default_size(450, 360);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(parent));

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&adw::HeaderBar::new());

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let group = adw::PreferencesGroup::new();
    group.set_title("API Keys");
    group.set_margin_top(16);
    group.set_margin_bottom(16);
    group.set_margin_start(16);
    group.set_margin_end(16);

    let steam_entry = adw::EntryRow::new();
    steam_entry.set_title("Steam Web API Key");
    steam_entry.set_text(&cfg.steam_api_key);
    steam_entry.set_input_purpose(gtk4::InputPurpose::Password);
    group.add(&steam_entry);

    let sgdb_entry = adw::EntryRow::new();
    sgdb_entry.set_title("SteamGridDB API Key");
    sgdb_entry.set_text(&cfg.steam_griddb_api_key);
    sgdb_entry.set_input_purpose(gtk4::InputPurpose::Password);
    group.add(&sgdb_entry);

    let notif_group = adw::PreferencesGroup::new();
    notif_group.set_title("Live Updates");
    notif_group.set_margin_top(16);
    notif_group.set_margin_start(16);
    notif_group.set_margin_end(16);

    let notif_row = adw::SwitchRow::new();
    notif_row.set_title("Notify on New Unlocks");
    notif_row.set_subtitle("Show a desktop notification the moment an achievement unlocks");
    notif_row.set_active(cfg.notifications_enabled);
    notif_group.add(&notif_row);

    let bg_row = adw::SwitchRow::new();
    bg_row.set_title("Close to Background");
    bg_row.set_subtitle("Closing the window keeps the watcher running silently in the background");
    bg_row.set_active(cfg.close_to_background);
    notif_group.add(&bg_row);

    let save_btn = gtk4::Button::with_label("Save");
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

fn show_add_game_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();
    let dialog = gtk4::FileDialog::new();
    dialog.set_title("Select Game Folder");

    let state_clone = state.clone();
    dialog.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let folder = path.to_string_lossy().into_owned();

        if let Some(app_id) = crate::gamesetup::detect_app_id(&folder) {
            finish_add_game(&state_clone, &folder, &app_id);
        } else {
            prompt_for_app_id(&state_clone, &folder);
        }
    });
}

fn prompt_for_app_id(state: &SharedState, folder: &str) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(
        Some("Enter Steam App ID"),
        Some("No steam_appid.txt was found in this folder. Enter the game's Steam App ID to continue."),
    );

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("e.g. 1687950"));
    entry.set_input_purpose(gtk4::InputPurpose::Digits);
    entry.set_margin_top(8);
    entry.set_margin_bottom(8);
    entry.set_margin_start(8);
    entry.set_margin_end(8);
    dialog.set_extra_child(Some(&entry));

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add Game");
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");

    let state_clone = state.clone();
    let folder_clone = folder.to_string();
    dialog.connect_response(None, move |_, response| {
        if response != "add" {
            return;
        }
        let app_id = entry.text().to_string();
        finish_add_game(&state_clone, &folder_clone, &app_id);
    });
    dialog.present(Some(&window));
}

fn finish_add_game(state: &SharedState, folder: &str, app_id: &str) {
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let sender = state.borrow().sender.clone();
    let app_id = app_id.to_string();
    let folder = folder.to_string();

    std::thread::spawn(move || {
        match crate::gamesetup::add_game_from_folder(&folder, &app_id, &steam, SAVE_DIR) {
            Ok(game_dir) => {
                let game_id = std::path::Path::new(&game_dir)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&app_id)
                    .to_string();
                match load_game(&game_id, std::path::Path::new(&game_dir)) {
                    Ok(mut game) => {
                        if game.name.starts_with("App ID:") {
                            if let Some(name) = steam.fetch_nemirtingas_game_name(&game.app_id) {
                                game.name = name;
                            }
                        }
                        if let Some(ref watcher) = watcher {
                            watcher.watch(&game.app_id, &game_dir, &game.achievements);
                        }
                        let _ = sender.send(AppMessage::NewGame(game));
                        enrich_game_async(
                            game_id,
                            game_dir,
                            steam,
                            watcher,
                            sender,
                            true,
                        );
                    }
                    Err(e) => eprintln!("Failed to load newly added game: {}", e),
                }
            }
            Err(e) => {
                eprintln!("Add game failed: {}", e);
                let _ = sender.send(AppMessage::AddGameError(e));
            }
        }
    });
}

fn confirm_mark_unlocked(state: &SharedState, game_dir: &str, ach: &MergedAchievement, reload: impl Fn() + 'static) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(
        Some("Mark as Already Unlocked?"),
        Some(&format!(
            "This will mark \u{201C}{}\u{201D} as earned without a real unlock time. \
             Use this only if you already unlocked it previously (e.g. before using this tool).",
            ach.display_name
        )),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("confirm", "Mark as Unlocked");
    dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let ach_name = ach.name.clone();
    let game_dir = game_dir.to_string();
    dialog.connect_response(None, move |_, response| {
        if response != "confirm" {
            return;
        }
        if let Err(e) = set_achievement_earned(&game_dir, &ach_name, true) {
            eprintln!("Failed to mark achievement as unlocked: {}", e);
            return;
        }
        reload();
    });
    dialog.present(Some(&window));
}

pub fn enrich_game_async(
    app_id: String,
    game_dir: String,
    steam: Arc<SteamClient>,
    watcher: Option<AchievementWatcher>,
    sender: Sender<AppMessage>,
    is_new: bool,
) {
    std::thread::spawn(move || {
        let meta_path = std::path::Path::new(&game_dir)
            .join("steam_settings")
            .join("achievements.json");
        if !meta_path.exists() {
            if let Err(e) = steam.generate_steam_settings(&app_id, std::path::Path::new(&game_dir)) {
                eprintln!("Could not generate steam_settings for {}: {}", app_id, e);
            }
        }

        let Ok(mut game) = load_game(&app_id, std::path::Path::new(&game_dir)) else {
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

        if let Some(pcts) = steam.fetch_global_achievements(&app_id) {
            for a in &mut game.achievements {
                if let Some(&pct) = pcts.get(&a.name) {
                    a.global_percent = pct;
                }
            }
        }

        if let Some(ref watcher) = watcher {
            watcher.watch(&app_id, &game_dir, &game.achievements);
        }

        let msg = if is_new {
            AppMessage::NewGame(game)
        } else {
            AppMessage::EnrichedGame(game)
        };
        let _ = sender.send(msg);
    });
}
