use ira_config::Config;
use ira_db::DbConn;
use ira_api::SteamClient;
use crate::strings as S;
use ira_watcher::AchievementWatcher;
use crate::AppSender;
use ira_models::{GroupSelection, SortMode};
use gtk4::prelude::*;
use adw::prelude::*;
use crate::Game;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use super::state::{AppState, SharedState};
use super::css::APP_CSS;
use super::sidebar::{rebuild_sidebar, select_row_silently};
use super::grid_view::show_grid_view;
use super::message_handler::switch_to_game;
use super::settings_dialog::show_settings_dialog;
use super::mass_match_dialog::show_mass_match_dialog;
use super::add_game_dialog::show_add_game_dialog;
use super::background::show_close_choice_dialog;

pub struct AppContext {
    pub steam: Arc<SteamClient>,
    pub watcher: Option<AchievementWatcher>,
    pub db: DbConn,
    pub sender: AppSender,
    pub game_names: Arc<Mutex<HashMap<String, String>>>,
}

pub fn build_ui(
    app: &adw::Application,
    games: Vec<Game>,
    cfg: Config,
    ctx: AppContext,
) -> SharedState {
    let save_dir = cfg.save_dir.clone();
    let groups = ira_db::get_all_groups(&ctx.db).unwrap_or_else(|e| {
        eprintln!("Failed to load groups: {}", e);
        Vec::new()
    });

    let sidebar_store = gio::ListStore::new::<super::sidebar_item::SidebarItem>();
    let sidebar_selection = super::game_selection_model::GameSelectionModel::new(Some(&sidebar_store));
    let sidebar_view = gtk4::ListView::new(None::<super::game_selection_model::GameSelectionModel>, None::<gtk4::SignalListItemFactory>);
    let state = Rc::new(RefCell::new(AppState {
        window: adw::ApplicationWindow::new(app),
        games,
        sidebar_store,
        sidebar_selection,
        sidebar_view,
        sidebar_scroll: gtk4::ScrolledWindow::new(),
        content_scroll: gtk4::ScrolledWindow::new(),
        content_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
        grid_header: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
        selected_id: String::new(),
        cfg: cfg.clone(),
        steam: ctx.steam.clone(),
        watcher: ctx.watcher.clone(),
        shadps4_watcher: None,
        db: ctx.db,
        sender: ctx.sender,
        game_names: ctx.game_names,
        content_unloaded: false,
        restoring: false,
        running_games: Arc::new(Mutex::new(HashMap::new())),
        grid_store: gio::ListStore::new::<crate::ui::game_item::GameItem>(),
        sidebar_rebuild_pending: false,
        view_generation: 0,
        settings_data: None,
        save_dir,
        search_query: String::new(),
        selected_group: GroupSelection::AllGames,
        groups,
        search_entry: gtk4::SearchEntry::new(),
        sort_label: gtk4::Label::new(Some(SortMode::Alphabetical.display_label())),
        collapsed_collections: HashSet::new(),
    }));

    build_window(&state, app);

    let win = state.borrow().window.clone();
    win.present();

    state
}

