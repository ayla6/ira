use gtk4::prelude::*;
use crate::Game;
use crate::strings as S;
use ira_models::{GroupSelection, SortMode};

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};
use super::state::SharedState;
use super::game_item::GameItem;
use super::grid_bin::GridBin;
use super::message_handler::switch_to_game;
use super::sidebar::scroll_to_row;
use super::context_menu::show_game_context_menu;
use super::helpers::clear_children;
use super::filter::filtered_games;

struct PendingCover {
    pic: gtk4::Picture,
    path: String,
    w: i32,
    h: i32,
    db_id: i64,
    vbox: gtk4::Box,
}

thread_local! {
    static COVER_QUEUE: RefCell<VecDeque<PendingCover>> = const { RefCell::new(VecDeque::new()) };
    static COVER_PROCESSOR_RUNNING: Cell<bool> = const { Cell::new(false) };
}

fn queue_cover_load(pic: gtk4::Picture, path: String, w: i32, h: i32, db_id: i64, vbox: gtk4::Box) {
    // If texture is already cached, set it immediately to avoid flash
    if ira_images::cached_texture(&path).is_some() {
        let stale = unsafe { vbox.data::<AtomicI64>("game-db-id") }
            .map(|ptr| unsafe { ptr.as_ref() }.load(Ordering::Relaxed) != db_id)
            .unwrap_or(false);
        if !stale {
            ira_images::set_picture_natural(&pic, &path, w, h);
        }
        return;
    }
    COVER_QUEUE.with(|q| q.borrow_mut().push_back(PendingCover { pic, path, w, h, db_id, vbox }));
    COVER_PROCESSOR_RUNNING.with(|r| {
        if !r.get() {
            r.set(true);
            glib::source::idle_add_local_full(glib::Priority::LOW, move || {
                let req = COVER_QUEUE.with(|q| q.borrow_mut().pop_front());
                if let Some(req) = req {
                    let stale = unsafe { req.vbox.data::<AtomicI64>("game-db-id") }
                        .map(|ptr| unsafe { ptr.as_ref() }.load(Ordering::Relaxed) != req.db_id)
                        .unwrap_or(false);
                    if !stale {
                        ira_images::set_picture_natural(&req.pic, &req.path, req.w, req.h);
                    }
                }
                let empty = COVER_QUEUE.with(|q| q.borrow().is_empty());
                if empty {
                    COVER_PROCESSOR_RUNNING.with(|r| r.set(false));
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }
    });
}

fn badge_text(game: &Game, mode: SortMode) -> Option<String> {
    match mode {
        SortMode::Alphabetical => None,
        SortMode::Completion => {
            if game.total_count == 0 { None }
            else { Some(format!("{}%", (game.completion_pct() as u8))) }
        }
        SortMode::HoursPlayed => {
            if game.playtime <= 0.0 { None }
            else { Some(super::game_display::format_playtime(game.playtime)) }
        }
        SortMode::LastPlayed => {
            if game.last_played == 0 { None }
            else {
                chrono::DateTime::from_timestamp(game.last_played, 0)
                    .map(|dt| dt.format("%b %-d").to_string())
            }
        }
        SortMode::ReleaseDate => {
            if game.release_timestamp == 0 { None }
            else {
                chrono::DateTime::from_timestamp(game.release_timestamp, 0)
                    .map(|dt| dt.format("%Y").to_string())
            }
        }
        SortMode::MetacriticScore => {
            if game.metacritic_score < 0 { None }
            else { Some(game.metacritic_score.to_string()) }
        }
        SortMode::SteamReview => {
            if game.steam_review_score < 0 { None }
            else { Some(format!("{}%", game.steam_review_score)) }
        }
    }
}

pub fn show_grid_view(state: &SharedState) {
    state.borrow_mut().selected_id.clear();

    let content_scroll = state.borrow().content_scroll.clone();
    let grid_header = state.borrow().grid_header.clone();

    content_scroll.vadjustment().set_value(0.0);
    clear_children(&grid_header);

    let cover_width = state.borrow().cfg.grid_cover_width.clamp(100, 350);
    let show_hidden = state.borrow().cfg.show_hidden_games;
    let cover_height = ((cover_width as f64) * 1.5) as i32;

    let header_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    header_box.set_margin_start(16);
    header_box.set_margin_end(16);
    header_box.set_margin_top(16);

    let searching = !state.borrow().search_query.is_empty();
    let selected_group = state.borrow().selected_group.clone();
    let show_recent = !searching && selected_group == GroupSelection::AllGames;

    let games = filtered_games(state);

    if show_recent {
        let mut recent: Vec<Game> = state
            .borrow()
            .games
            .iter()
            .filter(|g| g.last_played > 0 && (!g.hidden || show_hidden))
            .cloned()
            .collect();
        recent.sort_by(|a, b| b.last_played.cmp(&a.last_played));
        recent.truncate(8);

        if !recent.is_empty() {
            header_box.append(&build_recent_row(state, &recent, cover_height));
        }
    }

    let heading_text = if searching {
        format!("Search: \"{}\"", state.borrow().search_query)
    } else {
        match &selected_group {
            GroupSelection::AllGames => S::ALL_GAMES.to_string(),
            GroupSelection::Uncategorized => "Uncategorized".to_string(),
            GroupSelection::Collection(id) => {
                state.borrow().groups.iter()
                    .find(|g| g.id == *id)
                    .map(|g| g.name.clone())
                    .unwrap_or_else(|| S::ALL_GAMES.to_string())
            }
        }
    };

    let heading = gtk4::Label::new(Some(&heading_text));
    heading.set_xalign(0.0);
    heading.add_css_class("section-title");
    heading.set_margin_top(if show_recent && state.borrow().games.iter().any(|g| g.last_played > 0 && (!g.hidden || show_hidden)) { 20 } else { 0 });
    heading.set_margin_bottom(8);
    header_box.append(&heading);

    let sort_mode = state.borrow().cfg.sort_mode;

    let factory = gtk4::SignalListItemFactory::new();

    let state_for_setup = state.clone();
    factory.connect_setup(move |_, list_item_obj| {
        let list_item = list_item_obj.downcast_ref::<gtk4::ListItem>().unwrap();
        list_item.set_activatable(false);
        list_item.set_selectable(false);

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        vbox.set_valign(gtk4::Align::Start);
        vbox.set_halign(gtk4::Align::Center);
        vbox.set_margin_start(8);
        vbox.set_margin_end(8);
        vbox.set_margin_top(8);
        vbox.set_margin_bottom(8);
        vbox.set_size_request(cover_width, cover_height);
        vbox.add_css_class("cover-item");
        vbox.set_overflow(gtk4::Overflow::Visible);

        let overlay = gtk4::Overlay::new();
        overlay.set_overflow(gtk4::Overflow::Visible);

        let pic = gtk4::Picture::new();
        pic.set_content_fit(gtk4::ContentFit::Cover);
        pic.set_size_request(cover_width, cover_height);
        pic.add_css_class("game-cover-pic");
        let placeholder = ira_images::ScaledPaintable::new_empty(cover_width, cover_height);
        pic.set_paintable(Some(&placeholder));
        overlay.set_child(Some(&pic));

        let name_label = gtk4::Label::new(None);
        name_label.set_wrap(true);
        name_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        name_label.set_max_width_chars(15);
        name_label.set_halign(gtk4::Align::Center);
        name_label.set_valign(gtk4::Align::Center);
        name_label.set_margin_start(6);
        name_label.set_margin_end(6);
        name_label.add_css_class("cover-name-fallback");
        name_label.set_visible(false);
        overlay.add_overlay(&name_label);

        vbox.append(&overlay);

        unsafe { vbox.set_data::<AtomicI64>("game-db-id", AtomicI64::new(0)) };
        unsafe { vbox.set_data::<gtk4::Label>("name-label", name_label.clone()) };

        let sc = state_for_setup.clone();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |gesture, _, _, _| {
            let widget = gesture.widget().unwrap();
            if let Some(ptr) = unsafe { widget.data::<AtomicI64>("game-db-id") } {
                let db_id = unsafe { ptr.as_ref() }.load(Ordering::Relaxed);
                if db_id != 0 {
                    switch_to_game(&sc, db_id);
                    scroll_to_row(&sc, db_id);
                }
            }
        });
        vbox.add_controller(click);

        let sc2 = state_for_setup.clone();
        let right_click = gtk4::GestureClick::new();
        right_click.set_button(3);
        right_click.connect_pressed(move |gesture, _, x, y| {
            let widget = gesture.widget().unwrap();
            if let Some(ptr) = unsafe { widget.data::<AtomicI64>("game-db-id") } {
                let db_id = unsafe { ptr.as_ref() }.load(Ordering::Relaxed);
                if db_id != 0 {
                    let game = sc2
                        .borrow()
                        .games
                        .iter()
                        .find(|g| g.db_id == db_id)
                        .cloned();
                    if let Some(game) = game {
                        show_game_context_menu(&sc2, &game, &widget, x, y, None::<&gtk4::ListBoxRow>);
                    }
                }
            }
        });
        vbox.add_controller(right_click);

        list_item.set_child(Some(&vbox));
    });

    factory.connect_bind(move |_, list_item_obj| {
        let list_item = list_item_obj.downcast_ref::<gtk4::ListItem>().unwrap();
        let child = list_item.child().unwrap();
        let vbox = child.downcast_ref::<gtk4::Box>().unwrap();
        let overlay_widget = vbox.first_child().unwrap();
        let overlay = overlay_widget.downcast_ref::<gtk4::Overlay>().unwrap();
        let pic_widget = overlay.child().unwrap();
        let pic = pic_widget.downcast_ref::<gtk4::Picture>().unwrap();

        let name_label = unsafe { vbox.data::<gtk4::Label>("name-label") }
            .map(|ptr| unsafe { ptr.as_ref() }.clone());

        let game_item = list_item
            .item()
            .unwrap()
            .downcast::<GameItem>()
            .unwrap();

        if let Some(game) = game_item.game() {
            if let Some(ptr) = unsafe { vbox.data::<AtomicI64>("game-db-id") } {
                unsafe { ptr.as_ref() }.store(game.db_id, Ordering::Relaxed);
            }
            if !game.grid_path.is_empty() {
                queue_cover_load(pic.clone(), game.grid_path.clone(), cover_width, cover_height, game.db_id, vbox.clone());
                if let Some(ref label) = name_label {
                    label.set_visible(false);
                }
            } else {
                let placeholder = ira_images::ScaledPaintable::new_empty(cover_width, cover_height);
                pic.set_paintable(Some(&placeholder));
                if let Some(ref label) = name_label {
                    label.set_text(&game.name);
                    label.set_visible(true);
                }
            }

            if let Some(text) = badge_text(&game, sort_mode) {
                let badge = gtk4::Label::new(Some(&text));
                badge.set_valign(gtk4::Align::End);
                badge.set_halign(gtk4::Align::Center);
                badge.set_margin_bottom(-12);
                badge.add_css_class("cover-badge");
                overlay.add_overlay(&badge);
                unsafe { vbox.set_data::<gtk4::Label>("badge", badge) };
            }
        }
    });

    factory.connect_unbind(move |_, list_item_obj| {
        let list_item = list_item_obj.downcast_ref::<gtk4::ListItem>().unwrap();
        let child = list_item.child().unwrap();
        let vbox = child.downcast_ref::<gtk4::Box>().unwrap();
        let overlay_widget = vbox.first_child().unwrap();
        let overlay = overlay_widget.downcast_ref::<gtk4::Overlay>().unwrap();
        let pic_widget = overlay.child().unwrap();
        let pic = pic_widget.downcast_ref::<gtk4::Picture>().unwrap();

        if let Some(ptr) = unsafe { vbox.data::<AtomicI64>("game-db-id") } {
            unsafe { ptr.as_ref() }.store(0, Ordering::Relaxed);
        }
        let placeholder = ira_images::ScaledPaintable::new_empty(cover_width, cover_height);
        pic.set_paintable(Some(&placeholder));

        if let Some(label) = unsafe { vbox.data::<gtk4::Label>("name-label") } {
            let label = unsafe { label.as_ref() };
            label.set_text("");
            label.set_visible(false);
        }

        if let Some(badge) = unsafe { vbox.steal_data::<gtk4::Label>("badge") } {
            overlay.remove_overlay(&badge);
        }
    });

    let store = gio::ListStore::new::<GameItem>();
    for game in &games {
        store.append(&GameItem::new(game));
    }
    state.borrow_mut().grid_store = store.clone();

    let selection_model = gtk4::NoSelection::new(Some(store.upcast::<gio::ListModel>()));
    let grid = gtk4::GridView::new(Some(selection_model), Some(factory));
    grid.set_min_columns(1);
    grid.set_max_columns(30);
    grid.set_hexpand(true);
    grid.set_halign(gtk4::Align::Fill);
    grid.add_css_class("game-grid");
    grid.remove_css_class("view");

    let n_items = games.len() as u32;
    let row_h = cover_height + 16;
    let col_nat = cover_width + 16;
    let bin = GridBin::new(&grid, &header_box.upcast(), row_h, n_items, col_nat);
    bin.set_hexpand(true);
    bin.set_halign(gtk4::Align::Fill);
    bin.set_vexpand(true);
    bin.set_valign(gtk4::Align::Fill);

    content_scroll.set_child(Some(&bin));
}

fn build_recent_row(
    state: &SharedState,
    recent: &[Game],
    cover_height: i32,
) -> gtk4::Widget {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);

    let title_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    title_row.set_hexpand(true);

    let title = gtk4::Label::new(Some("Recently played"));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("section-title");
    title_row.append(&title);

    let left_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    left_btn.add_css_class("flat");
    left_btn.set_sensitive(false);

    let right_btn = gtk4::Button::from_icon_name("go-next-symbolic");
    right_btn.add_css_class("flat");

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    btn_box.append(&left_btn);
    btn_box.append(&right_btn);
    title_row.append(&btn_box);

    vbox.append(&title_row);

    let spacing = 12;
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, spacing);

    let mut game_widths: Vec<i32> = Vec::with_capacity(recent.len());
    for (i, game) in recent.iter().enumerate() {
        let (w, h, use_header) = if i == 0 {
            let w = ((cover_height as f64) * 460.0 / 215.0) as i32;
            (w, cover_height, true)
        } else {
            let w = ((cover_height as f64) * 2.0 / 3.0) as i32;
            (w, cover_height, false)
        };
        game_widths.push(w);
        let path = if use_header { ira_parser::full_image_path(&game.header_path) } else { game.grid_path.clone() };
        let item = build_cover(state, game, &path, w, h);
        hbox.append(&item);
    }

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);
    scrolled.set_hexpand(true);
    scrolled.set_vexpand(false);
    scrolled.set_valign(gtk4::Align::Start);
    scrolled.set_margin_top(4);
    scrolled.set_margin_bottom(4);
    scrolled.add_css_class("recent-scroll");
    scrolled.set_child(Some(&hbox));

    vbox.append(&scrolled);

    let adj = scrolled.hadjustment();

    let step_widths = Rc::new(
        game_widths.iter().map(|w| w + spacing).collect::<Vec<i32>>(),
    );
    let max_scroll = {
        let total: i32 = step_widths.iter().sum();
        total - spacing
    };

    let sw = step_widths.clone();
    let ms = max_scroll;
    let adj_clone = adj.clone();
    right_btn.connect_clicked(move |_| {
        let cur = adj_clone.value() as i32;
        let mut target = cur;
        for sw_i in sw.iter() {
            if target < cur + sw_i {
                target = cur + sw_i;
                break;
            }
            target += sw_i;
        }
        adj_clone.set_value(target.min(ms) as f64);
    });

    let sw = step_widths.clone();
    let adj_clone = adj.clone();
    left_btn.connect_clicked(move |_| {
        let cur = adj_clone.value() as i32;
        let mut target = 0;
        let mut running = 0;
        for sw_i in sw.iter() {
            let next = running + sw_i;
            if next >= cur {
                target = running;
                break;
            }
            running = next;
        }
        adj_clone.set_value(target.max(0) as f64);
    });

    let left_btn2 = left_btn.clone();
    let right_btn2 = right_btn.clone();
    let max_scroll2 = max_scroll;
    adj.connect_value_changed(move |adj| {
        let v = adj.value() as i32;
        left_btn2.set_sensitive(v > 0);
        right_btn2.set_sensitive(v < max_scroll2);
    });

    vbox.upcast()
}

