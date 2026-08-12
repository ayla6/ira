use super::css::{
    CSS_BINDING_SECTION_HEADER, CSS_BINDING_SUFFIX, CSS_FLAT, CSS_SOURCE_BADGE, CSS_SQUARE_BUTTON,
};
use super::input_profile_options::{
    activation_index, activation_labels, activator_index, gyro_mode_index, gyro_mode_labels,
    output_action, output_index, output_labels, recenter_index, source_options_for_device,
};
use adw::prelude::*;
use ira_input::{
    Activation, AxisDirection, AxisTransform, Binding, ChordMode, ControllerFamily, DeviceInfo,
    GamepadAxis, GamepadButton, GyroAxis, GyroMode, InputCategory, InputSource, OutputAction,
    RecenterMode,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(super) struct BindingRow {
    pub container: adw::ExpanderRow,
    pub source_options: Vec<(InputSource, String)>,
    pub activator_options: Vec<(InputSource, String)>,
    pub source: gtk4::DropDown,
    pub output: gtk4::DropDown,
    pub activation: gtk4::DropDown,
    pub activator: gtk4::DropDown,
    pub chord: adw::EntryRow,
    pub recenter: gtk4::DropDown,
    pub gyro_mode: gtk4::DropDown,
    pub dead_zone: gtk4::SpinButton,
    pub sensitivity: gtk4::SpinButton,
    pub exponent: gtk4::SpinButton,
    pub invert: gtk4::CheckButton,
}

pub(super) type SectionGroups = Rc<RefCell<Vec<Vec<(String, adw::PreferencesGroup)>>>>;

pub(super) fn binding_page_index(binding: &Binding) -> usize {
    match binding.source.category() {
        InputCategory::Buttons => 0,
        InputCategory::Dpad => 1,
        InputCategory::Triggers => 2,
        InputCategory::Joysticks => 3,
        InputCategory::Gyro => 4,
    }
}

pub(super) fn binding_section_title(binding: &Binding) -> &'static str {
    match binding.source {
        InputSource::Button(GamepadButton::LeftTrigger | GamepadButton::RightTrigger) => "Triggers",
        InputSource::Button(
            GamepadButton::A | GamepadButton::B | GamepadButton::X | GamepadButton::Y,
        ) => "Face Buttons",
        InputSource::Button(GamepadButton::LeftShoulder | GamepadButton::RightShoulder) => {
            "Bumpers"
        }
        InputSource::Button(GamepadButton::Back | GamepadButton::Start | GamepadButton::Guide) => {
            "Menu Buttons"
        }
        InputSource::Button(GamepadButton::LeftStick | GamepadButton::RightStick) => "Stick Clicks",
        InputSource::Button(
            GamepadButton::DpadUp
            | GamepadButton::DpadDown
            | GamepadButton::DpadLeft
            | GamepadButton::DpadRight,
        ) => "D-pad",
        InputSource::Button(
            GamepadButton::Paddle1
            | GamepadButton::Paddle2
            | GamepadButton::Paddle3
            | GamepadButton::Paddle4
            | GamepadButton::Paddle5
            | GamepadButton::Paddle6
            | GamepadButton::Paddle7
            | GamepadButton::Paddle8,
        ) => "Extended Buttons",
        InputSource::Axis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
        | InputSource::AxisDirection {
            axis: GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger,
            ..
        } => "Triggers",
        InputSource::Axis(GamepadAxis::LeftX | GamepadAxis::LeftY)
        | InputSource::AxisDirection {
            axis: GamepadAxis::LeftX | GamepadAxis::LeftY,
            ..
        } => "Left Stick",
        InputSource::Axis(GamepadAxis::RightX | GamepadAxis::RightY)
        | InputSource::AxisDirection {
            axis: GamepadAxis::RightX | GamepadAxis::RightY,
            ..
        } => "Right Stick",
        InputSource::Gyro(_) => "Gyro",
    }
}

