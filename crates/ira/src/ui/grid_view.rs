use crate::Game;
use gtk4::prelude::*;
use ira_models::{GroupSelection, SortMode};

use super::context_menu::show_game_context_menu;
use super::css::*;
use super::filter::filtered_games;
use super::game_item::GameItem;
use super::helpers::clear_children;
use super::message_helpers::switch_to_game;
use super::recent_row::build_recent_row;
use super::state::SharedState;
use super::virtual_grid::{BindFn, SetupFn, UnbindFn, VirtualGrid};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};

pub(super) fn is_stale(vbox: &gtk4::Box, db_id: i64, variant_id: i64) -> bool {
    let db_mismatch = unsafe { vbox.data::<AtomicI64>("game-db-id") }
        .map(|ptr| unsafe { ptr.as_ref() }.load(Ordering::Relaxed) != db_id)
        .unwrap_or(false);
    let var_mismatch = unsafe { vbox.data::<AtomicI64>("game-variant-id") }
        .map(|ptr| unsafe { ptr.as_ref() }.load(Ordering::Relaxed) != variant_id)
        .unwrap_or(false);
    db_mismatch || var_mismatch
}

pub(super) fn queue_cover_load(
    pic: gtk4::Picture,
    path: String,
    w: i32,
    h: i32,
    db_id: i64,
    variant_id: i64,
    vbox: gtk4::Box,
) {
    queue_cover_load_priority(
        pic,
        path,
        (w, h),
        db_id,
        variant_id,
        vbox,
        glib::Priority::LOW,
    );
}

pub(super) fn queue_cover_load_priority(
    pic: gtk4::Picture,
    path: String,
    dims: (i32, i32),
    db_id: i64,
    variant_id: i64,
    vbox: gtk4::Box,
    priority: glib::Priority,
) {
    let (w, h) = dims;
    let _s =
        tracing::info_span!("queue_cover_load", path = %path, w, h, db_id, variant_id).entered();
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
            if game.total_count == 0 {
                None
            } else {
                Some(format!("{}%", (game.completion_pct() as u8)))
            }
        }
        SortMode::HoursPlayed => {
            if game.playtime <= 0.0 {
                None
            } else {
                Some(super::game_display::format_playtime(game.playtime))
            }
        }
        SortMode::LastPlayed => {
            if game.last_played == 0 {
                None
            } else {
                chrono::DateTime::from_timestamp(game.last_played, 0)
                    .map(|dt| dt.format("%b %-d").to_string())
            }
        }
        SortMode::ReleaseDate => {
            if game.release_timestamp == 0 {
                None
            } else {
                chrono::DateTime::from_timestamp(game.release_timestamp, 0)
                    .map(|dt| dt.format("%Y").to_string())
            }
        }
        SortMode::MetacriticScore => {
            if game.metacritic_score < 0 {
                None
            } else {
                Some(game.metacritic_score.to_string())
            }
        }
        SortMode::SteamReview => {
            if game.steam_review_score < 0 {
                None
            } else {
                Some(format!("{}%", game.steam_review_score))
            }
        }
    }
}

fn build_grid_header(state: &SharedState, cover_height: i32) -> gtk4::Box {
    let header_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    header_box.set_margin_start(16);
    header_box.set_margin_end(16);
    header_box.set_margin_top(16);

    let searching = !state.borrow().search_query.is_empty();
    let selected_group = state.borrow().selected_group.clone();
    let show_recent = !searching && selected_group == GroupSelection::AllGames;

    if show_recent {
        let show_hidden = state.borrow().cfg.show_hidden_games;
        let mut recent: Vec<Game> = state
            .borrow()
            .games
            .iter()
            .filter(|g| g.last_played > 0 && (!g.hidden || show_hidden))
            .cloned()
            .collect();
        recent.sort_by_key(|a| std::cmp::Reverse(a.last_played));
        recent.truncate(8);

        if !recent.is_empty() {
            header_box.append(&build_recent_row(state, &recent, cover_height));
        }
    }

    let heading_text = if searching {
        crate::tr!("Search: \"{}\"").replacen("{}", &state.borrow().search_query, 1)
    } else {
        match &selected_group {
            GroupSelection::AllGames => crate::tr!("All games"),
            GroupSelection::Uncategorized => crate::tr!("Uncategorized"),
            GroupSelection::Collection(id) => state
                .borrow()
                .groups
                .iter()
                .find(|g| g.id == *id)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| crate::tr!("All games")),
        }
    };

    let heading = gtk4::Label::new(Some(&heading_text));
    heading.set_xalign(0.0);
    heading.add_css_class(CSS_SECTION_TITLE);
    let show_hidden = state.borrow().cfg.show_hidden_games;
    heading.set_margin_top(
        if show_recent
            && state
                .borrow()
                .games
                .iter()
                .any(|g| g.last_played > 0 && (!g.hidden || show_hidden))
        {
            20
        } else {
            0
        },
    );
    heading.set_margin_bottom(8);
    header_box.append(&heading);

    header_box
}

