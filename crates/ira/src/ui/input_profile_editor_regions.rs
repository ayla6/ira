//! Region pages for the action-set editor: inputs grouped where they sit on
//! the controller, one row per input with an activator summary, mirroring
//! Steam Input's per-input navigation.

use super::css::{CSS_FLAT, CSS_SQUARE_BUTTON};
use super::helpers::esc;
use super::input_profile_assets::{set_source_asset, source_badge};
use super::input_profile_options::{axis_label, button_label, output_display_label};
use adw::prelude::*;
use ira_input::{GamepadButton, InputSource};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    FaceButtons,
    Dpad,
    TriggersBumpers,
    Sticks,
    SystemPaddles,
}

impl Region {
    pub(crate) const ALL: [Region; 5] = [
        Region::FaceButtons,
        Region::Dpad,
        Region::TriggersBumpers,
        Region::Sticks,
        Region::SystemPaddles,
    ];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Region::FaceButtons => "face",
            Region::Dpad => "dpad",
            Region::TriggersBumpers => "triggers",
            Region::Sticks => "sticks",
            Region::SystemPaddles => "system",
        }
    }

    pub(crate) fn title(self) -> String {
        match self {
            Region::FaceButtons => crate::tr!("Face Buttons"),
            Region::Dpad => crate::tr!("D-pad"),
            Region::TriggersBumpers => crate::tr!("Triggers & Bumpers"),
            Region::Sticks => crate::tr!("Sticks"),
            Region::SystemPaddles => crate::tr!("System & Paddles"),
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Region::FaceButtons => "input-gaming-symbolic",
            Region::Dpad => "view-grid-symbolic",
            Region::TriggersBumpers => "media-seek-forward-symbolic",
            Region::Sticks => "media-playback-start-symbolic",
            Region::SystemPaddles => "emblem-system-symbolic",
        }
    }
}

/// Human label for any bindable source.
pub(crate) fn source_label(source: InputSource) -> String {
    match source {
        InputSource::Button(button) => button_label(button),
        InputSource::Axis(axis) => axis_label(axis),
        InputSource::AxisDirection { axis, direction } => {
            let sign = match direction {
                ira_input::AxisDirection::Negative => "-",
                ira_input::AxisDirection::Positive => "+",
            };
            crate::tr!("{} ({})")
                .replacen("{}", &axis_label(axis), 1)
                .replacen("{}", sign, 1)
        }
    }
}

/// Every button source a device supports, in stable display order.
pub(crate) fn supported_button_sources(device: Option<&ira_input::DeviceInfo>) -> Vec<InputSource> {
    let all = [
        GamepadButton::A,
        GamepadButton::B,
        GamepadButton::X,
        GamepadButton::Y,
        GamepadButton::LeftShoulder,
        GamepadButton::RightShoulder,
        GamepadButton::LeftTrigger,
        GamepadButton::RightTrigger,
        GamepadButton::Back,
        GamepadButton::Start,
        GamepadButton::Guide,
        GamepadButton::LeftStick,
        GamepadButton::RightStick,
        GamepadButton::DpadUp,
        GamepadButton::DpadDown,
        GamepadButton::DpadLeft,
        GamepadButton::DpadRight,
        GamepadButton::Paddle1,
        GamepadButton::Paddle2,
        GamepadButton::Paddle3,
        GamepadButton::Paddle4,
        GamepadButton::Paddle5,
        GamepadButton::Paddle6,
        GamepadButton::Paddle7,
        GamepadButton::Paddle8,
    ];
    all.into_iter()
        .filter(|button| {
            device.is_none_or(|device| {
                device.supported_buttons.contains(button)
                    || (button.is_paddle() && device.supported_buttons.contains(button))
            })
        })
        .map(InputSource::Button)
        .collect()
}

/// One input's summary line: "Click → A · Double press → Space".
pub(crate) fn mapping_summary(mapping: &ira_input::InputMapping) -> String {
    let parts: Vec<String> = mapping
        .activators
        .iter()
        .map(|activator| {
            let outputs: Vec<String> = activator
                .outputs
                .iter()
                .map(output_display_label)
                .collect();
            format!(
                "{} → {}",
                activator_kind_label(&activator.kind),
                outputs.join(", ")
            )
        })
        .collect();
    if parts.is_empty() {
        return crate::tr!("Not mapped");
    }
    parts.join(" · ")
}

pub(crate) fn activator_kind_label(kind: &ira_input::ActivatorKind) -> String {
    match kind {
        ira_input::ActivatorKind::FullPress => crate::tr!("Click"),
        ira_input::ActivatorKind::DoublePress { .. } => crate::tr!("Double press"),
        ira_input::ActivatorKind::LongPress { .. } => crate::tr!("Long press"),
        ira_input::ActivatorKind::StartPress => crate::tr!("On press down"),
        ira_input::ActivatorKind::Release => crate::tr!("On release"),
        ira_input::ActivatorKind::SoftPress { .. } => crate::tr!("Soft pull"),
    }
}

/// Build one input row for a region page.
pub(crate) fn input_row(
    mapping: &ira_input::InputMapping,
    family: ira_input::ControllerFamily,
    on_edit: &Rc<dyn Fn()>,
    on_remove: &Rc<dyn Fn()>,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(&source_label(mapping.source)));
    row.set_subtitle(&mapping_summary(mapping));

    let fallback = gtk4::Label::new(Some(&source_badge(mapping.source, family)));
    fallback.add_css_class(super::css::CSS_SOURCE_BADGE);
    fallback.set_valign(gtk4::Align::Center);
    let asset = gtk4::Image::new();
    asset.set_pixel_size(24);
    set_source_asset(&asset, &fallback, mapping.source, family);
    row.add_prefix(&asset);
    row.add_prefix(&fallback);

    let edit = gtk4::Button::from_icon_name("document-edit-symbolic");
    edit.add_css_class(CSS_FLAT);
    edit.add_css_class(CSS_SQUARE_BUTTON);
    edit.set_valign(gtk4::Align::Center);
    edit.set_tooltip_text(Some(&crate::tr!("Edit")));
    {
        let on_edit = on_edit.clone();
        edit.connect_clicked(move |_| on_edit());
    }
    row.add_suffix(&edit);

    let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove.add_css_class(CSS_FLAT);
    remove.add_css_class(CSS_SQUARE_BUTTON);
    remove.set_valign(gtk4::Align::Center);
    remove.set_tooltip_text(Some(&crate::tr!("Remove binding")));
    {
        let on_remove = on_remove.clone();
        remove.connect_clicked(move |_| on_remove());
    }
    row.add_suffix(&remove);

    row.set_activatable(true);
    {
        let on_edit = on_edit.clone();
        row.connect_activated(move |_| on_edit());
    }
    row
}
