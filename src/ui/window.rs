use crate::config::Config;
use crate::db::DbConn;
use crate::api::SteamClient;
use crate::strings as S;
use crate::watcher::AchievementWatcher;
use crate::AppMessage;
use crate::AppSender;
use gtk4::prelude::*;
use adw::prelude::*;
use crate::Game;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use super::state::{AppState, SharedState};
use super::css::APP_CSS;
use super::sidebar::{rebuild_sidebar, select_row_silently};
use super::grid_view::show_grid_view;
use super::message_handler::switch_to_game;
use super::dialogs::show_settings_dialog;
use super::mass_match_dialog::show_mass_match_dialog;
use super::add_game::show_add_game_dialog;
use super::background::show_close_choice_dialog;

pub fn build_ui(
    app: &adw::Application,
    games: Vec<Game>,
    cfg: Config,
    steam: Arc<SteamClient>,
    watcher: Option<AchievementWatcher>,
    db: DbConn,
    sender: AppSender,
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
        lutris_watcher: None,
        shadps4_watcher: None,
        db,
        sender,
        game_names,
        content_unloaded: false,
        restoring: false,
        running_games: Arc::new(Mutex::new(HashMap::new())),
        grid_refresh_pending: false,
        view_generation: 0,
        settings_data: None,
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

    let menu_btn = gtk4::MenuButton::new();
    menu_btn.set_icon_name("open-menu-symbolic");
    menu_btn.set_tooltip_text(Some(S::MENU));
    menu_btn.add_css_class("flat");
    header_bar.pack_end(&menu_btn);

    let (popover, settings_btn) = build_menu_popover(state);
    menu_btn.set_popover(Some(&popover));

    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some(S::ADD_GAME));
    add_btn.add_css_class("flat");
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

    if !state.borrow().content_unloaded {
        let row = state.borrow().game_list.row_at_index(0);
        select_row_silently(state, row.as_ref());
        show_grid_view(state);
    }

    connect_window_signals(state, &window, &game_list, &add_btn, &settings_btn);
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
        let s = state_clone.borrow();
        for (i, g) in s.games.iter().enumerate() {
            if i < s.rows.len() {
                s.rows[i].row.set_visible(!g.hidden || active);
            }
        }
        drop(s);
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

fn connect_window_signals(
    state: &SharedState,
    window: &adw::ApplicationWindow,
    game_list: &gtk4::ListBox,
    add_btn: &gtk4::Button,
    settings_btn: &gtk4::Button,
) {
    let state_clone = state.clone();
    game_list.connect_row_selected(move |_list, row| {
        let s = state_clone.borrow();
        if s.restoring {
            return;
        }
        let Some(row) = row else { return; };
        let idx = row.index();
        if idx == 0 {
            drop(s);
            state_clone.borrow_mut().selected_id.clear();
            show_grid_view(&state_clone);
        } else if idx >= 1 {
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

    let state_clone = state.clone();
    window.connect_notify_local(Some("is-active"), move |_, _| {
        let is_active = state_clone.borrow().window.is_active();
        if !is_active {
            return;
        }
        let sender = state_clone.borrow().sender.clone();
        std::thread::spawn(move || {
            if let Ok(data) = crate::platforms::lutris::load_lutris_playtime() {
                let _ = sender.send(AppMessage::LutrisDataChanged(data));
            }
        });
    });
}
