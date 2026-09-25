use super::edit_game_dialog::show_edit_game_dialog;
use super::helpers::{open_file_location, open_folder};
use super::state::SharedState;
use crate::AppMessage;
use crate::Game;
use adw::prelude::*;

/// The entity/group dialog list repainter, shared out so rows can
/// refresh the list after they mutate it.
type RepaintFn = std::rc::Rc<dyn Fn(&str)>;
type RepaintCell = std::rc::Rc<std::cell::RefCell<Option<RepaintFn>>>;

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

    pub(super) fn add_action(self) -> &'static str {
        match self {
            MassEntityList::Family => "game.mass_family",
            MassEntityList::Genre => "game.mass_genre",
            MassEntityList::Developer => "game.mass_developer",
            MassEntityList::Publisher => "game.mass_publisher",
        }
    }

    pub(super) fn add_dialog_title(self) -> String {
        match self {
            MassEntityList::Family => crate::tr!("Add to family…"),
            MassEntityList::Genre => crate::tr!("Add to genre…"),
            MassEntityList::Developer => crate::tr!("Add to developer…"),
            MassEntityList::Publisher => crate::tr!("Add to publisher…"),
        }
    }

    pub(super) fn entities(
        self,
        meta: &ira_models::ScraperMetadata,
    ) -> &[ira_models::ScraperEntity] {
        match self {
            MassEntityList::Family => &meta.families,
            MassEntityList::Genre => &meta.genres,
            MassEntityList::Developer => &meta.developers,
            MassEntityList::Publisher => &meta.publishers,
        }
    }
}

/// One pickable collection behind the organise dialogs: metadata
/// lists (family/genre/...) and plain groups share search, shared-
/// first display, add/remove and mint — one dialog, many sources.
trait OrganizeSource {
    fn dialog_title(&self) -> String;
    fn search(&self, db: &ira_db::DbConn, term: &str) -> Vec<ira_models::ScraperEntity>;
    fn mint(&self, state: &SharedState, term: &str) -> Option<ira_models::ScraperEntity>;
    fn held_by(&self, db: &ira_db::DbConn, db_id: i64) -> Vec<ira_models::ScraperEntity>;
    fn add(&self, state: &SharedState, db_id: i64, entity: &ira_models::ScraperEntity);
    fn remove(&self, state: &SharedState, db_id: i64, entity_id: &str);
}

impl OrganizeSource for MassEntityList {
    fn dialog_title(&self) -> String {
        self.add_dialog_title()
    }

    fn search(&self, db: &ira_db::DbConn, term: &str) -> Vec<ira_models::ScraperEntity> {
        MassEntityList::search(*self, db, term)
    }

    fn mint(&self, state: &SharedState, term: &str) -> Option<ira_models::ScraperEntity> {
        MassEntityList::mint(*self, &state.borrow().db, term)
    }

    fn held_by(&self, db: &ira_db::DbConn, db_id: i64) -> Vec<ira_models::ScraperEntity> {
        ira_db::scraper_metadata_for_game(db, db_id)
            .ok()
            .flatten()
            .map(|meta| self.entities(&meta).to_vec())
            .unwrap_or_default()
    }

    fn add(&self, state: &SharedState, db_id: i64, entity: &ira_models::ScraperEntity) {
        let db = state.borrow().db.clone();
        let mut meta = ira_db::scraper_metadata_for_game(&db, db_id)
            .ok()
            .flatten()
            .unwrap_or_default();
        let target = self.target(&mut meta);
        // The same entity twice is noise, not data.
        if target.iter().any(|held| held.id == entity.id) {
            return;
        }
        target.push(entity.clone());
        if let Err(e) = ira_db::store_scraper_metadata(&db, db_id, &meta) {
            eprintln!("Failed to store metadata for {db_id}: {e}");
        }
    }

    fn remove(&self, state: &SharedState, db_id: i64, entity_id: &str) {
        let db = state.borrow().db.clone();
        let mut meta = ira_db::scraper_metadata_for_game(&db, db_id)
            .ok()
            .flatten()
            .unwrap_or_default();
        let target = self.target(&mut meta);
        if !target.iter().any(|held| held.id == entity_id) {
            return;
        }
        target.retain(|held| held.id != entity_id);
        if let Err(e) = ira_db::store_scraper_metadata(&db, db_id, &meta) {
            eprintln!("Failed to store metadata for {db_id}: {e}");
        }
    }
}

