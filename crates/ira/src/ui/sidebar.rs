use super::context_menu::{show_game_context_menu, show_multi_game_context_menu};
use super::css::*;
use super::grid_view::show_grid_view;
use super::helpers::clear_children;
use super::search::SearchQuery;
use super::sidebar_item::{SidebarItem, SidebarItemKind};
use super::state::SharedState;
use crate::Game;
use adw::prelude::*;
use ira_models::GroupSelection;
use std::collections::{HashMap, HashSet};

fn queue_icon_load(icon: gtk4::Image, path: String) {
    let _s = tracing::info_span!("queue_icon_load", path = %path).entered();
    if let Some(t) = ira_images::cached_texture(&path) {
        icon.set_paintable(Some(&t));
        return;
    }
    ira_images::set_image_async(&icon, &path);
}

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

pub fn scroll_to_row(state: &SharedState, db_id: i64, variant_id: Option<i64>) {
    let view = state.borrow().sidebar_view.clone();
    if let Some(index) = find_game_index(state, db_id, variant_id) {
        glib::idle_add_local_once(move || {
            view.scroll_to(index, gtk4::ListScrollFlags::NONE, None);
        });
    }
}

pub fn find_game_index(state: &SharedState, db_id: i64, variant_id: Option<i64>) -> Option<u32> {
    let store = state.borrow().sidebar_store.clone();
    for i in 0..store.n_items() {
        if let Some(item) = store.item(i).and_then(|o| o.downcast::<SidebarItem>().ok()) {
            if item.kind() == SidebarItemKind::Game
                && item.db_id() == db_id
                && item.variant_id() == variant_id
            {
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
            if item.kind() == SidebarItemKind::Game
                && item.db_id() == db_id
                && item.variant_id().is_none()
            {
                if item.name() == name && item.icon_path() == icon_path {
                    return;
                }
                let new_item =
                    SidebarItem::new_game(db_id, name, icon_path, item.hidden(), item.playing());
                store.splice(i, 1, &[new_item]);
                break;
            }
        }
    }
}

pub fn set_sidebar_playing(state: &SharedState, db_id: i64, playing: bool) {
    let store = state.borrow().sidebar_store.clone();
    let sidebar_scroll = state.borrow().sidebar_scroll.clone();
    let saved_scroll = sidebar_scroll.vadjustment().value();
    for i in 0..store.n_items() {
        if let Some(item) = store.item(i).and_then(|o| o.downcast::<SidebarItem>().ok()) {
            if item.kind() == SidebarItemKind::Game
                && item.db_id() == db_id
                && item.playing() != playing
            {
                let new_item = SidebarItem::new_game_variant(
                    db_id,
                    item.variant_id(),
                    &item.name(),
                    &item.icon_path(),
                    item.hidden(),
                    playing,
                );
                store.splice(i, 1, &[new_item]);
            }
        }
    }
    let adj = sidebar_scroll.vadjustment();
    glib::idle_add_local_once(move || {
        let max = (adj.upper() - adj.page_size()).max(0.0);
        adj.set_value(saved_scroll.min(max));
    });

    // Splicing drops the selection on the replaced row; only restore when
    // something was actually selected, otherwise leave the sidebar unselected.
    let has_selection = {
        let s = state.borrow();
        !s.selected_id.is_empty()
            || !s.multi_selected_ids.is_empty()
            || s.selected_group != GroupSelection::AllGames
    };
    if has_selection {
        restore_selection(state);
    }
}

pub fn rebuild_sidebar(state: &SharedState) {
    let _span = tracing::info_span!("rebuild_sidebar").entered();
    // Auto-group memberships are derived, never stored: every rebuild
    // re-runs the rules over the live games, which is how a fresh match
    // or a play session moves games between groups by itself.
    super::auto_groups::refresh_auto_members(state);
    let sidebar_scroll = state.borrow().sidebar_scroll.clone();
    let store = state.borrow().sidebar_store.clone();
    let saved_scroll = sidebar_scroll.vadjustment().value();

    let (searching, show_hidden, groups, collapsed, games, group_members, running_games, group_by) = {
        let s = state.borrow();
        (
            !s.search_query.is_empty(),
            s.cfg.show_hidden_games,
            s.groups.clone(),
            s.collapsed_collections.clone(),
            s.games.clone(),
            s.group_members.clone(),
            s.running_games
                .lock()
                .map(|games| games.keys().copied().collect::<HashSet<_>>())
                .unwrap_or_default(),
            s.cfg.group_by,
        )
    };
    // Every game list below is ordered by the grid's own comparator, so
    // changing the sort reshapes the sidebar alongside the grid. The
    // entity sorts follow the credited names from the cache, exactly
    // like filtered_games does; other modes never touch the map.
    let (sort_mode, sort_descending) = {
        let s = state.borrow();
        (s.cfg.sort_mode, s.cfg.sort_descending)
    };
    let sort_entity_names = match sort_mode {
        ira_models::SortMode::Publisher => {
            ira_db::game_entity_names(&state.borrow().db, ira_db::KIND_COMPANY, Some("is_publisher"))
                .unwrap_or_default()
        }
        ira_models::SortMode::Developer => {
            ira_db::game_entity_names(&state.borrow().db, ira_db::KIND_COMPANY, Some("is_developer"))
                .unwrap_or_default()
        }
        _ => Default::default(),
    };

    state.borrow_mut().restoring = true;
    let old_n = store.n_items();

    let mut items: Vec<SidebarItem> = Vec::new();
    items.push(SidebarItem::new_all_games());

    let visible_games: Vec<&Game> = games.iter().filter(|g| !g.hidden || show_hidden).collect();

    let grouped_ids: HashSet<i64> = group_members
        .values()
        .flat_map(|ids| ids.iter().copied())
        .collect();

    if !searching && group_by != ira_models::GroupBy::Off {
        // ── Derived categories: one collapsible section per value of
        // the group-by dimension, replacing the collections view. The
        // members map feeds the grid filter when a section is selected.
        let db = {
            let s = state.borrow();
            s.db.clone()
        };
        let entity_names = match group_by {
            ira_models::GroupBy::Developer => ira_db::game_entity_names(
                &db,
                ira_db::KIND_COMPANY,
                Some("is_developer"),
            )
            .unwrap_or_default(),
            ira_models::GroupBy::Publisher => ira_db::game_entity_names(
                &db,
                ira_db::KIND_COMPANY,
                Some("is_publisher"),
            )
            .unwrap_or_default(),
            ira_models::GroupBy::Genre => {
                ira_db::game_entity_names(&db, ira_db::KIND_GENRE, None).unwrap_or_default()
            }
            ira_models::GroupBy::Family => {
                ira_db::game_entity_names(&db, ira_db::KIND_FAMILY, None).unwrap_or_default()
            }
            _ => Default::default(),
        };

        let categories =
            super::filter::group_categories(group_by, &visible_games, &entity_names);

        let mut derived: HashMap<String, HashSet<i64>> = HashMap::new();
        for (name, members) in &categories {
            derived.insert(
                name.clone(),
                members.iter().map(|game| game.db_id).collect(),
            );
        }
        state.borrow_mut().derived_members = derived;

        for (name, members) in &categories {
            let id = ira_models::derived_group_id(name);
            let is_collapsed = collapsed.contains(&id);
            items.push(SidebarItem::new_collection_header(
                id,
                name,
                members.len(),
                is_collapsed,
            ));
            if is_collapsed {
                continue;
            }
            let mut sorted: Vec<&Game> = members.to_vec();
            sort_grid_order(&mut sorted, sort_mode, sort_descending, &sort_entity_names);
            for game in &sorted {
                items.push(SidebarItem::from_game(game, &running_games));
            }
        }
    } else if !searching {
        for g in &groups {
            let member_ids = group_members.get(&g.id);
            let mut collection_games: Vec<&Game> = visible_games
                .iter()
                .filter(|game| member_ids.is_some_and(|ids| ids.contains(&game.db_id)))
                .copied()
                .collect();
            sort_grid_order(
                &mut collection_games,
                sort_mode,
                sort_descending,
                &sort_entity_names,
            );

            if collection_games.is_empty() && member_ids.is_some_and(|ids| !ids.is_empty()) {
                continue;
            }

            let is_collapsed = collapsed.contains(&g.id);
            items.push(SidebarItem::new_collection_header(
                g.id,
                &g.name,
                collection_games.len(),
                is_collapsed,
            ));

            if !is_collapsed {
                for game in &collection_games {
                    items.push(SidebarItem::from_game(game, &running_games));
                }
            }
        }

        // ── Auto groups: rule-managed sections between the collections
        // and the uncategorized tail. Their memberships were folded into
        // group_members under negated ids by the refresh above. ──
        let auto_groups = state.borrow().auto_groups.clone();
        for group in &auto_groups {
            let member_ids = state
                .borrow()
                .group_members
                .get(&group.id)
                .cloned()
                .unwrap_or_default();
            let mut collection_games: Vec<&Game> = visible_games
                .iter()
                .filter(|game| member_ids.contains(&game.db_id))
                .copied()
                .collect();
            sort_grid_order(
                &mut collection_games,
                sort_mode,
                sort_descending,
                &sort_entity_names,
            );

            let is_collapsed = collapsed.contains(&group.id);
            items.push(SidebarItem::new_auto_group_header(
                group.id,
                &group.name,
                collection_games.len(),
                is_collapsed,
            ));
            if !is_collapsed {
                for game in &collection_games {
                    items.push(SidebarItem::from_game(game, &running_games));
                }
            }
        }

        let mut uncategorized: Vec<&Game> = visible_games
            .iter()
            .filter(|g| !grouped_ids.contains(&g.db_id))
            .copied()
            .collect();
        sort_grid_order(
            &mut uncategorized,
            sort_mode,
            sort_descending,
            &sort_entity_names,
        );

        if !uncategorized.is_empty() {
            let is_collapsed = collapsed.contains(&0);
            items.push(SidebarItem::new_uncategorized_header(
                uncategorized.len(),
                is_collapsed,
            ));

            if !is_collapsed {
                for game in &uncategorized {
                    items.push(SidebarItem::from_game(game, &running_games));
                }
            }
        }
        state.borrow_mut().derived_members = HashMap::new();
    } else {
        state.borrow_mut().derived_members = HashMap::new();
        let search = SearchQuery::parse(&state.borrow().search_query);
        let mut filtered: Vec<&Game> = visible_games
            .iter()
            .filter(|g| search.matches(g))
            .copied()
            .collect();
        sort_grid_order(&mut filtered, sort_mode, sort_descending, &sort_entity_names);

        for game in &filtered {
            items.push(SidebarItem::from_game(game, &running_games));
        }
    }

    store.splice(0, old_n, &items);

    let adj = sidebar_scroll.vadjustment();
    glib::idle_add_local_once(move || {
        let max = (adj.upper() - adj.page_size()).max(0.0);
        adj.set_value(saved_scroll.min(max));
    });

    restore_selection(state);
    state.borrow_mut().restoring = false;
}

/// Order a sidebar game list exactly the way the grid orders its own:
/// filter.rs's shared comparator. This is what makes the sidebar reshape
/// itself when the sort changes instead of sitting still.
fn sort_grid_order(
    games: &mut [&Game],
    sort_mode: ira_models::SortMode,
    descending: bool,
    entity_names: &HashMap<i64, String>,
) {
    games.sort_by(|a, b| super::filter::game_compare(a, b, sort_mode, descending, entity_names));
}

fn restore_selection(state: &SharedState) {
    let multi_selected_ids = state.borrow().multi_selected_ids.clone();
    let store = state.borrow().sidebar_store.clone();

    if !multi_selected_ids.is_empty() {
        let selection = state.borrow().sidebar_selection.clone();
        let bitset = gtk4::Bitset::new_empty();
        for i in 0..store.n_items() {
            if let Some(item) = store.item(i).and_then(|o| o.downcast::<SidebarItem>().ok()) {
                if item.kind() == SidebarItemKind::Game {
                    let grid_id = match item.variant_id() {
                        Some(vid) => format!("{}-v{}", item.db_id(), vid),
                        None => item.db_id().to_string(),
                    };
                    if multi_selected_ids.contains(&grid_id) {
                        bitset.add(i);
                    }
                }
            }
        }
        selection.set_selection(&bitset, &bitset);
        return;
    }

    let selected_id = state.borrow().selected_id.clone();
    let selected_group = state.borrow().selected_group.clone();

    if !selected_id.is_empty() {
        let db_id = ira_models::parse_db_id(&selected_id);
        if db_id > 0 {
            let variant_id = selected_id
                .split("-v")
                .nth(1)
                .and_then(|s| s.parse::<i64>().ok());
            if let Some(index) = find_game_index(state, db_id, variant_id) {
                select_row_silently(state, Some(index));
                return;
            }
        }
    }

    if selected_group != GroupSelection::AllGames {
        let target_group_id = match &selected_group {
            GroupSelection::Collection(id) => *id,
            GroupSelection::Uncategorized => 0,
            GroupSelection::Derived(name) => ira_models::derived_group_id(name),
            GroupSelection::AllGames => return,
        };
        for i in 0..store.n_items() {
            if let Some(item) = store.item(i).and_then(|o| o.downcast::<SidebarItem>().ok()) {
                let is_header = item.kind() == SidebarItemKind::CollectionHeader
                    || item.kind() == SidebarItemKind::UncategorizedHeader
                    || item.kind() == SidebarItemKind::AutoGroupHeader;
                if is_header && item.group_id() == target_group_id {
                    select_row_silently(state, Some(i));
                    return;
                }
            }
        }
    }

    state.borrow_mut().selected_id.clear();
    state.borrow_mut().selected_group = GroupSelection::AllGames;
    select_row_silently(state, Some(0));
}

pub fn rebuild_sidebar_and_show_grid(state: &SharedState) {
    rebuild_sidebar(state);
    // While the initial load is still in flight (loading screen up) the grid
    // must wait for its GamesLoaded message: showing it now would present a
    // nearly-empty library and let stale progress messages fight the view.
    let show = {
        let s = state.borrow();
        s.selected_id.is_empty() && !s.content_unloaded && s.loading.is_none()
    };
    if show {
        show_grid_view(state);
    }
}

fn sidebar_setup_factory(_factory: &gtk4::SignalListItemFactory, list_item_obj: &glib::Object) {
    let list_item = list_item_obj.downcast_ref::<gtk4::ListItem>().unwrap();
    list_item.set_activatable(true);
    list_item.set_selectable(true);

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);

    list_item.set_child(Some(&row));
}

