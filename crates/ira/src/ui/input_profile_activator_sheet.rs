//! The per-input editor sheet window: activators with their press
//! patterns and outputs, analog behavior for sticks and triggers
//! (including dual-stage trigger activators), and mode shifts. Mutations
//! apply straight to the profile; `on_changed` fires after each one so
//! the region pages can refresh their summaries.

use super::input_profile_editor_regions::source_label;
use super::input_profile_mode_shifts::shifts_group;
use super::input_profile_sheet_base::{find_mapping, is_trigger_axis, OnChanged, ProfileRc};
use super::input_profile_source_modes::{behavior_group, mode_settings_group, modes_for};
use adw::prelude::*;
use ira_input::InputSource;
use std::rc::Rc;

/// Everything one input's sheet needs; built by the region pages.
pub(crate) struct InputSheetRequest {
    pub profile: ProfileRc,
    pub active_target: super::input_profile_sheet_base::EditingTarget,
    pub source: InputSource,
    pub device: Option<ira_input::DeviceInfo>,
    pub backend: ira_input::VirtualGamepadBackend,
    pub on_changed: OnChanged,
}

pub(crate) fn show_input_sheet(parent: &impl IsA<gtk4::Widget>, request: InputSheetRequest) {
    let window = adw::Dialog::new();
    window.set_content_width(560);
    // This sheet floats inside the profile editor, which itself caps at
    // 640 px — the sheet must fit under that with its own chrome.
    super::helpers::fit_dialog_height(&window, parent, 590);
    window.set_title(&sheet_title(&request));

    let header_bar = adw::HeaderBar::new();
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&content));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header_bar);
    toolbar.set_content(Some(&scroll));
    window.set_child(Some(&toolbar));

    let base = super::input_profile_sheet_base::SheetBase {
        content,
        profile: request.profile,
        active_target: request.active_target,
        source: request.source,
        device: request.device,
        backend: request.backend,
        on_changed: request.on_changed,
        rebuild_pending: Rc::new(std::cell::Cell::new(false)),
    };
    fill_sheet(&base);
    window.present(Some(parent));
}

fn sheet_title(request: &InputSheetRequest) -> String {
    crate::tr!("Edit {input}").replace("{input}", &source_label(request.source))
}

/// Rebuild the whole sheet after every structural change (mode or activator
/// add/remove); value edits mutate widgets in place.
///
/// The rebuild is deferred to the next idle and coalesced: clearing children
/// while a signal emission is still unwinding through them finalizes widgets
/// GTK touches afterwards (the recurring get_parent/add_css_class criticals).
fn fill_sheet(base: &super::input_profile_sheet_base::SheetBase) {
    if base.rebuild_pending.replace(true) {
        return;
    }
    let base = base.clone();
    gtk4::glib::idle_add_local_once(move || {
        base.rebuild_pending.set(false);
        rebuild_sheet(&base);
    });
}

fn rebuild_sheet(base: &super::input_profile_sheet_base::SheetBase) {
    let reopen: super::input_profile_sheet_base::Reopen = {
        let base = base.clone();
        Rc::new(move || fill_sheet(&base))
    };
    super::helpers::clear_children(&base.content);

    if matches!(base.source, InputSource::Axis(_)) {
        base.content
            .append(&behavior_group(base, &reopen, modes_for(base.source)));
        if let Some(mapping) = find_mapping(base) {
            if let Some(ref mode) = mapping.mode {
                if is_trigger_axis(base.source) {
                    base.content
                        .append(&mode_settings_group(base, mode, &reopen));
                } else {
                    // Sticks get Steam's full settings layout: sensitivity,
                    // output, and deadzone groups.
                    for group in
                        super::input_profile_stick_settings::stick_mode_groups(base, mode, &reopen)
                    {
                        base.content.append(&group);
                    }
                }
            }
            // Triggers carry activators alongside their analog mode — the
            // dual-stage soft/full pull. Every axis can also be shifted.
            if is_trigger_axis(base.source) {
                base.content
                    .append(&super::input_profile_activator_edit::activators_group(
                        base, &reopen, &mapping,
                    ));
            }
            base.content.append(&shifts_group(base, &reopen));
        }
        return;
    }

    if let Some(mapping) = find_mapping(base) {
        base.content
            .append(&super::input_profile_activator_edit::activators_group(
                base, &reopen, &mapping,
            ));
        base.content.append(&shifts_group(base, &reopen));
    }
}
