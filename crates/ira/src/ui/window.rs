use ira_config::Config;
use ira_db::DbConn;
use ira_api::SteamDataClient;
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
use super::css::*;
use super::sidebar::{rebuild_sidebar, rebuild_sidebar_and_show_grid, select_row_silently};
use super::grid_view::show_grid_view;
use super::message_helpers::switch_to_game;
use super::settings_dialog::show_settings_dialog;
use super::mass_match_dialog::show_mass_match_dialog;
use super::add_game_dialog::show_add_game_dialog;
use super::background::show_close_choice_dialog;

pub struct AppContext {
    pub steam: Arc<SteamDataClient>,
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
    let _span = tracing::info_span!("build_ui").entered();
    let save_dir = cfg.save_dir.clone();
    let groups = ira_db::get_all_groups(&ctx.db).unwrap_or_else(|e| {
        eprintln!("Failed to load groups: {}", e);
        Vec::new()
    });
    let group_members: HashMap<i64, HashSet<i64>> = groups.iter()
        .map(|g| {
            let ids = ira_db::get_game_ids_in_group(&ctx.db, g.id).unwrap_or_default();
            (g.id, ids.into_iter().collect())
        })
        .collect();

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
        grid_item_height: Cell::new(0),
        selected_id: String::new(),
        displayed_db_id: 0,
        cfg: cfg.clone(),
        steam: ctx.steam.clone(),
        watcher: ctx.watcher.clone(),
        shadps4_watcher: None,
        rpcs3_watcher: None,
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
        group_members,
        search_entry: gtk4::SearchEntry::new(),
        sort_label: gtk4::Label::new(Some(SortMode::Alphabetical.display_label())),
        collapsed_collections: HashSet::new(),
        multi_selected_ids: HashSet::new(),
    }));

    {
        let _s = tracing::info_span!("build_window").entered();
        build_window(&state, app);
    }

    let win = state.borrow().window.clone();
    win.present();

    state
}

pub(crate) fn build_window(state: &SharedState, app: &adw::Application) {
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some(S::APP_TITLE));
    window.set_default_size(1200, 720);
    window.set_size_request(900, 650);

    let css = gtk4::CssProvider::new();
    css.load_from_string(APP_CSS);
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().expect("no default display"),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );

    let split_view = adw::NavigationSplitView::new();
    split_view.set_sidebar_width_fraction(0.22);
    split_view.set_min_sidebar_width(200.0);
    split_view.set_max_sidebar_width(280.0);

    // ─── Sidebar ───
    let sidebar_toolbar = adw::ToolbarView::new();

    let sidebar_header = adw::HeaderBar::new();
    sidebar_header.set_show_title(false);
    sidebar_header.add_css_class("flat");

    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some(S::ADD_GAME));
    add_btn.add_css_class(CSS_FLAT);
    sidebar_header.pack_start(&add_btn);

    let popover = build_menu_popover(state);
    let menu_btn = gtk4::MenuButton::new();
    menu_btn.set_icon_name("open-menu-symbolic");
    menu_btn.set_tooltip_text(Some(S::MENU));
    menu_btn.add_css_class(CSS_FLAT);
    menu_btn.set_popover(Some(&popover));
    sidebar_header.pack_end(&menu_btn);

    sidebar_toolbar.add_top_bar(&sidebar_header);

    let sidebar_scroll = gtk4::ScrolledWindow::new();
    sidebar_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    sidebar_scroll.set_size_request(220, -1);
    sidebar_scroll.set_vexpand(true);

    let sidebar_store = gio::ListStore::new::<super::sidebar_item::SidebarItem>();
    let sidebar_selection = super::game_selection_model::GameSelectionModel::new(Some(&sidebar_store));
    let factory = super::sidebar::build_factory(state);
    let sidebar_view = gtk4::ListView::new(Some(sidebar_selection.clone()), Some(factory));
    sidebar_view.add_css_class(CSS_NAVIGATION_SIDEBAR);
    sidebar_view.set_show_separators(false);
    sidebar_scroll.set_child(Some(&sidebar_view));
    sidebar_toolbar.set_content(Some(&sidebar_scroll));

    let sidebar_page = adw::NavigationPage::new(&sidebar_toolbar, "Games");
    split_view.set_sidebar(Some(&sidebar_page));

    // ─── Content area (search + grid/detail) ───
    let content_toolbar = adw::ToolbarView::new();

    let content_header = adw::HeaderBar::new();
    content_header.add_css_class("flat");
    content_header.add_css_class("app-content-header");

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some(S::SEARCH_GAMES));
    search_entry.set_hexpand(true);
    let title_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    title_box.set_hexpand(true);
    title_box.append(&search_entry);
    content_header.set_title_widget(Some(&title_box));

    let (_sort_popover, sort_btn, sort_label) = build_sort_popover(state);
    sort_btn.set_icon_name("view-sort-descending-symbolic");
    sort_btn.set_tooltip_text(Some(S::SORT_BY));
    sort_btn.add_css_class(CSS_FLAT);
    content_header.pack_end(&sort_btn);

    content_toolbar.add_top_bar(&content_header);

    let grid_header = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let content_scroll = gtk4::ScrolledWindow::new();
    content_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    content_scroll.set_vexpand(true);
    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let content_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content_area.append(&grid_header);
    content_area.append(&content_scroll);
    content_toolbar.set_content(Some(&content_area));

    let content_page = adw::NavigationPage::new(&content_toolbar, "Library");
    split_view.set_content(Some(&content_page));

    window.set_content(Some(&split_view));

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

    connect_window_signals(state, &window, &sidebar_view, &sidebar_selection, &add_btn, &search_entry);
}