pub(super) fn add_binding_row(
    group: &adw::PreferencesGroup,
    page: &gtk4::Box,
    section_groups: &SectionGroups,
    rows: &Rc<RefCell<Vec<BindingRow>>>,
    binding: Binding,
    device: Option<&DeviceInfo>,
    on_dirty: &Rc<dyn Fn()>,
) {
    let page_index = binding_page_index(&binding);
    let row = make_binding_row(binding, page_index, device);
    connect_dirty(&row, on_dirty);
    let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove.add_css_class(CSS_FLAT);
    remove.add_css_class(CSS_SQUARE_BUTTON);
    remove.set_valign(gtk4::Align::Center);
    remove.set_tooltip_text(Some("Remove binding"));
    let container = row.container.clone();
    let group_for_remove = group.clone();
    let page_for_remove = page.clone();
    let section_groups_for_remove = section_groups.clone();
    let rows_for_remove = rows.clone();
    let on_dirty_for_remove = on_dirty.clone();
    let page_index_for_remove = page_index;
    remove.connect_clicked(move |_| {
        group_for_remove.remove(&container);
        rows_for_remove
            .borrow_mut()
            .retain(|candidate| candidate.container != container);
        if group_for_remove.first_child().is_none() {
            page_for_remove.remove(&group_for_remove);
            if let Some(sections) = section_groups_for_remove
                .borrow_mut()
                .get_mut(page_index_for_remove)
            {
                sections.retain(|(_, candidate)| candidate != &group_for_remove);
            }
            if !page_for_remove
                .first_child()
                .is_some_and(|child| child.is::<adw::PreferencesGroup>())
            {
                add_empty_page_state(&page_for_remove);
            }
        }
        on_dirty_for_remove();
    });
    row.container.add_suffix(&remove);
    group.add(&row.container);
    rows.borrow_mut().push(row);
}