fn sidebar_bind_all_games(state: &SharedState, row: &gtk4::Box) {
    row.add_css_class(CSS_SIDEBAR_ROW_PAD_GAME);
    let icon = gtk4::Image::from_icon_name("view-grid-symbolic");
    icon.set_pixel_size(16);
    row.append(&icon);
    let label = gtk4::Label::new(Some(&crate::tr!("All games")));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);

    // The auto-group button sits left of the plain one: search-like
    // icon for a rule-defined group.
    let auto_btn = gtk4::Button::from_icon_name("funnel-symbolic");
    auto_btn.add_css_class(CSS_FLAT);
    auto_btn.set_tooltip_text(Some(&crate::tr!("New auto group")));
    auto_btn.set_valign(gtk4::Align::Center);
    let sc = state.clone();
    auto_btn.connect_clicked(move |_| {
        super::auto_group_dialog::show_auto_group_create(&sc);
    });
    row.append(&auto_btn);

    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.add_css_class(CSS_FLAT);
    add_btn.set_tooltip_text(Some(&crate::tr!("Add group")));
    add_btn.set_valign(gtk4::Align::Center);
    let sc = state.clone();
    add_btn.connect_clicked(move |_| {
        super::group_dialog::show_create_group_dialog(&sc);
    });
    row.append(&add_btn);
}

