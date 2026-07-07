use crate::parser::Game;
use crate::state::{select_row_silently, SharedState, SidebarRowWidgets};
use crate::strings as S;
use gtk4::pango;
use gtk4::prelude::*;

pub fn rebuild_sidebar(state: &SharedState) {
    let game_list = state.borrow().game_list.clone();
    let sidebar_scroll = state.borrow().sidebar_scroll.clone();
    let saved_scroll = sidebar_scroll.vadjustment().value();

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
    let show_hidden = state.borrow().cfg.show_hidden_games;
    let games: Vec<Game> = state.borrow().games.to_vec();
    let mut rows = Vec::with_capacity(games.len());
    for g in &games {
        rows.push(build_sidebar_row(&game_list, g, state, show_hidden));
    }
    state.borrow_mut().rows = rows;

    // Restore scroll position after rebuild
    let adj = sidebar_scroll.vadjustment();
    let upper = adj.upper();
    let max = (upper - adj.page_size()).max(0.0);
    adj.set_value(saved_scroll.min(max));

    // Restore the previously selected row
    let selected_id = state.borrow().selected_id.clone();
    if selected_id.is_empty() {
        let row = game_list.row_at_index(0);
        select_row_silently(state, row.as_ref());
    } else if let Ok(lutris_id) = selected_id.parse::<i64>() {
        let idx = {
            let s = state.borrow();
            s.games.iter().position(|g| g.lutris_id == lutris_id)
        };
        if let Some(idx) = idx {
            let row = game_list.row_at_index((idx + 1) as i32);
            select_row_silently(state, row.as_ref());
        } else {
            let row = game_list.row_at_index(0);
            select_row_silently(state, row.as_ref());
        }
    }
}

fn build_sidebar_row(
    list: &gtk4::ListBox,
    game: &Game,
    state: &SharedState,
    show_hidden: bool,
) -> SidebarRowWidgets {
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
    title_label.add_css_class("sidebar-row-title");
    title_label.set_ellipsize(pango::EllipsizeMode::End);
    title_label.set_hexpand(true);
    title_label.set_tooltip_text(Some(&format!("{} ({})", game.name, game.app_id)));
    vbox.append(&title_label);

    hbox.append(&vbox);
    row.set_child(Some(&hbox));
    list.append(&row);
    row.set_visible(!game.hidden || show_hidden);
    if game.hidden {
        row.add_css_class("hidden-game");
    }

    // Right-click context menu for game settings
    let state_clone = state.clone();
    let game_clone = game.clone();
    let row_clone = row.clone();
    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    right_click.connect_pressed(move |_, _, _, _| {
        crate::dialogs::show_game_context_menu(&state_clone, &game_clone, &row_clone);
    });
    row.add_controller(right_click);

    SidebarRowWidgets {
        row,
        icon,
        title: title_label,
        subtitle: gtk4::Label::new(None),
    }
}
