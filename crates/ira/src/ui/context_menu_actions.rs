use super::edit_game_dialog::show_edit_game_dialog;
use super::helpers::{open_file_location, open_folder};
use super::state::SharedState;
use crate::AppMessage;
use crate::Game;
use adw::prelude::*;

pub(super) fn setup_play_action(actions: &gio::SimpleActionGroup, state: SharedState, game: Game) {
    let play_action = gio::SimpleAction::new("play", None);
    play_action.connect_activate(move |_, _| {
        let db_id = game.db_id;
        let is_running = state
            .borrow()
            .running_games
            .lock()
            .unwrap()
            .contains_key(&db_id);
        if is_running {
            super::play_button::stop_game(&state, db_id);
        } else {
            match super::play_button::launch_game(&state, db_id, game.variant_id) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Failed to launch game: {}", e);
                    let _ = state.borrow().sender.send(AppMessage::AddGameError(e));
                }
            }
        }
    });
    actions.add_action(&play_action);
}

pub(super) fn setup_edit_action(actions: &gio::SimpleActionGroup, state: SharedState, db_id: i64) {
    let edit_action = gio::SimpleAction::new("edit", None);
    edit_action.connect_activate(move |_, _| {
        show_edit_game_dialog(&state, db_id);
    });
    actions.add_action(&edit_action);
}

pub(super) fn setup_controller_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let controller_action = gio::SimpleAction::new("controller", None);
    controller_action.connect_activate(move |_, _| {
        super::edit_game_controller::open_controller_settings(&state, &game);
    });
    actions.add_action(&controller_action);
}

pub(super) fn setup_play_history_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    db_id: i64,
    variant_id: Option<i64>,
) {
    let play_hist_action = gio::SimpleAction::new("play_history", None);
    play_hist_action.connect_activate(move |_, _| {
        super::play_history::show_play_history_dialog(&state, db_id, variant_id);
    });
    actions.add_action(&play_hist_action);
}

pub(super) fn setup_hide_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    db_id: i64,
    current_hidden: bool,
) {
    let hide_action = gio::SimpleAction::new("hide", None);
    hide_action.connect_activate(move |_, _| {
        let new_hidden = !current_hidden;
        {
            let s = state.borrow();
            if let Some(g) = s.games.iter().find(|g| g.db_id == db_id) {
                if let Err(e) = ira_db::set_game_hidden(&s.db, g.db_id, new_hidden) {
                    eprintln!("Failed to set hidden: {}", e);
                }
            }
        }
        if let Some(g) = state
            .borrow_mut()
            .games
            .iter_mut()
            .find(|g| g.db_id == db_id)
        {
            g.hidden = new_hidden;
        }
        super::sidebar::rebuild_sidebar(&state);
        super::grid_view::refresh_grid_store(&state);
        super::grid_view::refresh_grid_header(&state);
    });
    actions.add_action(&hide_action);
}

pub(super) fn setup_delete_game_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let delete_game_action = gio::SimpleAction::new("delete_game", None);
    delete_game_action.connect_activate(move |_, _| {
        let window = state.borrow().window.clone();
        let sc = state.clone();
        super::helpers::confirm_dialog(
            &window,
            &crate::tr!("Remove game?"),
            &crate::tr!("Remove \"{}\"?").replacen("{}", &game.name, 1),
            &crate::tr!("Remove"),
            adw::ResponseAppearance::Destructive,
            move || {
                let db = sc.borrow().db.clone();
                let db_id = game.db_id;
                if let Err(e) = ira_db::delete_game_config(&db, db_id) {
                    eprintln!("Failed to delete game config: {}", e);
                }
                if let Err(e) = ira_db::remove_game(&db, db_id) {
                    eprintln!("Failed to remove game: {}", e);
                    return;
                }
                let mut s = sc.borrow_mut();
                s.games.retain(|g| g.db_id != db_id);
                let was_selected = ira_models::parse_db_id(&s.selected_id) == db_id;
                if was_selected {
                    s.selected_id = String::new();
                    s.selected_group = ira_models::GroupSelection::AllGames;
                    drop(s);
                    super::sidebar::rebuild_sidebar(&sc);
                    super::message_helpers::clear_content(&sc);
                } else {
                    drop(s);
                    super::sidebar::rebuild_sidebar(&sc);
                }
                super::grid_view::refresh_grid_store(&sc);
                super::grid_view::refresh_grid_header(&sc);
            },
        );
    });
    actions.add_action(&delete_game_action);
}

