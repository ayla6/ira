//! Profile editor shell: canonical action-set profile in an `Rc<RefCell>`,
//! Steam-style region pages + per-input sheet for editing, Gyro and Action
//! Sets pages, Cancel / Apply / Save footer with a canonical dirty check,
//! and an unsaved-changes guard.

use super::css::{CSS_ERROR, CSS_SUGGESTED_ACTION};
use super::helpers::DialogLayout;
use super::input_profile_action_sets::build_sets_page;
use super::input_profile_set_indicator::SetIndicator;
use super::input_profile_editor_regions::Region;
use super::input_profile_editor_save::{
    connect_persist, connect_unsaved_guard, is_dirty, persist_closure, EditorForm, SaveOutcome,
};
use super::input_profile_gyro_card::add_gyro_group;
use super::input_profile_region_pages::{
    rebuild_region_pages, EditingTarget, PagesCtx, RegionPages,
};
use super::input_profile_store::read_profile;
use adw::prelude::*;
use ira_input::{ActionSet, DeviceInfo, GyroConfig, InputProfile, VirtualGamepadBackend};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

pub(super) struct InputProfileEditorParams {
    pub save_dir: String,
    pub profile_path: Option<PathBuf>,
    /// Seed content for a new layout: `profile_path` is None (the seed is
    /// never written back to) but the editor starts from this file's layout
    /// instead of the blank default — the "create a copy for this game"
    /// choice. The copy saves under its own name.
    pub seed_from: Option<PathBuf>,
    pub game_id: Option<i64>,
    /// The presenting game's platform, when it has one. New profiles are
    /// scoped to it so a Wii layout never offers itself to a PS1 game.
    pub platform_id: Option<String>,
    pub layout_name: Option<String>,
    pub registry: Arc<ira_input::ControllerRegistry>,
    pub device: Option<DeviceInfo>,
}

