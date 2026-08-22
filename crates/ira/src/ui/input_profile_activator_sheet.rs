//! The per-input editor sheet: every activator of one input, Steam-style —
//! press patterns, outputs (with keyboard/mouse capture), gating, toggling,
//! repeat rates, stick behaviors, and mode shifts. Mutations apply straight
//! to the profile; `on_changed` fires after each one so the region pages can
//! refresh their summaries.

use super::css::{CSS_FLAT, CSS_SQUARE_BUTTON};
use super::helpers::esc;
use super::input_profile_editor_regions::{
    activator_kind_label, source_label, supported_button_sources,
};
use super::input_profile_options::{
    activation_index, activation_labels, output_display_label, output_index, output_labels,
    output_option, OutputOption,
};
use super::input_profile_output_capture::{
    show_keyboard_output_capture, show_mouse_output_capture,
};
use adw::prelude::*;
use gtk4::prelude::*;
use ira_input::{
    Activation, InputMapping, InputSource, OutputAction, SourceMode, StickOutput,
    VirtualGamepadBackend,
};
use std::cell::RefCell;
use std::rc::Rc;

type ProfileRc = Rc<RefCell<ira_input::InputProfile>>;
type OnChanged = Rc<dyn Fn()>;

fn find_mapping(profile: &ProfileRc, active_set: usize, source: InputSource) -> Option<InputMapping> {
    let borrow = profile.borrow();
    borrow
        .action_sets
        .get(active_set)?
        .inputs
        .iter()
        .find(|input| input.source == source)
        .cloned()
}

fn with_mapping(
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    apply: impl FnOnce(&mut InputMapping),
) {
    let mut borrow = profile.borrow_mut();
    if let Some(set) = borrow.action_sets.get_mut(active_ref(active_set))
        && let Some(input) = set.inputs.iter_mut().find(|input| input.source == source)
    {
        apply(input);
    }
}

fn active_ref(index: usize) -> usize {
    index
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn show_input_sheet(
    parent: &adw::Window,
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    device: Option<&ira_input::DeviceInfo>,
    backend: VirtualGamepadBackend,
    family: ira_input::ControllerFamily,
    on_changed: OnChanged,
) {
    let window = adw::Window::new();
    window.set_modal(true);
    window.set_transient_for(Some(parent));
    window.set_destroy_with_parent(true);
    window.set_hide_on_close(false);
    window.set_default_size(540, 680);
    window.set_title(Some(&source_label(source)));

    let header_bar = adw::HeaderBar::new();
    header_bar.set_show_title(false);
    let close = gtk4::Button::from_icon_name("window-close-symbolic");
    close.add_css_class(CSS_FLAT);
    header_bar.pack_end(&close);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&content));

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.append(&header_bar);
    root.append(&scroll);
    window.set_content(Some(&root));

    close.connect_clicked(move |button| {
        if let Some(host) = button
            .ancestor(gtk4::Window::static_type())
            .and_downcast::<adw::Window>()
        {
            host.close();
        }
    });

    fill_sheet(
        &content,
        &profile.clone(),
        active_set,
        source,
        device,
        backend,
        family,
        on_changed,
    );

    window.present();
}

/// Rebuild the whole sheet. Called after every structural change (adding or
/// removing activators/outputs/shifts); value edits mutate widgets in place.
#[allow(clippy::too_many_arguments)]
fn fill_sheet(
    content: &gtk4::Box,
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    device: Option<&ira_input::DeviceInfo>,
    backend: VirtualGamepadBackend,
    family: ira_input::ControllerFamily,
    on_changed: OnChanged,
) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }

    let mapping = match find_mapping(profile, active_set, source) {
        Some(mapping) => mapping,
        None => return,
    };

    if matches!(source, InputSource::Axis(_)) {
        content.append(&behavior_section(
            profile, active_set, source, &mapping, device, family, on_changed.clone(),
        ));
    }
    if !matches!(source, InputSource::Axis(_)) {
        content.append(&activators_section(
            profile, active_set, source, &mapping, device, backend, family, on_changed.clone(),
        ));
        content.append(&shifts_section(
            profile, active_set, source, device, on_changed.clone(),
        ));
    }
}

// ---------------------------------------------------------------------------
// Behavior (analog inputs)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn behavior_section(
    profile: &ProfileRC_PLACEHOLDER,
    _active_set: usize,
    _source: InputSource,
    _mapping: &InputMapping,
    _device: Option<&ira_input::DeviceInfo>,
    _family: ira_input::ControllerFamily,
    _on_changed: OnChanged,
) -> adw::PreferencesGroup {
    unimplemented!()
}
