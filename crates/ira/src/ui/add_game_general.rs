use super::css::*;
use super::state::SharedState;
use adw::prelude::*;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;

pub(super) fn build_general_page(
    win: &adw::Dialog,
    profiles: &[ira_models::WineProfile],
    state: &SharedState,
) -> (
    gtk4::Box,
    adw::EntryRow,
    adw::ComboRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow,
    gtk4::Button,
    adw::ComboRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow,
) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let info_group = adw::PreferencesGroup::new();
    info_group.set_title(&crate::tr!("Game info"));

    let name_entry = adw::EntryRow::new();
    name_entry.set_title(&crate::tr!("Name"));
    info_group.add(&name_entry);

    let kind_strings = [crate::tr!("Native Linux"), crate::tr!("Wine (Windows)")];
    let kind_refs: Vec<&str> = kind_strings.iter().map(String::as_str).collect();
    let kind_model = gtk4::StringList::new(&kind_refs);
    let kind_row = adw::ComboRow::new();
    kind_row.set_title(&crate::tr!("Kind"));
    kind_row.set_model(Some(&kind_model));
    kind_row.set_selected(1);
    info_group.add(&kind_row);

    let folder_entry = adw::EntryRow::new();
    folder_entry.set_title(&crate::tr!("Game folder"));

    let games_root = state.borrow().cfg.default_game_folder.clone();
    let folder_browse = super::helpers::make_browse_button(
        Some(win),
        &crate::tr!("Select game folder"),
        true,
        None,
        {
            let entry = folder_entry.clone();
            let games_root = games_root.clone();
            move || super::helpers::entry_path_closure(&entry)().or_else(|| {
                (!games_root.is_empty()).then_some(games_root.clone())
            })
        },
        {
            let entry = folder_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    folder_entry.add_suffix(&folder_browse);
    info_group.add(&folder_entry);

    let exe_entry = adw::EntryRow::new();
    exe_entry.set_title(&crate::tr!("Executable"));

    let exe_browse = super::helpers::make_browse_button(
        Some(win),
        &crate::tr!("Select executable"),
        false,
        Some((
            &crate::tr!("Executable"),
            &["application/x-executable", "application/x-msdos-program"],
        )),
        {
            let entry = exe_entry.clone();
            let folder = folder_entry.clone();
            let games_root = games_root.clone();
            move || {
                super::helpers::entry_path_closure(&entry)().or_else(|| {
                    let folder = folder.text().trim().to_string();
                    (!folder.is_empty()).then_some(folder).or_else(|| {
                        (!games_root.is_empty()).then_some(games_root.clone())
                    })
                })
            }
        },
        {
            let entry = exe_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    exe_entry.add_suffix(&exe_browse);
    info_group.add(&exe_entry);

    let args_entry = adw::EntryRow::new();
    args_entry.set_title(&crate::tr!("Arguments"));
    info_group.add(&args_entry);

    let wd_entry = adw::EntryRow::new();
    wd_entry.set_title(&crate::tr!("Working directory"));

    let wd_browse = super::helpers::make_browse_button(
        Some(win),
        &crate::tr!("Select working directory"),
        true,
        None,
        super::helpers::entry_path_closure(&wd_entry),
        {
            let entry = wd_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    wd_entry.add_suffix(&wd_browse);
    info_group.add(&wd_entry);

    let profile_row =
        super::wine_profile_picker::build_wine_profile_picker(profiles, None, None, state, win);
    info_group.add(&profile_row);

    page.append(&info_group);

    let ids_group = adw::PreferencesGroup::new();
    ids_group.set_title(&crate::tr!("Service IDs"));
    let steam_id_entry = adw::EntryRow::new();
    steam_id_entry.set_title(&crate::tr!("Steam app ID"));
    let steam_search_btn = gtk4::Button::from_icon_name("system-search-symbolic");
    steam_search_btn.set_valign(gtk4::Align::Center);
    steam_search_btn.set_tooltip_text(Some(&crate::tr!("Search Steam store")));
    steam_search_btn.add_css_class(CSS_FLAT);
    {
        let sc = state.clone();
        let win_c = win.clone();
        let row_c = steam_id_entry.clone();
        let name_entry_c = name_entry.clone();
        steam_search_btn.connect_clicked(move |_| {
            let search_text = name_entry_c.text().to_string();
            let on_select = Rc::new(|_: &str, _: &str| {});
            super::steam_search::show_steam_id_search_popup(
                &sc,
                &search_text,
                &win_c,
                &row_c,
                &crate::tr!("Select"),
                on_select,
            );
        });
    }
    steam_id_entry.add_suffix(&steam_search_btn);
    ids_group.add(&steam_id_entry);
    let gog_id_entry = adw::EntryRow::new();
    gog_id_entry.set_title(&crate::tr!("GOG product ID"));
    let gog_browse_btn = super::helpers::make_browse_button(
        Some(win),
        &crate::tr!("Select game folder"),
        true,
        None,
        super::helpers::entry_path_closure(&gog_id_entry),
        {
            let row = gog_id_entry.clone();
            let name_row = name_entry.clone();
            move |path| {
                if let Some((_, product_id, name)) =
                    ira_platforms::gog::find_gog_info(&path.to_string_lossy())
                {
                    row.set_text(&product_id);
                    if name_row.text().is_empty() {
                        name_row.set_text(&name);
                    }
                }
            }
        },
    );
    gog_id_entry.add_suffix(&gog_browse_btn);
    ids_group.add(&gog_id_entry);
    let sgdb_id_entry = adw::EntryRow::new();
    sgdb_id_entry.set_title(&crate::tr!("SteamGridDB ID"));
    sgdb_id_entry.set_input_purpose(gtk4::InputPurpose::Number);
    let sgdb_search_btn = gtk4::Button::from_icon_name("system-search-symbolic");
    sgdb_search_btn.set_valign(gtk4::Align::Center);
    sgdb_search_btn.set_tooltip_text(Some(&crate::tr!("Search SteamGridDB")));
    sgdb_search_btn.add_css_class(CSS_FLAT);
    {
        let sc = state.clone();
        let win_c = win.clone();
        let row_c = sgdb_id_entry.clone();
        let name_entry_c = name_entry.clone();
        sgdb_search_btn.connect_clicked(move |_| {
            show_sgdb_id_search_popup(&sc, &name_entry_c.text(), &win_c, &row_c);
        });
    }
    sgdb_id_entry.add_suffix(&sgdb_search_btn);
    ids_group.add(&sgdb_id_entry);
    page.append(&ids_group);    page.append(&ids_group);

    let detect_btn = super::helpers::make_browse_button(
        Some(win),
        &crate::tr!("Select game folder"),
        true,
        None,
        || None,
        {
            let n = name_entry.clone();
            let exe = exe_entry.clone();
            let sid = steam_id_entry.clone();
            let gid = gog_id_entry.clone();
            move |path| {
                let folder = path.to_string_lossy().into_owned();
                if let Some(app_id) = ira_platforms::steam::detect_app_id(&folder) {
                    sid.set_text(&app_id);
                }
                if ira_platforms::gog::is_gog_game(&folder) {
                    if let Some((_info_dir, product_id, game_name)) =
                        ira_platforms::gog::find_gog_info(&folder)
                    {
                        gid.set_text(&product_id);
                        if n.text().is_empty() {
                            n.set_text(&game_name);
                        }
                    }
                }
                if exe.text().is_empty() {
                    let shared = Arc::new(Mutex::new(None::<String>));
                    let shared_c = shared.clone();
                    let exe_c = exe.clone();
                    let folder_c = folder;
                    std::thread::spawn(move || {
                        let path = std::fs::read_dir(&folder_c).ok().and_then(|dir| {
                            for entry in dir.flatten() {
                                let p = entry.path();
                                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                                    if ext == "exe"
                                        || ext == "x86_64"
                                        || ext == "AppRun"
                                        || ext == "sh"
                                    {
                                        return Some(p.to_string_lossy().into_owned());
                                    }
                                }
                            }
                            None
                        });
                        if let Some(p) = path {
                            *shared_c.lock().unwrap() = Some(p);
                        }
                    });
                    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
                        if let Some(p) = shared.lock().unwrap().take() {
                            exe_c.set_text(&p);
                            glib::ControlFlow::Break
                        } else {
                            glib::ControlFlow::Continue
                        }
                    });
                }
            }
        },
    );

    (
        page,
        name_entry,
        kind_row,
        folder_entry,
        exe_entry,
        args_entry,
        wd_entry,
        detect_btn,
        profile_row,
        steam_id_entry,
        gog_id_entry,
        sgdb_id_entry,
    )
}

/// SGDB entry search for the manual-add page: picks an SGDB id into the
/// field before the game exists in the database.
fn show_sgdb_id_search_popup(
    state: &SharedState,
    game_name: &str,
    parent: &adw::Dialog,
    sgdb_row: &adw::EntryRow,
) {
    use super::helpers::{clear_children, poll_channel, status_row};
    use super::steam_search_dialog::{build_search_dialog, match_result_row, SearchDialogWidgets};
    use std::sync::mpsc;

    let SearchDialogWidgets {
        dialog,
        entry,
        search_btn,
        list,
    } = build_search_dialog(
        &crate::tr!("Search SteamGridDB"),
        500,
        400,
        500,
        game_name,
        Some(&crate::tr!("Game name…")),
    );

    let do_search = {
        let state = state.clone();
        let entry = entry.clone();
        let list = list.clone();
        let sgdb_row = sgdb_row.downgrade();
        let dialog = dialog.downgrade();
        move || {
            let term = entry.text().trim().to_string();
            if term.is_empty() {
                return;
            }
            let steam = state.borrow().steam.clone();
            let (tx, rx) = mpsc::channel::<Vec<(String, String)>>();
            std::thread::spawn(move || {
                let _ = tx.send(steam.search_sgdb(&term));
            });
            clear_children(&list);
            list.append(&status_row(&crate::tr!("Searching SteamGridDB…")));
            let list = list.clone();
            let sgdb_row = sgdb_row.clone();
            let dialog = dialog.clone();
            poll_channel(rx, move |results| {
                clear_children(&list);
                if results.is_empty() {
                    list.append(&status_row(&crate::tr!("No results found")));
                    return;
                }
                for (id, name) in results {
                    let sgdb_row = sgdb_row.clone();
                    let dialog = dialog.clone();
                    let row = match_result_row(&name, &format!("SGDB {id}"), move || {
                        if let Some(row) = sgdb_row.upgrade() {
                            row.set_text(&id);
                        }
                        if let Some(dialog) = dialog.upgrade() {
                            dialog.close();
                        }
                    });
                    list.append(&row);
                }
            });
        }
    };

    let do_search = Rc::new(do_search);
    entry.connect_activate({
        let ds = do_search.clone();
        move |_| ds()
    });
    search_btn.connect_clicked({
        let ds = do_search.clone();
        move |_| ds()
    });

    dialog.present(Some(parent));
}