pub(crate) fn build_window(state: &SharedState, app: &adw::Application) {
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some(S::APP_TITLE));
    window.set_default_size(1100, 720);

    let css = gtk4::CssProvider::new();
    css.load_from_string(APP_CSS);
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().expect("no default display"),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );

    let split_view = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    split_view.set_position(260);
    split_view.set_shrink_start_child(false);
    split_view.set_resize_start_child(false);

    let sidebar_scroll = gtk4::ScrolledWindow::new();
    sidebar_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    sidebar_scroll.set_size_request(220, -1);
    sidebar_scroll.set_vexpand(true);

    let sidebar_store = gio::ListStore::new::<super::sidebar_item::SidebarItem>();
    let sidebar_selection = super::game_selection_model::GameSelectionModel::new(Some(&sidebar_store));
    let factory = super::sidebar::build_factory(state);
    let sidebar_view = gtk4::ListView::new(Some(sidebar_selection.clone()), Some(factory));
    sidebar_view.add_css_class("navigation-sidebar");
    sidebar_view.set_show_separators(false);
    sidebar_scroll.set_child(Some(&sidebar_view));

    split_view.set_start_child(Some(&sidebar_scroll));

    let content_scroll = gtk4::ScrolledWindow::new();
    content_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    content_scroll.set_vexpand(true);
    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let grid_header = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let content_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content_area.append(&grid_header);
    content_area.append(&content_scroll);
    split_view.set_end_child(Some(&content_area));

    let header_bar = adw::HeaderBar::new();

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some(S::SEARCH_GAMES));
    search_entry.set_hexpand(false);
    search_entry.set_max_width_chars(90);
    search_entry.set_width_chars(50);
    header_bar.set_title_widget(Some(&search_entry));

    let menu_btn = gtk4::MenuButton::new();
    menu_btn.set_icon_name("open-menu-symbolic");
    menu_btn.set_tooltip_text(Some(S::MENU));
    menu_btn.add_css_class("flat");
    header_bar.pack_end(&menu_btn);

    let (_sort_popover, sort_btn, sort_label) = build_sort_popover(state);
    sort_btn.set_icon_name("view-sort-descending-symbolic");
    sort_btn.set_tooltip_text(Some(S::SORT_BY));
    sort_btn.add_css_class("flat");
    header_bar.pack_end(&sort_btn);

    let (popover, settings_btn) = build_menu_popover(state);
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
        s.sidebar_store = sidebar_store.clone();
        s.sidebar_selection = sidebar_selection.clone();
        s.sidebar_view = sidebar_view.clone();
        s.sidebar_scroll = sidebar_scroll.clone();
        s.content_scroll = content_scroll.clone();
        s.content_box = content_box.clone();
        s.grid_header = grid_header.clone();
        s.search_entry = search_entry.clone();
        s.sort_label = sort_label;
    }

    rebuild_sidebar(state);

    if !state.borrow().content_unloaded {
        select_row_silently(state, Some(0));
        show_grid_view(state);
    }

    connect_window_signals(state, &window, &sidebar_view, &sidebar_selection, &add_btn, &settings_btn, &search_entry);
}

