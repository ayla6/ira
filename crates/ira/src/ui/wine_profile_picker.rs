use adw::prelude::*;
use ira_models::WineProfile;
use super::css::CSS_FLAT;
use super::helpers::string_list_from;
use super::profile_dialog::show_profile_dialog;
use super::state::SharedState;

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
    let labels: Vec<String> = std::iter::once("New prefix".to_string())
        .chain(profiles.iter().map(|p| p.name.clone()))
        .collect();
    let model = string_list_from(&labels);
    let row = adw::ComboRow::new();
    row.set_title("Wine Profile");
    row.set_subtitle("Links wine version + prefix together");
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
    edit_btn.set_tooltip_text(Some("Edit profile"));
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
        if idx == 0 {
            show_profile_dialog(&parent, &db, None, &state_c, &win_c, None, slug_arg);
        } else if let Some(p) = profiles_c.get(idx - 1) {
            show_profile_dialog(&parent, &db, Some(p.clone()), &state_c, &win_c, None, slug_arg);
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