pub(super) fn setup_run_manual_script_action(
    actions: &gio::SimpleActionGroup,
    game: Game,
    script: String,
    working_dir: Option<String>,
) {
    let run_manual_script = gio::SimpleAction::new("run_manual_script", None);
    run_manual_script.connect_activate(move |_, _| {
        // The script runs outside any launcher wrapper, so it gets the app's
        // own environment; spawn_detached logs its output into the game log.
        let env: Vec<(String, String)> = std::env::vars().collect();
        let header = format!("Started manual script for {}", game.name);
        if let Err(e) = ira_launcher::wrapper::spawn_detached(
            &["sh".to_string(), "-c".to_string(), script.clone()],
            &env,
            working_dir.as_deref(),
            game.db_id,
            header,
            None,
        ) {
            eprintln!("Failed to run manual script: {}", e);
        }
    });
    actions.add_action(&run_manual_script);
}

pub(super) fn setup_open_game_folder_action(
    actions: &gio::SimpleActionGroup,
    game_file: Option<String>,
    game_folder: Option<String>,
) {
    let open_game_folder = gio::SimpleAction::new("open_game_folder", None);
    open_game_folder.connect_activate(move |_, _| {
        if let Some(ref file) = game_file {
            open_file_location(file);
        } else if let Some(ref folder) = game_folder {
            open_folder(folder);
        }
    });
    actions.add_action(&open_game_folder);
}

pub(super) fn setup_open_wine_prefix_action(
    actions: &gio::SimpleActionGroup,
    wine_prefix: Option<String>,
) {
    if let Some(pfx) = wine_prefix {
        let open_wine_prefix = gio::SimpleAction::new("open_wine_prefix", None);
        open_wine_prefix.connect_activate(move |_, _| {
            open_folder(&pfx);
        });
        actions.add_action(&open_wine_prefix);
    }
}

pub(super) fn setup_open_images_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let open_images = gio::SimpleAction::new("open_images", None);
    let save_dir = state.borrow().save_dir.clone();
    open_images.connect_activate(move |_, _| {
        let path = ira_parser::game_data_dir(&save_dir, &game);
        open_folder(&path.to_string_lossy());
    });
    actions.add_action(&open_images);
}

pub(super) fn setup_open_save_location_action(actions: &gio::SimpleActionGroup, path: String) {
    let open_save = gio::SimpleAction::new("open_save_location", None);
    open_save.connect_activate(move |_, _| {
        open_folder(&path);
    });
    actions.add_action(&open_save);
}

/// The game's centralized save folder (`saves/<app_id>`), or `None` when the
/// game has no centralized save data yet. Emulator save folders (Gse/Nge) are
/// intentionally not covered here — their per-game folders are reachable via
/// the "Achievement status" menu items instead.
pub(super) fn centralized_save_path(save_dir: &str, game: &Game) -> Option<String> {
    let dir = ira_launcher::game_saves::centralized_save_dir(save_dir, &game.app_id);
    if dir.is_dir() && ira_launcher::game_saves::dir_has_save_data(&dir) {
        Some(dir.to_string_lossy().into_owned())
    } else {
        None
    }
}

pub(super) fn setup_open_steam_status_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let open_status = gio::SimpleAction::new("open_steam_status", None);
    let save_dir = state.borrow().save_dir.clone();
    open_status.connect_activate(move |_, _| {
        let path = format!("{}/emulator_saves/gbe/{}", save_dir, game.app_id);
        open_folder(&path);
    });
    actions.add_action(&open_status);
}

pub(super) fn setup_open_gog_status_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let open_gog = gio::SimpleAction::new("open_gog_status", None);
    let save_dir = state.borrow().save_dir.clone();
    open_gog.connect_activate(move |_, _| {
        let path = format!(
            "{}/emulator_saves/nge/{}/{}",
            save_dir,
            ira_parser::GALAXY_ID,
            game.platform_id
        );
        open_folder(&path);
    });
    actions.add_action(&open_gog);
}