pub(super) fn show_input_profile_editor(
    parent: &impl IsA<gtk4::Widget>,
    params: InputProfileEditorParams,
    on_saved: impl Fn(PathBuf) + 'static,
) {
    let InputProfileEditorParams {
        save_dir,
        profile_path,
        seed_from,
        game_id,
        platform_id,
        layout_name,
        registry,
        device,
    } = params;
    let layout = super::helpers::dialog_layout();
    layout.window.set_content_width(980);
    // The editor floats inside other dialogs (the game settings, at their
    // default 720 px height) and a floating sheet adds its own chrome, so
    // anything above ~680 px overflows and spams layout warnings. 640 fits
    // with room to spare; the pages scroll, and fit_dialog_height shrinks
    // further when the presenting parent reports a smaller allocation.
    super::helpers::fit_dialog_height(&layout.window, parent, 640);
    layout.stack.set_hexpand(true);
    layout.stack.set_vexpand(true);

    let (mut profile, initial_status, initial_error) = match profile_path.as_deref() {
        Some(path) => match read_profile(path) {
            Ok(profile) => (profile, String::new(), false),
            Err(error) => (
                InputProfile::default(),
                crate::tr!("Could not load profile: {}").replacen("{}", &error, 1),
                true,
            ),
        },
        // Seeded copy: start from another layout's content but stay a
        // never-saved new file, renamed to the presenting layout name.
        None => match seed_from.as_deref().map(read_profile) {
            Some(Ok(mut seeded)) => {
                if let Some(name) = layout_name.as_deref().filter(|name| !name.is_empty()) {
                    seeded.name = name.to_string();
                }
                (seeded, String::new(), false)
            }
            _ => {
                let mut profile = InputProfile::default();
                if let Some(platform_id) = platform_id.as_deref() {
                    if !platform_id.is_empty() {
                        profile.compatible_platform_ids.push(platform_id.to_string());
                    }
                }
                (profile, String::new(), false)
            }
        },
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
    let device = device.or_else(|| detected_devices.first().cloned());
    if profile.action_sets.is_empty() {
        let backend = profile.backend;
        profile.action_sets = default_action_sets(device.as_ref(), backend);
    }

    let profile_rc = Rc::new(RefCell::new(profile.clone()));
    let active_target = Rc::new(Cell::new(EditingTarget::Set(0)));
    // Dirty baseline: the on-disk form with this game attached, the same
    // canonical shape build_profile produces.
    let baseline = Rc::new(RefCell::new(with_game(profile.clone(), game_id)));
    // A layout opened without a file has never been written: it counts as
    // dirty from the start so it can be saved without forcing a dummy edit.
    let unsaved = Rc::new(std::cell::Cell::new(profile_path.is_none()));
    let gyro = Rc::new(RefCell::new(profile.gyro.clone()));
    let calibration = Rc::new(RefCell::new(profile.controller_calibration));
    let compatible_game_ids = profile.compatible_game_ids.clone();
    let profile_name = Rc::new(RefCell::new(profile.name.clone()));
    let current_path = Rc::new(RefCell::new(profile_path));

    let status = gtk4::Label::new(Some(&initial_status));
    status.set_xalign(0.0);
    status.set_visible(!initial_status.is_empty());
    // Validation errors can be long; wrapped text keeps the label's natural
    // width bounded so the floating sheet never asks for more than its
    // presenter has (Adwaita warns and clips that).
    status.set_wrap(true);
    status.set_max_width_chars(70);
    status.set_hexpand(true);
    if initial_error {
        status.add_css_class(CSS_ERROR);
    }
    let save = gtk4::Button::with_label(&crate::tr!("Save"));
    let apply = gtk4::Button::with_label(&crate::tr!("Apply"));
    save.add_css_class(CSS_SUGGESTED_ACTION);
    save.set_sensitive(false);
    apply.set_sensitive(false);

    // Two-phase wiring: pages need an on_dirty hook, the hook needs the
    // pages. The indirection cell resolves the cycle.
    type DirtyHook = Rc<RefCell<Option<Rc<dyn Fn()>>>>;
    let dirty_hook: DirtyHook = Rc::new(RefCell::new(None));
    let on_dirty: Rc<dyn Fn()> = {
        let dirty_hook = dirty_hook.clone();
        Rc::new(move || {
            if let Some(hook) = dirty_hook.borrow().as_ref() {
                hook();
            }
        })
    };

    // Steam-style top-left display of the set (or layer) being edited.
    let indicator = Rc::new(SetIndicator::new());
    layout
        .sidebar_area
        .insert_child_after(&indicator.root, None::<&gtk4::Widget>);

    let ctx = PagesCtx {
        window: layout.window.clone(),
        profile: profile_rc.clone(),
        active_target: active_target.clone(),
        indicator: indicator.clone(),
        device,
        on_dirty: on_dirty.clone(),
        gyro: gyro.clone(),
        expansion: std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::new())),
    };
    let pages = build_pages(&layout, &ctx, &gyro, &profile_name);

    let form = EditorForm {
        name: profile_name,
        profile: profile_rc,
        calibration,
        gyro,
        compatible_game_ids,
        game_id,
    };
    {
        let ctx = ctx.clone();
        let pages = pages.clone();
        let form = form.clone();
        let baseline = baseline.clone();
        let unsaved = unsaved.clone();
        let save = save.clone();
        let apply = apply.clone();
        let rebuild_pending = Rc::new(std::cell::Cell::new(false));
        *dirty_hook.borrow_mut() = Some(Rc::new(move || {
            // The rebuild destroys and recreates every region page. Running
            // that while a signal emission is still unwinding through those
            // widgets finalizes them under GTK's feet (the recurring
            // gtk_widget_get_parent/add_css_class criticals), so defer to the
            // next idle and coalesce bursts of change notifications.
            if rebuild_pending.replace(true) {
                return;
            }
            let ctx = ctx.clone();
            let pages = pages.clone();
            let form = form.clone();
            let baseline = baseline.clone();
            let unsaved = unsaved.clone();
            let save = save.clone();
            let apply = apply.clone();
            let rebuild_pending = rebuild_pending.clone();
            gtk4::glib::idle_add_local_once(move || {
                // Rebuilding the pages makes their widgets emit change
                // signals during construction, and every one of those
                // handlers calls on_dirty. Keep the coalescing flag held
                // across the whole rebuild so those echoes never schedule
                // another pass — rebuilding from inside a rebuild consumed
                // memory without bound and took the session down.
                rebuild_pending.set(true);
                rebuild_region_pages(&ctx, &pages);
                let dirty = is_dirty(unsaved.get(), &form, &baseline.borrow());
                save.set_sensitive(dirty);
                apply.set_sensitive(dirty);
                rebuild_pending.set(false);
            });
        }));
    }
    (on_dirty)();

    setup_sidebar_navigation(&layout);

    let cancel = add_editor_footer(&layout, &save, &apply, &status);

    let on_saved: Rc<dyn Fn(PathBuf)> = Rc::new(on_saved);
    let window_for_save = layout.window.clone();
    let shared = |button: &gtk4::Button, close_on_success: bool| SaveOutcome {
        save_dir: save_dir.clone(),
        current_path: current_path.clone(),
        baseline: baseline.clone(),
        unsaved: unsaved.clone(),
        status: status.clone(),
        button: button.clone(),
        window: window_for_save.clone(),
        on_saved: on_saved.clone(),
        close_on_success,
    };
    let persist_save = persist_closure(form.clone(), shared(&save, true));
    connect_persist(&save, persist_save.clone());
    connect_persist(&apply, persist_closure(form.clone(), shared(&apply, false)));

    // Cancel means discard: close without the unsaved-changes prompt.
    let force_close = Rc::new(std::cell::Cell::new(false));
    let window_for_cancel = layout.window.clone();
    let force_close_for_cancel = force_close.clone();
    cancel.connect_clicked(move |_| {
        force_close_for_cancel.set(true);
        window_for_cancel.close();
    });

    connect_unsaved_guard(&layout.window, &persist_save, &baseline, &form, &force_close);

    layout.window.present(Some(parent));
}

