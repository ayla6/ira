use super::css::{CSS_ERROR, CSS_FLAT, CSS_SUGGESTED_ACTION};
use super::input_profile_binding::{binding_from_row, BindingRow, SectionGroups};
use super::input_profile_editor_calibration::add_calibration_group;
use super::input_profile_editor_pages::{
    connect_backend_change, reset_to_defaults, setup_pages, setup_sidebar,
};
use super::input_profile_editor_sections::default_profile_bindings;
use super::input_profile_gyro_card::add_gyro_group;
use super::input_profile_store::{new_managed_profile_path, read_profile, write_profile};
use adw::prelude::*;
use ira_input::{DeviceInfo, GyroConfig, InputProfile, VirtualGamepadBackend};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone)]
struct ProfileForm {
    name: Rc<RefCell<String>>,
    rows: Rc<RefCell<Vec<BindingRow>>>,
    calibration: Rc<RefCell<ira_input::GyroCalibration>>,
    gyro: Rc<RefCell<GyroConfig>>,
    compatible_game_ids: Vec<i64>,
    game_id: Option<i64>,
    backend: Rc<RefCell<VirtualGamepadBackend>>,
}

pub(super) struct InputProfileEditorParams {
    pub save_dir: String,
    pub profile_path: Option<PathBuf>,
    pub game_id: Option<i64>,
    pub layout_name: Option<String>,
    pub registry: Arc<ira_input::ControllerRegistry>,
    pub device: Option<DeviceInfo>,
}

pub(super) fn show_input_profile_editor(
    parent: &gtk4::Window,
    params: InputProfileEditorParams,
    on_saved: impl Fn(PathBuf) + 'static,
) {
    let InputProfileEditorParams {
        save_dir,
        profile_path,
        game_id,
        layout_name,
        registry,
        device,
    } = params;
    let layout = super::helpers::dialog_layout(parent);
    layout.window.set_default_size(900, 720);
    layout.stack.set_hexpand(true);
    layout.stack.set_vexpand(true);

    let (mut profile, initial_status, initial_error) = match profile_path.as_deref() {
        Some(path) => match read_profile(path) {
            Ok(profile) => (profile, String::new(), false),
            Err(error) => (
                InputProfile::default(),
                format!("Could not load profile: {error}"),
                true,
            ),
        },
        None => (InputProfile::default(), String::new(), false),
    };
    if let Some(layout_name) = layout_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if profile.name.trim().is_empty() {
            profile.name = layout_name.to_string();
        }
    }
    let detected_devices = registry.snapshot();
    let device_was_explicit = device.is_some();
    let device = device.or_else(|| detected_devices.first().cloned());
    if profile_path.is_none() {
        profile = seed_new_profile_bindings(profile, device.as_ref());
    }
    // Dirty-check baseline: the profile as it is on disk, in the same
    // canonical form build_profile produces. Apply/Save update it so the
    // buttons only reflect real, unsaved edits.
    let original_profile = Rc::new(RefCell::new(
        profile_with_game(profile.clone(), game_id).into_action_set_form(),
    ));
    flatten_for_editor(&mut profile);
    let calibration = Rc::new(RefCell::new(profile.gyro_calibration));
    let gyro = Rc::new(RefCell::new(profile.gyro.clone()));
    let compatible_game_ids = profile.compatible_game_ids.clone();
    let profile_name = Rc::new(RefCell::new(profile.name.clone()));
    let calibration_device = (device_was_explicit || detected_devices.len() == 1)
        .then(|| device.clone())
        .flatten();
    let backend = Rc::new(RefCell::new(profile.backend));

    let rows = Rc::new(RefCell::new(Vec::<BindingRow>::new()));
    let current_path = Rc::new(RefCell::new(profile_path));
    let status = gtk4::Label::new(Some(&initial_status));
    status.set_xalign(0.0);
    status.set_visible(!initial_status.is_empty());
    if initial_error {
        status.add_css_class(CSS_ERROR);
    }
    let save = gtk4::Button::with_label(&crate::tr!("Save"));
    let apply = gtk4::Button::with_label(&crate::tr!("Apply"));
    save.add_css_class(CSS_SUGGESTED_ACTION);
    save.set_sensitive(false);
    apply.set_sensitive(false);
    let mark_dirty: Rc<dyn Fn()> = {
        let save = save.clone();
        let apply = apply.clone();
        let rows = rows.clone();
        let name = profile_name.clone();
        let compatible_game_ids = compatible_game_ids.clone();
        let calibration = calibration.clone();
        let gyro = gyro.clone();
        let backend = backend.clone();
        let original_profile = original_profile.clone();
        Rc::new(move || {
            let result = build_profile(
                &name.borrow(),
                &rows.borrow(),
                *calibration.borrow(),
                gyro.borrow().clone(),
                &compatible_game_ids,
                game_id,
                *backend.borrow(),
            );
            let dirty = result
                .as_ref()
                .is_ok_and(|profile| *profile != *original_profile.borrow());
            save.set_sensitive(dirty);
            apply.set_sensitive(dirty);
        })
    };

    let setup = setup_pages(
        &layout,
        &backend,
        &rows,
        &device,
        &mark_dirty,
        profile.bindings,
    );
    let page_boxes = setup.page_boxes;
    let section_groups = setup.section_groups;
    let backend_dropdown = setup.backend_dropdown;

    add_gyro_group(&page_boxes[4], &gyro, device.as_ref(), &mark_dirty);
    add_calibration_group(
        &page_boxes[4],
        ira_input::calibration_store_path(&save_dir),
        calibration_device,
        registry.clone(),
    );

    let reset = reset_button(
        &layout.window,
        &page_boxes,
        &section_groups,
        &rows,
        &device,
        &backend,
        &mark_dirty,
    );
    connect_backend_change(
        &backend_dropdown,
        &backend,
        &page_boxes,
        &section_groups,
        &rows,
        &device,
        &mark_dirty,
    );
    layout.header.pack_end(&reset);

    setup_sidebar(&layout);

    let cancel = add_editor_footer(&layout, &save, &apply, &status, &profile_name, &mark_dirty);

    let form = ProfileForm {
        name: profile_name,
        rows,
        calibration,
        gyro,
        compatible_game_ids,
        game_id,
        backend,
    };
    let on_saved: Rc<dyn Fn(PathBuf)> = Rc::new(on_saved);
    let window_for_save = layout.window.clone();
    let shared = |button: &gtk4::Button, close_on_success: bool| SaveOutcome {
        save_dir: save_dir.clone(),
        current_path: current_path.clone(),
        baseline: original_profile.clone(),
        status: status.clone(),
        button: button.clone(),
        window: window_for_save.clone(),
        on_saved: on_saved.clone(),
        close_on_success,
    };
    connect_save(&save, shared(&save, true), form.clone());
    connect_save(&apply, shared(&apply, false), form);
    let window_for_cancel = layout.window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());
    layout.window.present();
}

