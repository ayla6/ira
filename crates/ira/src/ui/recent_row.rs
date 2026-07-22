use gtk4::prelude::*;
use crate::Game;
use std::rc::Rc;
use super::state::SharedState;
use super::grid_view::queue_cover_load;
use super::message_helpers::switch_to_game;
use super::sidebar::scroll_to_row;
use super::context_menu::show_game_context_menu;

pub(super) fn build_recent_row(
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

pub(super) fn build_cover(
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
        queue_cover_load(pic.clone(), image_path.to_string(), w, h, game.db_id, game.variant_id.unwrap_or(0), vbox.clone());
    }

    vbox.append(&pic);

    let state_clone = state.clone();
    let db_id = game.db_id;
    let variant_id = game.variant_id;
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| {
        switch_to_game(&state_clone, db_id, variant_id);
        scroll_to_row(&state_clone, db_id, variant_id);
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