pub(super) fn setup_toggle_group_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let toggle_group = gio::SimpleAction::new("toggle_group", Some(&i64::static_variant_type()));
    toggle_group.connect_activate(move |_, param| {
        let group_id = param.and_then(|p| p.get::<i64>()).unwrap_or(0);
        let db = state.borrow().db.clone();
        let existing =
            super::helpers::logged_db_vec("Failed to read game groups", ira_db::get_groups_for_game(&db, game.db_id));
        if existing.iter().any(|g| g.id == group_id) {
            if let Err(e) = ira_db::remove_game_from_group(&db, game.db_id, group_id) {
                eprintln!("Failed to remove game from group: {}", e);
            }
            if let Some(members) = state.borrow_mut().group_members.get_mut(&group_id) {
                members.remove(&game.db_id);
            }
        } else {
            if let Err(e) = ira_db::add_game_to_group(&db, game.db_id, group_id) {
                eprintln!("Failed to add game to group: {}", e);
            }
            state
                .borrow_mut()
                .group_members
                .entry(group_id)
                .or_default()
                .insert(game.db_id);
        }
        super::sidebar::rebuild_sidebar(&state);
    });
    actions.add_action(&toggle_group);
}

pub(super) fn setup_new_collection_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let new_collection = gio::SimpleAction::new("new_collection", None);
    new_collection.connect_activate(move |_, _| {
        let window = state.borrow().window.clone();
        let sc = state.clone();
        show_collection_name_dialog(window, sc, move |db, group_id| {
            if let Err(e) = ira_db::add_game_to_group(db, game.db_id, group_id) {
                eprintln!("Failed to add game to new group: {}", e);
            }
        });
    });
    actions.add_action(&new_collection);
}

pub(super) fn setup_multi_toggle_group_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    ids: Vec<i64>,
) {
    let toggle_group = gio::SimpleAction::new("toggle_group", Some(&i64::static_variant_type()));
    toggle_group.connect_activate(move |_, param| {
        let group_id = param.and_then(|p| p.get::<i64>()).unwrap_or(0);
        let db = state.borrow().db.clone();

        let all_in = ids.iter().all(|&db_id| {
            let game_groups = super::helpers::logged_db_vec(
                "Failed to read game groups",
                ira_db::get_groups_for_game(&db, db_id),
            );
            game_groups.iter().any(|g| g.id == group_id)
        });

        for &db_id in &ids {
            if all_in {
                if let Err(e) = ira_db::remove_game_from_group(&db, db_id, group_id) {
                    eprintln!("Failed to remove game from group: {}", e);
                }
                if let Some(members) = state.borrow_mut().group_members.get_mut(&group_id) {
                    members.remove(&db_id);
                }
            } else if let Err(e) = ira_db::add_game_to_group(&db, db_id, group_id) {
                eprintln!("Failed to add game to group: {}", e);
            } else {
                state
                    .borrow_mut()
                    .group_members
                    .entry(group_id)
                    .or_default()
                    .insert(db_id);
            }
        }
        super::sidebar::rebuild_sidebar(&state);
    });
    actions.add_action(&toggle_group);
}

/// Which metadata list a mass entity add targets.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MassEntityList {
    Family,
    Genre,
    Developer,
    Publisher,
}

impl MassEntityList {
    fn search(
        self,
        db: &ira_db::DbConn,
        term: &str,
    ) -> Vec<ira_models::ScraperEntity> {
        match self {
            MassEntityList::Family => {
                ira_db::search_families(db, term).unwrap_or_default()
            }
            MassEntityList::Genre => ira_db::search_genres(db, term).unwrap_or_default(),
            MassEntityList::Developer | MassEntityList::Publisher => {
                ira_db::scraper_companies_search(db, term).unwrap_or_default()
            }
        }
    }

    /// The entity for a hand-typed name, reusing the cache's row when
    /// it has one.
    fn mint(self, db: &ira_db::DbConn, term: &str) -> Option<ira_models::ScraperEntity> {
        match self {
            MassEntityList::Family => ira_db::local_family_entity(db, term),
            MassEntityList::Genre => ira_db::local_genre_entity(db, term),
            MassEntityList::Developer | MassEntityList::Publisher => {
                ira_db::steam_company_entity(db, term)
            }
        }
    }

