use gtk4::prelude::*;
use crate::Game;
use ira_models::GroupSelection;
use crate::strings as S;
use super::state::SharedState;
use super::context_menu::show_game_context_menu;
use super::helpers::clear_children;
use super::sidebar_item::{SidebarItem, SidebarItemKind};
use std::collections::{HashMap, HashSet};

pub fn select_row_silently(state: &SharedState, index: Option<u32>) {
    state.borrow_mut().restoring = true;
    let model = state.borrow().sidebar_selection.clone();
    if let Some(pos) = index {
        model.select_item(pos, true);
    } else {
        model.unselect_all();
    }
    state.borrow_mut().restoring = false;
}

pub fn scroll_to_row(state: &SharedState, db_id: i64) {
    let view = state.borrow().sidebar_view.clone();
    if let Some(index) = find_game_index(state, db_id) {
        glib::idle_add_local_once(move || {
            view.scroll_to(index, gtk4::ListScrollFlags::NONE, None);
        });
    }
}

pub fn find_game_index(state: &SharedState, db_id: i64) -> Option<u32> {
    let store = state.borrow().sidebar_store.clone();
    for i in 0..store.n_items() {
        if let Some(item) = store.item(i).and_then(|o| o.downcast::<SidebarItem>().ok()) {
            if item.kind() == SidebarItemKind::Game && item.db_id() == db_id {
                return Some(i);
            }
        }
    }
    None
}

pub fn update_sidebar_game(state: &SharedState, db_id: i64, name: &str, icon_path: &str) {
    let store = state.borrow().sidebar_store.clone();
    for i in 0..store.n_items() {
        if let Some(item) = store.item(i).and_then(|o| o.downcast::<SidebarItem>().ok()) {
            if item.kind() == SidebarItemKind::Game && item.db_id() == db_id {
                let new_item = SidebarItem::new_game(
                    db_id, name, icon_path, item.hidden(), item.playing(),
                );
                store.splice(i, 1, &[new_item]);
                break;
            }
        }
    }
}

pub fn set_sidebar_playing(state: &SharedState, db_id: i64, playing: bool) {
    let store = state.borrow().sidebar_store.clone();
    for i in 0..store.n_items() {
        if let Some(item) = store.item(i).and_then(|o| o.downcast::<SidebarItem>().ok()) {
            if item.kind() == SidebarItemKind::Game && item.db_id() == db_id {
                let new_item = SidebarItem::new_game(
                    db_id, &item.name(), &item.icon_path(), item.hidden(), playing,
                );
                store.splice(i, 1, &[new_item]);
                break;
            }
        }
    }
}

pub fn rebuild_sidebar(state: &SharedState) {
    let sidebar_scroll = state.borrow().sidebar_scroll.clone();
    let store = state.borrow().sidebar_store.clone();
    let saved_scroll = sidebar_scroll.vadjustment().value();

    let (searching, show_hidden, groups, collapsed, games, group_members, running_games) = {
        let s = state.borrow();
        let group_members: HashMap<i64, Vec<i64>> = s.groups.iter()
            .map(|g| {
                let ids = ira_db::get_game_ids_in_group(&s.db, g.id).unwrap_or_default();
                (g.id, ids)
            })
            .collect();
        (
            !s.search_query.is_empty(),
            s.cfg.show_hidden_games,
            s.groups.clone(),
            s.collapsed_collections.clone(),
            s.games.clone(),
            group_members,
            s.running_games.clone(),
        )
    };

    state.borrow_mut().restoring = true;
    store.remove_all();

    let mut items: Vec<SidebarItem> = Vec::new();
    items.push(SidebarItem::new_all_games());

    let visible_games: Vec<&Game> = games.iter()
        .filter(|g| !g.hidden || show_hidden)
        .collect();

    let grouped_ids: HashSet<i64> = group_members.values()
        .flat_map(|ids| ids.iter().copied())
        .collect();

    if !searching {
        for g in &groups {
            let member_ids = group_members.get(&g.id).cloned().unwrap_or_default();
            let collection_games: Vec<&Game> = visible_games.iter()
                .filter(|game| member_ids.contains(&game.db_id))
                .copied()
                .collect();

            if collection_games.is_empty() && !member_ids.is_empty() {
                continue;
            }

            let is_collapsed = collapsed.contains(&g.id);
            items.push(SidebarItem::new_collection_header(
                g.id, &g.name, collection_games.len(), is_collapsed,
            ));

            if !is_collapsed {
                for game in &collection_games {
                    let is_running = running_games.lock().unwrap().contains_key(&game.db_id);
                    items.push(SidebarItem::new_game(
                        game.db_id, &game.name, &game.icon_path, game.hidden, is_running,
                    ));
                }
            }
        }

        let uncategorized: Vec<&Game> = visible_games.iter()
            .filter(|g| !grouped_ids.contains(&g.db_id))
            .copied()
            .collect();

        if !uncategorized.is_empty() {
            let is_collapsed = collapsed.contains(&0);
            items.push(SidebarItem::new_uncategorized_header(
                uncategorized.len(), is_collapsed,
            ));

            if !is_collapsed {
                for game in &uncategorized {
                    let is_running = running_games.lock().unwrap().contains_key(&game.db_id);
                    items.push(SidebarItem::new_game(
                        game.db_id, &game.name, &game.icon_path, game.hidden, is_running,
                    ));
                }
            }
        }
    } else {
        let (search, sort_mode, sort_descending) = {
            let s = state.borrow();
            (s.search_query.to_lowercase(), s.cfg.sort_mode, s.cfg.sort_descending)
        };
        let mut filtered: Vec<&Game> = visible_games.iter()
            .filter(|g| g.name.to_lowercase().contains(&search))
            .copied()
            .collect();
        filtered.sort_by(|a, b| {
            let ord = sort_mode.compare(a, b);
            if sort_descending { ord.reverse() } else { ord }
        });

        for game in &filtered {
            let is_running = running_games.lock().unwrap().contains_key(&game.db_id);
            items.push(SidebarItem::new_game(
                game.db_id, &game.name, &game.icon_path, game.hidden, is_running,
            ));
        }
    }

    store.splice(0, 0, &items);

    let adj = sidebar_scroll.vadjustment();
    let upper = adj.upper();
    let max = (upper - adj.page_size()).max(0.0);
    adj.set_value(saved_scroll.min(max));

    restore_selection(state);
    state.borrow_mut().restoring = false;
}

