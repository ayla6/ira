use std::cell::Cell;
use std::rc::Rc;

use super::css::CSS_FLAT;
use super::helpers::string_list_from;
use super::profile_dialog::{show_profile_dialog, ProfileDialogCallbacks};
use super::state::SharedState;
use adw::prelude::*;
use ira_models::WineProfile;

/// Build a wine profile `ComboRow` with an attached edit button.
///
/// Index 0 is the "New prefix…" sentinel: selecting it opens the profile
/// editor, and saving there creates the profile and selects it. Indices
/// 1..N correspond to `profiles[i-1]`. `saved_profile_id` preselects an
/// existing profile. `game_slug` is forwarded to the profile dialog so a
/// new prefix path can be auto-generated from it.
pub fn build_wine_profile_picker(
    profiles: &[WineProfile],
    saved_profile_id: Option<i64>,
    game_slug: Option<&str>,
    state: &SharedState,
    win: &impl IsA<gtk4::Widget>,
) -> adw::ComboRow {
    let win: &gtk4::Widget = win.upcast_ref();
    let row = adw::ComboRow::new();
    row.set_title(&crate::tr!("Wine profile"));
    row.set_subtitle(&crate::tr!("Links wine version + prefix together"));
    row.set_model(Some(&string_list_from(&profile_labels(profiles))));

    if let Some(pid) = saved_profile_id {
        for (i, p) in profiles.iter().enumerate() {
            if p.id == pid {
                row.set_selected((i + 1) as u32);
                break;
            }
        }
    }

    // Rebuilding the model programmatically resets the selection to the
    // sentinel for a moment; while this flag is set that must not re-open
    // the profile editor.
    let rebuilding = Rc::new(Cell::new(false));
    // Last non-sentinel selection, restored when the editor is cancelled.
    let previous = Rc::new(Cell::new(row.selected()));
    let slug = game_slug.map(|s| s.to_string());

    let state_n = state.clone();
    let win_n = win.clone();
    let rebuilding_n = rebuilding.clone();
    let previous_n = previous.clone();
    let slug_n = slug.clone();
    row.connect_selected_notify(move |row| {
        if rebuilding_n.get() {
            return;
        }
        let idx = row.selected();
        if idx != 0 {
            previous_n.set(idx);
            return;
        }
        open_profile_editor(row, &state_n, &win_n, slug_n.as_deref(), &rebuilding_n, &previous_n);
    });

    let edit_btn = gtk4::Button::new();
    edit_btn.set_icon_name("document-edit-symbolic");
    edit_btn.set_tooltip_text(Some(&crate::tr!("Edit profile")));
    edit_btn.set_valign(gtk4::Align::Center);
    edit_btn.add_css_class(CSS_FLAT);

    let row_c = glib::clone::Downgrade::downgrade(&row);
    let state_c = state.clone();
    let win_c = glib::clone::Downgrade::downgrade(win);
    let rebuilding_c = rebuilding.clone();
    let previous_c = previous.clone();
    let slug_c = slug;
    edit_btn.connect_clicked(move |_| {
        let Some(row) = row_c.upgrade() else {
            return;
        };
        let Some(win) = win_c.upgrade() else {
            return;
        };
        open_profile_editor(&row, &state_c, &win, slug_c.as_deref(), &rebuilding_c, &previous_c);
    });
    row.add_suffix(&edit_btn);
    row
}

fn profile_labels(profiles: &[WineProfile]) -> Vec<String> {
    std::iter::once(crate::tr!("New prefix…"))
        .chain(profiles.iter().map(|p| p.name.clone()))
        .collect()
}

/// Open the profile editor for the row's current selection: the sentinel
/// index 0 creates a new profile, any other index edits the profile at
/// `index - 1`. On save the rebuilt model selects the saved profile; on
/// cancel the selection falls back to `previous`.
fn open_profile_editor(
    row: &adw::ComboRow,
    state: &SharedState,
    win: &impl IsA<gtk4::Widget>,
    game_slug: Option<&str>,
    rebuilding: &Rc<Cell<bool>>,
    previous: &Rc<Cell<u32>>,
) {
    let (parent, db) = {
        let s = state.borrow();
        (s.window.clone(), s.db.clone())
    };
    let selected = row.selected();
    let existing = if selected == 0 {
        None
    } else {
        let profiles = match ira_db::get_all_profiles(&db) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to load wine profiles: {}", e);
                return;
            }
        };
        match profiles.get(selected as usize - 1) {
            Some(p) => Some(p.clone()),
            None => return,
        }
    };
    let on_saved = make_on_saved(row, state, rebuilding);
    let editor = show_profile_dialog(
        &parent,
        &db,
        existing,
        state,
        win,
        game_slug,
        ProfileDialogCallbacks {
            list_rc: None,
            on_saved: Some(on_saved),
        },
    );
    let row_c = row.clone();
    let rebuilding_c = rebuilding.clone();
    let previous_c = previous.clone();
    editor.connect_closed(move |_| {
        // A save already selected the created/updated profile; a cancel
        // leaves the sentinel selected, so restore the previous pick.
        if !rebuilding_c.get() && row_c.selected() == 0 {
            row_c.set_selected(previous_c.get());
        }
    });
}

/// Rebuild the picker's model from the database and select `new_id` after
/// a profile was saved in the editor.
fn make_on_saved(
    row: &adw::ComboRow,
    state: &SharedState,
    rebuilding: &Rc<Cell<bool>>,
) -> Rc<dyn Fn(i64)> {
    let row_c = row.clone();
    let state_c = state.clone();
    let rebuilding_c = rebuilding.clone();
    Rc::new(move |new_id: i64| {
        let profiles = match ira_db::get_all_profiles(&state_c.borrow().db) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to load wine profiles: {}", e);
                return;
            }
        };
        let model = string_list_from(&profile_labels(&profiles));
        rebuilding_c.set(true);
        row_c.set_model(Some(&model));
        rebuilding_c.set(false);
        for (i, p) in profiles.iter().enumerate() {
            if p.id == new_id {
                row_c.set_selected((i + 1) as u32);
                break;
            }
        }
    })
}

/// Resolve the selected profile id from a `ComboRow` built by
/// `build_wine_profile_picker`. Returns `None` for the "New prefix…" sentinel.
/// Profiles are read from the database so one created moments ago in the
/// editor resolves too.
pub fn selected_profile_id(row: &adw::ComboRow, db: &ira_db::DbConn) -> Option<i64> {
    let idx = row.selected();
    if idx == 0 {
        return None;
    }
    match ira_db::get_all_profiles(db) {
        Ok(profiles) => profiles.get((idx - 1) as usize).map(|p| p.id),
        Err(e) => {
            eprintln!("Failed to load wine profiles: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::profile_labels;
    use ira_models::WineProfile;

    #[test]
    fn test_profile_labels_sentinel_first_then_names() {
        let p = WineProfile {
            name: "Proton".to_string(),
            ..Default::default()
        };
        let labels = profile_labels(&[p]);
        assert_eq!(labels.len(), 2);
        assert!(labels[0].starts_with("New prefix"));
        assert!(labels[0].ends_with('\u{2026}'));
        assert_eq!(labels[1], "Proton");
    }
}