fn connect_dirty(row: &BindingRow, on_dirty: &Rc<dyn Fn()>) {
    row.source.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.output.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.activation.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.activator.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.chord.connect_changed({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.recenter.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.gyro_mode.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    for spin in [&row.dead_zone, &row.sensitivity, &row.exponent] {
        spin.connect_value_changed({
            let on_dirty = on_dirty.clone();
            move |_| on_dirty()
        });
    }
    row.invert.connect_toggled({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
}

pub(super) fn add_empty_page_state(page: &gtk4::Box) {
    let empty = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    empty.set_widget_name("input-empty-state");
    empty.set_halign(gtk4::Align::Center);
    empty.set_valign(gtk4::Align::Start);
    empty.set_margin_top(36);
    let icon = gtk4::Image::from_icon_name("input-gaming-symbolic");
    let label = gtk4::Label::new(Some("No bindings"));
    label.add_css_class("dim-label");
    empty.append(&icon);
    empty.append(&label);
    page.append(&empty);
}

fn make_binding_row(
    binding: Binding,
    page_index: usize,
    device: Option<&DeviceInfo>,
) -> BindingRow {
    let page_source_options = source_options_for_page(page_index, device, Some(binding.source));
    let binding_source_labels = page_source_options
        .iter()
        .map(|(_, label)| label.clone())
        .collect::<Vec<_>>();
    let source = combo_row(
        &binding_source_labels,
        source_index_for(&page_source_options, binding.source),
    );
    let output = combo_row(&output_labels(), output_index(&binding.output));
    let activation = combo_row(&activation_labels(), activation_index(&binding.activation));
    let mut activator_options = source_options_for_device(device);
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
    let activator_labels = activator_options
        .iter()
        .map(|(_, label)| label.clone())
        .collect::<Vec<_>>();
    let activator = combo_row(
        &activator_labels,
        activator_index(&binding.activation, &activator_options),
    );
    let chord = adw::EntryRow::new();
    chord.set_title("Chord sources");
    chord.set_text(&chord_text_for_options(
        &binding.activation,
        &activator_options,
    ));
    chord.set_tooltip_text(Some("Comma-separated input sources"));
    let recenter = combo_row(
        &[
            "Never".to_string(),
            "On enable".to_string(),
            "On disable".to_string(),
            "On enable and disable".to_string(),
        ],
        recenter_index(binding.recenter),
    );
    let gyro_mode = combo_row(&gyro_mode_labels(), gyro_mode_index(binding.gyro_mode));
    let dead_zone = gtk4::SpinButton::with_range(0.0, 0.99, 0.01);
    dead_zone.set_digits(2);
    dead_zone.set_value(binding.transform.dead_zone as f64);
    let sensitivity = gtk4::SpinButton::with_range(0.0, 1000.0, 0.1);
    sensitivity.set_digits(2);
    sensitivity.set_value(binding.transform.sensitivity as f64);
    let exponent = gtk4::SpinButton::with_range(0.1, 5.0, 0.1);
    exponent.set_digits(2);
    exponent.set_value(binding.transform.exponent as f64);
    let invert = gtk4::CheckButton::with_label("Invert");
    invert.set_active(binding.transform.invert);

    let container = adw::ExpanderRow::new();
    update_binding_summary(&source, &output, &container);

    let family = device.map(DeviceInfo::family).unwrap_or_default();
    let current_source = Rc::new(Cell::new(binding.source));
    let fallback = gtk4::Label::new(Some(&source_badge(binding.source, family)));
    fallback.add_css_class(CSS_SOURCE_BADGE);
    fallback.set_valign(gtk4::Align::Center);
    let asset = gtk4::Image::new();
    asset.set_pixel_size(24);
    set_source_asset(&asset, &fallback, binding.source, family);
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

    add_control_row(&container, "Source", &source);
    add_control_row(&container, "Output", &output);
    add_section_header(&container, "Behavior");
    add_control_row(&container, "Activation", &activation);
    let activator_row = add_control_row(&container, "Activator", &activator);
    container.add_row(&chord);
    let recenter_row = add_control_row(&container, "Recenter", &recenter);
    let gyro_mode_row = add_control_row(&container, "Gyro output", &gyro_mode);
    add_section_header(&container, "Response");
    let dead_zone_row = add_control_row(&container, "Dead zone", &dead_zone);
    let sensitivity_row = add_control_row(&container, "Sensitivity", &sensitivity);
    let exponent_row = add_control_row(&container, "Exponent", &exponent);
    let invert_row = add_control_row(&container, "Invert", &invert);

    let analog_source = is_analog_source(binding.source);
    dead_zone_row.set_visible(analog_source);
    sensitivity_row.set_visible(analog_source);
    exponent_row.set_visible(analog_source);
    invert_row.set_visible(analog_source);
    recenter_row.set_visible(matches!(binding.source, InputSource::Gyro(_)));
    gyro_mode_row.set_visible(uses_gyro_stick_output(binding.source, &binding.output));

    update_activation_controls(&activation, &activator, &activator_row, &chord);
    {
        let activator = activator.clone();
        let activator_row = activator_row.clone();
        let chord = chord.clone();
        activation.connect_selected_notify(move |row| {
            update_activation_controls(row, &activator, &activator_row, &chord);
        });
    }
    let source_options_for_badge = page_source_options.clone();
    let fallback_for_source = fallback.clone();
    let asset_for_source = asset.clone();
    let source_for_asset = current_source.clone();
    let dead_zone_for_source = dead_zone_row.clone();
    let sensitivity_for_source = sensitivity_row.clone();
    let exponent_for_source = exponent_row.clone();
    let invert_for_source = invert_row.clone();
    let recenter_for_source = recenter_row.clone();
    let gyro_mode_for_source = gyro_mode_row.clone();
    let output_for_source = output.clone();
    let row_for_source = container.clone();
    source.connect_selected_notify(move |source| {
        update_binding_summary(source, &output_for_source, &row_for_source);
        if let Some((source, _)) = source_options_for_badge.get(source.selected() as usize) {
            source_for_asset.set(*source);
            fallback_for_source.set_text(&source_badge(*source, family));
            set_source_asset(&asset_for_source, &fallback_for_source, *source, family);
            let analog = is_analog_source(*source);
            dead_zone_for_source.set_visible(analog);
            sensitivity_for_source.set_visible(analog);
            exponent_for_source.set_visible(analog);
            invert_for_source.set_visible(analog);
            recenter_for_source.set_visible(matches!(*source, InputSource::Gyro(_)));
            let action = output_action(output_for_source.selected())
                .unwrap_or(OutputAction::GamepadAxis(GamepadAxis::LeftX));
            gyro_mode_for_source.set_visible(uses_gyro_stick_output(*source, &action));
        }
    });
    let source_for_output = source.clone();
    let source_options_for_output = page_source_options.clone();
    let gyro_mode_for_output = gyro_mode_row.clone();
    let row_for_output = container.clone();
    output.connect_selected_notify(move |output| {
        update_binding_summary(&source_for_output, output, &row_for_output);
        let action = output_action(output.selected())
            .unwrap_or(OutputAction::GamepadAxis(GamepadAxis::LeftX));
        if let Some((source, _)) =
            source_options_for_output.get(source_for_output.selected() as usize)
        {
            gyro_mode_for_output.set_visible(uses_gyro_stick_output(*source, &action));
        }
    });
    BindingRow {
        container,
        source_options: page_source_options,
        activator_options,
        source,
        output,
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

fn add_control_row<W: IsA<gtk4::Widget>>(
    container: &adw::ExpanderRow,
    title: &str,
    control: &W,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(title);
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
    let label = gtk4::Label::new(Some(title));
    let after = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    after.set_hexpand(true);
    content.append(&before);
    content.append(&label);
    content.append(&after);
    row.set_child(Some(&content));
    container.add_row(&row);
}

fn is_analog_source(source: InputSource) -> bool {
    matches!(
        source,
        InputSource::Axis(_) | InputSource::AxisDirection { .. } | InputSource::Gyro(_)
    )
}

fn uses_gyro_stick_output(source: InputSource, output: &OutputAction) -> bool {
    matches!(source, InputSource::Gyro(_)) && matches!(output, OutputAction::GamepadAxis(_))
}

fn set_source_asset(
    image: &gtk4::Image,
    fallback: &gtk4::Label,
    source: InputSource,
    family: ControllerFamily,
) {
    let Some(asset_name) = source_asset_name(source, family) else {
        image.clear();
        image.set_visible(false);
        fallback.set_visible(true);
        return;
    };
    let dark = !gtk4::Settings::default()
        .is_some_and(|settings| settings.is_gtk_application_prefer_dark_theme());
    if let Some(path) = steam_asset_path(asset_name, dark) {
        image.set_from_file(Some(&path));
        image.set_visible(true);
        fallback.set_visible(false);
    } else {
        image.clear();
        image.set_visible(false);
        fallback.set_visible(true);
    }
}

fn source_asset_name(source: InputSource, family: ControllerFamily) -> Option<&'static str> {
    match source {
        InputSource::Button(button) => match button {
            GamepadButton::A => Some("shared_buttons_s.svg"),
            GamepadButton::B => Some("shared_buttons_e.svg"),
            GamepadButton::X => Some("shared_buttons_w.svg"),
            GamepadButton::Y => Some("shared_buttons_n.svg"),
            GamepadButton::LeftShoulder => family_asset(family, "lb"),
            GamepadButton::RightShoulder => family_asset(family, "rb"),
            GamepadButton::LeftTrigger => family_asset(family, "lt"),
            GamepadButton::RightTrigger => family_asset(family, "rt"),
            GamepadButton::Back => family_asset(family, "back"),
            GamepadButton::Start => family_asset(family, "start"),
            GamepadButton::Guide => family_asset(family, "guide"),
            GamepadButton::LeftStick => Some("shared_lstick_click.svg"),
            GamepadButton::RightStick => Some("shared_rstick_click.svg"),
            GamepadButton::DpadUp => Some("shared_dpad_up.svg"),
            GamepadButton::DpadDown => Some("shared_dpad_down.svg"),
            GamepadButton::DpadLeft => Some("shared_dpad_left.svg"),
            GamepadButton::DpadRight => Some("shared_dpad_right.svg"),
            GamepadButton::Paddle1
            | GamepadButton::Paddle2
            | GamepadButton::Paddle3
            | GamepadButton::Paddle4
            | GamepadButton::Paddle5
            | GamepadButton::Paddle6
            | GamepadButton::Paddle7
            | GamepadButton::Paddle8 => paddle_asset(family, button),
        },
        InputSource::AxisDirection { axis, .. } => {
            source_asset_name(InputSource::Axis(axis), family)
        }
        InputSource::Axis(axis) => match axis {
            GamepadAxis::LeftX | GamepadAxis::LeftY => Some("shared_lstick.svg"),
            GamepadAxis::RightX | GamepadAxis::RightY => Some("shared_rstick.svg"),
            GamepadAxis::LeftTrigger => family_asset(family, "lt"),
            GamepadAxis::RightTrigger => family_asset(family, "rt"),
        },
        InputSource::Gyro(axis) => match axis {
            GyroAxis::X => Some("shared_gyro_pitch.svg"),
            GyroAxis::Y => Some("shared_gyro_yaw.svg"),
            GyroAxis::Z => Some("shared_gyro_roll.svg"),
        },
    }
}

fn paddle_asset(family: ControllerFamily, button: GamepadButton) -> Option<&'static str> {
    let number = match button {
        GamepadButton::Paddle1 => "1",
        GamepadButton::Paddle2 => "2",
        GamepadButton::Paddle3 => "3",
        GamepadButton::Paddle4 => "4",
        GamepadButton::Paddle5 => "5",
        GamepadButton::Paddle6 => "6",
        GamepadButton::Paddle7 => "7",
        GamepadButton::Paddle8 => "8",
        _ => return None,
    };
    if family == ControllerFamily::EightBitDo {
        return match number {
            "1" => Some("sc_r4.svg"),
            "2" => Some("sc_l4.svg"),
            "3" => Some("shared_pr.svg"),
            "4" => Some("shared_pl.svg"),
            _ => Some(universal_paddle_asset(number)),
        };
    }
    Some(universal_paddle_asset(number))
}

fn universal_paddle_asset(number: &str) -> &'static str {
    match number {
        "1" => "shared_m1.svg",
        "2" => "shared_m2.svg",
        "3" => "shared_m3.svg",
        "4" => "shared_m4.svg",
        "5" => "shared_m5.svg",
        "6" => "shared_m6.svg",
        "7" => "shared_m7.svg",
        _ => "shared_m8.svg",
    }
}

fn family_asset(family: ControllerFamily, control: &str) -> Option<&'static str> {
    match family {
        ControllerFamily::PlayStation => match control {
            "lb" => Some("ps_l1.svg"),
            "rb" => Some("ps_r1.svg"),
            "lt" => Some("ps_l2.svg"),
            "rt" => Some("ps_r2.svg"),
            "back" => Some("ps4_button_share.svg"),
            "start" => Some("ps4_button_options.svg"),
            "guide" => Some("ps4_button_logo.svg"),
            "dpad_up" => Some("ps_dpad_up.svg"),
            "dpad_down" => Some("ps_dpad_down.svg"),
            "dpad_left" => Some("ps_dpad_left.svg"),
            "dpad_right" => Some("ps_dpad_right.svg"),
            _ => None,
        },
        ControllerFamily::Nintendo => match control {
            "lb" => Some("switchpro_l.svg"),
            "rb" => Some("switchpro_r.svg"),
            "lt" => Some("switchpro_l2.svg"),
            "rt" => Some("switchpro_r2.svg"),
            "back" => Some("switchpro_button_minus.svg"),
            "start" => Some("switchpro_button_plus.svg"),
            "guide" => Some("switchpro_button_home.svg"),
            "dpad_up" => Some("switchpro_dpad_up.svg"),
            "dpad_down" => Some("switchpro_dpad_down.svg"),
            "dpad_left" => Some("switchpro_dpad_left.svg"),
            "dpad_right" => Some("switchpro_dpad_right.svg"),
            _ => None,
        },
        ControllerFamily::EightBitDo => match control {
            "back" => Some("switchpro_button_minus.svg"),
            "start" => Some("switchpro_button_plus.svg"),
            "guide" => Some("8bitdo_button_home.svg"),
            _ => family_asset(ControllerFamily::Generic, control),
        },
        ControllerFamily::Steam => match control {
            "guide" => Some("sc_button_steam.svg"),
            _ => family_asset(ControllerFamily::Generic, control),
        },
        ControllerFamily::Xbox | ControllerFamily::Generic => match control {
            "lb" => Some("xbox_lb.svg"),
            "rb" => Some("xbox_rb.svg"),
            "lt" => Some("xbox_lt.svg"),
            "rt" => Some("xbox_rt.svg"),
            "back" => Some("xbox_button_select.svg"),
            "start" => Some("xbox_button_start.svg"),
            "guide" => Some("xbox_button_logo.svg"),
            "dpad_up" => Some("shared_dpad_up.svg"),
            "dpad_down" => Some("shared_dpad_down.svg"),
            "dpad_left" => Some("shared_dpad_left.svg"),
            "dpad_right" => Some("shared_dpad_right.svg"),
            _ => None,
        },
    }
}

fn steam_asset_path(name: &str, dark: bool) -> Option<std::path::PathBuf> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share"))
        })?;
    let path = data_home
        .join(format!(
            "Steam/controller_base/images/api/{}",
            if dark { "dark" } else { "light" }
        ))
        .join(name);
    path.is_file().then_some(path)
}

