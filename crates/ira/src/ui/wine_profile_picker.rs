use std::rc::Rc;

use super::css::CSS_FLAT;
use super::helpers::string_list_from;
use super::profile_dialog::{show_profile_dialog, ProfileDialogCallbacks};
use super::state::SharedState;
use adw::prelude::*;
use ira_models::WineProfile;

/// Build a wine profile `ComboRow` with an attached edit button.
///
/// Index 0 is the "New prefix" sentinel (creates a profile); indices 1..N
/// correspond to `profiles[i-1]`. `saved_profile_id` preselects an existing
/// profile. `game_slug` is forwarded to the profile dialog so a new prefix
/// path can be auto-generated from it.
pub fn build_wine_profile_picker(
    profiles: &[WineProfile],
    saved_profile_id: Option<i64>,
    game_slug: Option<&str>,
    state: &SharedState,
    win: &adw::Window,
) -> adw::ComboRow {
    let labels: Vec<String> = std::iter::once(crate::tr!("New prefix"))
        .chain(profiles.iter().map(|p| p.name.clone()))
        .collect();
    let model = string_list_from(&labels);
    let row = adw::ComboRow::new();
    row.set_title(&crate::tr!("Wine profile"));
    row.set_subtitle(&crate::tr!("Links wine version + prefix together"));
    row.set_model(Some(&model));

    if let Some(pid) = saved_profile_id {
        for (i, p) in profiles.iter().enumerate() {
            if p.id == pid {
                row.set_selected((i + 1) as u32);
                break;
            }
        }
    }

    let edit_btn = gtk4::Button::new();
    edit_btn.set_icon_name("document-edit-symbolic");
    edit_btn.set_tooltip_text(Some(&crate::tr!("Edit profile")));
    edit_btn.set_valign(gtk4::Align::Center);
    edit_btn.add_css_class(CSS_FLAT);

    let profiles_c: Vec<WineProfile> = profiles.to_vec();
    let row_c = row.clone();
    let state_c = state.clone();
    let win_c = win.clone();
    let slug_c = game_slug.map(|s| s.to_string());
    edit_btn.connect_clicked(move |_| {
        let idx = row_c.selected() as usize;
        let parent = state_c.borrow().window.clone();
        let db = state_c.borrow().db.clone();
        let slug_arg = slug_c.as_deref();

        let make_on_saved = |select_id: bool| -> Rc<dyn Fn(i64)> {
            let row_c2 = row_c.clone();
            let state_c2 = state_c.clone();
            Rc::new(move |new_id: i64| {
                let profiles = ira_db::get_all_profiles(&state_c2.borrow().db).unwrap_or_default();
                let labels: Vec<String> = std::iter::once(crate::tr!("New prefix"))
                    .chain(profiles.iter().map(|p| p.name.clone()))
                    .collect();
                let model = string_list_from(&labels);
                row_c2.set_model(Some(&model));
                if select_id {
                    for (i, p) in profiles.iter().enumerate() {
                        if p.id == new_id {
                            row_c2.set_selected((i + 1) as u32);
                            break;
                        }
                    }
                }
            })
        };

        if idx == 0 {
            show_profile_dialog(
                &parent,
                &db,
                None,
                &state_c,
                &win_c,
                slug_arg,
                ProfileDialogCallbacks {
                    list_rc: None,
                    on_saved: Some(make_on_saved(true)),
                },
            );
        } else if let Some(p) = profiles_c.get(idx - 1) {
            show_profile_dialog(
                &parent,
                &db,
                Some(p.clone()),
                &state_c,
                &win_c,
                slug_arg,
                ProfileDialogCallbacks {
                    list_rc: None,
                    on_saved: Some(make_on_saved(false)),
                },
            );
        }
    });
    row.add_suffix(&edit_btn);
    row
}

/// Resolve the selected profile id from a `ComboRow` built by
/// `build_wine_profile_picker`. Returns `None` for the "New prefix" sentinel.
pub fn selected_profile_id(row: &adw::ComboRow, profiles: &[WineProfile]) -> Option<i64> {
    let idx = row.selected();
    if idx == 0 {
        None
    } else {
        profiles.get((idx - 1) as usize).map(|p| p.id)
    }
}