fn make_setup(state: &SharedState, item_size: Rc<Cell<(i32, i32)>>) -> SetupFn {
    let state = state.clone();
    Rc::new(move || {
        let (cover_width, cover_height) = item_size.get();
        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        vbox.set_valign(gtk4::Align::Start);
        vbox.set_halign(gtk4::Align::Center);
        vbox.set_size_request(cover_width, cover_height);
        vbox.add_css_class(CSS_COVER_ITEM);
        vbox.set_overflow(gtk4::Overflow::Visible);

        let overlay = gtk4::Overlay::new();
        overlay.set_overflow(gtk4::Overflow::Visible);

        let pic = gtk4::Picture::new();
        pic.set_content_fit(gtk4::ContentFit::Cover);
        pic.set_size_request(cover_width, cover_height);
        pic.add_css_class(CSS_GAME_COVER_PIC);
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
        name_label.add_css_class(CSS_COVER_NAME_FALLBACK);
        name_label.set_visible(false);
        overlay.add_overlay(&name_label);

        vbox.append(&overlay);

        unsafe { vbox.set_data::<AtomicI64>("game-db-id", AtomicI64::new(0)) };
        unsafe { vbox.set_data::<AtomicI64>("game-variant-id", AtomicI64::new(0)) };
        unsafe { vbox.set_data::<gtk4::Label>("name-label", name_label.clone()) };

        let sc = state.clone();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |gesture, _, _, _| {
            let widget = gesture.widget().unwrap();
            if let Some(ptr) = unsafe { widget.data::<AtomicI64>("game-db-id") } {
                let db_id = unsafe { ptr.as_ref() }.load(Ordering::Relaxed);
                if db_id != 0 {
                    let variant_id = unsafe { widget.data::<AtomicI64>("game-variant-id") }
                        .and_then(|ptr| {
                            let v = unsafe { ptr.as_ref() }.load(Ordering::Relaxed);
                            if v > 0 {
                                Some(v)
                            } else {
                                None
                            }
                        });
                    switch_to_game(&sc, db_id, variant_id);
                    super::sidebar::scroll_to_row(&sc, db_id, variant_id);
                }
            }
        });
        vbox.add_controller(click);

        let sc2 = state.clone();
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
                            if v > 0 {
                                Some(v)
                            } else {
                                None
                            }
                        });
                    let game = sc2
                        .borrow()
                        .games
                        .iter()
                        .find(|g| g.db_id == db_id && g.variant_id == variant_id)
                        .cloned();
                    if let Some(game) = game {
                        show_game_context_menu(
                            &sc2,
                            &game,
                            &widget,
                            x,
                            y,
                            None::<&gtk4::ListBoxRow>,
                        );
                    }
                }
            }
        });
        vbox.add_controller(right_click);

        vbox.upcast::<gtk4::Widget>()
    })
}