/// The rule-managed group header: collapse arrow and name like a
/// collection, plus a pencil straight into the rule editor and a
/// right-click menu for editing or deleting the group.
fn sidebar_bind_auto_group_header(state: &SharedState, row: &gtk4::Box, item: &SidebarItem) {
    row.add_css_class(CSS_SIDEBAR_ROW_PAD_HEADER);
    let group_id = item.group_id();
    let collapsed = item.collapsed();

    let arrow_icon = if collapsed {
        "pan-end-symbolic"
    } else {
        "pan-down-symbolic"
    };
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
    row.append(&arrow);

    let label = gtk4::Label::new(Some(&item.name()));
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_tooltip_text(Some(&item.name()));
    row.append(&label);

    let count_label = gtk4::Label::new(Some(&item.count().to_string()));
    count_label.add_css_class(CSS_DIM_LABEL);
    row.append(&count_label);

    let edit = gtk4::Button::from_icon_name("document-edit-symbolic");
    edit.add_css_class(CSS_FLAT);
    edit.set_tooltip_text(Some(&crate::tr!("Edit rules")));
    edit.set_valign(gtk4::Align::Center);
    {
        let sc = state.clone();
        edit.connect_clicked(move |_| {
            super::auto_group_dialog::show_auto_group_edit(&sc, group_id);
        });
    }
    row.append(&edit);

    let sc = state.clone();
    let row_weak = row.downgrade();
    let group_name = item.name();
    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    right_click.connect_pressed(move |_, _, x, y| {
        let Some(r) = row_weak.upgrade() else {
            return;
        };
        let menu = gio::Menu::new();
        menu.append(Some(&crate::tr!("Edit rules")), Some("grp.auto-edit"));
        menu.append(Some(&crate::tr!("Delete")), Some("grp.auto-delete"));

        let popover = gtk4::PopoverMenu::from_model(Some(&menu));
        popover.set_halign(gtk4::Align::Start);
        popover.set_has_arrow(false);

        let actions = gio::SimpleActionGroup::new();
        let sc2 = sc.clone();
        let gname = group_name.clone();
        let edit_action = gio::SimpleAction::new("auto-edit", None);
        edit_action.connect_activate(move |_, _| {
            super::auto_group_dialog::show_auto_group_edit(&sc2, group_id);
        });
        actions.add_action(&edit_action);

        let sc2 = sc.clone();
        let delete_action = gio::SimpleAction::new("auto-delete", None);
        delete_action.connect_activate(move |_, _| {
            confirm_delete_auto_group(&sc2, group_id, &gname);
        });
        actions.add_action(&delete_action);

        super::helpers::popup_context_popover(&r, &popover, &actions, "grp.auto", x as i32, y as i32);
    });
    row.add_controller(right_click);
}