fn source_badge(source: InputSource, family: ControllerFamily) -> String {
    match source {
        InputSource::Button(button) => match button {
            GamepadButton::A => "A".to_string(),
            GamepadButton::B => "B".to_string(),
            GamepadButton::X => "X".to_string(),
            GamepadButton::Y => "Y".to_string(),
            GamepadButton::LeftShoulder => "LB".to_string(),
            GamepadButton::RightShoulder => "RB".to_string(),
            GamepadButton::LeftTrigger => "LT".to_string(),
            GamepadButton::RightTrigger => "RT".to_string(),
            GamepadButton::Back if family == ControllerFamily::EightBitDo => "-".to_string(),
            GamepadButton::Start if family == ControllerFamily::EightBitDo => "+".to_string(),
            GamepadButton::Guide if family == ControllerFamily::EightBitDo => "Home".to_string(),
            GamepadButton::Back => "Back".to_string(),
            GamepadButton::Start => "Start".to_string(),
            GamepadButton::Guide => "Guide".to_string(),
            GamepadButton::LeftStick => "L3".to_string(),
            GamepadButton::RightStick => "R3".to_string(),
            GamepadButton::DpadUp => "D-Up".to_string(),
            GamepadButton::DpadDown => "D-Down".to_string(),
            GamepadButton::DpadLeft => "D-Left".to_string(),
            GamepadButton::DpadRight => "D-Right".to_string(),
            GamepadButton::Paddle1 if family == ControllerFamily::EightBitDo => "R4".to_string(),
            GamepadButton::Paddle2 if family == ControllerFamily::EightBitDo => "L4".to_string(),
            GamepadButton::Paddle3 if family == ControllerFamily::EightBitDo => "PR".to_string(),
            GamepadButton::Paddle4 if family == ControllerFamily::EightBitDo => "PL".to_string(),
            GamepadButton::Paddle1 => "P1".to_string(),
            GamepadButton::Paddle2 => "P2".to_string(),
            GamepadButton::Paddle3 => "P3".to_string(),
            GamepadButton::Paddle4 => "P4".to_string(),
            GamepadButton::Paddle5 => "P5".to_string(),
            GamepadButton::Paddle6 => "P6".to_string(),
            GamepadButton::Paddle7 => "P7".to_string(),
            GamepadButton::Paddle8 => "P8".to_string(),
        },
        InputSource::Axis(axis) => match axis {
            GamepadAxis::LeftX => "LX".to_string(),
            GamepadAxis::LeftY => "LY".to_string(),
            GamepadAxis::RightX => "RX".to_string(),
            GamepadAxis::RightY => "RY".to_string(),
            GamepadAxis::LeftTrigger => "LT".to_string(),
            GamepadAxis::RightTrigger => "RT".to_string(),
        },
        InputSource::AxisDirection { axis, direction } => {
            let sign = match direction {
                AxisDirection::Negative => "-",
                AxisDirection::Positive => "+",
            };
            format!("{}{sign}", source_badge(InputSource::Axis(axis), family))
        }
        InputSource::Gyro(axis) => format!("G-{}", gyro_axis_label(axis)),
    }
}