fn build_menu_popover(state: &SharedState) -> (gtk4::Popover, gtk4::Button) {
    let popover = gtk4::Popover::new();
    popover.set_size_request(300, -1);
    let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    popover_box.set_margin_start(12);
    popover_box.set_margin_end(12);
    popover_box.set_margin_top(8);
    popover_box.set_margin_bottom(8);

    let settings_btn = gtk4::Button::new();
    let settings_label = gtk4::Label::new(Some(S::SETTINGS));
    settings_label.set_xalign(0.0);
    settings_btn.set_child(Some(&settings_label));
    settings_btn.add_css_class("flat");
    settings_btn.set_halign(gtk4::Align::Fill);
    settings_btn.set_size_request(-1, 36);
    settings_btn.add_css_class("popover-menu-row");
    popover_box.append(&settings_btn);

    let match_btn = gtk4::Button::new();
    let match_label = gtk4::Label::new(Some("Match unmatched games"));
    match_label.set_xalign(0.0);
    match_btn.set_child(Some(&match_label));
    match_btn.add_css_class("flat");
    match_btn.set_halign(gtk4::Align::Fill);
    match_btn.set_size_request(-1, 36);
    match_btn.add_css_class("popover-menu-row");

    let state_clone2 = state.clone();
    match_btn.connect_clicked(move |_| show_mass_match_dialog(&state_clone2));

    popover_box.append(&match_btn);

    let history_btn = gtk4::Button::new();
    let history_label = gtk4::Label::new(Some(S::PLAY_HISTORY));
    history_label.set_xalign(0.0);
    history_btn.set_child(Some(&history_label));
    history_btn.add_css_class("flat");
    history_btn.set_halign(gtk4::Align::Fill);
    history_btn.set_size_request(-1, 36);
    history_btn.add_css_class("popover-menu-row");

    let state_clone_hist = state.clone();
    history_btn.connect_clicked(move |_| super::play_history::show_daily_history_dialog(&state_clone_hist));

    popover_box.append(&history_btn);

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.set_margin_top(4);
    sep.set_margin_bottom(4);
    popover_box.append(&sep);

    let hidden_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hidden_row.set_hexpand(true);
    hidden_row.set_size_request(-1, 36);
    hidden_row.add_css_class("popover-menu-row");
    let hidden_label = gtk4::Label::new(Some(S::SHOW_HIDDEN_GAMES));
    hidden_label.set_xalign(0.0);
    hidden_label.set_hexpand(true);
    let hidden_switch = gtk4::Switch::new();
    hidden_switch.set_active(state.borrow().cfg.show_hidden_games);
    hidden_row.append(&hidden_label);
    hidden_row.append(&hidden_switch);
    popover_box.append(&hidden_row);

    let zoom_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    zoom_row.set_hexpand(true);
    zoom_row.set_size_request(-1, 36);
    zoom_row.add_css_class("popover-menu-row");
    let zoom_label = gtk4::Label::new(Some(S::COVER_SIZE));
    zoom_label.set_xalign(0.0);
    zoom_label.set_hexpand(true);
    let zoom_scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 100.0, 350.0, 10.0);
    zoom_scale.set_value(state.borrow().cfg.grid_cover_width.clamp(100, 350) as f64);
    zoom_scale.set_hexpand(true);
    zoom_scale.set_draw_value(false);
    zoom_scale.set_digits(0);
    zoom_scale.set_margin_end(4);
    zoom_row.append(&zoom_label);
    zoom_row.append(&zoom_scale);
    popover_box.append(&zoom_row);

    popover.set_child(Some(&popover_box));

    let state_clone = state.clone();
    hidden_switch.connect_active_notify(move |sw| {
        let active = sw.is_active();
        state_clone.borrow_mut().cfg.show_hidden_games = active;
        let _ = state_clone.borrow().cfg.save();
        rebuild_sidebar(&state_clone);
        if state_clone.borrow().selected_id.is_empty() && !state_clone.borrow().content_unloaded {
            show_grid_view(&state_clone);
        }
    });

    let state_clone = state.clone();
    let zoom_gen: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    zoom_scale.connect_value_changed(move |scale| {
        let val = scale.value() as i32;
        state_clone.borrow_mut().cfg.grid_cover_width = val;
        let _ = state_clone.borrow().cfg.save();
        if !state_clone.borrow().selected_id.is_empty()
            || state_clone.borrow().content_unloaded
        {
            return;
        }
        let gen = zoom_gen.get() + 1;
        zoom_gen.set(gen);
        let gen_cell = zoom_gen.clone();
        let s = state_clone.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            if gen_cell.get() == gen {
                show_grid_view(&s);
            }
        });
    });

    (popover, settings_btn)
}

fn build_sort_popover(state: &SharedState) -> (gtk4::Popover, gtk4::MenuButton, gtk4::Label) {
    let popover = gtk4::Popover::new();
    popover.set_size_request(200, -1);
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);

    let current_mode = state.borrow().cfg.sort_mode;
    let mut first_btn: Option<gtk4::CheckButton> = None;

    for mode in SortMode::ALL {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_size_request(-1, 36);

        let check = gtk4::CheckButton::new();
        if *mode == current_mode {
            check.set_active(true);
        }
        if let Some(ref first) = first_btn {
            check.set_group(Some(first));
        } else {
            first_btn = Some(check.clone());
        }

        let label = gtk4::Label::new(Some(mode.display_label()));
        label.set_xalign(0.0);
        label.set_hexpand(true);

        row.append(&check);
        row.append(&label);
        vbox.append(&row);

        let state_clone = state.clone();
        let check_clone = check.clone();
        let mode_c = *mode;
        check.connect_toggled(move |_| {
            if check_clone.is_active() {
                state_clone.borrow_mut().cfg.sort_mode = mode_c;
                let _ = state_clone.borrow().cfg.save();
                state_clone.borrow().sort_label.set_text(mode_c.display_label());
                rebuild_sidebar(&state_clone);
                if state_clone.borrow().selected_id.is_empty()
                    && !state_clone.borrow().content_unloaded
                {
                    show_grid_view(&state_clone);
                }
            }
        });
    }

    popover.set_child(Some(&vbox));

    let btn = gtk4::MenuButton::new();
    btn.set_popover(Some(&popover));

    let sort_label = gtk4::Label::new(Some(current_mode.display_label()));
    sort_label.set_visible(false);

    (popover, btn, sort_label)
}

