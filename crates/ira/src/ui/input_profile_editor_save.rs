//! Save machinery for the profile editor: building the canonical profile
//! from the editor state, writing it (Save closes, Apply stays), and the
//! unsaved-changes close guard.

use super::css::CSS_ERROR;
use super::input_profile_store::{new_managed_profile_path, write_profile};
use adw::prelude::*;
use ira_input::{ControllerCalibration, GyroConfig, InputProfile};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone)]
pub(super) struct EditorForm {
    pub(super) name: Rc<RefCell<String>>,
    pub(super) profile: Rc<RefCell<InputProfile>>,
    pub(super) calibration: Rc<RefCell<ControllerCalibration>>,
    pub(super) gyro: Rc<RefCell<GyroConfig>>,
    pub(super) compatible_game_ids: Vec<i64>,
    pub(super) game_id: Option<i64>,
}
pub(super) fn build_profile(form: &EditorForm) -> Result<InputProfile, String> {
    let mut profile = form.profile.borrow().clone();
    profile.name = form.name.borrow().trim().to_string();
    profile.gyro = form.gyro.borrow().clone();
    profile.controller_calibration = *form.calibration.borrow();
    let mut ids = form.compatible_game_ids.clone();
    if let Some(game_id) = form.game_id {
        if !ids.contains(&game_id) {
            ids.push(game_id);
        }
    }
    profile.compatible_game_ids = ids;
    profile.validate()?;
    Ok(profile)
}
#[derive(Clone)]
pub(super) struct SaveOutcome {
    pub(super) save_dir: String,
    pub(super) current_path: Rc<RefCell<Option<PathBuf>>>,
    pub(super) baseline: Rc<RefCell<InputProfile>>,
    pub(super) status: gtk4::Label,
    pub(super) button: gtk4::Button,
    pub(super) window: adw::Window,
    pub(super) on_saved: Rc<dyn Fn(PathBuf)>,
    pub(super) close_on_success: bool,
}

pub(super) fn persist_closure(form: EditorForm, outcome: SaveOutcome) -> Rc<dyn Fn()> {
    Rc::new(move || match build_profile(&form) {
        Ok(profile) => {
            let path = profile_path_for_save(
                &outcome.save_dir,
                outcome.current_path.borrow().as_deref(),
                &profile.name,
            );
            match write_profile(&path, &profile) {
                Ok(()) => {
                    *outcome.current_path.borrow_mut() = Some(path.clone());
                    // The just-saved state is the new dirty-check baseline.
                    *outcome.baseline.borrow_mut() = profile;
                    outcome.status.set_visible(false);
                    outcome.button.set_sensitive(false);
                    (outcome.on_saved)(path);
                    if outcome.close_on_success {
                        outcome.window.close();
                    }
                }
                Err(error) => set_error(&outcome.status, &error),
            }
        }
        Err(error) => set_error(&outcome.status, &error),
    })
}

pub(super) fn connect_persist(button: &gtk4::Button, persist: Rc<dyn Fn()>) {
    button.connect_clicked(move |_| persist());
}

/// Closing with unsaved changes offers Save / Discard / Cancel. The footer's
/// Cancel button sets `force_close` first: pressing Cancel means discard and
/// close, never another prompt.
pub(super) fn connect_unsaved_guard(
    window: &adw::Window,
    save: &gtk4::Button,
    persist: &Rc<dyn Fn()>,
    baseline: &Rc<RefCell<InputProfile>>,
    form: &EditorForm,
    force_close: &Rc<std::cell::Cell<bool>>,
) {
    let save_for_guard = save.clone();
    let persist_for_guard = persist.clone();
    let baseline_for_guard = baseline.clone();
    let form_for_guard = form.clone();
    let force_close_for_guard = force_close.clone();
    window.connect_close_request(move |win| {
        if !save_for_guard.is_sensitive() || force_close_for_guard.get() {
            return gtk4::glib::Propagation::Proceed;
        }
        let dialog = adw::AlertDialog::new(
            Some(&crate::tr!("Unsaved changes")),
            Some(&crate::tr!("Save this layout before closing?")),
        );
        dialog.add_response("cancel", &crate::tr!("Cancel"));
        dialog.add_response("discard", &crate::tr!("Discard"));
        dialog.add_response("save", &crate::tr!("Save"));
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        let win_for_response = win.clone();
        let persist = persist_for_guard.clone();
        let baseline = baseline_for_guard.clone();
        let form = form_for_guard.clone();
        let save = save_for_guard.clone();
        dialog.connect_response(None, move |_, response| match response {
            "save" => persist(),
            "discard" => {
                // Pretend the current state is the baseline so the next
                // close request passes the guard.
                if let Ok(built) = build_profile(&form) {
                    *baseline.borrow_mut() = built;
                }
                save.set_sensitive(false);
                win_for_response.close();
            }
            _ => {}
        });
        dialog.present(Some(win));
        gtk4::glib::Propagation::Stop
    });
}
pub(super) fn profile_path_for_save(
    save_dir: &str,
    current_path: Option<&Path>,
    name: &str,
) -> PathBuf {
    current_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| new_managed_profile_path(save_dir, name))
}
fn set_error(status: &gtk4::Label, text: &str) {
    status.set_text(text);
    status.set_visible(true);
    status.set_css_classes(&[CSS_ERROR]);
}
