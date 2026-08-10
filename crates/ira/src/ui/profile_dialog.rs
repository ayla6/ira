use super::css::*;
use super::state::SharedState;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

type ListRef = Rc<RefCell<gtk4::ListBox>>;

pub fn build_profiles_page(
    state: &SharedState,
    settings_win: &adw::Window,
) -> (gtk4::ScrolledWindow, adw::EntryRow) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let settings_group = adw::PreferencesGroup::new();
    settings_group.set_title("Prefix settings");

    let prefix_base_row = adw::EntryRow::new();
    prefix_base_row.set_title("Prefix base directory (new prefixes created as {base}/{game-slug})");
    prefix_base_row.set_text(&state.borrow().cfg.prefix_base_dir);
    let prefix_browse = super::helpers::make_browse_button(
        Some(settings_win),
        "Select prefix base directory",
        true,
        None,
        super::helpers::entry_path_closure(&prefix_base_row),
        {
            let entry = prefix_base_row.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    prefix_base_row.add_suffix(&prefix_browse);
    settings_group.add(&prefix_base_row);
    page.append(&settings_group);

    let group = adw::PreferencesGroup::new();
    group.set_title("Wine profiles");

    let list = gtk4::ListBox::new();
    list.add_css_class(CSS_BOXED_LIST);
    list.set_valign(gtk4::Align::Start);
    let list_rc: ListRef = Rc::new(RefCell::new(list.clone()));

    let db = state.borrow().db.clone();
    let window = state.borrow().window.clone();
    let state_clone = state.clone();
    let settings_win_clone = settings_win.clone();

    repopulate_profiles(&list_rc, &db, &window, &state_clone, &settings_win_clone);

    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some("Create profile"));
    add_btn.set_valign(gtk4::Align::Center);
    add_btn.add_css_class(CSS_FLAT);
    let win_add = window.clone();
    let db_add = db.clone();
    let sc_add = state_clone.clone();
    let sw_add = settings_win_clone.clone();
    let list_rc_add = list_rc.clone();
    add_btn.connect_clicked(move |_| {
        show_profile_dialog(
            &win_add,
            &db_add,
            None,
            &sc_add,
            &sw_add,
            None,
            ProfileDialogCallbacks {
                list_rc: Some(list_rc_add.clone()),
                on_saved: None,
            },
        );
    });
    group.set_header_suffix(Some(&add_btn));

    group.add(&list);

    page.append(&group);

    let sw = gtk4::ScrolledWindow::new();
    sw.set_hexpand(true);
    sw.set_vexpand(true);
    sw.set_child(Some(&page));
    (sw, prefix_base_row)
}

fn repopulate_profiles(
    list_rc: &ListRef,
    db: &ira_db::DbConn,
    window: &adw::ApplicationWindow,
    state: &SharedState,
    settings_win: &adw::Window,
) {
    let list = list_rc.borrow().clone();
    super::helpers::clear_children(&list);

    let profiles = ira_db::get_all_profiles(db).unwrap_or_default();
    for p in &profiles {
        let row = adw::ActionRow::new();
        row.set_title(&p.name);
        row.set_subtitle(&format!(
            "{} — {}{}",
            p.wine_version,
            if p.prefix.is_empty() {
                "(no prefix)"
            } else {
                &p.prefix
            },
            if p.umu_enabled { " — UMU" } else { "" }
        ));

        let edit_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
        edit_btn.add_css_class(CSS_FLAT);
        edit_btn.set_valign(gtk4::Align::Center);
        edit_btn.set_vexpand(false);
        let p_edit = p.clone();
        let win_edit = window.clone();
        let db_edit = db.clone();
        let sc_edit = state.clone();
        let sw_edit = settings_win.clone();
        let list_rc_edit = list_rc.clone();
        edit_btn.connect_clicked(move |_| {
            show_profile_dialog(
                &win_edit,
                &db_edit,
                Some(p_edit.clone()),
                &sc_edit,
                &sw_edit,
                None,
                ProfileDialogCallbacks {
                    list_rc: Some(list_rc_edit.clone()),
                    on_saved: None,
                },
            );
        });
        row.add_suffix(&edit_btn);

        let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
        del_btn.add_css_class(CSS_FLAT);
        del_btn.set_valign(gtk4::Align::Center);
        del_btn.set_vexpand(false);
        let p_id = p.id;
        let db_del = db.clone();
        let sw_del = settings_win.clone();
        let list_rc_del = list_rc.clone();
        let row_for_del = row.clone();
        del_btn.connect_clicked(move |_| {
            let alert = adw::AlertDialog::new(
                Some("Delete profile"),
                Some("Are you sure you want to delete this profile? Games using it will keep their settings but lose the profile link."),
            );
            alert.add_response("cancel", "Cancel");
            alert.add_response("delete", "Delete");
            alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            alert.set_default_response(Some("cancel"));
            alert.set_close_response("cancel");
            let db_c = db_del.clone();
            let list_c = list_rc_del.clone();
            let row_c = row_for_del.clone();
            alert.connect_response(None, move |_, response| {
                if response == "delete" {
                    if let Err(e) = ira_db::delete_profile(&db_c, p_id) {
                        eprintln!("Failed to delete profile: {}", e);
                    } else {
                        list_c.borrow().remove(&row_c);
                    }
                }
            });
            alert.present(Some(&sw_del));
        });
        row.add_suffix(&del_btn);

        list.append(&row);
    }
}