fn reset_button(
    window: &adw::Window,
    page_boxes: &[gtk4::Box],
    section_groups: &SectionGroups,
    rows: &Rc<RefCell<Vec<BindingRow>>>,
    device: &Option<DeviceInfo>,
    backend: &Rc<RefCell<VirtualGamepadBackend>>,
    mark_dirty: &Rc<dyn Fn()>,
) -> gtk4::Button {
    let reset = gtk4::Button::new();
    reset.add_css_class(CSS_FLAT);
    reset.set_tooltip_text(Some(&crate::tr!(
        "Replace the layout with the standard default bindings for this controller"
    )));
    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    content.append(&gtk4::Image::from_icon_name("view-refresh-symbolic"));
    content.append(&gtk4::Label::new(Some(&crate::tr!("Reset to defaults"))));
    reset.set_child(Some(&content));
    let page_boxes = page_boxes.to_vec();
    let section_groups = section_groups.clone();
    let rows = rows.clone();
    let device = device.clone();
    let backend = backend.clone();
    let mark_dirty = mark_dirty.clone();
    let window = window.clone();
    reset.connect_clicked(move |_| {
        let dialog = adw::AlertDialog::new(
            Some(&crate::tr!("Reset to defaults?")),
            Some(&crate::tr!("Replace the current layout with the standard one-to-one default bindings for this controller.")),
        );
        dialog.add_response("cancel", &crate::tr!("Cancel"));
        dialog.add_response("reset", &crate::tr!("Reset"));
        dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let page_boxes = page_boxes.clone();
        let section_groups = section_groups.clone();
        let rows = rows.clone();
        let device = device.clone();
        let backend = backend.clone();
        let mark_dirty = mark_dirty.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "reset" {
                reset_to_defaults(
                    &page_boxes,
                    &section_groups,
                    &rows,
                    device.as_ref(),
                    &backend,
                    &mark_dirty,
                );
            }
        });
        dialog.present(Some(&window));
    });
    reset
}

fn add_editor_footer(
    layout: &super::helpers::DialogLayout,
    save: &gtk4::Button,
    apply: &gtk4::Button,
    status: &gtk4::Label,
    profile_name: &Rc<RefCell<String>>,
    mark_dirty: &Rc<dyn Fn()>,
) -> gtk4::Button {
    let cancel = gtk4::Button::with_label(&crate::tr!("Cancel"));
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    actions.set_halign(gtk4::Align::End);
    actions.set_margin_start(16);
    actions.set_margin_end(16);
    actions.set_margin_top(8);
    actions.set_margin_bottom(12);
    actions.append(&cancel);
    actions.append(apply);
    actions.append(save);
    let name = adw::EntryRow::new();
    name.set_title(&crate::tr!("Profile name"));
    name.set_text(&profile_name.borrow());
    let profile_name_for_changed = profile_name.clone();
    let mark_dirty_for_changed = mark_dirty.clone();
    name.connect_changed(move |entry| {
        *profile_name_for_changed.borrow_mut() = entry.text().to_string();
        mark_dirty_for_changed();
    });
    layout.content_area.append(&name);
    layout.content_area.append(status);
    layout.content_area.append(&actions);
    cancel
}

