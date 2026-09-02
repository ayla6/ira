use adw::prelude::*;
use ira_input::{ControllerFamily, GamepadAxis, GamepadButton, InputSource};

/// Loads a Steam glyph into `image`, reporting whether one was found. The
/// caller shows its text fallback only when this returns false.
pub(crate) fn set_asset_from_name(image: &gtk4::Image, asset_name: Option<&str>) -> bool {
    let Some(asset_name) = asset_name else {
        image.clear();
        image.set_visible(false);
        return false;
    };
    let dark = !gtk4::Settings::default()
        .is_some_and(|settings| settings.is_gtk_application_prefer_dark_theme());
    if let Some(path) = steam_asset_path(asset_name, dark) {
        image.set_from_file(Some(&path));
        image.set_visible(true);
        true
    } else {
        image.clear();
        image.set_visible(false);
        false
    }
}

pub(crate) fn set_source_asset(
    image: &gtk4::Image,
    fallback: &gtk4::Label,
    source: InputSource,
    family: ControllerFamily,
) {
    if set_asset_from_name(image, source_asset_name(source, family)) {
        fallback.set_visible(false);
    } else {
        fallback.set_visible(true);
    }
}

pub(super) fn source_asset_name(
    source: InputSource,
    family: ControllerFamily,
) -> Option<&'static str> {
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

pub(crate) fn source_badge(source: InputSource, family: ControllerFamily) -> String {
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
                ira_input::AxisDirection::Negative => "-",
                ira_input::AxisDirection::Positive => "+",
            };
            format!("{}{sign}", source_badge(InputSource::Axis(axis), family))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::source_asset_name;
    use ira_input::{ControllerFamily, GamepadButton, InputSource};

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
        for (button, asset) in [
            (GamepadButton::Paddle1, "shared_m1.svg"),
            (GamepadButton::Paddle4, "shared_m4.svg"),
            (GamepadButton::Paddle8, "shared_m8.svg"),
        ] {
            assert_eq!(
                source_asset_name(InputSource::Button(button), ControllerFamily::Generic),
                Some(asset)
            );
        }
        for (button, asset) in [
            (GamepadButton::Paddle1, "sc_r4.svg"),
            (GamepadButton::Paddle2, "sc_l4.svg"),
            (GamepadButton::Paddle3, "shared_pr.svg"),
            (GamepadButton::Paddle4, "shared_pl.svg"),
        ] {
            assert_eq!(
                source_asset_name(InputSource::Button(button), ControllerFamily::EightBitDo),
                Some(asset)
            );
        }
    }
}
