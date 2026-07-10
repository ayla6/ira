use gtk4::prelude::*;
use adw::prelude::*;
use crate::Game;
use crate::strings as S;

use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};
use super::state::SharedState;
use super::game_item::GameItem;
use super::grid_bin::GridBin;
use super::message_handler::switch_to_game;
use super::context_menu::show_game_context_menu;

pub fn show_grid_view(state: &SharedState) {
    let content_box = state.borrow().content_box.clone();
    let content_scroll = state.borrow().content_scroll.clone();

    content_scroll.vadjustment().set_value(0.0);
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }
    crate::images::clear_texture_cache();

    let cover_width = state.borrow().cfg.grid_cover_width.clamp(100, 350);
    let show_hidden = state.borrow().cfg.show_hidden_games;
    let cover_height = ((cover_width as f64) * 1.5) as i32;

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.set_margin_start(16);
    outer.set_margin_end(16);
    outer.set_margin_top(16);
    outer.set_margin_bottom(32);

    let mut recent: Vec<Game> = state
        .borrow()
        .games
        .iter()
        .filter(|g| g.lastplayed > 0 && (!g.hidden || show_hidden))
        .cloned()
        .collect();
    recent.sort_by(|a, b| b.lastplayed.cmp(&a.lastplayed));
    recent.truncate(8);

    if !recent.is_empty() {
        outer.append(&build_recent_row(state, &recent, cover_height));
    }

    let heading = gtk4::Label::new(Some(S::ALL_GAMES));
    heading.set_xalign(0.0);
    heading.add_css_class("section-title");
    heading.set_margin_top(if recent.is_empty() { 0 } else { 20 });
    heading.set_margin_bottom(8);
    outer.append(&heading);

    let games: Vec<Game> = state.borrow().games.clone();

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

        let pic = gtk4::Picture::new();
        pic.set_content_fit(gtk4::ContentFit::Cover);
        pic.set_size_request(cover_width, cover_height);
        pic.add_css_class("game-cover-pic");
        let placeholder = crate::images::ScaledPaintable::new_empty(cover_width, cover_height);
        pic.set_paintable(Some(&placeholder));
        vbox.append(&pic);

        unsafe { vbox.set_data::<AtomicI64>("lutris-id", AtomicI64::new(0)) };

        let sc = state_for_setup.clone();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |gesture, _, _, _| {
            let widget = gesture.widget().unwrap();
            if let Some(ptr) = unsafe { widget.data::<AtomicI64>("lutris-id") } {
                let lutris_id = unsafe { ptr.as_ref() }.load(Ordering::Relaxed);
                if lutris_id != 0 {
                    switch_to_game(&sc, lutris_id);
                }
            }
        });
        vbox.add_controller(click);

        let sc2 = state_for_setup.clone();
        let right_click = gtk4::GestureClick::new();
        right_click.set_button(3);
        right_click.connect_pressed(move |gesture, _, x, y| {
            let widget = gesture.widget().unwrap();
            if let Some(ptr) = unsafe { widget.data::<AtomicI64>("lutris-id") } {
                let lutris_id = unsafe { ptr.as_ref() }.load(Ordering::Relaxed);
                if lutris_id != 0 {
                    let game = sc2
                        .borrow()
                        .games
                        .iter()
                        .find(|g| g.lutris_id == lutris_id)
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
        let first = vbox.first_child().unwrap();
        let pic = first.downcast_ref::<gtk4::Picture>().unwrap();

        let game_item = list_item
            .item()
            .unwrap()
            .downcast::<GameItem>()
            .unwrap();

        if let Some(game) = game_item.game() {
            if let Some(ptr) = unsafe { vbox.data::<AtomicI64>("lutris-id") } {
                unsafe { ptr.as_ref() }.store(game.lutris_id, Ordering::Relaxed);
            }
            if !game.grid_path.is_empty() {
                crate::images::set_picture_natural(pic, &game.grid_path, cover_width, cover_height);
            } else {
                pic.set_paintable(None::<&gdk4::Texture>);
            }
        }
    });

    factory.connect_unbind(move |_, list_item_obj| {
        let list_item = list_item_obj.downcast_ref::<gtk4::ListItem>().unwrap();
        let child = list_item.child().unwrap();
        let vbox = child.downcast_ref::<gtk4::Box>().unwrap();
        let first = vbox.first_child().unwrap();
        let pic = first.downcast_ref::<gtk4::Picture>().unwrap();

        if let Some(ptr) = unsafe { vbox.data::<AtomicI64>("lutris-id") } {
            unsafe { ptr.as_ref() }.store(0, Ordering::Relaxed);
        }
        let placeholder = crate::images::ScaledPaintable::new_empty(cover_width, cover_height);
        pic.set_paintable(Some(&placeholder));
    });

    let store = gio::ListStore::new::<GameItem>();
    for game in &games {
        if game.hidden && !show_hidden {
            continue;
        }
        store.append(&GameItem::new(game));
    }

    let selection_model = gtk4::NoSelection::new(Some(store.upcast::<gio::ListModel>()));
    let grid = gtk4::GridView::new(Some(selection_model), Some(factory));
    grid.set_min_columns(1);
    grid.set_max_columns(30);
    grid.set_hexpand(true);
    grid.set_halign(gtk4::Align::Fill);
    grid.add_css_class("game-grid");
    grid.remove_css_class("view");

    let n_items = games.iter().filter(|g| !g.hidden || show_hidden).count() as u32;
    let row_h = cover_height + 16;
    let col_nat = cover_width + 16;
    let bin = GridBin::new(&grid, row_h, n_items, col_nat);
    bin.set_hexpand(true);
    bin.set_halign(gtk4::Align::Fill);
    bin.set_vexpand(false);
    bin.set_valign(gtk4::Align::Start);
    bin.set_overflow(gtk4::Overflow::Visible);

    outer.append(&bin);
    content_box.append(&outer);
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
        let path = if use_header { &game.header_path } else { &game.grid_path };
        let item = build_cover(state, game, path, w, h);
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
        crate::images::set_picture_natural(&pic, image_path, w, h);
    }

    vbox.append(&pic);

    let state_clone = state.clone();
    let lutris_id = game.lutris_id;
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| {
        switch_to_game(&state_clone, lutris_id);
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
