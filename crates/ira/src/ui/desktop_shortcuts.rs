use super::state::SharedState;
use gio::prelude::*;
use std::{cell::Cell, rc::Rc};

struct ShortcutItem {
    menu: gio::Menu,
    index: i32,
    exists: Cell<bool>,
}

impl ShortcutItem {
    fn new(menu: &gio::Menu) -> Self {
        let index = menu.n_items();
        menu.append(
            Some(&crate::tr!("App menu shortcut")),
            Some("game.shortcut"),
        );
        Self {
            menu: menu.clone(),
            index,
            exists: Cell::new(false),
        }
    }

    fn set_exists(&self, exists: bool) {
        self.exists.set(exists);
        let label = if exists {
            crate::tr!("Remove app menu shortcut")
        } else {
            crate::tr!("Create app menu shortcut")
        };
        self.menu.remove(self.index);
        self.menu
            .insert(self.index, Some(&label), Some("game.shortcut"));
    }
}

pub(super) fn add_action(
    menu: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    state: &SharedState,
    game: &ira_models::Game,
) {
    let item = Rc::new(ShortcutItem::new(menu));
    let action = gio::SimpleAction::new("shortcut", None);
    action.set_enabled(false);
    action.connect_activate({
        let item = item.clone();
        let state = Rc::downgrade(state);
        let game = game.clone();
        move |action, _| {
            if let Some(state) = state.upgrade() {
                change_shortcut(&state, &game, item.clone(), action);
            }
        }
    });
    actions.add_action(&action);
    let game = game.clone();
    let weak = Rc::downgrade(state);
    glib::MainContext::default().spawn_local(async move {
        let result = gio::spawn_blocking(move || {
            crate::desktop::integration::shortcut_exists(
                &ira_config::data_home(),
                game.db_id,
                game.variant_id,
            )
        })
        .await
        .map_err(|_| "Shortcut state worker panicked".to_string())
        .and_then(|result| result);
        match result {
            Ok(exists) => {
                item.set_exists(exists);
                action.set_enabled(true);
            }
            Err(error) => {
                eprintln!("Read shortcut state: {error}");
                if let Some(state) = weak.upgrade() {
                    super::helpers::toast_on_root(&state.borrow().window, &error);
                }
            }
        }
    });
}

fn change_shortcut(
    state: &SharedState,
    game: &ira_models::Game,
    item: Rc<ShortcutItem>,
    action: &gio::SimpleAction,
) {
    action.set_enabled(false);
    let action = action.clone();
    let create = !item.exists.get();
    let game = state
        .borrow()
        .find_game(game.db_id, game.variant_id)
        .unwrap_or_else(|| game.clone());
    let weak = Rc::downgrade(state);
    glib::MainContext::default().spawn_local(async move {
        let result = gio::spawn_blocking(move || {
            crate::desktop::integration::shortcut_for_game(&game, create)
        })
        .await
        .map_err(|_| "Shortcut worker panicked".to_string())
        .and_then(|result| result);
        match result {
            Ok(()) => {
                item.set_exists(create);
                action.set_enabled(true);
                if let Some(state) = weak.upgrade() {
                    let message = if create {
                        crate::tr!("App menu shortcut created.")
                    } else {
                        crate::tr!("App menu shortcut removed.")
                    };
                    super::helpers::toast_on_root(&state.borrow().window, &message);
                }
            }
            Err(error) => {
                eprintln!("Change shortcut: {error}");
                action.set_enabled(true);
                if let Some(state) = weak.upgrade() {
                    super::helpers::toast_on_root(&state.borrow().window, &error);
                }
            }
        }
    });
}

pub(super) fn refresh_game(state: &SharedState, db_id: i64) {
    let state = state.borrow();
    refresh_existing(state.games.iter().filter(|game| game.db_id == db_id));
}

pub(super) fn refresh_existing<'a>(games: impl IntoIterator<Item = &'a ira_models::Game>) {
    let games: Vec<_> = games
        .into_iter()
        .map(|game| ira_models::Game {
            db_id: game.db_id,
            variant_id: game.variant_id,
            name: game.name.clone(),
            icon_path: game.icon_path.clone(),
            square_path: game.square_path.clone(),
            ..Default::default()
        })
        .collect();
    if games.is_empty() {
        return;
    }
    glib::MainContext::default().spawn_local(async move {
        if gio::spawn_blocking(move || {
            let root = ira_config::data_home();
            for game in games {
                if let Err(error) = crate::desktop::integration::refresh_shortcut(&root, &game) {
                    eprintln!("Refresh shortcut for {}: {error}", game.db_id);
                }
            }
        })
        .await
        .is_err()
        {
            eprintln!("Shortcut refresh worker panicked");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shortcut_item_is_one_direct_action_that_tracks_file_state() {
        let menu = gio::Menu::new();
        menu.append(Some("Other action"), Some("game.other"));
        let item = ShortcutItem::new(&menu);
        menu.append(Some("Last action"), Some("game.last"));
        for (exists, label) in [
            (false, crate::tr!("Create app menu shortcut")),
            (true, crate::tr!("Remove app menu shortcut")),
            (false, crate::tr!("Create app menu shortcut")),
        ] {
            item.set_exists(exists);
            assert_eq!(menu.n_items(), 3);
            assert_eq!(item.exists.get(), exists);
            assert!(menu.item_link(1, gio::MENU_LINK_SUBMENU).is_none());
            assert_eq!(
                menu.item_attribute_value(1, gio::MENU_ATTRIBUTE_LABEL, None)
                    .unwrap()
                    .get::<String>(),
                Some(label)
            );
            assert_eq!(
                menu.item_attribute_value(1, gio::MENU_ATTRIBUTE_ACTION, None)
                    .unwrap()
                    .get::<String>()
                    .as_deref(),
                Some("game.shortcut")
            );
        }
    }
}