fn build_cover(
    state: &SharedState,
    game: &Game,
    image_path: &str,
    w: i32,
    h: i32,
) -> gtk4::Widget {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_valign(gtk4::Align::Start);
    vbox.set_halign(gtk4::Align::Center);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);
    vbox.add_css_class("cover-item");
    vbox.set_size_request(w, h);
    vbox.set_overflow(gtk4::Overflow::Visible);

    let pic = gtk4::Picture::new();
    pic.set_content_fit(gtk4::ContentFit::Cover);
    pic.set_size_request(w, h);
    pic.add_css_class("game-cover-pic");
    if !image_path.is_empty() {
        queue_cover_load(pic.clone(), image_path.to_string(), w, h, game.db_id, gtk4::Box::new(gtk4::Orientation::Vertical, 0));
    }

    vbox.append(&pic);

    let state_clone = state.clone();
    let db_id = game.db_id;
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| {
        switch_to_game(&state_clone, db_id);
        scroll_to_row(&state_clone, db_id);
    });
    vbox.add_controller(click);

    let sc = state.clone();
    let gc = game.clone();
    let v_weak = vbox.downgrade();
    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    right_click.connect_pressed(move |_, _, x, y| {
        if let Some(v) = v_weak.upgrade() {
            show_game_context_menu(&sc, &gc, &v, x, y, None::<&gtk4::ListBoxRow>);
        }
    });
    vbox.add_controller(right_click);

    vbox.upcast()
}