fn build_pages(
    layout: &DialogLayout,
    ctx: &PagesCtx,
    gyro: &Rc<RefCell<GyroConfig>>,
    name: &Rc<RefCell<String>>,
) -> RegionPages {
    let mut region_boxes = Vec::new();
    // Steam-style sidebar: the profile-level General page, then grouped
    // sections of pages rather than one flat list.
    let (general_scroll, general_box) = scrolling_page();
    layout.stack.add_named(&general_scroll, Some("general"));
    layout
        .sidebar
        .append(&super::settings_dialog::settings_sidebar_row(
            "emblem-system-symbolic",
            &crate::tr!("General"),
            "general",
        ));
    general_box.append(&super::input_profile_general_page::build_general_page(
        ctx, name,
    ));
    // The Controller page was one lonely haptics group; it lives on General
    // now instead of its own near-empty page.
    super::input_profile_controller_page::add_controller_groups(
        &general_box,
        &ctx.profile,
        &ctx.on_dirty,
    );

    for region in Region::ALL {
        let (scroll, content) = scrolling_page();
        layout.stack.add_named(&scroll, Some(region.id()));
        layout
            .sidebar
            .append(&super::settings_dialog::settings_sidebar_row(
                region.icon(),
                &region.title(),
                region.id(),
            ));
        region_boxes.push(content);
    }

    let (gyro_scroll, gyro_box) = scrolling_page();
    layout.stack.add_named(&gyro_scroll, Some("gyro"));
    layout
        .sidebar
        .append(&super::settings_dialog::settings_sidebar_row(
            "view-refresh-symbolic",
            &crate::tr!("Gyro"),
            "gyro",
        ));
    add_gyro_group(&gyro_box, gyro, ctx.device.as_ref(), &ctx.on_dirty);
    super::input_profile_gyro_motion::add_gyro_motion_groups(&gyro_box, gyro, &ctx.on_dirty);

    let (sets_scroll, sets_box) = scrolling_page();
    layout.stack.add_named(&sets_scroll, Some("sets"));
    layout
        .sidebar
        .append(&super::settings_dialog::settings_sidebar_row(
            "view-grid-symbolic",
            &crate::tr!("Action Sets"),
            "sets",
        ));
    sets_box.append(&build_sets_page(ctx, &ctx.on_dirty));

    RegionPages { region_boxes }
}

fn scrolling_page() -> (gtk4::ScrolledWindow, gtk4::Box) {
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    scroll.set_child(Some(&content));
    (scroll, content)
}

fn setup_sidebar_navigation(layout: &DialogLayout) {
    let stack = layout.stack.clone();
    layout.sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            stack.set_visible_child_name(&row.widget_name());
        }
    });
    // The sidebar leads with non-selectable section titles, so the first
    // selectable row with a page name is the one to open on.
    let mut child = layout.sidebar.first_child();
    while let Some(widget) = child {
        if let Some(row) = widget.downcast_ref::<gtk4::ListBoxRow>() {
            if row.is_selectable() && !row.widget_name().is_empty() {
                layout.sidebar.select_row(Some(row));
                break;
            }
        }
        child = widget.next_sibling();
    }
}

fn default_action_sets(
    device: Option<&DeviceInfo>,
    backend: VirtualGamepadBackend,
) -> Vec<ActionSet> {
    let base = match device {
        Some(device) => InputProfile::default_gamepad_for_backend_and_buttons(
            backend,
            &device.supported_buttons,
        ),
        None => InputProfile::default_gamepad_for_backend(backend),
    };
    base.action_sets
}

fn with_game(mut profile: InputProfile, game_id: Option<i64>) -> InputProfile {
    if let Some(game_id) = game_id {
        if !profile.compatible_game_ids.contains(&game_id) {
            profile.compatible_game_ids.push(game_id);
        }
    }
    profile
}

fn add_editor_footer(
    layout: &DialogLayout,
    save: &gtk4::Button,
    apply: &gtk4::Button,
    status: &gtk4::Label,
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
    let footer = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    footer.set_margin_start(16);
    footer.set_margin_end(16);
    footer.append(status);
    footer.append(&actions);
    layout.content_area.append(&footer);
    cancel
}

/// Used by tests through the region module's default mapping.
#[cfg(test)]
pub(super) fn test_default_output(source: ira_input::InputSource) -> ira_input::OutputAction {
    super::input_profile_input_rows::default_mapping(source)
        .activators
        .first()
        .and_then(|activator| activator.outputs.first().cloned())
        .unwrap_or(ira_input::OutputAction::GamepadButton(
            ira_input::GamepadButton::A,
        ))
}

#[cfg(test)]
#[path = "input_profile_editor_tests.rs"]
mod tests;