fn restore_selection(state: &SharedState) {
    let selected_id = state.borrow().selected_id.clone();
    let selected_group = state.borrow().selected_group.clone();
    let store = state.borrow().sidebar_store.clone();

    if !selected_id.is_empty() {
        if let Ok(db_id) = selected_id.parse::<i64>() {
            if let Some(index) = find_game_index(state, db_id) {
                select_row_silently(state, Some(index));
            }
            return;
        }
    }

    if selected_group != GroupSelection::AllGames {
        let target_group_id = match &selected_group {
            GroupSelection::Collection(id) => *id,
            GroupSelection::Uncategorized => 0,
            GroupSelection::AllGames => return,
        };
        for i in 0..store.n_items() {
            if let Some(item) = store.item(i).and_then(|o| o.downcast::<SidebarItem>().ok()) {
                let is_header = item.kind() == SidebarItemKind::CollectionHeader
                    || item.kind() == SidebarItemKind::UncategorizedHeader;
                if is_header && item.group_id() == target_group_id {
                    select_row_silently(state, Some(i));
                    return;
                }
            }
        }
    }

    select_row_silently(state, Some(0));
}

pub fn build_factory(state: &SharedState) -> gtk4::SignalListItemFactory {
    let factory = gtk4::SignalListItemFactory::new();

    factory.connect_setup(move |_, list_item_obj| {
        let list_item = list_item_obj.downcast_ref::<gtk4::ListItem>().unwrap();
        list_item.set_activatable(true);
        list_item.set_selectable(true);

        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        row.set_margin_start(24);
        row.set_margin_end(10);

        list_item.set_child(Some(&row));
    });

    let state_for_bind = state.clone();
    factory.connect_bind(move |_, list_item_obj| {
        let list_item = list_item_obj.downcast_ref::<gtk4::ListItem>().unwrap();
        let row = list_item.child().unwrap().downcast::<gtk4::Box>().unwrap();

        let item = list_item.item().unwrap()
            .downcast::<SidebarItem>().unwrap();

        clear_children(&row);

        match item.kind() {
            SidebarItemKind::AllGames => {
                row.set_margin_start(24);
                let icon = gtk4::Image::from_icon_name("view-grid-symbolic");
                icon.set_pixel_size(16);
                row.append(&icon);
                let label = gtk4::Label::new(Some(S::ALL_GAMES));
                label.set_xalign(0.0);
                label.set_hexpand(true);
                row.append(&label);

                let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
                add_btn.add_css_class("flat");
                add_btn.set_tooltip_text(Some(S::ADD_COLLECTION));
                add_btn.set_valign(gtk4::Align::Center);
                let sc = state_for_bind.clone();
                add_btn.connect_clicked(move |_| {
                    super::group_dialog::show_create_group_dialog(&sc);
                });
                row.append(&add_btn);
            }
            SidebarItemKind::CollectionHeader | SidebarItemKind::UncategorizedHeader => {
                row.set_margin_start(4);
                let group_id = item.group_id();
                let collapsed = item.collapsed();

                let arrow_icon = if collapsed { "pan-end-symbolic" } else { "pan-down-symbolic" };
                let arrow = gtk4::Image::from_icon_name(arrow_icon);
                arrow.set_pixel_size(14);
                arrow.set_valign(gtk4::Align::Center);

                let sc = state_for_bind.clone();
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
                row.append(&arrow);

                let header_name = if item.kind() == SidebarItemKind::UncategorizedHeader {
                    "Uncategorized".to_string()
                } else {
                    item.name()
                };
                let label = gtk4::Label::new(Some(&header_name));
                label.set_xalign(0.0);
                label.set_hexpand(true);
                label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                row.append(&label);

                let count_label = gtk4::Label::new(Some(&item.count().to_string()));
                count_label.add_css_class("dim-label");
                row.append(&count_label);

                if item.kind() == SidebarItemKind::CollectionHeader {
                    let sc = state_for_bind.clone();
                    let group_name = header_name.clone();
                    let row_weak = row.downgrade();
                    let right_click = gtk4::GestureClick::new();
                    right_click.set_button(3);
                    right_click.connect_pressed(move |_, _, x, y| {
                        if let Some(r) = row_weak.upgrade() {
                            let menu = gio::Menu::new();
                            menu.append(Some("Rename"), Some("grp.rename"));
                            menu.append(Some("Delete"), Some("grp.delete"));

                            let popover = gtk4::PopoverMenu::from_model(Some(&menu));
                            popover.set_halign(gtk4::Align::Start);
                            popover.set_has_arrow(false);

                            let actions = gio::SimpleActionGroup::new();
                            let sc2 = sc.clone();
                            let gname = group_name.clone();
                            let rename_action = gio::SimpleAction::new("rename", None);
                            rename_action.connect_activate(move |_, _| {
                                super::group_dialog::show_rename_group_dialog(&sc2, group_id, &gname);
                            });
                            actions.add_action(&rename_action);

                            let sc2 = sc.clone();
                            let gname = group_name.clone();
                            let delete_action = gio::SimpleAction::new("delete", None);
                            delete_action.connect_activate(move |_, _| {
                                super::group_dialog::show_delete_group_dialog(&sc2, group_id, &gname);
                            });
                            actions.add_action(&delete_action);

                            r.insert_action_group("grp", Some(&actions));
                            popover.set_parent(&r);
                            popover.set_pointing_to(Some(&gdk4::Rectangle::new(x as i32, y as i32, 1, 1)));
                            popover.popup();
                        }
                    });
                    row.add_controller(right_click);
                }
            }
            SidebarItemKind::Game => {
                row.remove_css_class("hidden-game");
                row.remove_css_class("playing-game");

                row.set_margin_start(24);
                let icon = if item.icon_path().is_empty() {
                    gtk4::Image::from_icon_name("application-x-executable")
                } else {
                    ira_images::new_image_from_file(&item.icon_path())
                };
                icon.set_pixel_size(24);
                row.append(&icon);

                let title_label = gtk4::Label::new(Some(&item.name()));
                title_label.set_xalign(0.0);
                title_label.add_css_class("sidebar-row-title");
                title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                title_label.set_hexpand(true);
                title_label.set_tooltip_text(Some(&item.name()));
                row.append(&title_label);

                if item.hidden() {
                    row.add_css_class("hidden-game");
                }
                if item.playing() {
                    row.add_css_class("playing-game");
                }

                let sc = state_for_bind.clone();
                let db_id = item.db_id();
                let row_weak = row.downgrade();
                let right_click = gtk4::GestureClick::new();
                right_click.set_button(3);
                right_click.connect_pressed(move |_, _, x, y| {
                    if let Some(r) = row_weak.upgrade() {
                        let game = sc.borrow().games.iter()
                            .find(|g| g.db_id == db_id)
                            .cloned();
                        if let Some(game) = game {
                            show_game_context_menu(&sc, &game, &r, x, y, None::<&gtk4::ListBoxRow>);
                        }
                    }
                });
                row.add_controller(right_click);
            }
        }
    });

    factory.connect_unbind(move |_, list_item_obj| {
        let list_item = list_item_obj.downcast_ref::<gtk4::ListItem>().unwrap();
        if let Some(child) = list_item.child() {
            let row = child.downcast::<gtk4::Box>().unwrap();
            clear_children(&row);
        }
    });

    factory
}
