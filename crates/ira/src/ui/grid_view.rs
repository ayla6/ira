use gtk4::prelude::*;
use crate::Game;
use crate::strings as S;
use ira_models::{GroupSelection, SortMode};

use std::sync::atomic::{AtomicI64, Ordering};
use super::state::SharedState;
use super::recent_row::build_recent_row;
use super::game_item::GameItem;
use super::grid_bin::GridBin;
use super::message_helpers::switch_to_game;
use super::sidebar::scroll_to_row;
use super::context_menu::show_game_context_menu;
use super::helpers::clear_children;
use super::filter::filtered_games;

pub(super) fn is_stale(vbox: &gtk4::Box, db_id: i64, variant_id: i64) -> bool {
    let db_mismatch = unsafe { vbox.data::<AtomicI64>("game-db-id") }
        .map(|ptr| unsafe { ptr.as_ref() }.load(Ordering::Relaxed) != db_id)
        .unwrap_or(false);
    let var_mismatch = unsafe { vbox.data::<AtomicI64>("game-variant-id") }
        .map(|ptr| unsafe { ptr.as_ref() }.load(Ordering::Relaxed) != variant_id)
        .unwrap_or(false);
    db_mismatch || var_mismatch
}

pub(super) fn queue_cover_load(pic: gtk4::Picture, path: String, w: i32, h: i32, db_id: i64, variant_id: i64, vbox: gtk4::Box) {
    queue_cover_load_priority(pic, path, (w, h), db_id, variant_id, vbox, glib::Priority::LOW);
}

pub(super) fn queue_cover_load_priority(pic: gtk4::Picture, path: String, dims: (i32, i32), db_id: i64, variant_id: i64, vbox: gtk4::Box, priority: glib::Priority) {
    let (w, h) = dims;
    let _s = tracing::info_span!("queue_cover_load", path = %path, w, h, db_id, variant_id).entered();
    if ira_images::cached_texture(&path).is_some() {
        if !is_stale(&vbox, db_id, variant_id) {
            ira_images::set_picture_natural(&pic, &path, w, h);
        }
        return;
    }
    let pic_weak = pic.downgrade();
    let vbox_weak = vbox.downgrade();
    ira_images::load_texture_async_with_priority(&path, priority, move |texture| {
        if let (Some(pic), Some(vbox)) = (pic_weak.upgrade(), vbox_weak.upgrade()) {
            if !is_stale(&vbox, db_id, variant_id) {
                if let Some(t) = texture {
                    let paintable = ira_images::ScaledPaintable::new(&t, w, h);
                    pic.set_paintable(Some(&paintable));
                }
            }
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
        unsafe { vbox.set_data::<AtomicI64>("game-variant-id", AtomicI64::new(0)) };
        unsafe { vbox.set_data::<gtk4::Label>("name-label", name_label.clone()) };

        let sc = state_for_setup.clone();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |gesture, _, _, _| {
            let widget = gesture.widget().unwrap();
            if let Some(ptr) = unsafe { widget.data::<AtomicI64>("game-db-id") } {
                let db_id = unsafe { ptr.as_ref() }.load(Ordering::Relaxed);
                if db_id != 0 {
                    let variant_id = unsafe { widget.data::<AtomicI64>("game-variant-id") }
                        .and_then(|ptr| {
                            let v = unsafe { ptr.as_ref() }.load(Ordering::Relaxed);
                            if v > 0 { Some(v) } else { None }
                        });
                    switch_to_game(&sc, db_id, variant_id);
                    scroll_to_row(&sc, db_id, variant_id);
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
                    let variant_id = unsafe { widget.data::<AtomicI64>("game-variant-id") }
                        .and_then(|ptr| {
                            let v = unsafe { ptr.as_ref() }.load(Ordering::Relaxed);
                            if v > 0 { Some(v) } else { None }
                        });
                    let game = sc2
                        .borrow()
                        .games
                        .iter()
                        .find(|g| g.db_id == db_id && g.variant_id == variant_id)
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
            if let Some(ptr) = unsafe { vbox.data::<AtomicI64>("game-variant-id") } {
                unsafe { ptr.as_ref() }.store(game.variant_id.unwrap_or(0), Ordering::Relaxed);
            }
            if !game.grid_path.is_empty() {
                queue_cover_load(pic.clone(), game.grid_path.clone(), cover_width, cover_height, game.db_id, game.variant_id.unwrap_or(0), vbox.clone());
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
        if let Some(ptr) = unsafe { vbox.data::<AtomicI64>("game-variant-id") } {
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