fn connect_window_signals(
    state: &SharedState,
    window: &adw::ApplicationWindow,
    sidebar_view: &gtk4::ListView,
    sidebar_selection: &super::game_selection_model::GameSelectionModel,
    add_btn: &gtk4::Button,
    settings_btn: &gtk4::Button,
    search_entry: &gtk4::SearchEntry,
) {
    let state_clone = state.clone();
    sidebar_selection.connect_selection_changed(move |model, _position, _n_items| {
        let s = state_clone.borrow();
        if s.restoring {
            return;
        }
        let position = model.clicked_position();
        if position == gtk4::INVALID_LIST_POSITION {
            return;
        }
        let store = s.sidebar_store.clone();
        let Some(item) = store.item(position).and_then(|o| o.downcast::<super::sidebar_item::SidebarItem>().ok()) else {
            return;
        };
        let kind = item.kind();
        let db_id = item.db_id();
        let group_id = item.group_id();
        drop(s);

        match kind {
            super::sidebar_item::SidebarItemKind::AllGames => {
                state_clone.borrow_mut().selected_id.clear();
                state_clone.borrow_mut().selected_group = GroupSelection::AllGames;
                show_grid_view(&state_clone);
            }
            super::sidebar_item::SidebarItemKind::CollectionHeader => {
                state_clone.borrow_mut().selected_id.clear();
                state_clone.borrow_mut().selected_group = GroupSelection::Collection(group_id);
                show_grid_view(&state_clone);
            }
            super::sidebar_item::SidebarItemKind::UncategorizedHeader => {
                state_clone.borrow_mut().selected_id.clear();
                state_clone.borrow_mut().selected_group = GroupSelection::Uncategorized;
                show_grid_view(&state_clone);
            }
            super::sidebar_item::SidebarItemKind::Game => {
                switch_to_game(&state_clone, db_id);
            }
        }
    });

    let state_clone = state.clone();
    sidebar_view.connect_activate(move |_view, position| {
        let store = state_clone.borrow().sidebar_store.clone();
        let Some(item) = store.item(position).and_then(|o| o.downcast::<super::sidebar_item::SidebarItem>().ok()) else {
            return;
        };
        if item.kind() == super::sidebar_item::SidebarItemKind::Game {
            let db_id = item.db_id();
            let s = state_clone.borrow();
            if s.games.iter().any(|g| g.db_id == db_id) {
                let vid = ira_db::get_default_variant(&s.db, db_id);
                drop(s);
                if !state_clone.borrow().running_games.lock().unwrap().contains_key(&db_id) {
                    let _ = super::play_button::launch_game(&state_clone, db_id, vid);
                    let _ = state_clone.borrow().sender.send(crate::AppMessage::GameStarted(db_id));
                }
            }
        }
    });

    let state_clone = state.clone();
    let search_gen: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    search_entry.connect_search_changed(move |entry| {
        let text = entry.text().to_string();
        state_clone.borrow_mut().search_query = text;
        let gen = search_gen.get() + 1;
        search_gen.set(gen);
        let gen_cell = search_gen.clone();
        let s = state_clone.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            if gen_cell.get() == gen {
                rebuild_sidebar(&s);
                if s.borrow().selected_id.is_empty()
                    && !s.borrow().content_unloaded
                {
                    show_grid_view(&s);
                }
            }
        });
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
