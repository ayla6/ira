use gtk4::prelude::*;
use adw::prelude::*;
use super::state::SharedState;

pub fn build_profiles_page(state: &SharedState, settings_win: &adw::Window) -> gtk4::Box {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let group = adw::PreferencesGroup::new();
    group.set_title("Wine Profiles");

    let list = gtk4::ListBox::new();
    list.add_css_class("boxed-list");

    let profiles = crate::db::get_all_profiles(&state.borrow().db).unwrap_or_default();
    let db = state.borrow().db.clone();
    let window = state.borrow().window.clone();
    let state_clone = state.clone();
    let settings_win_clone = settings_win.clone();

    for p in &profiles {
        let row = adw::ActionRow::new();
        row.set_title(&p.name);
        row.set_subtitle(&format!("{} — {}", p.wine_version, if p.prefix.is_empty() { "(default prefix)" } else { &p.prefix }));

        let edit_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
        edit_btn.add_css_class("flat");
        let p_edit = p.clone();
        let win_edit = window.clone();
        let db_edit = db.clone();
        let sc_edit = state_clone.clone();
        let sw_edit = settings_win_clone.clone();
        edit_btn.connect_clicked(move |_| {
            show_profile_dialog(&win_edit, &db_edit, Some(p_edit.clone()), &sc_edit, &sw_edit);
        });
        row.add_suffix(&edit_btn);

        let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
        del_btn.add_css_class("flat");
        let p_id = p.id;
        let db_del = db.clone();
        let sc_del = state_clone.clone();
        let sw_del = settings_win_clone.clone();
        del_btn.connect_clicked(move |_| {
            let alert = adw::AlertDialog::new(
                Some("Delete Profile"),
                Some("Are you sure you want to delete this profile? Games using it will keep their settings but lose the profile link."),
            );
            alert.add_response("cancel", "Cancel");
            alert.add_response("delete", "Delete");
            alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            alert.set_default_response(Some("cancel"));
            alert.set_close_response("cancel");
            let db_c = db_del.clone();
            let sc_c = sc_del.clone();
            let sw_c = sw_del.clone();
            alert.connect_response(None, move |_, response| {
                if response == "delete" {
                    if let Err(e) = crate::db::delete_profile(&db_c, p_id) {
                        eprintln!("Failed to delete profile: {}", e);
                    } else {
                        sw_c.close();
                        refresh_settings(&sc_c);
                    }
                }
            });
            alert.present(None::<&gtk4::Widget>);
        });
        row.add_suffix(&del_btn);

        list.append(&row);
    }

    group.add(&list);

    let add_btn = gtk4::Button::with_label("Create Profile");
    add_btn.add_css_class("flat");
    let win_add = window.clone();
    let db_add = db.clone();
    let sc_add = state_clone.clone();
    let sw_add = settings_win_clone.clone();
    add_btn.connect_clicked(move |_| {
        show_profile_dialog(&win_add, &db_add, None, &sc_add, &sw_add);
    });
    group.add(&add_btn);

    page.append(&group);
    page
}

fn refresh_settings(state: &SharedState) {
    let sc = state.clone();
    glib::idle_add_local_once(move || {
        let s = sc.borrow();
        let window = s.window.clone();
        let cfg = s.cfg.clone();
        let steam = s.steam.clone();
        drop(s);
        crate::ui::dialogs::show_settings_dialog(&window, cfg, steam, &sc);
    });
}