#[derive(Clone)]
struct SaveOutcome {
    save_dir: String,
    current_path: Rc<RefCell<Option<PathBuf>>>,
    baseline: Rc<RefCell<InputProfile>>,
    status: gtk4::Label,
    button: gtk4::Button,
    window: adw::Window,
    on_saved: Rc<dyn Fn(PathBuf)>,
    close_on_success: bool,
}

fn connect_save(
    button: &gtk4::Button,
    outcome: SaveOutcome,
    form: ProfileForm,
) {
    button.connect_clicked(move |_| {
        let result = build_profile(
            &form.name.borrow(),
            &form.rows.borrow(),
            *form.calibration.borrow(),
            form.gyro.borrow().clone(),
            &form.compatible_game_ids,
            form.game_id,
            *form.backend.borrow(),
        );
        persist(result, &outcome);
    });
}

/// Write the built profile; on success reset the dirty baseline, notify the
/// caller and optionally close. Save closes, Apply stays open.
fn persist(result: Result<InputProfile, String>, outcome: &SaveOutcome) {
    let result = result.and_then(|profile| {
        let path =
            profile_path_for_save(&outcome.save_dir, outcome.current_path.borrow().as_deref(), &profile.name);
        write_profile(&path, &profile)?;
        Ok((path, profile))
    });
    match result {
        Ok((path, profile)) => {
            *outcome.current_path.borrow_mut() = Some(path.clone());
            // The just-saved state is the new dirty-check baseline; only
            // further edits re-enable the buttons.
            *outcome.baseline.borrow_mut() = profile;
            set_saved_status(&outcome.status);
            outcome.button.set_sensitive(false);
            (outcome.on_saved)(path);
            if outcome.close_on_success {
                outcome.window.close();
            }
        }
        Err(error) => set_error(&outcome.status, &error),
    }
}

fn set_saved_status(status: &gtk4::Label) {
    status.remove_css_class(CSS_ERROR);
    status.set_text(&crate::tr!("Saved"));
    status.set_visible(true);
}

fn profile_path_for_save(save_dir: &str, current_path: Option<&Path>, name: &str) -> PathBuf {
    current_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| new_managed_profile_path(save_dir, name))
}

fn build_profile(
    name: &str,
    rows: &[BindingRow],
    calibration: ira_input::GyroCalibration,
    gyro: GyroConfig,
    compatible_game_ids: &[i64],
    game_id: Option<i64>,
    backend: VirtualGamepadBackend,
) -> Result<InputProfile, String> {
    let mut compatible_game_ids = compatible_game_ids.to_vec();
    if let Some(game_id) = game_id {
        if !compatible_game_ids.contains(&game_id) {
            compatible_game_ids.push(game_id);
        }
    }
    let profile = InputProfile {
        name: name.trim().to_string(),
        bindings: rows
            .iter()
            .map(binding_from_row)
            .collect::<Result<_, _>>()?,
        gyro_calibration: calibration,
        gyro,
        compatible_game_ids,
        backend,
        ..InputProfile::default()
    }
    .into_action_set_form();
    profile.validate()?;
    Ok(profile)
}

/// Profiles are stored in canonical action-set form; the row-based editor
/// edits a flattened view of the primary set. Extra sets and layers stay
/// in the file untouched but are not editable yet.
fn flatten_for_editor(profile: &mut InputProfile) {
    if !profile.bindings.is_empty() || profile.action_sets.is_empty() {
        return;
    }
    if profile.action_sets.len() > 1 || !profile.action_layers.is_empty() {
        eprintln!(
            "input profile editor: editing the first action set only ({} set(s), {} layer(s) on disk)",
            profile.action_sets.len(),
            profile.action_layers.len()
        );
    }
    profile.bindings = profile.editor_bindings();
}

fn seed_new_profile_bindings(
    mut profile: InputProfile,
    device: Option<&DeviceInfo>,
) -> InputProfile {
    if profile.bindings.is_empty() {
        profile.bindings = default_profile_bindings(device, profile.backend);
    }
    profile
}

fn profile_with_game(mut profile: InputProfile, game_id: Option<i64>) -> InputProfile {
    if let Some(game_id) = game_id {
        if !profile.compatible_game_ids.contains(&game_id) {
            profile.compatible_game_ids.push(game_id);
        }
    }
    profile
}

fn set_error(status: &gtk4::Label, text: &str) {
    status.set_text(text);
    status.set_visible(true);
    status.set_css_classes(&[CSS_ERROR]);
}

#[cfg(test)]
#[path = "input_profile_editor_tests.rs"]
mod tests;
