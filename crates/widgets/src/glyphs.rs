//! Controller glyph mapping shared by the main app and overlay.
//!
//! The existing controller SVGs are used as-is. Controls without an asset
//! deliberately return no icon so each caller can show a readable text badge.
use gtk4::prelude::*;
use ira_input::{ControllerFamily, GamepadAxis, GamepadButton, InputSource};

#[cfg(test)]
const GLYPH_KEYS: &[&str] = &[
    "bumper-l",
    "bumper-r",
    "button-a",
    "button-b",
    "button-circle",
    "button-cross",
    "button-square",
    "button-triangle",
    "button-x",
    "button-y",
    "click-l",
    "click-r",
    "dpad-down",
    "dpad-left",
    "dpad-right",
    "dpad-up",
    "home",
    "minus",
    "plus",
    "stick-l",
    "stick-r",
    "trigger-l",
    "trigger-r",
];

/// The icon name a glyph key resolves under.
pub fn icon_name(key: &str) -> String {
    format!("ira-{key}-symbolic")
}

/// Set a source icon. Returns false when the source has no SVG and the caller
/// should show its text fallback.
pub fn set_source_asset(
    image: &gtk4::Image,
    source: InputSource,
    family: ControllerFamily,
) -> bool {
    let Some(key) = source_asset_name(source, family) else {
        image.clear();
        image.set_visible(false);
        return false;
    };
    image.set_icon_name(Some(&icon_name(key)));
    image.set_visible(true);
    true
}

/// The bundled glyph key for a source, when an SVG currently exists.
pub fn source_asset_name(source: InputSource, family: ControllerFamily) -> Option<&'static str> {
    match source {
        InputSource::Button(button) => match button {
            GamepadButton::A | GamepadButton::B | GamepadButton::X | GamepadButton::Y => {
                face_button_asset(button, family)
            }
            GamepadButton::LeftShoulder => Some("bumper-l"),
            GamepadButton::RightShoulder => Some("bumper-r"),
            GamepadButton::LeftTrigger => Some("trigger-l"),
            GamepadButton::RightTrigger => Some("trigger-r"),
            GamepadButton::Back => Some("minus"),
            GamepadButton::Start => Some("plus"),
            GamepadButton::Guide => Some("home"),
            GamepadButton::LeftStick => Some("click-l"),
            GamepadButton::RightStick => Some("click-r"),
            GamepadButton::DpadUp => Some("dpad-up"),
            GamepadButton::DpadDown => Some("dpad-down"),
            GamepadButton::DpadLeft => Some("dpad-left"),
            GamepadButton::DpadRight => Some("dpad-right"),
            GamepadButton::Paddle1
            | GamepadButton::Paddle2
            | GamepadButton::Paddle3
            | GamepadButton::Paddle4
            | GamepadButton::Paddle5
            | GamepadButton::Paddle6
            | GamepadButton::Paddle7
            | GamepadButton::Paddle8 => None,
        },
        InputSource::AxisDirection { axis, .. } => {
            source_asset_name(InputSource::Axis(axis), family)
        }
        InputSource::Axis(axis) => match axis {
            GamepadAxis::LeftX | GamepadAxis::LeftY => Some("stick-l"),
            GamepadAxis::RightX | GamepadAxis::RightY => Some("stick-r"),
            GamepadAxis::LeftTrigger => Some("trigger-l"),
            GamepadAxis::RightTrigger => Some("trigger-r"),
        },
    }
}

fn face_button_asset(button: GamepadButton, family: ControllerFamily) -> Option<&'static str> {
    if family == ControllerFamily::PlayStation {
        return Some(match button {
            GamepadButton::A => "button-cross",
            GamepadButton::B => "button-circle",
            GamepadButton::X => "button-square",
            GamepadButton::Y => "button-triangle",
            _ => unreachable!("face_button_asset called for a non-face button"),
        });
    }
    Some(match button {
        GamepadButton::A => "button-a",
        GamepadButton::B => "button-b",
        GamepadButton::X => "button-x",
        GamepadButton::Y => "button-y",
        _ => unreachable!("face_button_asset called for a non-face button"),
    })
}