fn show_profile_dialog(
    parent: &adw::ApplicationWindow,
    db: &crate::db::DbConn,
    existing: Option<crate::models::WineProfile>,
    state: &SharedState,
    settings_win: &adw::Window,
) {
    let win = adw::Window::new();
    win.set_default_width(450);
    win.set_default_height(400);
    win.set_transient_for(Some(parent));
    win.set_modal(true);
    win.set_deletable(false);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar.add_top_bar(&header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);

    let group = adw::PreferencesGroup::new();
    if existing.is_some() {
        group.set_title("Edit Profile");
    } else {
        group.set_title("Create Profile");
    }

    let name_entry = adw::EntryRow::new();
    name_entry.set_title("Profile name");
    if let Some(p) = existing.as_ref() { name_entry.set_text(&p.name); }
    group.add(&name_entry);

    let version_model = {
        let versions = crate::launcher::wine_launch::detect_wine_versions();
        let labels: Vec<&str> = versions.iter().map(|(l, _)| l.as_str()).collect();
        gtk4::StringList::new(&labels)
    };
    let version_row = adw::ComboRow::new();
    version_row.set_title("Wine version");
    version_row.set_model(Some(&version_model));
    let versions = crate::launcher::wine_launch::detect_wine_versions();
    if let Some(p) = existing.as_ref() {
        if let Some(idx) = versions.iter().position(|(_, v)| v == &p.wine_version) {
            version_row.set_selected(idx as u32);
        }
    } else {
        version_row.set_selected(0);
    }
    group.add(&version_row);

    let prefix_entry = adw::EntryRow::new();
    prefix_entry.set_title("Wine prefix path (empty = default ~/.wine)");
    if let Some(p) = existing.as_ref() { prefix_entry.set_text(&p.prefix); }

    let prefix_browse = gtk4::Button::from_icon_name("folder-open-symbolic");
    prefix_browse.add_css_class("flat");
    prefix_browse.set_valign(gtk4::Align::Center);
    let pe_b = prefix_entry.clone();
    let win_b = win.clone();
    prefix_browse.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Select wine prefix");
        let entry = pe_b.clone();
        dialog.select_folder(Some(&win_b), None::<&gio::Cancellable>, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    entry.set_text(&path.to_string_lossy());
                }
            }
        });
    });
    prefix_entry.add_suffix(&prefix_browse);
    group.add(&prefix_entry);

    let arch_model = gtk4::StringList::new(&["Auto", "32-bit (win32)", "64-bit (win64)"]);
    let arch_row = adw::ComboRow::new();
    arch_row.set_title("Architecture");
    arch_row.set_model(Some(&arch_model));
    if let Some(p) = existing.as_ref() {
        let idx = match p.arch.as_str() {
            "win32" => 1,
            "win64" => 2,
            _ => 0,
        };
        arch_row.set_selected(idx);
    }
    group.add(&arch_row);

    content.append(&group);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());

    let save_btn = gtk4::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    content.append(&btn_row);

    toolbar.set_content(Some(&content));
    win.set_content(Some(&toolbar));
    win.present();

    let name_c = name_entry.clone();
    let version_row_c = version_row.clone();
    let prefix_c = prefix_entry.clone();
    let arch_row_c = arch_row.clone();
    let win_c2 = win.clone();
    let db_c = db.clone();
    let versions_c = versions.clone();
    let sc = state.clone();
    let sw_c = settings_win.clone();
    save_btn.connect_clicked(move |_| {
        let name = name_c.text().to_string();
        if name.is_empty() {
            name_c.add_css_class("error");
            return;
        }
        let vidx = version_row_c.selected() as usize;
        let wine_version = versions_c.get(vidx).map(|(_, v)| v.clone()).unwrap_or_else(|| "system".to_string());
        let prefix = prefix_c.text().to_string();
        let arch = match arch_row_c.selected() {
            1 => "win32".to_string(),
            2 => "win64".to_string(),
            _ => "auto".to_string(),
        };
        if let Some(p) = existing.as_ref() {
            if let Err(e) = crate::db::update_profile(&db_c, p.id, &name, &wine_version, "", &prefix, &arch) {
                eprintln!("Failed to update profile: {}", e);
            }
        } else {
            if let Err(e) = crate::db::add_profile(&db_c, &name, &wine_version, "", &prefix, &arch) {
                eprintln!("Failed to add profile: {}", e);
            }
        }
        win_c2.close();
        sw_c.close();
        refresh_settings(&sc);
    });
}