/// Deleting a rule group drops the rules, never the games — say so in
/// the confirmation, then rebuild everything.
fn confirm_delete_auto_group(state: &SharedState, group_id: i64, name: &str) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(
        Some(&crate::tr!("Delete auto group")),
        Some(
            &crate::tr!("The rules of \"{}\" go away. The games themselves stay.")
                .replacen("{}", name, 1),
        ),
    );
    dialog.add_response("cancel", &crate::tr!("Cancel"));
    dialog.add_response("delete", &crate::tr!("Delete"));
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    let sc = state.clone();
    dialog.connect_response(Some("delete"), move |_, _| {
        let db = sc.borrow().db.clone();
        let db_id = super::auto_groups::to_db_id(group_id);
        if let Err(e) = ira_db::delete_auto_group(&db, db_id) {
            eprintln!("Failed to delete the auto group: {e}");
            return;
        }
        sc.borrow_mut().auto_groups.retain(|g| g.id != group_id);
        sc.borrow_mut().collapsed_collections.remove(&group_id);
        sc.borrow_mut().selected_group = GroupSelection::AllGames;
        rebuild_sidebar(&sc);
    });
    dialog.present(Some(&window));
}

fn sidebar_bind_collection_header(state: &SharedState, row: &gtk4::Box, item: &SidebarItem) {
    row.add_css_class(CSS_SIDEBAR_ROW_PAD_HEADER);
    let group_id = item.group_id();
    let collapsed = item.collapsed();

    let arrow_icon = if collapsed {
        "pan-end-symbolic"
    } else {
        "pan-down-symbolic"
    };
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
    row.append(&arrow);

    let header_name = if item.kind() == SidebarItemKind::UncategorizedHeader {
        crate::tr!("Uncategorized")
    } else {
        item.name()
    };
    let label = gtk4::Label::new(Some(&header_name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    row.append(&label);

    let count_label = gtk4::Label::new(Some(&item.count().to_string()));
    count_label.add_css_class(CSS_DIM_LABEL);
    row.append(&count_label);

    if item.kind() == SidebarItemKind::CollectionHeader
        && state.borrow().groups.iter().any(|g| g.id == group_id)
    {
        let sc = state.clone();
        let group_name = header_name;
        let row_weak = row.downgrade();
        let right_click = gtk4::GestureClick::new();
        right_click.set_button(3);
        right_click.connect_pressed(move |_, _, x, y| {
            if let Some(r) = row_weak.upgrade() {
                let menu = gio::Menu::new();
                menu.append(Some(&crate::tr!("Rename")), Some("grp.rename"));
                menu.append(Some(&crate::tr!("Delete")), Some("grp.delete"));

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

                super::helpers::popup_context_popover(&r, &popover, &actions, "grp", x as i32, y as i32);
            }
        });
        row.add_controller(right_click);
    }
}

fn sidebar_bind_game(state: &SharedState, row: &gtk4::Box, item: &SidebarItem) {
    row.add_css_class(CSS_SIDEBAR_ROW_PAD_GAME);
    let icon = gtk4::Image::from_icon_name("games-symbolic");
    icon.set_pixel_size(24);
    if !item.icon_path().is_empty() {
        queue_icon_load(icon.clone(), item.icon_path());
    }
    row.append(&icon);

    let title_label = gtk4::Label::new(Some(&item.name()));
    title_label.set_xalign(0.0);
    title_label.add_css_class(CSS_SIDEBAR_ROW_TITLE);
    title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    title_label.set_hexpand(true);
    title_label.set_tooltip_text(Some(&item.name()));
    row.append(&title_label);

    if item.hidden() {
        row.add_css_class(CSS_HIDDEN_GAME);
    }
    if item.playing() {
        row.add_css_class(CSS_PLAYING_GAME);
    }

    let sc = state.clone();
    let db_id = item.db_id();
    let variant_id = item.variant_id();
    let row_weak = row.downgrade();
    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    right_click.connect_pressed(move |_, _, x, y| {
        if let Some(r) = row_weak.upgrade() {
            let selected_ids = {
                let s = sc.borrow();
                s.sidebar_selection.selected_db_ids()
            };
            let item_grid_id = match variant_id {
                Some(vid) => format!("{}-v{}", db_id, vid),
                None => db_id.to_string(),
            };
            if selected_ids.len() > 1 && selected_ids.contains(&item_grid_id) {
                let db_ids: HashSet<i64> = selected_ids
                    .iter()
                    .map(|s| ira_models::parse_db_id(s))
                    .collect();
                show_multi_game_context_menu(&sc, &db_ids, &r, x, y);
            } else {
                let game = sc.borrow().find_game(db_id, variant_id);
                if let Some(game) = game {
                    show_game_context_menu(&sc, &game, &r, x, y, None::<&gtk4::ListBoxRow>);
                }
            }
        }
    });
    row.add_controller(right_click);
}

fn sidebar_bind_factory(
    _factory: &gtk4::SignalListItemFactory,
    list_item_obj: &glib::Object,
    state: &SharedState,
) {
    let _span = tracing::info_span!("sidebar_bind").entered();
    let list_item = list_item_obj.downcast_ref::<gtk4::ListItem>().unwrap();
    let row = list_item.child().unwrap().downcast::<gtk4::Box>().unwrap();

    let item = list_item.item().unwrap().downcast::<SidebarItem>().unwrap();

    clear_children(&row);

    row.remove_css_class(CSS_PLAYING_GAME);
    row.remove_css_class(CSS_HIDDEN_GAME);
    row.remove_css_class(CSS_SIDEBAR_ROW_PAD_GAME);
    row.remove_css_class(CSS_SIDEBAR_ROW_PAD_HEADER);

    match item.kind() {
        SidebarItemKind::AllGames => sidebar_bind_all_games(state, &row),
        SidebarItemKind::CollectionHeader | SidebarItemKind::UncategorizedHeader => {
            sidebar_bind_collection_header(state, &row, &item);
        }
        SidebarItemKind::AutoGroupHeader => {
            sidebar_bind_auto_group_header(state, &row, &item);
        }
        SidebarItemKind::Game => sidebar_bind_game(state, &row, &item),
    }
}

fn sidebar_unbind_factory(_factory: &gtk4::SignalListItemFactory, list_item_obj: &glib::Object) {
    let list_item = list_item_obj.downcast_ref::<gtk4::ListItem>().unwrap();
    if let Some(child) = list_item.child() {
        let row = child.downcast::<gtk4::Box>().unwrap();
        let controllers = row.observe_controllers();
        let to_remove: Vec<gtk4::EventController> = (0..controllers.n_items())
            .filter_map(|i| {
                controllers
                    .item(i)
                    .and_then(|o| o.downcast::<gtk4::EventController>().ok())
            })
            .collect();
        for ctrl in to_remove {
            row.remove_controller(&ctrl);
        }
        clear_children(&row);
    }
}

pub fn build_factory(state: &SharedState) -> gtk4::SignalListItemFactory {
    let factory = gtk4::SignalListItemFactory::new();

    factory.connect_setup(sidebar_setup_factory);

    let state_for_bind = state.clone();
    factory.connect_bind(move |f, item| sidebar_bind_factory(f, item, &state_for_bind));

    factory.connect_unbind(sidebar_unbind_factory);

    factory
}