/// Plain groups as an organise source: membership rows instead of a
/// metadata record, creation instead of minting.
struct GroupSource;

impl OrganizeSource for GroupSource {
    fn dialog_title(&self) -> String {
        crate::tr!("Groups")
    }

    fn search(&self, db: &ira_db::DbConn, term: &str) -> Vec<ira_models::ScraperEntity> {
        let query = term.trim().to_lowercase();
        ira_db::get_all_groups(db)
            .unwrap_or_default()
            .into_iter()
            .filter(|group| {
                query.is_empty() || group.name.to_lowercase().contains(&query)
            })
            .map(|group| ira_models::ScraperEntity {
                id: group.id.to_string(),
                name: group.name,
            })
            .collect()
    }

    fn mint(&self, state: &SharedState, term: &str) -> Option<ira_models::ScraperEntity> {
        let name = term.trim().to_string();
        if name.is_empty() {
            return None;
        }
        let db = state.borrow().db.clone();
        match ira_db::create_group(&db, &name) {
            Ok(id) => {
                state.borrow_mut().groups = super::helpers::logged_db_vec(
                    "Failed to read groups",
                    ira_db::get_all_groups(&db),
                );
                Some(ira_models::ScraperEntity {
                    id: id.to_string(),
                    name,
                })
            }
            Err(e) => {
                eprintln!("Failed to create group: {e}");
                None
            }
        }
    }

    fn held_by(&self, db: &ira_db::DbConn, db_id: i64) -> Vec<ira_models::ScraperEntity> {
        ira_db::get_groups_for_game(db, db_id)
            .unwrap_or_default()
            .iter()
            .map(|group| ira_models::ScraperEntity {
                id: group.id.to_string(),
                name: group.name.clone(),
            })
            .collect()
    }

    fn add(&self, state: &SharedState, db_id: i64, entity: &ira_models::ScraperEntity) {
        let Ok(group_id) = entity.id.parse::<i64>() else {
            return;
        };
        let db = state.borrow().db.clone();
        if let Err(e) = ira_db::add_game_to_group(&db, db_id, group_id) {
            eprintln!("Failed to add game to group: {e}");
            return;
        }
        state
            .borrow_mut()
            .group_members
            .entry(group_id)
            .or_default()
            .insert(db_id);
    }

    fn remove(&self, state: &SharedState, db_id: i64, entity_id: &str) {
        let Ok(group_id) = entity_id.parse::<i64>() else {
            return;
        };
        let db = state.borrow().db.clone();
        if let Err(e) = ira_db::remove_game_from_group(&db, db_id, group_id) {
            eprintln!("Failed to remove game from group: {e}");
            return;
        }
        if let Some(members) = state.borrow_mut().group_members.get_mut(&group_id) {
            members.remove(&db_id);
        }
    }
}

/// Split one metadata list across a selection: entities every selected
/// game holds come first and toggle to remove-from-all; everything else
/// toggles to complete-the-set. Pure over the loaded lists, so menu
/// builders stay thin and this stays tested.
pub(super) fn partition_entities(
    all: &[Vec<ira_models::ScraperEntity>],
) -> (
    Vec<ira_models::ScraperEntity>,
    Vec<ira_models::ScraperEntity>,
) {
    use std::collections::{HashMap, HashSet};
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut by_id: HashMap<&str, &ira_models::ScraperEntity> = HashMap::new();
    for list in all {
        // One vote per game even when a record repeats an entity.
        let mut seen = HashSet::new();
        for entity in list {
            if seen.insert(entity.id.as_str()) {
                *counts.entry(entity.id.as_str()).or_default() += 1;
                by_id.entry(entity.id.as_str()).or_insert(entity);
            }
        }
    }
    let mut shared = Vec::new();
    let mut rest = Vec::new();
    for (id, entity) in by_id {
        if counts[id] == all.len() {
            shared.push(entity.clone());
        } else {
            rest.push(entity.clone());
        }
    }
    // Stable menu order within each section.
    let by_name = |a: &ira_models::ScraperEntity, b: &ira_models::ScraperEntity| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    };
    shared.sort_by(by_name);
    rest.sort_by(by_name);
    (shared, rest)
}