/// Text shown when a controller source has no SVG yet.
pub fn source_badge(source: InputSource, family: ControllerFamily) -> String {
    match source {
        InputSource::Button(button) => match button {
            GamepadButton::A => "A",
            GamepadButton::B => "B",
            GamepadButton::X => "X",
            GamepadButton::Y => "Y",
            GamepadButton::LeftShoulder => "LB",
            GamepadButton::RightShoulder => "RB",
            GamepadButton::LeftTrigger => "LT",
            GamepadButton::RightTrigger => "RT",
            GamepadButton::Back => "-",
            GamepadButton::Start => "+",
            GamepadButton::Guide => "Home",
            GamepadButton::LeftStick => "L3",
            GamepadButton::RightStick => "R3",
            GamepadButton::DpadUp => "D-Up",
            GamepadButton::DpadDown => "D-Down",
            GamepadButton::DpadLeft => "D-Left",
            GamepadButton::DpadRight => "D-Right",
            GamepadButton::Paddle1 if family == ControllerFamily::EightBitDo => "R4",
            GamepadButton::Paddle2 if family == ControllerFamily::EightBitDo => "L4",
            GamepadButton::Paddle3 if family == ControllerFamily::EightBitDo => "PR",
            GamepadButton::Paddle4 if family == ControllerFamily::EightBitDo => "PL",
            GamepadButton::Paddle1 => "P1",
            GamepadButton::Paddle2 => "P2",
            GamepadButton::Paddle3 => "P3",
            GamepadButton::Paddle4 => "P4",
            GamepadButton::Paddle5 => "P5",
            GamepadButton::Paddle6 => "P6",
            GamepadButton::Paddle7 => "P7",
            GamepadButton::Paddle8 => "P8",
        }
        .to_string(),
        InputSource::Axis(axis) => match axis {
            GamepadAxis::LeftX => "LX",
            GamepadAxis::LeftY => "LY",
            GamepadAxis::RightX => "RX",
            GamepadAxis::RightY => "RY",
            GamepadAxis::LeftTrigger => "LT",
            GamepadAxis::RightTrigger => "RT",
        }
        .to_string(),
        InputSource::AxisDirection { axis, direction } => {
            let sign = match direction {
                ira_input::AxisDirection::Negative => "-",
                ira_input::AxisDirection::Positive => "+",
            };
            format!("{}{sign}", source_badge(InputSource::Axis(axis), family))
        }
    }
}

// TODO: Hey, let's come back to the universal glyph set later. Until the
// missing SVGs are designed, text badges are the intentional fallback.

#[cfg(test)]
mod tests {
    use super::{GLYPH_KEYS, icon_name, source_asset_name, source_badge};
    use ira_input::{AxisDirection, ControllerFamily, GamepadAxis, GamepadButton, InputSource};