fn gyro_axis_label(axis: GyroAxis) -> &'static str {
    match axis {
        GyroAxis::X => "X (Pitch)",
        GyroAxis::Y => "Y (Yaw)",
        GyroAxis::Z => "Z (Roll)",
    }
}

fn update_binding_summary(
    source: &gtk4::DropDown,
    output: &gtk4::DropDown,
    row: &adw::ExpanderRow,
) {
    let source_text = source
        .model()
        .and_then(|model| model.item(source.selected()))
        .and_then(|item| item.downcast::<gtk4::StringObject>().ok())
        .map(|item| item.string().to_string())
        .unwrap_or_else(|| "Input".to_string());
    let output_text = output
        .model()
        .and_then(|model| model.item(output.selected()))
        .and_then(|item| item.downcast::<gtk4::StringObject>().ok())
        .map(|item| item.string().to_string())
        .unwrap_or_else(|| "Output".to_string());
    row.set_title(&source_text);
    if output_text == source_text {
        row.set_subtitle("");
    } else {
        row.set_subtitle(&format!("→ {output_text}"));
    }
}

pub(super) fn binding_from_row(row: &BindingRow) -> Result<Binding, String> {
    let source = row
        .source_options
        .get(row.source.selected() as usize)
        .map(|(source, _)| *source)
        .ok_or_else(|| "Invalid binding source".to_string())?;
    let output = output_action(row.output.selected())?;
    let activation = activation_from_row(row)?;
    Ok(Binding {
        source,
        output,
        gyro_mode: match row.gyro_mode.selected() {
            1 => GyroMode::HoldLast,
            _ => GyroMode::Rate,
        },
        activation,
        transform: AxisTransform {
            dead_zone: row.dead_zone.value() as f32,
            sensitivity: row.sensitivity.value() as f32,
            exponent: row.exponent.value() as f32,
            invert: row.invert.is_active(),
        },
        recenter: match row.recenter.selected() {
            1 => RecenterMode::OnEnable,
            2 => RecenterMode::OnDisable,
            3 => RecenterMode::OnEnableOrDisable,
            _ => RecenterMode::Never,
        },
    })
}