fn build_menu_popover(state: &SharedState) -> gtk4::Popover {
    let popover = gtk4::Popover::new();
    popover.set_size_request(300, -1);
    let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    popover_box.set_margin_start(12);
    popover_box.set_margin_end(12);
    popover_box.set_margin_top(8);
    popover_box.set_margin_bottom(8);

    let popup_settings_btn = gtk4::Button::new();
    let popup_settings_label = gtk4::Label::new(Some(S::SETTINGS));
    popup_settings_label.set_xalign(0.0);
    popup_settings_btn.set_child(Some(&popup_settings_label));
    popup_settings_btn.add_css_class(CSS_FLAT);
    popup_settings_btn.set_halign(gtk4::Align::Fill);
    popup_settings_btn.set_size_request(-1, 36);
    popup_settings_btn.add_css_class(CSS_POPOVER_MENU_ROW);
    {
        let state_clone = state.clone();
        popup_settings_btn.connect_clicked(move |_| {
            let (window, cfg, steam) = {
                let s = state_clone.borrow();
                (s.window.clone(), s.cfg.clone(), s.steam.clone())
            };
            show_settings_dialog(&window, cfg, steam, &state_clone);
        });
    }
    popover_box.append(&popup_settings_btn);

    let match_btn = gtk4::Button::new();
    let match_label = gtk4::Label::new(Some("Match unmatched games"));
    match_label.set_xalign(0.0);
    match_btn.set_child(Some(&match_label));
    match_btn.add_css_class(CSS_FLAT);
    match_btn.set_halign(gtk4::Align::Fill);
    match_btn.set_size_request(-1, 36);
    match_btn.add_css_class(CSS_POPOVER_MENU_ROW);

    let state_clone2 = state.clone();
    match_btn.connect_clicked(move |_| show_mass_match_dialog(&state_clone2));

    popover_box.append(&match_btn);

    let history_btn = gtk4::Button::new();
    let history_label = gtk4::Label::new(Some(S::PLAY_HISTORY));
    history_label.set_xalign(0.0);
    history_btn.set_child(Some(&history_label));
    history_btn.add_css_class(CSS_FLAT);
    history_btn.set_halign(gtk4::Align::Fill);
    history_btn.set_size_request(-1, 36);
    history_btn.add_css_class(CSS_POPOVER_MENU_ROW);

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
    hidden_row.add_css_class(CSS_POPOVER_MENU_ROW);
    let hidden_label = gtk4::Label::new(Some(S::SHOW_HIDDEN_GAMES));
    hidden_label.set_xalign(0.0);
    hidden_label.set_hexpand(true);
    let hidden_switch = gtk4::Switch::new();
    hidden_switch.set_active(state.borrow().cfg.show_hidden_games);
    hidden_row.append(&hidden_label);
    hidden_row.append(&hidden_switch);
    popover_box.append(&hidden_row);

    popover.set_child(Some(&popover_box));

    let state_clone = state.clone();
    hidden_switch.connect_active_notify(move |sw| {
        let active = sw.is_active();
        state_clone.borrow_mut().cfg.show_hidden_games = active;
        if let Err(e) = state_clone.borrow().cfg.save() {
            eprintln!("Failed to save config: {}", e);
        }
        rebuild_sidebar_and_show_grid(&state_clone);
    });

    popover
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
                if let Err(e) = state_clone.borrow().cfg.save() {
                    eprintln!("Failed to save config: {}", e);
                }
                state_clone.borrow().sort_label.set_text(mode_c.display_label());
                rebuild_sidebar_and_show_grid(&state_clone);
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

        if !model.select_single() {
            let selected_ids = model.selected_db_ids();
            drop(s);
            state_clone.borrow_mut().multi_selected_ids = selected_ids;
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
                state_clone.borrow_mut().multi_selected_ids.clear();
                show_grid_view(&state_clone);
            }
            super::sidebar_item::SidebarItemKind::CollectionHeader => {
                state_clone.borrow_mut().selected_id.clear();
                state_clone.borrow_mut().selected_group = GroupSelection::Collection(group_id);
                state_clone.borrow_mut().multi_selected_ids.clear();
                show_grid_view(&state_clone);
            }
            super::sidebar_item::SidebarItemKind::UncategorizedHeader => {
                state_clone.borrow_mut().selected_id.clear();
                state_clone.borrow_mut().selected_group = GroupSelection::Uncategorized;
                state_clone.borrow_mut().multi_selected_ids.clear();
                show_grid_view(&state_clone);
            }
            super::sidebar_item::SidebarItemKind::Game => {
                state_clone.borrow_mut().multi_selected_ids = HashSet::new();
                switch_to_game(&state_clone, db_id, item.variant_id());
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
            let item_variant_id = item.variant_id();
            let s = state_clone.borrow();
            if s.games.iter().any(|g| g.db_id == db_id) {
                let vid = item_variant_id.or_else(|| ira_db::get_default_variant(&s.db, db_id));
                drop(s);
                if !state_clone.borrow().running_games.lock().unwrap().contains_key(&db_id) {
                    let _ = super::play_button::launch_game(&state_clone, db_id, vid);
                    let _ = state_clone.borrow().sender.send(crate::AppMessage::GameStarted(db_id, item_variant_id));
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
                rebuild_sidebar_and_show_grid(&s);
            }
        });
    });

    let state_clone = state.clone();
    add_btn.connect_clicked(move |_| {
        show_add_game_dialog(&state_clone);
    });

    let state_clone = state.clone();
    window.connect_close_request(move |_| {
        let close_to_bg = state_clone.borrow().cfg.close_to_background;
        if close_to_bg {
            show_close_choice_dialog(&state_clone);
            glib::Propagation::Stop
        } else {
            let app = state_clone.borrow().window.application()
                .and_then(|a| a.downcast::<adw::Application>().ok());
            if let Some(app) = app {
                app.quit();
            }
            glib::Propagation::Proceed
        }
    });
}