    fn target(self, meta: &mut ira_models::ScraperMetadata) -> &mut Vec<ira_models::ScraperEntity> {
        match self {
            MassEntityList::Family => &mut meta.families,
            MassEntityList::Genre => &mut meta.genres,
            MassEntityList::Developer => &mut meta.developers,
            MassEntityList::Publisher => &mut meta.publishers,
        }
    }
}

/// Mass metadata adds: search the entity cache, pick one, and it lands
/// on every selected game's record.
pub(super) fn setup_multi_entity_add_actions(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    ids: Vec<i64>,
) {
    let choices: [(&str, String, MassEntityList); 4] = [
        ("mass_family", crate::tr!("Add to family…"), MassEntityList::Family),
        ("mass_genre", crate::tr!("Add to genre…"), MassEntityList::Genre),
        ("mass_developer", crate::tr!("Add to developer…"), MassEntityList::Developer),
        ("mass_publisher", crate::tr!("Add to publisher…"), MassEntityList::Publisher),
    ];
    for (name, title, list) in choices {
        let action = gio::SimpleAction::new(name, None);
        let state = state.clone();
        let ids = ids.clone();
        action.connect_activate(move |_, _| {
            let window = state.borrow().window.clone();
            show_mass_entity_add(&state, &window, ids.clone(), list, title.clone());
        });
        actions.add_action(&action);
    }
}

/// The search dialog behind a mass add: the cache's matches plus a
/// custom-add row for names the sources have never listed. A pick
/// writes every selected game's record at once.
fn show_mass_entity_add(
    state: &SharedState,
    parent: &impl glib::object::IsA<gtk4::Window>,
    ids: Vec<i64>,
    list: MassEntityList,
    title: String,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&title);
    dialog.set_content_width(460);
    dialog.set_content_height(440);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let heading = gtk4::Label::new(Some(&title));
    heading.add_css_class("heading");
    header.set_title_widget(Some(&heading));
    toolbar.add_top_bar(&header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some(&crate::tr!("Search…")));
    content.append(&entry);
    let (scrolled, results) = super::helpers::clamped_boxed_list(460);
    scrolled.set_vexpand(true);
    content.append(&scrolled);
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    let state = std::rc::Rc::new(state.clone());
    let ids = std::rc::Rc::new(ids);
    let apply = {
        let state = state.clone();
        let ids = ids.clone();
        let dialog = dialog.clone();
        move |entity: ira_models::ScraperEntity| {
            let db = state.borrow().db.clone();
            for &db_id in ids.iter() {
                let mut meta = ira_db::scraper_metadata_for_game(&db, db_id)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let target = list.target(&mut meta);
                // The same entity twice is noise, not data.
                if target.iter().any(|held| held.id == entity.id) {
                    continue;
                }
                target.push(entity.clone());
                if let Err(e) = ira_db::store_scraper_metadata(&db, db_id, &meta) {
                    eprintln!("Failed to store metadata for {db_id}: {e}");
                }
            }
            super::sidebar::rebuild_sidebar_and_show_grid(&state);
            for &db_id in ids.iter() {
                super::edit_game_scraper::refresh_scraper_section(&state, db_id);
            }
            dialog.close();
        }
    };

    let populate = {
        let state = state.clone();
        let apply = apply.clone();
        let results = results.clone();
        let ids = ids.clone();
        move |term: &str| {
            super::helpers::clear_children(&results);
            let db = state.borrow().db.clone();
            let term = term.trim();
            for entity in list.search(&db, term) {
                let apply = apply.clone();
                let row_entity = entity.clone();
                let row = adw::ActionRow::new();
                row.set_use_markup(false);
                row.set_title(&entity.name);
                row.set_subtitle(&format!("id {}", entity.id));
                let pick = gtk4::Button::with_label(&crate::tr!("Add"));
                pick.add_css_class(super::css::CSS_SUGGESTED_ACTION);
                pick.set_valign(gtk4::Align::Center);
                pick.connect_clicked(move |_| apply(row_entity.clone()));
                row.add_suffix(&pick);
                results.append(&row);
            }
            // The typed name itself, minted on click only — the db
            // must not fill up with every prefix the user tried.
            if !term.is_empty() {
                let already_held = |db_id: i64| {
                    let mut meta = ira_db::scraper_metadata_for_game(&db, db_id)
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    list.target(&mut meta)
                        .iter()
                        .any(|held| held.name.eq_ignore_ascii_case(term))
                };
                let all_held = ids.iter().all(|&db_id| already_held(db_id));
                let apply = apply.clone();
                let term_c = term.to_string();
                let db_for_mint = db.clone();
                let row = adw::ActionRow::new();
                row.set_use_markup(false);
                row.set_title(&crate::tr!("Add \"{}\"").replacen("{}", term, 1));
                let pick = gtk4::Button::with_label(&crate::tr!("Add"));
                pick.add_css_class(super::css::CSS_SUGGESTED_ACTION);
                pick.set_valign(gtk4::Align::Center);
                pick.set_sensitive(!all_held);
                pick.connect_clicked(move |_| {
                    if let Some(entity) = list.mint(&db_for_mint, &term_c) {
                        apply(entity);
                    }
                });
                row.add_suffix(&pick);
                results.append(&row);
            }
        }
    };
    let populate_for_changed = populate.clone();
    entry.connect_search_changed(move |entry| populate_for_changed(&entry.text()));
    populate("");

    dialog.present(Some(parent.upcast_ref()));
}