    #[test]
    fn test_playstation_uses_playstation_glyphs() {
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::A),
                ControllerFamily::PlayStation
            ),
            Some("button-cross")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::B),
                ControllerFamily::PlayStation
            ),
            Some("button-circle")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::X),
                ControllerFamily::PlayStation
            ),
            Some("button-square")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Y),
                ControllerFamily::PlayStation
            ),
            Some("button-triangle")
        );
    }

    #[test]
    fn test_available_glyphs_use_shared_keys() {
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::A),
                ControllerFamily::Generic
            ),
            Some("button-a")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Guide),
                ControllerFamily::Generic
            ),
            Some("home")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::LeftStick),
                ControllerFamily::Generic
            ),
            Some("click-l")
        );
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::RightTrigger),
                ControllerFamily::Generic
            ),
            Some("trigger-r")
        );
        for family in [
            ControllerFamily::Generic,
            ControllerFamily::Xbox,
            ControllerFamily::PlayStation,
            ControllerFamily::Nintendo,
            ControllerFamily::EightBitDo,
            ControllerFamily::Steam,
        ] {
            assert_eq!(
                source_asset_name(InputSource::Button(GamepadButton::Back), family),
                Some("minus")
            );
            assert_eq!(
                source_asset_name(InputSource::Button(GamepadButton::Start), family),
                Some("plus")
            );
            assert_eq!(
                source_asset_name(InputSource::Button(GamepadButton::Guide), family),
                Some("home")
            );
        }
        assert_eq!(icon_name("button-a"), "ira-button-a-symbolic");
    }

    #[test]
    fn test_missing_glyphs_use_text_badges() {
        assert_eq!(
            source_asset_name(
                InputSource::Button(GamepadButton::Paddle1),
                ControllerFamily::Generic
            ),
            None
        );
        assert_eq!(
            source_badge(
                InputSource::Button(GamepadButton::Back),
                ControllerFamily::Xbox
            ),
            "-"
        );
        assert_eq!(
            source_badge(
                InputSource::Button(GamepadButton::Start),
                ControllerFamily::Xbox
            ),
            "+"
        );
        assert_eq!(
            source_badge(
                InputSource::Button(GamepadButton::Guide),
                ControllerFamily::PlayStation
            ),
            "Home"
        );
        assert_eq!(
            source_badge(
                InputSource::Button(GamepadButton::Paddle1),
                ControllerFamily::EightBitDo
            ),
            "R4"
        );
        assert_eq!(
            source_badge(
                InputSource::Button(GamepadButton::Paddle2),
                ControllerFamily::EightBitDo
            ),
            "L4"
        );
        assert_eq!(
            source_badge(
                InputSource::Button(GamepadButton::Paddle3),
                ControllerFamily::EightBitDo
            ),
            "PR"
        );
        assert_eq!(
            source_badge(
                InputSource::Button(GamepadButton::Paddle4),
                ControllerFamily::EightBitDo
            ),
            "PL"
        );
        assert_eq!(
            source_badge(
                InputSource::AxisDirection {
                    axis: GamepadAxis::RightTrigger,
                    direction: AxisDirection::Positive,
                },
                ControllerFamily::Generic
            ),
            "RT+"
        );
    }

    #[test]
    fn test_gresources_ship_available_glyphs() {
        const IRA_GRESOURCE: &str = include_str!("../../ira/resources/ira.gresource.xml");
        const OVERLAY_GRESOURCE: &str =
            include_str!("../../overlay-ui/resources/ira-overlay.gresource.xml");
        for key in GLYPH_KEYS {
            let alias = format!("icons/scalable/actions/ira-{key}-symbolic.svg");
            assert!(
                IRA_GRESOURCE.contains(&alias),
                "missing {key} from ira resources"
            );
            assert!(
                OVERLAY_GRESOURCE.contains(&alias),
                "missing {key} from overlay resources"
            );
        }
    }

    #[test]
    fn test_custom_glyphs_use_shared_symbolic_structure() {
        let glyphs = [
            include_str!("../resources/glyphs/bumper-l.svg"),
            include_str!("../resources/glyphs/bumper-r.svg"),
            include_str!("../resources/glyphs/button-home.svg"),
            include_str!("../resources/glyphs/button-minus.svg"),
            include_str!("../resources/glyphs/button-plus.svg"),
            include_str!("../resources/glyphs/stick-l-click.svg"),
            include_str!("../resources/glyphs/stick-r-click.svg"),
            include_str!("../resources/glyphs/trigger-l.svg"),
            include_str!("../resources/glyphs/trigger-r.svg"),
        ];
        for svg in glyphs {
            assert!(svg.contains("gpa:rendering=\"symbolic\""));
            assert!(svg.contains("xmlns:cc="));
            assert!(svg.contains("xmlns:dc="));
            assert!(svg.contains("xmlns:rdf="));
            assert!(svg.contains("stroke-opacity=\"1\""));
            assert!(svg.contains("stroke-miterlimit=\"4\""));
            assert!(svg.contains("stroke-dasharray=\"none\""));
            assert!(svg.contains("gpa:stroke=\"foreground\""));
            assert!(!svg.contains("viewBox="));
            assert!(!svg.contains("id=\"Layer_1\""));
        }
    }
}