fn source_options_for_page(
    page_index: usize,
    device: Option<&DeviceInfo>,
    current_source: Option<InputSource>,
) -> Vec<(InputSource, String)> {
    let category = match page_index {
        0 => InputCategory::Buttons,
        1 => InputCategory::Dpad,
        2 => InputCategory::Triggers,
        3 => InputCategory::Joysticks,
        4 => InputCategory::Gyro,
        _ => return Vec::new(),
    };
    let mut options = source_options_for_device(device)
        .into_iter()
        .filter(|(source, _)| source.category() == category)
        .collect::<Vec<_>>();
    if let Some(source) =
        current_source.filter(|source| !options.iter().any(|(candidate, _)| candidate == source))
    {
        options.push((
            source,
            format!(
                "{} (unavailable)",
                source_badge(source, ControllerFamily::default())
            ),
        ));
    }
    options
}

fn activation_sources(activation: &Activation) -> Vec<InputSource> {
    match activation {
        Activation::Hold(source)
        | Activation::Toggle(source)
        | Activation::DisableWhile(source) => vec![*source],
        Activation::Chord { sources, .. } => sources.clone(),
        Activation::Always => Vec::new(),
    }
}

fn chord_text_for_options(activation: &Activation, options: &[(InputSource, String)]) -> String {
    match activation {
        Activation::Chord { sources, .. } => sources
            .iter()
            .filter_map(|source| {
                options
                    .iter()
                    .find(|(candidate, _)| candidate == source)
                    .map(|(_, label)| label.clone())
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

fn source_index_for(options: &[(InputSource, String)], source: InputSource) -> u32 {
    options
        .iter()
        .position(|(candidate, _)| *candidate == source)
        .unwrap_or(0) as u32
}

fn activation_from_row(row: &BindingRow) -> Result<Activation, String> {
    let source = row
        .activator_options
        .get(row.activator.selected() as usize)
        .map(|(source, _)| *source)
        .ok_or_else(|| "Invalid activator source".to_string())?;
    Ok(match row.activation.selected() {
        1 => Activation::Hold(source),
        2 => Activation::Toggle(source),
        3 => Activation::DisableWhile(source),
        4 => Activation::Chord {
            sources: parse_chord(&row.chord.text(), &row.activator_options)?,
            mode: ChordMode::Hold,
        },
        _ => Activation::Always,
    })
}

fn parse_chord(text: &str, options: &[(InputSource, String)]) -> Result<Vec<InputSource>, String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            options
                .iter()
                .find(|(_, label)| label.eq_ignore_ascii_case(part))
                .map(|(source, _)| *source)
                .ok_or_else(|| format!("Unknown chord source: {part}"))
        })
        .collect()
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

fn update_activation_controls(
    activation: &gtk4::DropDown,
    activator: &gtk4::DropDown,
    activator_row: &adw::ActionRow,
    chord: &adw::EntryRow,
) {
    let is_always = activation.selected() == 0;
    let is_chord = activation.selected() == 4;
    let show_activator = !is_always && !is_chord;
    activator.set_sensitive(show_activator);
    activator_row.set_visible(show_activator);
    chord.set_sensitive(is_chord);
    chord.set_visible(is_chord);
}

#[cfg(test)]
mod tests {
    use super::{activation_sources, is_analog_source, source_asset_name, source_badge};
    use ira_input::{
        Activation, AxisDirection, ChordMode, ControllerFamily, GamepadAxis, GamepadButton,
        GyroAxis, InputSource,
    };

    #[test]
    fn test_button_binding_does_not_use_axis_controls() {
        assert!(!is_analog_source(InputSource::Button(GamepadButton::A)));
        assert!(is_analog_source(InputSource::Axis(GamepadAxis::LeftX)));
        assert!(is_analog_source(InputSource::AxisDirection {
            axis: GamepadAxis::RightY,
            direction: AxisDirection::Positive,
        }));
        assert!(is_analog_source(InputSource::Gyro(GyroAxis::X)));
    }

    #[test]
    fn test_activation_sources_preserves_unavailable_chord_inputs() {
        let sources = activation_sources(&Activation::Chord {
            sources: vec![
                InputSource::Button(GamepadButton::Paddle1),
                InputSource::Button(GamepadButton::Guide),
            ],
            mode: ChordMode::Hold,
        });
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0], InputSource::Button(GamepadButton::Paddle1));
    }

    #[test]
    fn test_source_asset_name_uses_shared_standard_controls() {
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::A),
                ControllerFamily::Generic
            ),
            Some("shared_buttons_s.svg")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::A),
                ControllerFamily::PlayStation
            ),
            Some("shared_buttons_s.svg")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::DpadUp),
                ControllerFamily::PlayStation
            ),
            Some("shared_dpad_up.svg")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Back),
                ControllerFamily::EightBitDo,
            ),
            Some("switchpro_button_minus.svg")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Start),
                ControllerFamily::EightBitDo,
            ),
            Some("switchpro_button_plus.svg")
        );
    }

    #[test]
    fn test_paddles_use_controller_specific_steam_icons() {
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Paddle1),
                ControllerFamily::Generic
            ),
            Some("shared_m1.svg")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Paddle4),
                ControllerFamily::Generic
            ),
            Some("shared_m4.svg")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Paddle8),
                ControllerFamily::Generic
            ),
            Some("shared_m8.svg")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Paddle1),
                ControllerFamily::EightBitDo
            ),
            Some("sc_r4.svg")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Paddle2),
                ControllerFamily::EightBitDo
            ),
            Some("sc_l4.svg")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Paddle3),
                ControllerFamily::EightBitDo
            ),
            Some("shared_pr.svg")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Paddle4),
                ControllerFamily::EightBitDo
            ),
            Some("shared_pl.svg")
        );
    }

    #[test]
    fn test_gyro_assets_follow_sdl_axis_semantics() {
        assert_eq!(
            source_asset_name(InputSource::Gyro(GyroAxis::X), ControllerFamily::Generic),
            Some("shared_gyro_pitch.svg")
        );
        assert_eq!(
            source_asset_name(InputSource::Gyro(GyroAxis::Y), ControllerFamily::Generic),
            Some("shared_gyro_yaw.svg")
        );
        assert_eq!(
            source_asset_name(InputSource::Gyro(GyroAxis::Z), ControllerFamily::Generic),
            Some("shared_gyro_roll.svg")
        );
    }

    #[test]
    fn test_gyro_badge_uses_semantic_labels() {
        assert_eq!(
            source_badge(InputSource::Gyro(GyroAxis::X), ControllerFamily::Generic),
            "G-X (Pitch)"
        );
        assert_eq!(
            source_badge(InputSource::Gyro(GyroAxis::Y), ControllerFamily::Generic),
            "G-Y (Yaw)"
        );
        assert_eq!(
            source_badge(InputSource::Gyro(GyroAxis::Z), ControllerFamily::Generic),
            "G-Z (Roll)"
        );
    }
}