pub(super) fn setup_multi_new_collection_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    ids: Vec<i64>,
) {
    let new_collection = gio::SimpleAction::new("new_collection", None);
    new_collection.connect_activate(move |_, _| {
        let window = state.borrow().window.clone();
        let sc = state.clone();
        let ids = ids.clone();
        show_collection_name_dialog(window, sc, move |db, group_id| {
            for &db_id in &ids {
                if let Err(e) = ira_db::add_game_to_group(db, db_id, group_id) {
                    eprintln!("Failed to add game to new group: {}", e);
                }
            }
        });
    });
    actions.add_action(&new_collection);
}

pub(super) fn setup_multi_toggle_hide_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    ids: Vec<i64>,
    all_hidden: bool,
) {
    let toggle_hide = gio::SimpleAction::new("toggle_hide", None);
    toggle_hide.connect_activate(move |_, _| {
        let new_hidden = !all_hidden;
        let db = state.borrow().db.clone();
        for &db_id in &ids {
            if let Err(e) = ira_db::set_game_hidden(&db, db_id, new_hidden) {
                eprintln!("Failed to set hidden: {}", e);
            }
        }
        {
            let mut s = state.borrow_mut();
            for g in s.games.iter_mut() {
                if ids.contains(&g.db_id) {
                    g.hidden = new_hidden;
                }
            }
        }
        super::sidebar::rebuild_sidebar(&state);
        super::grid_view::refresh_grid_store(&state);
    });
    actions.add_action(&toggle_hide);
}

pub(super) fn show_collection_name_dialog(
    window: adw::ApplicationWindow,
    state: SharedState,
    add_games: impl Fn(&ira_db::DbConn, i64) + 'static,
) {
    let dialog = adw::AlertDialog::new(
        Some(&crate::tr!("New collection")),
        Some(&crate::tr!("Enter a name for the collection:")),
    );
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some(&crate::tr!("Collection name")));
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    entry.set_margin_top(8);
    entry.set_margin_bottom(8);
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", &crate::tr!("Cancel"));
    dialog.add_response("create", &crate::tr!("Create"));
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    let entry_clone = entry;
    dialog.connect_response(None, move |_, resp| {
        if resp != "create" {
            return;
        }
        let name = entry_clone.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let db = state.borrow().db.clone();
        match ira_db::create_group(&db, &name) {
            Ok(group_id) => {
                add_games(&db, group_id);
                let groups =
                    super::helpers::logged_db_vec("Failed to read groups", ira_db::get_all_groups(&db));
                let members = super::helpers::logged_db_vec(
                    "Failed to read group members",
                    ira_db::get_game_ids_in_group(&db, group_id),
                );
                state.borrow_mut().groups = groups;
                state
                    .borrow_mut()
                    .group_members
                    .insert(group_id, members.into_iter().collect());
                super::sidebar::rebuild_sidebar(&state);
            }
            Err(e) => {
                eprintln!("Failed to create group: {}", e);
            }
        }
    });
    dialog.present(Some(&window));
}