fn make_bind(item_size: Rc<Cell<(i32, i32)>>, sort_mode: SortMode) -> BindFn {
    Rc::new(move |widget, game| {
        let _span = tracing::info_span!("grid_bind", db_id = game.db_id).entered();
        let (cover_width, cover_height) = item_size.get();
        let vbox = widget.downcast_ref::<gtk4::Box>().unwrap();
        let overlay_widget = vbox.first_child().unwrap();
        let overlay = overlay_widget.downcast_ref::<gtk4::Overlay>().unwrap();
        let pic_widget = overlay.child().unwrap();
        let pic = pic_widget.downcast_ref::<gtk4::Picture>().unwrap();
        vbox.set_size_request(cover_width, cover_height);
        pic.set_size_request(cover_width, cover_height);

        let name_label = unsafe { vbox.data::<gtk4::Label>("name-label") }
            .map(|ptr| unsafe { ptr.as_ref() }.clone());

        if let Some(ptr) = unsafe { vbox.data::<AtomicI64>("game-db-id") } {
            unsafe { ptr.as_ref() }.store(game.db_id, Ordering::Relaxed);
        }
        if let Some(ptr) = unsafe { vbox.data::<AtomicI64>("game-variant-id") } {
            unsafe { ptr.as_ref() }.store(game.variant_id.unwrap_or(0), Ordering::Relaxed);
        }
        if !game.grid_path.is_empty() {
            queue_cover_load(
                pic.clone(),
                game.grid_path.clone(),
                cover_width,
                cover_height,
                game.db_id,
                game.variant_id.unwrap_or(0),
                vbox.clone(),
            );
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

        if let Some(badge) = unsafe { vbox.steal_data::<gtk4::Label>("badge") } {
            overlay.remove_overlay(&badge);
        }
        if let Some(text) = badge_text(game, sort_mode) {
            let badge = gtk4::Label::new(Some(&text));
            badge.set_valign(gtk4::Align::End);
            badge.set_halign(gtk4::Align::Center);
            badge.set_margin_bottom(-12);
            badge.add_css_class(CSS_COVER_BADGE);
            overlay.add_overlay(&badge);
            unsafe { vbox.set_data::<gtk4::Label>("badge", badge) };
        }
    })
}

fn make_unbind(item_size: Rc<Cell<(i32, i32)>>) -> UnbindFn {
    Rc::new(move |widget| {
        let (cover_width, cover_height) = item_size.get();
        let vbox = widget.downcast_ref::<gtk4::Box>().unwrap();
        let overlay_widget = vbox.first_child().unwrap();
        let overlay = overlay_widget.downcast_ref::<gtk4::Overlay>().unwrap();
        let pic_widget = overlay.child().unwrap();
        let pic = pic_widget.downcast_ref::<gtk4::Picture>().unwrap();
        vbox.set_size_request(cover_width, cover_height);
        pic.set_size_request(cover_width, cover_height);

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
    })
}

fn build_grid_view(
    state: &SharedState,
    games: &[Game],
    cover_width: i32,
    _cover_height: i32,
    sort_mode: SortMode,
    header_box: &gtk4::Box,
    content_scroll: &gtk4::ScrolledWindow,
) {
    let _span = tracing::info_span!("build_grid_view", count = games.len()).entered();
    let store = gio::ListStore::new::<GameItem>();
    for game in games {
        store.append(&GameItem::new(game));
    }
    state.borrow_mut().grid_store = store.clone();

    let grid = VirtualGrid::new(cover_width);
    let item_size = grid.item_size_cell();
    grid.set_factory(
        make_setup(state, item_size.clone()),
        make_bind(item_size.clone(), sort_mode),
        make_unbind(item_size),
    );

    let state_clone = state.clone();
    let grid_weak = grid.downgrade();
    let gen = state.borrow().view_generation;
    grid.set_size_changed(Rc::new(move |_w, h| {
        if state_clone.borrow().view_generation != gen {
            return;
        }
        state_clone.borrow_mut().grid_item_height.set(h);
        if let Some(grid) = grid_weak.upgrade() {
            let new_header = build_grid_header(&state_clone, h);
            grid.set_header(Some(&new_header));
        }
    }));

    grid.set_model(&store);
    grid.set_header(Some(header_box));
    grid.set_hexpand(true);
    grid.set_halign(gtk4::Align::Fill);
    grid.set_vexpand(true);
    grid.set_valign(gtk4::Align::Fill);
    grid.add_css_class(CSS_GAME_GRID);

    content_scroll.set_child(Some(&grid));
}

pub fn show_grid_view(state: &SharedState) {
    let _span = tracing::info_span!("show_grid_view").entered();
    {
        let mut s = state.borrow_mut();
        s.selected_id.clear();
        s.view_generation += 1;
        s.loading_status = None;
        s.loading_progress = None;
    }

    let content_scroll = state.borrow().content_scroll.clone();
    let grid_header = state.borrow().grid_header.clone();

    content_scroll.vadjustment().set_value(0.0);
    clear_children(&grid_header);

    let min_w = 110;
    let stored_h = state.borrow().grid_item_height.get();
    let item_h = if stored_h > 0 {
        stored_h
    } else {
        let (cw, ch) = {
            let cs = state.borrow().content_scroll.clone();
            let w = cs.width();
            let h = cs.height();
            if w > 0 {
                (w, h)
            } else {
                let win = state.borrow().window.clone();
                let surface = win.surface();
                surface
                    .map(|s| (s.width(), s.height()))
                    .unwrap_or((800, 600))
            }
        };
        let (_, h) = VirtualGrid::compute_item_size(cw.max(1), ch.max(1), min_w);
        h
    };

    let header_box = build_grid_header(state, item_h);

    let sort_mode = state.borrow().cfg.sort_mode;
    let games = filtered_games(state);

    if games.is_empty() && !state.borrow().search_query.is_empty() {
        show_empty_search_view(&content_scroll);
        return;
    }

    build_grid_view(
        state,
        &games,
        min_w,
        item_h,
        sort_mode,
        &header_box,
        &content_scroll,
    );
}

pub fn show_loading_view(state: &SharedState, status: &str, completed: usize, total: usize) {
    let content_scroll = state.borrow().content_scroll.clone();
    let grid_header = state.borrow().grid_header.clone();
    clear_children(&grid_header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_halign(gtk4::Align::Center);
    content.set_valign(gtk4::Align::Center);
    content.set_margin_start(32);
    content.set_margin_end(32);
    content.set_margin_top(32);
    content.set_margin_bottom(32);
    content.set_width_request(420);

    let icon = gtk4::Image::from_icon_name("library-symbolic");
    icon.set_pixel_size(48);
    icon.set_halign(gtk4::Align::Center);
    content.append(&icon);

    let title = gtk4::Label::new(Some(&crate::tr!("Loading game library")));
    title.add_css_class(CSS_SECTION_TITLE);
    title.set_halign(gtk4::Align::Center);
    content.append(&title);

    let status_label = gtk4::Label::new(Some(status));
    status_label.set_halign(gtk4::Align::Center);
    status_label.set_wrap(true);
    content.append(&status_label);

    let progress = gtk4::ProgressBar::new();
    progress.set_hexpand(true);
    progress.set_show_text(true);
    progress.set_fraction(progress_fraction(completed, total));
    progress.set_text(Some(&progress_text(completed, total)));
    content.append(&progress);

    content_scroll.set_child(Some(&content));
    let mut s = state.borrow_mut();
    s.loading_status = Some(status_label);
    s.loading_progress = Some(progress);
}

fn show_empty_search_view(content_scroll: &gtk4::ScrolledWindow) {
    let status = adw::StatusPage::new();
    status.set_icon_name(Some("system-search-symbolic"));
    status.set_title(&crate::tr!("No games found"));
    status.set_description(Some(&crate::tr!(
        "Try a different title, platform, or sort title"
    )));
    content_scroll.set_child(Some(&status));
}

pub fn update_loading_view(state: &SharedState, status: &str, completed: usize, total: usize) {
    let (status_label, progress) = {
        let s = state.borrow();
        (s.loading_status.clone(), s.loading_progress.clone())
    };
    if let (Some(status_label), Some(progress)) = (status_label, progress) {
        status_label.set_label(status);
        progress.set_fraction(progress_fraction(completed, total));
        progress.set_text(Some(&progress_text(completed, total)));
    } else {
        show_loading_view(state, status, completed, total);
    }
}

fn progress_fraction(completed: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (completed as f64 / total as f64).clamp(0.0, 1.0)
    }
}

fn progress_text(completed: usize, total: usize) -> String {
    crate::tr!("{completed} of {total} sources")
        .replace("{completed}", &completed.to_string())
        .replace("{total}", &total.to_string())
}

pub fn refresh_grid_store(state: &SharedState) {
    let games = filtered_games(state);
    let store = state.borrow().grid_store.clone();
    let new_items: Vec<GameItem> = games.iter().map(GameItem::new).collect();
    store.splice(0, store.n_items(), &new_items);
}
