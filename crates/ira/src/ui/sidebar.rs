use gtk4::prelude::*;
use crate::Game;
use ira_models::GroupSelection;
use crate::strings as S;
use super::state::SharedState;
use super::context_menu::show_game_context_menu;
use super::helpers::clear_children;
use super::filter::filtered_games;
use std::collections::HashSet;

#[derive(Clone)]
pub struct SidebarRowWidgets {
    pub row: gtk4::ListBoxRow,
    pub icon: gtk4::Image,
    pub title: gtk4::Label,
}

pub fn select_row_silently(state: &SharedState, row: Option<&gtk4::ListBoxRow>) {
    state.borrow_mut().restoring = true;
    state.borrow().game_list.select_row(row);
    state.borrow_mut().restoring = false;
}

/// Add `selected-game` CSS class to all sidebar rows matching the
/// currently selected db_id, and remove it from all others.
pub fn apply_selected_highlight(state: &SharedState) {
    let selected_id = state.borrow().selected_id.clone();
    let selected_db_id: i64 = selected_id.parse().unwrap_or(0);
    let all_rows: Vec<(i64, Vec<SidebarRowWidgets>)> = state.borrow().rows.iter()
        .map(|(k, v)| (*k, v.clone()))
        .collect();
    for (db_id, rows) in &all_rows {
        let is_selected = *db_id == selected_db_id && selected_db_id != 0;
        for rw in rows {
            if is_selected {
                rw.row.add_css_class("selected-game");
            } else {
                rw.row.remove_css_class("selected-game");
            }
        }
    }
}

/// Scroll the sidebar so that `row` is visible. Defers to an idle callback
/// to ensure the row has been allocated. Uses `allocation()` (position within
/// the ListBox) instead of `compute_bounds()` which returns unreliable values.
#[allow(deprecated)]
pub fn scroll_to_row(state: &SharedState, db_id: i64) {
    let sidebar_scroll = state.borrow().sidebar_scroll.clone();
    let row = state.borrow().rows.get(&db_id)
        .and_then(|v| v.first())
        .map(|rw| rw.row.clone());

    if let Some(row) = row {
        glib::idle_add_local_once(move || {
            let alloc = row.allocation();
            if alloc.height() <= 0 {
                return;
            }
            let adj = sidebar_scroll.vadjustment();
            let row_y = alloc.y() as f64;
            let row_h = alloc.height() as f64;
            let scroll = adj.value();
            let page = adj.page_size();
            if row_y < scroll {
                adj.set_value(row_y - 8.0);
            } else if row_y + row_h > scroll + page {
                adj.set_value(row_y + row_h - page + 8.0);
            }
        });
    }
}

fn find_row_by_name(list: &gtk4::ListBox, name: &str) -> Option<gtk4::ListBoxRow> {
    let mut i = 0;
    while let Some(row) = list.row_at_index(i) {
        if row.widget_name().as_str() == name {
            return Some(row);
        }
        i += 1;
    }
    None
}

fn build_collection_header(
    name: &str,
    count: usize,
    row_name: &str,
    collapsed: bool,
    state: &SharedState,
    group_id: i64,
) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name(row_name);
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);
    hbox.set_margin_start(4);
    hbox.set_margin_end(10);

    let arrow_icon = if collapsed { "pan-end-symbolic" } else { "pan-down-symbolic" };
    let arrow = gtk4::Image::from_icon_name(arrow_icon);
    arrow.set_pixel_size(14);
    arrow.set_valign(gtk4::Align::Center);

    let sc = state.clone();
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| {
        let mut s = sc.borrow_mut();
        if s.collapsed_collections.contains(&group_id) {
            s.collapsed_collections.remove(&group_id);
        } else {
            s.collapsed_collections.insert(group_id);
        }
        drop(s);
        rebuild_sidebar(&sc);
    });
    arrow.add_controller(click);

    hbox.append(&arrow);

    let label = gtk4::Label::new(Some(name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    hbox.append(&label);

    let count_label = gtk4::Label::new(Some(&count.to_string()));
    count_label.add_css_class("dim-label");
    hbox.append(&count_label);

    row.set_child(Some(&hbox));
    row
}

fn build_all_games_row(state: &SharedState) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name("all-games");
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);
    hbox.set_margin_start(24);
    hbox.set_margin_end(6);
    let icon = gtk4::Image::from_icon_name("view-grid-symbolic");
    icon.set_pixel_size(16);
    hbox.append(&icon);
    let label = gtk4::Label::new(Some(S::ALL_GAMES));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    hbox.append(&label);

    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.add_css_class("flat");
    add_btn.set_tooltip_text(Some(S::ADD_COLLECTION));
    add_btn.set_valign(gtk4::Align::Center);
    let sc = state.clone();
    add_btn.connect_clicked(move |_| {
        super::group_dialog::show_create_group_dialog(&sc);
    });
    hbox.append(&add_btn);

    row.set_child(Some(&hbox));
    row
}

