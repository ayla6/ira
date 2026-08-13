use super::super::css::{
    CSS_BINDING_SECTION_HEADER, CSS_BINDING_SUFFIX, CSS_FLAT, CSS_SOURCE_BADGE, CSS_SQUARE_BUTTON,
};
use super::super::input_profile_options::{
    activation_index, activation_labels, activator_index, gyro_mode_index, gyro_mode_labels,
    output_index, output_labels, recenter_index, source_options_for_device,
};
use super::assets::{set_source_asset, source_badge};
use super::categories::{
    activation_sources, chord_text_for_options, is_analog_source, section_source_options,
    section_title_label, source_index_for, uses_gyro_stick_output,
};
use super::signals::{
    connect_dirty, connect_output_changes, connect_source_changes, update_activation_controls,
    update_binding_summary,
};
use super::types::{BindingRow, BindingRowContext, OutputChangeContext, SourceChangeContext};
use adw::prelude::*;
use ira_input::{
    Binding, ControllerFamily, DeviceInfo, InputSource, OutputAction, VirtualGamepadBackend,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(crate) fn add_binding_row(
    group: &adw::PreferencesGroup,
    binding: Binding,
    context: &BindingRowContext,
) {
    let page_index = super::binding_page_index(&binding);
    let row = make_binding_row(
        binding,
        page_index,
        context.device.as_ref(),
        context.backend,
        &context.on_dirty,
    );
    connect_dirty(&row, &context.on_dirty);
    let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove.add_css_class(CSS_FLAT);
    remove.add_css_class(CSS_SQUARE_BUTTON);
    remove.set_valign(gtk4::Align::Center);
    remove.set_tooltip_text(Some(&crate::tr!("Remove binding")));
    connect_remove(&remove, group, &row, context, page_index);
    row.container.add_suffix(&remove);
    group.add(&row.container);
    context.rows.borrow_mut().push(row);
}

fn connect_remove(
    remove: &gtk4::Button,
    group: &adw::PreferencesGroup,
    row: &BindingRow,
    context: &BindingRowContext,
    page_index: usize,
) {
    let container = row.container.clone();
    let group = group.clone();
    let page = context.page.clone();
    let section_groups = context.section_groups.clone();
    let rows = context.rows.clone();
    let on_dirty = context.on_dirty.clone();
    remove.connect_clicked(move |_| {
        group.remove(&container);
        rows.borrow_mut()
            .retain(|candidate| candidate.container != container);
        if group.first_child().is_none() {
            page.remove(&group);
            if let Some(sections) = section_groups.borrow_mut().get_mut(page_index) {
                sections.retain(|(_, candidate)| candidate != &group);
            }
            if !page
                .first_child()
                .is_some_and(|child| child.is::<adw::PreferencesGroup>())
            {
                add_empty_page_state(&page);
            }
        }
        on_dirty();
    });
}

pub(crate) fn add_empty_page_state(page: &gtk4::Box) {
    let empty = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    empty.set_widget_name("input-empty-state");
    empty.set_halign(gtk4::Align::Center);
    empty.set_valign(gtk4::Align::Start);
    empty.set_margin_top(36);
    let icon = gtk4::Image::from_icon_name("input-gaming-symbolic");
    let label = gtk4::Label::new(Some(&crate::tr!("No bindings")));
    label.add_css_class("dim-label");
    empty.append(&icon);
    empty.append(&label);
    page.append(&empty);
}

struct RowControls {
    source_options: Vec<(InputSource, String)>,
    activator_options: Vec<(InputSource, String)>,
    source: gtk4::DropDown,
    output: gtk4::DropDown,
    output_action: Rc<RefCell<OutputAction>>,
    activation: gtk4::DropDown,
    activator: gtk4::DropDown,
    chord: adw::EntryRow,
    recenter: gtk4::DropDown,
    gyro_mode: gtk4::DropDown,
    dead_zone: gtk4::SpinButton,
    sensitivity: gtk4::SpinButton,
    exponent: gtk4::SpinButton,
    invert: gtk4::CheckButton,
}

struct BindingRows {
    activator: adw::ActionRow,
    recenter: adw::ActionRow,
    gyro_mode: adw::ActionRow,
    dead_zone: adw::ActionRow,
    sensitivity: adw::ActionRow,
    exponent: adw::ActionRow,
    invert: adw::ActionRow,
}

fn make_binding_row(
    binding: Binding,
    page_index: usize,
    device: Option<&DeviceInfo>,
    backend: VirtualGamepadBackend,
    on_dirty: &Rc<dyn Fn()>,
) -> BindingRow {
    let source_options = section_source_options(page_index, device, backend, Some(binding.source));
    let controls = build_controls(&binding, source_options, device, backend);
    let container = adw::ExpanderRow::new();
    update_binding_summary(
        &controls.source,
        &controls.output_action.borrow(),
        &container,
    );
    let family = device.map(DeviceInfo::family).unwrap_or_default();
    let (fallback, asset, current_source) = add_source_prefix(&container, binding.source, family);
    let rows = add_binding_controls(&container, &controls);
    configure_control_visibility(&rows, binding.source, &binding.output);
    connect_activation_changes(
        &controls.activation,
        &controls.activator,
        &rows.activator,
        &controls.chord,
    );
    connect_source_changes(
        &controls.source,
        SourceChangeContext {
            source_options: controls.source_options.clone(),
            family,
            fallback,
            asset,
            current_source,
            output: controls.output_action.clone(),
            row: container.clone(),
            dead_zone_row: rows.dead_zone.clone(),
            sensitivity_row: rows.sensitivity.clone(),
            exponent_row: rows.exponent.clone(),
            invert_row: rows.invert.clone(),
            recenter_row: rows.recenter.clone(),
            gyro_mode_row: rows.gyro_mode.clone(),
        },
    );
    connect_output_changes(
        &controls.output,
        OutputChangeContext {
            source: controls.source.clone(),
            source_options: controls.source_options.clone(),
            output: controls.output_action.clone(),
            gyro_mode_row: rows.gyro_mode.clone(),
            row: container.clone(),
            on_dirty: on_dirty.clone(),
            backend,
        },
    );
    BindingRow {
        container,
        source_options: controls.source_options,
        activator_options: controls.activator_options,
        source: controls.source,
        output: controls.output,
        output_action: controls.output_action,
        activation: controls.activation,
        activator: controls.activator,
        chord: controls.chord,
        recenter: controls.recenter,
        gyro_mode: controls.gyro_mode,
        dead_zone: controls.dead_zone,
        sensitivity: controls.sensitivity,
        exponent: controls.exponent,
        invert: controls.invert,
    }
}

fn build_controls(
    binding: &Binding,
    source_options: Vec<(InputSource, String)>,
    device: Option<&DeviceInfo>,
    backend: VirtualGamepadBackend,
) -> RowControls {
    let source = combo_row(
        &labels(&source_options),
        source_index_for(&source_options, binding.source),
    );
    let output = combo_row(
        &output_labels(backend),
        output_index(&binding.output, backend),
    );
    let output_action = Rc::new(RefCell::new(binding.output.clone()));
    let activation = combo_row(&activation_labels(), activation_index(&binding.activation));
    let mut activator_options = source_options_for_device(device, backend);
    for source in activation_sources(&binding.activation) {
        if !activator_options
            .iter()
            .any(|(candidate, _)| *candidate == source)
        {
            activator_options.push((
                source,
                format!(
                    "{} (unavailable)",
                    source_badge(source, ControllerFamily::default())
                ),
            ));
        }
    }
    let activator = combo_row(
        &labels(&activator_options),
        activator_index(&binding.activation, &activator_options),
    );
    let chord = adw::EntryRow::new();
    chord.set_title(&crate::tr!("Chord sources"));
    chord.set_text(&chord_text_for_options(
        &binding.activation,
        &activator_options,
    ));
    chord.set_tooltip_text(Some(&crate::tr!("Comma-separated input sources")));
    let recenter = combo_row(
        &[
            crate::tr!("Never"),
            crate::tr!("On enable"),
            crate::tr!("On disable"),
            crate::tr!("On enable and disable"),
        ],
        recenter_index(binding.recenter),
    );
    let gyro_mode = combo_row(&gyro_mode_labels(), gyro_mode_index(binding.gyro_mode));
    let dead_zone = spin_button(0.0, 0.99, 0.01, 2, binding.transform.dead_zone);
    let sensitivity = spin_button(0.0, 1000.0, 0.1, 2, binding.transform.sensitivity);
    let exponent = spin_button(0.1, 5.0, 0.1, 2, binding.transform.exponent);
    let invert = gtk4::CheckButton::with_label(&crate::tr!("Invert"));
    invert.set_active(binding.transform.invert);
    RowControls {
        source_options,
        activator_options,
        source,
        output,
        output_action,
        activation,
        activator,
        chord,
        recenter,
        gyro_mode,
        dead_zone,
        sensitivity,
        exponent,
        invert,
    }
}

fn labels(options: &[(InputSource, String)]) -> Vec<String> {
    options.iter().map(|(_, label)| label.clone()).collect()
}

fn spin_button(min: f64, max: f64, step: f64, digits: u32, value: f32) -> gtk4::SpinButton {
    let spin = gtk4::SpinButton::with_range(min, max, step);
    spin.set_digits(digits);
    spin.set_value(value as f64);
    spin
}

fn add_source_prefix(
    container: &adw::ExpanderRow,
    source: InputSource,
    family: ControllerFamily,
) -> (gtk4::Label, gtk4::Image, Rc<Cell<InputSource>>) {
    let fallback = gtk4::Label::new(Some(&source_badge(source, family)));
    fallback.add_css_class(CSS_SOURCE_BADGE);
    fallback.set_valign(gtk4::Align::Center);
    let asset = gtk4::Image::new();
    asset.set_pixel_size(24);
    set_source_asset(&asset, &fallback, source, family);
    let current_source = Rc::new(Cell::new(source));
    if let Some(settings) = gtk4::Settings::default() {
        let asset_for_theme = asset.clone();
        let fallback_for_theme = fallback.clone();
        let source_for_theme = current_source.clone();
        settings.connect_gtk_application_prefer_dark_theme_notify(move |_| {
            set_source_asset(
                &asset_for_theme,
                &fallback_for_theme,
                source_for_theme.get(),
                family,
            );
        });
    }
    container.add_prefix(&asset);
    container.add_prefix(&fallback);
    (fallback, asset, current_source)
}

fn add_binding_controls(container: &adw::ExpanderRow, controls: &RowControls) -> BindingRows {
    add_control_row(container, "Source", &controls.source);
    add_control_row(container, "Output", &controls.output);
    add_section_header(container, "Behavior");
    add_control_row(container, "Activation", &controls.activation);
    let activator = add_control_row(container, "Activator", &controls.activator);
    container.add_row(&controls.chord);
    let recenter = add_control_row(container, "Recenter", &controls.recenter);
    let gyro_mode = add_control_row(container, "Gyro output", &controls.gyro_mode);
    add_section_header(container, "Response");
    let dead_zone = add_control_row(container, "Dead zone", &controls.dead_zone);
    let sensitivity = add_control_row(container, "Sensitivity", &controls.sensitivity);
    let exponent = add_control_row(container, "Exponent", &controls.exponent);
    let invert = add_control_row(container, "Invert", &controls.invert);
    BindingRows {
        activator,
        recenter,
        gyro_mode,
        dead_zone,
        sensitivity,
        exponent,
        invert,
    }
}

fn add_control_row<W: IsA<gtk4::Widget>>(
    container: &adw::ExpanderRow,
    title: &str,
    control: &W,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    let display_title = match title {
        "Source" => crate::tr!("Source"),
        "Output" => crate::tr!("Output"),
        "Activation" => crate::tr!("Activation"),
        "Activator" => crate::tr!("Activator"),
        "Recenter" => crate::tr!("Recenter"),
        "Gyro output" => crate::tr!("Gyro output"),
        "Dead zone" => crate::tr!("Dead zone"),
        "Sensitivity" => crate::tr!("Sensitivity"),
        "Exponent" => crate::tr!("Exponent"),
        "Invert" => crate::tr!("Invert"),
        _ => title.to_string(),
    };
    row.set_title(&display_title);
    control.add_css_class(CSS_BINDING_SUFFIX);
    control.set_valign(gtk4::Align::Center);
    row.add_suffix(control);
    container.add_row(&row);
    row
}

fn add_section_header(container: &adw::ExpanderRow, title: &str) {
    let row = adw::PreferencesRow::new();
    row.set_selectable(false);
    row.set_activatable(false);
    row.add_css_class(CSS_BINDING_SECTION_HEADER);
    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let before = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    before.set_hexpand(true);
    let label = gtk4::Label::new(Some(&section_title_label(title)));
    let after = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    after.set_hexpand(true);
    content.append(&before);
    content.append(&label);
    content.append(&after);
    row.set_child(Some(&content));
    container.add_row(&row);
}

fn configure_control_visibility(rows: &BindingRows, source: InputSource, output: &OutputAction) {
    let analog = is_analog_source(source);
    for row in [
        &rows.dead_zone,
        &rows.sensitivity,
        &rows.exponent,
        &rows.invert,
    ] {
        row.set_visible(analog);
    }
    rows.recenter
        .set_visible(matches!(source, InputSource::Gyro(_)));
    rows.gyro_mode
        .set_visible(uses_gyro_stick_output(source, output));
}

fn connect_activation_changes(
    activation: &gtk4::DropDown,
    activator: &gtk4::DropDown,
    activator_row: &adw::ActionRow,
    chord: &adw::EntryRow,
) {
    update_activation_controls(activation, activator, activator_row, chord);
    let activator = activator.clone();
    let activator_row = activator_row.clone();
    let chord = chord.clone();
    activation.connect_selected_notify(move |row| {
        update_activation_controls(row, &activator, &activator_row, &chord);
    });
}

fn combo_row(labels: &[String], selected: u32) -> gtk4::DropDown {
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let row = gtk4::DropDown::new(
        Some(gtk4::StringList::new(&refs)),
        None::<&gtk4::Expression>,
    );
    row.set_selected(selected);
    row
}