/// One toggle per metadata list: held-by-all removes from every
/// selected game, otherwise the missing games gain it — the group
/// toggle's shape, applied to entities. The target carries the
/// entity's id and display name, fresh from the row just shown.
pub(super) fn setup_multi_entity_add_actions(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    ids: Vec<i64>,
) {
    for list in [
        MassEntityList::Family,
        MassEntityList::Genre,
        MassEntityList::Developer,
        MassEntityList::Publisher,
    ] {
        let action = gio::SimpleAction::new(list.add_action().trim_start_matches("game."), None);
        let state = state.clone();
        let ids = ids.clone();
        let source: std::rc::Rc<dyn OrganizeSource> = std::rc::Rc::new(list);
        action.connect_activate(move |_, _| {
            let window = state.borrow().window.clone();
            show_organize_picker(&state, &window, ids.clone(), source.clone());
        });
        actions.add_action(&action);
    }
}

/// The search dialog behind a mass add: the cache's matches plus a
/// custom-add row for names the sources have never listed. A pick
/// writes every selected game's record at once.
fn show_organize_picker(
    state: &SharedState,
    parent: &impl glib::object::IsA<gtk4::Window>,
    ids: Vec<i64>,
    source: std::rc::Rc<dyn OrganizeSource>,
) {
    let title = source.dialog_title();
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
    results.set_selection_mode(gtk4::SelectionMode::None);
    content.append(&scrolled);
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    let state = std::rc::Rc::new(state.clone());
    let ids = std::rc::Rc::new(ids);
    let refresh_views = {
        let state = state.clone();
        let ids = ids.clone();
        move || {
            super::sidebar::rebuild_sidebar_and_show_grid(&state);
            for &db_id in ids.iter() {
                super::edit_game_scraper::refresh_scraper_section(&state, db_id);
            }
        }
    };
    let apply = {
        let state = state.clone();
        let ids = ids.clone();
        let source = source.clone();
        let refresh_views = refresh_views.clone();
        move |entity: ira_models::ScraperEntity| {
            for &db_id in ids.iter() {
                source.add(&state, db_id, &entity);
            }
            refresh_views();
        }
    };
    let remove = {
        let state = state.clone();
        let ids = ids.clone();
        let source = source.clone();
        let refresh_views = refresh_views.clone();
        move |entity_id: &str| {
            for &db_id in ids.iter() {
                source.remove(&state, db_id, entity_id);
            }
            refresh_views();
        }
    };

    let slf: RepaintCell = Default::default();
    {
        let slf_c = slf.clone();
        let state = state.clone();
        let source = source.clone();
        let apply = apply.clone();
        let remove = remove.clone();
        let results = results.clone();
        let ids = ids.clone();
        let entry = entry.clone();
        let populate_fn = move |term: &str| {
            super::helpers::clear_children(&results);
            let db = state.borrow().db.clone();
            let term = term.trim();
            let query = term.to_lowercase();
            // Shared entities pin to the top with Remove; the search
            // below only offers what isn't already everywhere.
            let all: Vec<Vec<ira_models::ScraperEntity>> = ids
                .iter()
                .map(|&db_id| source.held_by(&db, db_id))
                .collect();
            let (shared, _) = partition_entities(&all);
            let shared_id: std::collections::HashSet<&str> = shared
                .iter()
                .map(|entity| entity.id.as_str())
                .collect();
            for entity in &shared {
                if !query.is_empty() && !entity.name.to_lowercase().contains(&query) {
                    continue;
                }
                let remove = remove.clone();
                let slf = slf_c.clone();
                let entry = entry.clone();
                let entity_id = entity.id.clone();
                let row = adw::ActionRow::new();
                row.set_use_markup(false);
                row.set_title(&entity.name);
                row.set_subtitle(&format!("id {}", entity.id));
                let pick = gtk4::Button::with_label(&crate::tr!("Remove"));
                pick.set_valign(gtk4::Align::Center);
                pick.connect_clicked(move |_| {
                    remove(&entity_id);
                    if let Some(populate) = slf.borrow().as_ref() {
                        populate(&entry.text());
                    }
                });
                row.add_suffix(&pick);
                results.append(&row);
            }
            for entity in source.search(&db, term) {
                if shared_id.contains(entity.id.as_str()) {
                    continue;
                }
                let apply = apply.clone();
                let slf = slf_c.clone();
                let entry = entry.clone();
                let row_entity = entity.clone();
                let row = adw::ActionRow::new();
                row.set_use_markup(false);
                row.set_title(&entity.name);
                row.set_subtitle(&format!("id {}", entity.id));
                let pick = gtk4::Button::with_label(&crate::tr!("Add"));
                pick.add_css_class(super::css::CSS_SUGGESTED_ACTION);
                pick.set_valign(gtk4::Align::Center);
                pick.connect_clicked(move |_| {
                    apply(row_entity.clone());
                    if let Some(populate) = slf.borrow().as_ref() {
                        populate(&entry.text());
                    }
                });
                row.add_suffix(&pick);
                results.append(&row);
            }
            // The typed name itself, minted on click only — the db
            // must not fill up with every prefix the user tried.
            if !term.is_empty() {
                let already_held = |db_id: i64| {
                    source
                        .held_by(&db, db_id)
                        .iter()
                        .any(|held| held.name.eq_ignore_ascii_case(term))
                };
                let all_held = ids.iter().all(|&db_id| already_held(db_id));
                let apply = apply.clone();
                let slf_c = slf_c.clone();
                let entry = entry.clone();
                let term_c = term.to_string();
                let source = source.clone();
                let state = state.clone();
                let row = adw::ActionRow::new();
                row.set_use_markup(false);
                row.set_title(&crate::tr!("Add \"{}\"").replacen("{}", term, 1));
                let pick = gtk4::Button::with_label(&crate::tr!("Add"));
                pick.add_css_class(super::css::CSS_SUGGESTED_ACTION);
                pick.set_valign(gtk4::Align::Center);
                pick.set_sensitive(!all_held);
                pick.connect_clicked(move |_| {
                    if let Some(entity) = source.mint(&state, &term_c) {
                        apply(entity);
                        if let Some(populate) = slf_c.borrow().as_ref() {
                            populate(&entry.text());
                        }
                    }
                });
                row.add_suffix(&pick);
                results.append(&row);
            }
        };
        *slf.borrow_mut() = Some(std::rc::Rc::new(populate_fn));
    }
    {
        let slf = slf.clone();
        entry.connect_search_changed(move |entry| {
            if let Some(populate) = slf.borrow().as_ref() {
                populate(&entry.text());
            }
        });
    }
    if let Some(populate) = slf.borrow().as_ref() {
        populate("");
    }

    dialog.present(Some(parent.upcast_ref()));
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

/// The Groups entry behind single Organise and the flattened multi
/// menu: one picker for one game or a whole selection.
pub(super) fn setup_organize_groups_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    ids: Vec<i64>,
) {
    let action = gio::SimpleAction::new("organize_groups", None);
    action.connect_activate(move |_, _| {
        let window = state.borrow().window.clone();
        let source: std::rc::Rc<dyn OrganizeSource> = std::rc::Rc::new(GroupSource);
        show_organize_picker(&state, &window, ids.clone(), source);
    });
    actions.add_action(&action);
}


#[cfg(test)]
mod tests {
    use super::partition_entities;

    fn entity(id: &str, name: &str) -> ira_models::ScraperEntity {
        ira_models::ScraperEntity {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn test_partition_entities_splits_shared_first() {
        let action_rpg = entity("2620", "Action RPG");
        let adventure = entity("12", "Adventure");
        let all = vec![
            vec![action_rpg.clone(), adventure.clone()],
            vec![action_rpg.clone()],
            vec![],
        ];
        let (shared, rest) = partition_entities(&all);
        // Only the id every game holds counts — the third game holds
        // nothing, so nothing is shared.
        assert!(shared.is_empty());
        assert_eq!(rest.len(), 2);
        let all = vec![
            vec![action_rpg.clone(), adventure.clone()],
            vec![adventure.clone(), action_rpg.clone()],
        ];
        let (shared, rest) = partition_entities(&all);
        assert_eq!(shared.len(), 2);
        assert!(rest.is_empty());
        // Order within a section is by name, stable across runs.
        assert!(shared[0].name <= shared[1].name);
    }
}