pub fn rebuild_sidebar(state: &SharedState) {
    let game_list = state.borrow().game_list.clone();
    let sidebar_scroll = state.borrow().sidebar_scroll.clone();
    let saved_scroll = sidebar_scroll.vadjustment().value();
    let searching = !state.borrow().search_query.is_empty();

    state.borrow_mut().restoring = true;
    clear_children(&game_list);

    game_list.append(&build_all_games_row(state));

    let show_hidden = state.borrow().cfg.show_hidden_games;
    let groups = state.borrow().groups.clone();

    let all_games = state.borrow().games.clone();
    let visible_games: Vec<Game> = all_games.iter()
        .filter(|g| !g.hidden || show_hidden)
        .cloned()
        .collect();

    let grouped_ids: HashSet<i64> = {
        let db = state.borrow().db.clone();
        let mut ids = HashSet::new();
        for g in &groups {
            let group_ids = ira_db::get_game_ids_in_group(&db, g.id).unwrap_or_default();
            ids.extend(group_ids);
        }
        ids
    };

    let mut all_rows: Vec<(i64, SidebarRowWidgets)> = Vec::new();

    if !searching {
        let collapsed = state.borrow().collapsed_collections.clone();
        for g in &groups {
            let member_ids: Vec<i64> = {
                let db = state.borrow().db.clone();
                ira_db::get_game_ids_in_group(&db, g.id).unwrap_or_default()
            };
            let collection_games: Vec<Game> = visible_games.iter()
                .filter(|g| member_ids.contains(&g.db_id))
                .cloned()
                .collect();

            if collection_games.is_empty() && !member_ids.is_empty() {
                continue;
            }

            let is_collapsed = collapsed.contains(&g.id);
            game_list.append(&build_collection_header(
                &g.name,
                collection_games.len(),
                &format!("collection:{}", g.id),
                is_collapsed,
                state,
                g.id,
            ));
            add_collection_context_menu(state, &game_list, g.id, &g.name);

            if !is_collapsed {
                for game in &collection_games {
                    let rw = build_sidebar_row(&game_list, game, state, show_hidden, 0);
                    all_rows.push((game.db_id, rw));
                }
            }
        }

        let uncategorized: Vec<Game> = visible_games.iter()
            .filter(|g| !grouped_ids.contains(&g.db_id))
            .cloned()
            .collect();

        if !uncategorized.is_empty() {
            let is_collapsed = collapsed.contains(&0);
            game_list.append(&build_collection_header(
                "Uncategorized",
                uncategorized.len(),
                "collection:0",
                is_collapsed,
                state,
                0,
            ));

            if !is_collapsed {
                for game in &uncategorized {
                    let rw = build_sidebar_row(&game_list, game, state, show_hidden, 0);
                    all_rows.push((game.db_id, rw));
                }
            }
        }
    } else {
        let games = filtered_games(state);
        for game in &games {
            let rw = build_sidebar_row(&game_list, game, state, show_hidden, 10);
            all_rows.push((game.db_id, rw));
        }
    }

    let mut rows: std::collections::HashMap<i64, Vec<SidebarRowWidgets>> = std::collections::HashMap::new();
    for (db_id, rw) in all_rows {
        rows.entry(db_id).or_default().push(rw);
    }
    state.borrow_mut().rows = rows;

    let adj = sidebar_scroll.vadjustment();
    let upper = adj.upper();
    let max = (upper - adj.page_size()).max(0.0);
    adj.set_value(saved_scroll.min(max));

    restore_selection(state, &game_list, searching);
    apply_selected_highlight(state);
    state.borrow_mut().restoring = false;
}