pub struct ProfileDialogCallbacks {
    pub list_rc: Option<ListRef>,
    pub on_saved: Option<Rc<dyn Fn(i64)>>,
}

pub fn show_profile_dialog(
    parent: &adw::ApplicationWindow,
    db: &ira_db::DbConn,
    existing: Option<ira_models::WineProfile>,
    state: &SharedState,
    settings_win: &adw::Window,
    game_slug: Option<&str>,
    callbacks: ProfileDialogCallbacks,
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
        group.set_title("Edit profile");
    } else {
        group.set_title("Create profile");
    }

    let name_entry = adw::EntryRow::new();
    name_entry.set_title("Profile name");
    if let Some(p) = existing.as_ref() {
        name_entry.set_text(&p.name);
    }
    group.add(&name_entry);

    let version_model = {
        let versions = ira_launcher::wine_launch::detect_wine_versions();
        let labels: Vec<&str> = versions.iter().map(|(l, _)| l.as_str()).collect();
        gtk4::StringList::new(&labels)
    };
    let version_row = adw::ComboRow::new();
    version_row.set_title("Wine version");
    version_row.set_model(Some(&version_model));
    let versions = ira_launcher::wine_launch::detect_wine_versions();
    if let Some(p) = existing.as_ref() {
        if let Some(idx) = versions.iter().position(|(_, v)| v == &p.wine_version) {
            version_row.set_selected(idx as u32);
        }
    } else {
        version_row.set_selected(0);
    }
    group.add(&version_row);

    let prefix_entry = adw::EntryRow::new();
    prefix_entry.set_title("Wine prefix path");

    // Track whether the user has manually modified the prefix field.
    // When false, the prefix auto-updates from the profile name.
    let prefix_manually_edited = Rc::new(RefCell::new(existing.is_some()));

    if let Some(p) = existing.as_ref() {
        prefix_entry.set_text(&p.prefix);
    } else if let Some(slug) = game_slug {
        if !slug.is_empty() {
            let base = state.borrow().cfg.prefix_base_dir.clone();
            let generated = ira_launcher::wine_launch::generate_prefix_path(&base, slug);
            prefix_entry.set_text(&generated);
        }
    }

    // Auto-update prefix from name when not manually edited
    let prefix_entry_for_name = prefix_entry.clone();
    let prefix_me_for_name = prefix_manually_edited.clone();
    let state_for_name = state.clone();
    name_entry.connect_changed(move |entry| {
        if *prefix_me_for_name.borrow() {
            return;
        }
        let name = entry.text().to_string();
        if name.is_empty() {
            return;
        }
        let base = state_for_name.borrow().cfg.prefix_base_dir.clone();
        let generated = ira_launcher::wine_launch::generate_prefix_path(&base, &name);
        prefix_entry_for_name.set_text(&generated);
    });

    // Mark as manually edited when the user types in the prefix field
    let prefix_me_for_edit = prefix_manually_edited.clone();
    prefix_entry.connect_changed(move |_| {
        *prefix_me_for_edit.borrow_mut() = true;
    });

    let prefix_browse = super::helpers::make_browse_button(
        Some(&win),
        "Select wine prefix",
        true,
        None,
        super::helpers::entry_path_closure(&prefix_entry),
        {
            let entry = prefix_entry.clone();
            let me = prefix_manually_edited.clone();
            move |path| {
                entry.set_text(&path.to_string_lossy());
                *me.borrow_mut() = true;
            }
        },
    );
    prefix_entry.add_suffix(&prefix_browse);

    // Reset button — regenerate prefix from name, clear manual flag
    let prefix_reset = gtk4::Button::from_icon_name("edit-undo-symbolic");
    prefix_reset.add_css_class(CSS_FLAT);
    prefix_reset.set_valign(gtk4::Align::Center);
    prefix_reset.set_tooltip_text(Some("Reset to auto-generated path"));
    let prefix_entry_for_reset = prefix_entry.clone();
    let name_entry_for_reset = name_entry.clone();
    let me_for_reset = prefix_manually_edited.clone();
    let state_for_reset = state.clone();
    prefix_reset.connect_clicked(move |_| {
        let name = name_entry_for_reset.text().to_string();
        if name.is_empty() {
            return;
        }
        let base = state_for_reset.borrow().cfg.prefix_base_dir.clone();
        let generated = ira_launcher::wine_launch::generate_prefix_path(&base, &name);
        prefix_entry_for_reset.set_text(&generated);
        *me_for_reset.borrow_mut() = false;
    });
    prefix_entry.add_suffix(&prefix_reset);

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

    let umu_row = adw::SwitchRow::new();
    umu_row.set_title("UMU launcher");
    umu_row.set_subtitle("Launch via umu-run (required for Proton versions)");
    umu_row.set_active(existing.as_ref().is_none_or(|p| p.umu_enabled));

    // Auto-enable and disable UMU toggle based on wine version selection
    let umu_row_for_version = umu_row.clone();
    let version_row_for_cb = version_row.clone();
    version_row_for_cb
        .clone()
        .connect_selected_notify(move |_| {
            let idx = version_row_for_cb.selected() as usize;
            if let Some((_, ver)) = ira_launcher::wine_launch::detect_wine_versions().get(idx) {
                let is_proton = ver.to_lowercase().contains("proton");
                umu_row_for_version.set_active(is_proton || umu_row_for_version.is_active());
                umu_row_for_version.set_sensitive(!is_proton);
            }
        });

    // Set initial state based on selected version
    {
        let idx = version_row.selected() as usize;
        if let Some((_, ver)) = versions.get(idx) {
            let is_proton = ver.to_lowercase().contains("proton");
            if is_proton {
                umu_row.set_active(true);
                umu_row.set_sensitive(false);
            }
        }
    }

    group.add(&umu_row);

    content.append(&group);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());

    let save_btn = gtk4::Button::with_label("Save");
    save_btn.add_css_class(CSS_SUGGESTED_ACTION);

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
    let umu_row_c = umu_row.clone();
    let win_c2 = win.clone();
    let db_c = db.clone();
    let versions_c = versions.clone();
    let parent_c = parent.clone();
    let sc_c = state.clone();
    let sw_c = settings_win.clone();
    let list_rc_c = callbacks.list_rc.clone();
    let on_saved_c = callbacks.on_saved.clone();
    save_btn.connect_clicked(move |_| {
        let name = name_c.text().to_string();
        if name.is_empty() {
            name_c.add_css_class(CSS_ERROR);
            return;
        }
        let prefix = prefix_c.text().trim().to_string();
        if prefix.is_empty() {
            prefix_c.add_css_class(CSS_ERROR);
            return;
        }
        name_c.remove_css_class(CSS_ERROR);
        prefix_c.remove_css_class(CSS_ERROR);
        let vidx = version_row_c.selected() as usize;
        let wine_version = versions_c
            .get(vidx)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| "system".to_string());
        let arch = match arch_row_c.selected() {
            1 => "win32".to_string(),
            2 => "win64".to_string(),
            _ => "auto".to_string(),
        };
        let umu_enabled = umu_row_c.is_active();
        let saved_id = if let Some(p) = existing.as_ref() {
            let mut updated = p.clone();
            updated.name = name;
            updated.wine_version = wine_version;
            updated.custom_wine_path = String::new();
            updated.prefix = prefix;
            updated.arch = arch;
            updated.umu_enabled = umu_enabled;
            if let Err(e) = ira_db::update_profile(&db_c, &updated) {
                eprintln!("Failed to update profile: {}", e);
            }
            Some(p.id)
        } else {
            let new_profile = ira_models::WineProfile {
                id: 0,
                name,
                wine_version,
                custom_wine_path: String::new(),
                prefix,
                arch,
                umu_enabled,
            };
            match ira_db::add_profile(&db_c, &new_profile) {
                Ok(id) => Some(id),
                Err(e) => {
                    eprintln!("Failed to add profile: {}", e);
                    None
                }
            }
        };
        win_c2.close();
        if let Some(ref lr) = list_rc_c {
            repopulate_profiles(lr, &db_c, &parent_c, &sc_c, &sw_c);
        }
        if let (Some(id), Some(cb)) = (saved_id, &on_saved_c) {
            cb(id);
        }
    });
}
