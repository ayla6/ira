use std::rc::Rc;
use gtk4::prelude::*;
use adw::prelude::*;
use super::state::SharedState;

pub(super) fn build_general_page(win: &adw::Window, profiles: &[ira_models::WineProfile], state: &SharedState) -> (gtk4::Box, adw::EntryRow, adw::ComboRow, adw::EntryRow, adw::EntryRow, adw::EntryRow, gtk4::Button, adw::ComboRow, adw::EntryRow, adw::EntryRow) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let info_group = adw::PreferencesGroup::new();
    info_group.set_title("Game Info");

    let name_entry = adw::EntryRow::new();
    name_entry.set_title("Name");
    info_group.add(&name_entry);

    let kind_model = gtk4::StringList::new(&["Native Linux", "Wine (Windows)"]);
    let kind_row = adw::ComboRow::new();
    kind_row.set_title("Kind");
    kind_row.set_model(Some(&kind_model));
    kind_row.set_selected(1);
    info_group.add(&kind_row);

    let exe_entry = adw::EntryRow::new();
    exe_entry.set_title("Executable");

    let exe_browse = super::helpers::make_browse_button(
        Some(win),
        "Select executable",
        false,
        Some(("Executable", &["application/x-executable", "application/x-msdos-program"])),
        {
            let entry = exe_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    exe_entry.add_suffix(&exe_browse);
    info_group.add(&exe_entry);

    let args_entry = adw::EntryRow::new();
    args_entry.set_title("Arguments");
    info_group.add(&args_entry);

    let wd_entry = adw::EntryRow::new();
    wd_entry.set_title("Working directory");

    let wd_browse = super::helpers::make_browse_button(
        Some(win),
        "Select working directory",
        true,
        None,
        {
            let entry = wd_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    wd_entry.add_suffix(&wd_browse);
    info_group.add(&wd_entry);

    let profile_labels: Vec<String> = std::iter::once("New prefix".to_string())
        .chain(profiles.iter().map(|p| p.name.clone()))
        .collect();
    let profile_model = super::helpers::string_list_from(&profile_labels);
    let profile_row = adw::ComboRow::new();
    profile_row.set_title("Wine Profile");
    profile_row.set_subtitle("Links wine version + prefix together");
    profile_row.set_model(Some(&profile_model));

    let edit_btn = gtk4::Button::new();
    edit_btn.set_icon_name("document-edit-symbolic");
    edit_btn.set_tooltip_text(Some("Edit profile"));
    edit_btn.set_valign(gtk4::Align::Center);
    edit_btn.add_css_class("flat");
    let profiles_c: Vec<ira_models::WineProfile> = profiles.to_vec();
    let pr_c = profile_row.clone();
    let state_c = state.clone();
    let win_c = win.clone();
    edit_btn.connect_clicked(move |_| {
        let idx = pr_c.selected() as usize;
        let parent = state_c.borrow().window.clone();
        let db = state_c.borrow().db.clone();
        if idx == 0 {
            super::profile_dialog::show_profile_dialog(&parent, &db, None, &state_c, &win_c, None, None);
        } else if let Some(p) = profiles_c.get(idx - 1) {
            super::profile_dialog::show_profile_dialog(&parent, &db, Some(p.clone()), &state_c, &win_c, None, None);
        }
    });
    profile_row.add_suffix(&edit_btn);

    info_group.add(&profile_row);

    page.append(&info_group);

    let ids_group = adw::PreferencesGroup::new();
    ids_group.set_title("Service IDs");
    let steam_id_entry = adw::EntryRow::new();
    steam_id_entry.set_title("Steam App ID");
    let steam_search_btn = gtk4::Button::from_icon_name("system-search-symbolic");
    steam_search_btn.set_valign(gtk4::Align::Center);
    steam_search_btn.set_tooltip_text(Some("Search Steam Store"));
    steam_search_btn.add_css_class("flat");
    {
        let sc = state.clone();
        let win_c = win.clone();
        let row_c = steam_id_entry.clone();
        let name_entry_c = name_entry.clone();
        steam_search_btn.connect_clicked(move |_| {
            let search_text = name_entry_c.text().to_string();
            let on_select = Rc::new(|_: &str| {});
            super::steam_search::show_steam_id_search_popup(&sc, &search_text, &win_c, &row_c, "Select", on_select);
        });
    }
    steam_id_entry.add_suffix(&steam_search_btn);
    ids_group.add(&steam_id_entry);
    let gog_id_entry = adw::EntryRow::new();
    gog_id_entry.set_title("GOG Product ID");
    let gog_browse_btn = super::helpers::make_browse_button(
        Some(win),
        "Select game folder",
        true,
        None,
        {
            let row = gog_id_entry.clone();
            let name_row = name_entry.clone();
            move |path| {
                if let Some((_, product_id, name)) = ira_platforms::gog::find_gog_info(&path.to_string_lossy()) {
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
    page.append(&ids_group);

    let detect_btn = super::helpers::make_browse_button(
        Some(win),
        "Select game folder",
        true,
        None,
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
                    if let Some((_info_dir, product_id, game_name)) = ira_platforms::gog::find_gog_info(&folder) {
                        gid.set_text(&product_id);
                        if n.text().is_empty() {
                            n.set_text(&game_name);
                        }
                    }
                }
                if exe.text().is_empty() {
                    if let Ok(entries) = std::fs::read_dir(&folder) {
                        for e in entries.flatten() {
                            let p = e.path();
                            if let Some(ext) = p.extension() {
                                if ext == "exe" || ext == "x86_64" || ext == "AppRun" || ext == "sh" {
                                    exe.set_text(&p.to_string_lossy());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        },
    );

    (page, name_entry, kind_row, exe_entry, args_entry, wd_entry, detect_btn, profile_row, steam_id_entry, gog_id_entry)
}