fn restore_selection(state: &SharedState, game_list: &gtk4::ListBox, _searching: bool) {
    let selected_id = state.borrow().selected_id.clone();
    let selected_group = state.borrow().selected_group.clone();

    if !selected_id.is_empty() {
        if let Ok(db_id) = selected_id.parse::<i64>() {
            if let Some(rows) = state.borrow().rows.get(&db_id) {
                if let Some(first) = rows.first() {
                    let row = first.row.clone();
                    game_list.select_row(Some(&row));
                    return;
                }
            }
        }
    }

    if selected_group != GroupSelection::AllGames {
        let name = match &selected_group {
            GroupSelection::Collection(id) => format!("collection:{}", id),
            GroupSelection::Uncategorized => "collection:0".to_string(),
            GroupSelection::AllGames => String::new(),
        };
        if let Some(row) = find_row_by_name(game_list, &name) {
            game_list.select_row(Some(&row));
            return;
        }
    }

    let row = game_list.row_at_index(0);
    game_list.select_row(row.as_ref());
}

pub fn build_sidebar_row(
    list: &gtk4::ListBox,
    game: &Game,
    state: &SharedState,
    show_hidden: bool,
    _indent: i32,
) -> SidebarRowWidgets {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name(&format!("game:{}", game.db_id));

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);
    hbox.set_margin_start(24);
    hbox.set_margin_end(10);

    let icon = if game.icon_path.is_empty() {
        gtk4::Image::from_icon_name("application-x-executable")
    } else {
        ira_images::new_image_from_file(&game.icon_path)
    };
    icon.set_pixel_size(24);
    hbox.append(&icon);

    let title_label = gtk4::Label::new(Some(&game.name));
    title_label.set_xalign(0.0);
    title_label.add_css_class("sidebar-row-title");
    title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    title_label.set_hexpand(true);
    title_label.set_tooltip_text(Some(&format!("{} ({})", game.name, game.app_id)));
    hbox.append(&title_label);

    row.set_child(Some(&hbox));
    list.append(&row);
    row.set_visible(!game.hidden || show_hidden);
    if game.hidden {
        row.add_css_class("hidden-game");
    }
    {
        let is_running = state.borrow().running_games.lock().unwrap().contains_key(&game.db_id);
        if is_running {
            row.add_css_class("playing-game");
        }
    }

    let state_clone = state.clone();
    let game_clone = game.clone();
    let row_weak = row.downgrade();
    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    right_click.connect_pressed(move |_, _, x, y| {
        if let Some(row) = row_weak.upgrade() {
            show_game_context_menu(&state_clone, &game_clone, &row, x, y, Some(&row));
        }
    });
    row.add_controller(right_click);

    SidebarRowWidgets {
        row,
        icon,
        title: title_label,
    }
}

fn add_collection_context_menu(state: &SharedState, game_list: &gtk4::ListBox, group_id: i64, group_name: &str) {
    let row_name = format!("collection:{}", group_id);
    let row = find_row_by_name(game_list, &row_name);
    if row.is_none() {
        return;
    }
    let row = row.unwrap();

    let state_clone = state.clone();
    let group_name = group_name.to_string();
    let row_weak = row.downgrade();
    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    right_click.connect_pressed(move |_, _, x, y| {
        if let Some(row) = row_weak.upgrade() {
            let menu = gio::Menu::new();
            menu.append(Some("Rename"), Some("grp.rename"));
            menu.append(Some("Delete"), Some("grp.delete"));

            let popover = gtk4::PopoverMenu::from_model(Some(&menu));
            popover.set_halign(gtk4::Align::Start);
            popover.set_has_arrow(false);

            let actions = gio::SimpleActionGroup::new();

            let sc = state_clone.clone();
            let gname = group_name.clone();
            let rename_action = gio::SimpleAction::new("rename", None);
            rename_action.connect_activate(move |_, _| {
                super::group_dialog::show_rename_group_dialog(&sc, group_id, &gname);
            });
            actions.add_action(&rename_action);

            let sc = state_clone.clone();
            let gname = group_name.clone();
            let delete_action = gio::SimpleAction::new("delete", None);
            delete_action.connect_activate(move |_, _| {
                super::group_dialog::show_delete_group_dialog(&sc, group_id, &gname);
            });
            actions.add_action(&delete_action);

            row.insert_action_group("grp", Some(&actions));
            popover.set_parent(&row);
            popover.set_pointing_to(Some(&gdk4::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        }
    });
    row.add_controller(right_click);
}
