use crate::{VirtualGamepad, VirtualGamepadBackend};

const VIRTUAL_XBOX_VENDOR: u16 = 0x045e;
const VIRTUAL_XBOX_PRODUCT: u16 = 0x028e;
const SWITCH_PRO_VENDOR: u16 = 0x057e;
const SWITCH_PRO_PRODUCT: u16 = 0x2009;
// Same ids the Sony kernel drivers report for the real hardware; the virtual
// pads reuse them so SDL's built-in mappings apply.
const DUAL_SHOCK_4_VENDOR: u16 = 0x054c;
const DUAL_SHOCK_4_PRODUCT: u16 = 0x09cc;
const DUAL_SENSE_VENDOR: u16 = 0x054c;
const DUAL_SENSE_PRODUCT: u16 = 0x0ce6;
// Valve's Steam Input output identity (see virtual_gamepad.rs).
const STEAM_INPUT_VENDOR: u16 = 0x28de;
const STEAM_INPUT_PRODUCT: u16 = 0x11ff;

pub(crate) fn inject_flatpak_env(program: &str, args: &mut Vec<String>, key: &str, value: &str) {
    let program = std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str());
    let is_flatpak = program == Some("flatpak");
    let is_flatpak_spawn = program == Some("flatpak-spawn")
        && args
            .windows(2)
            .any(|window| window == ["--host", "flatpak"]);
    if !is_flatpak && !is_flatpak_spawn {
        return;
    }
    let Some(run_index) = args.iter().position(|argument| argument == "run") else {
        return;
    };
    args.insert(run_index + 1, format!("--env={key}={value}"));
}

/// The SDL environment a spawned game needs for the session's backend.
/// Native-twin backends (Switch Pro, DS4, DualSense) keep hidapi enabled —
/// it is what claims the twin and delivers its motion sensors — and hide
/// the physical pad from every SDL layer. Each twin also ships the exact
/// mapping for the identity its uhid device presents: no SDL database maps
/// those GUIDs, and SDL's HIDAPI auto-mapping is signature-gated, so
/// without it the twins appear as raw joysticks no gamecontroller-based
/// game can open. The remaining backends keep the raw-evdev setup: hidapi
/// off, the backend's mapping, and the physical pad ignored at the
/// gamecontroller layer.
pub(crate) fn target_env_for(
    backend: VirtualGamepadBackend,
    vendor: Option<u16>,
    product: Option<u16>,
    passthrough: bool,
) -> Vec<(String, String)> {
    // A passthrough session leaves the physical pad fully native: no SDL
    // environment tampering at all.
    if passthrough {
        return Vec::new();
    }
    let mut envs = Vec::new();
    let twin_mapping = match backend {
        VirtualGamepadBackend::DualShock4 => Some(crate::hid_ds4::sdl_mapping()),
        VirtualGamepadBackend::SwitchPro => Some(VirtualGamepad::switch_pro_sdl_mapping()),
        VirtualGamepadBackend::DualSense => Some(crate::hid_dualsense::sdl_mapping()),
        VirtualGamepadBackend::XInput
        | VirtualGamepadBackend::DirectInput
        | VirtualGamepadBackend::SteamInput
        | VirtualGamepadBackend::Dsu => None,
    };
    if let Some(mapping) = twin_mapping {
        // SDL2's default treats accelerometer nodes as joysticks, which
        // would list the twin's motion sensor as a second controller with
        // gyro-shaped axes AND stop it from ever being a sensor (SDL keeps
        // a node in one list only). The hint turns it sensor-only.
        envs.push(("SDL_ACCELEROMETER_AS_JOYSTICK".to_string(), "0".to_string()));
        envs.push((
            "SDL_GAMECONTROLLERCONFIG".to_string(),
            mapping,
        ));
        if let (Some(vendor), Some(product)) = (vendor, product) {
            let ignored = format!("0x{vendor:04x}/0x{product:04x}");
            envs.push((
                "SDL_GAMECONTROLLER_IGNORE_DEVICES".to_string(),
                ignored.clone(),
            ));
            envs.push((
                "SDL_JOYSTICK_BLACKLIST_DEVICES".to_string(),
                ignored.clone(),
            ));
            envs.push(("SDL_HIDAPI_IGNORE_DEVICES".to_string(), ignored));
        }
        return envs;
    }
    envs.push(("SDL_JOYSTICK_HIDAPI".to_string(), "0".to_string()));
    // The uinput backends' native motion is an accelerometer-class evdev
    // node; SDL2's default hint lists such nodes as joysticks with
    // gyro-shaped axes instead of the sensors they are.
    envs.push((
        "SDL_ACCELEROMETER_AS_JOYSTICK".to_string(),
        "0".to_string(),
    ));
    if let Some(mapping) = sdl_mapping_for_backend(backend) {
        envs.push(("SDL_GAMECONTROLLERCONFIG".to_string(), mapping));
    }
    if let (Some(vendor), Some(product)) = (vendor, product) {
        if let Some(ignored_device) = ignored_device_for_target(vendor, product, backend) {
            envs.push((
                "SDL_GAMECONTROLLER_IGNORE_DEVICES".to_string(),
                ignored_device.clone(),
            ));
            // The gamecontroller hint only stops the pad from becoming a
            // gamepad; the blacklist removes it from SDL's joystick layer
            // entirely, so games poking raw joysticks never see it either.
            envs.push((
                "SDL_JOYSTICK_BLACKLIST_DEVICES".to_string(),
                ignored_device,
            ));
        }
    }
    envs
}



pub(crate) fn sdl_mapping_for_backend(backend: VirtualGamepadBackend) -> Option<String> {
    match backend {
        VirtualGamepadBackend::XInput => None,
        VirtualGamepadBackend::DirectInput => Some(VirtualGamepad::direct_input_sdl_mapping()),
        VirtualGamepadBackend::SwitchPro => Some(VirtualGamepad::switch_pro_sdl_mapping()),
        VirtualGamepadBackend::DualShock4 => Some(VirtualGamepad::dual_shock_4_sdl_mapping()),
        VirtualGamepadBackend::DualSense => Some(VirtualGamepad::dual_sense_sdl_mapping()),
        // Valve's identity is in no SDL database of its own: the game only
        // maps the pad through this env mapping, the same way Steam itself
        // hands games its controller configuration.
        VirtualGamepadBackend::SteamInput => Some(VirtualGamepad::steam_input_sdl_mapping()),
        // The DSU backend presents no kernel device, so there is nothing to
        // map in SDL; the emulator binds to the cemuhook stream instead.
        VirtualGamepadBackend::Dsu => None,
    }
}

pub(crate) fn ignored_device_for_target(
    vendor: u16,
    product: u16,
    backend: VirtualGamepadBackend,
) -> Option<String> {
    let same_identity = |expected: (u16, u16)| (vendor, product) == expected;
    let virtual_identity = match backend {
        VirtualGamepadBackend::XInput => Some((VIRTUAL_XBOX_VENDOR, VIRTUAL_XBOX_PRODUCT)),
        VirtualGamepadBackend::SwitchPro => Some((SWITCH_PRO_VENDOR, SWITCH_PRO_PRODUCT)),
        VirtualGamepadBackend::DualShock4 => Some((DUAL_SHOCK_4_VENDOR, DUAL_SHOCK_4_PRODUCT)),
        VirtualGamepadBackend::DualSense => Some((DUAL_SENSE_VENDOR, DUAL_SENSE_PRODUCT)),
        VirtualGamepadBackend::SteamInput => Some((STEAM_INPUT_VENDOR, STEAM_INPUT_PRODUCT)),
        // Private identities: the physical pad is hidden so only Ira's
        // carrier shows up in the game.
        VirtualGamepadBackend::DirectInput | VirtualGamepadBackend::Dsu => None,
    };
    match virtual_identity {
        // The physical pad shares the virtual one's identity, so it cannot be
        // hidden without hiding the virtual pad too.
        Some(identity) if same_identity(identity) => None,
        // DirectInput presents a private BUS_VIRTUAL identity; the physical
        // pad is hidden so only Ira's device shows up.
        _ => Some(format!("0x{vendor:04x}/0x{product:04x}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_flatpak_env_places_mapping_after_run() {
        let mut args = vec!["run".to_string(), "net.shadps4.shadPS4".to_string()];
        inject_flatpak_env("/usr/bin/flatpak", &mut args, "KEY", "value");
        assert_eq!(args, ["run", "--env=KEY=value", "net.shadps4.shadPS4"]);

        let mut native = vec!["--fullscreen".to_string()];
        inject_flatpak_env("shadps4", &mut native, "KEY", "value");
        assert_eq!(native, ["--fullscreen"]);

        let mut nested = vec![
            "--host".to_string(),
            "flatpak".to_string(),
            "run".to_string(),
            "net.shadps4.shadPS4".to_string(),
        ];
        inject_flatpak_env("flatpak-spawn", &mut nested, "KEY", "value");
        assert_eq!(
            nested,
            [
                "--host",
                "flatpak",
                "run",
                "--env=KEY=value",
                "net.shadps4.shadPS4"
            ]
        );
    }

    #[test]
    fn test_ignored_device_for_target_preserves_virtual_xbox() {
        assert_eq!(
            ignored_device_for_target(
                VIRTUAL_XBOX_VENDOR,
                VIRTUAL_XBOX_PRODUCT,
                VirtualGamepadBackend::XInput,
            ),
            None
        );
        assert_eq!(
            ignored_device_for_target(0x2dc8, 0x3106, VirtualGamepadBackend::XInput),
            Some("0x2dc8/0x3106".to_string())
        );
    }

    #[test]
    fn test_ignored_device_for_target_preserves_switch_pro_identity() {
        assert_eq!(
            ignored_device_for_target(
                SWITCH_PRO_VENDOR,
                SWITCH_PRO_PRODUCT,
                VirtualGamepadBackend::SwitchPro,
            ),
            None
        );
    }

    #[test]
    fn test_ignored_device_for_target_preserves_sony_identity() {
        assert_eq!(
            ignored_device_for_target(
                DUAL_SHOCK_4_VENDOR,
                DUAL_SHOCK_4_PRODUCT,
                VirtualGamepadBackend::DualShock4,
            ),
            None
        );
        assert_eq!(
            ignored_device_for_target(
                DUAL_SENSE_VENDOR,
                DUAL_SENSE_PRODUCT,
                VirtualGamepadBackend::DualSense,
            ),
            None
        );
    }

    #[test]
    fn test_sdl_mapping_is_configured_for_sony_backends() {
        assert!(sdl_mapping_for_backend(VirtualGamepadBackend::DualShock4)
            .unwrap()
            .starts_with("030000004c050000cc09000000010000"));
        assert!(sdl_mapping_for_backend(VirtualGamepadBackend::DualSense)
            .unwrap()
            .starts_with("030000004c050000e60c000011010000"));
    }

    #[test]
    fn test_inject_flatpak_target_env_configures_switch_pro_isolation() {
        // The Switch Pro twin keeps hidapi enabled (it is what claims the
        // twin and delivers its sensors) while the physical pad is hidden
        // from every SDL layer.
        let args = {
            let mut args = vec!["run".to_string(), "com.example.Game".to_string()];
            for (key, value) in
                target_env_for(VirtualGamepadBackend::SwitchPro, Some(0x057e), Some(0x2009), false)
            {
                inject_flatpak_env("/usr/bin/flatpak", &mut args, &key, &value);
            }
            args
        };

        assert!(args.contains(&"--env=SDL_GAMECONTROLLER_IGNORE_DEVICES=0x057e/0x2009".to_string()));
        assert!(args.contains(&"--env=SDL_JOYSTICK_BLACKLIST_DEVICES=0x057e/0x2009".to_string()));
        assert!(args.contains(&"--env=SDL_HIDAPI_IGNORE_DEVICES=0x057e/0x2009".to_string()));
        assert!(!args
            .iter()
            .any(|argument| argument.starts_with("--env=SDL_JOYSTICK_HIDAPI=")));
        // The twin ships the mapping for the identity its uhid device
        // presents; no SDL database carries it.
        assert!(args.iter().any(|argument| {
            argument.starts_with("--env=SDL_GAMECONTROLLERCONFIG=030000007e0500000920000011810000,")
        }));
    }

    #[test]
    fn test_twin_env_maps_the_sony_twins_by_their_presented_identity() {
        for (backend, guid_prefix) in [
            (
                VirtualGamepadBackend::DualShock4,
                "030000000d0f0000ee00000000000000,Wireless Controller",
            ),
            (
                VirtualGamepadBackend::DualSense,
                "030000000d0f00006301000000000000,DualSense Wireless Controller",
            ),
        ] {
            let envs = target_env_for(backend, Some(0x2dc8), Some(0x3106), false);
            let mapping = envs
                .iter()
                .find(|(key, _)| key == "SDL_GAMECONTROLLERCONFIG")
                .map(|(_, value)| value)
                .unwrap_or_else(|| panic!("{backend:?} twin env carries no mapping"));
            assert!(mapping.starts_with(guid_prefix));
            // The physical pad is hidden while the sensor hint keeps the
            // paired IMU off the joystick lists.
            assert!(envs.contains(&(
                "SDL_GAMECONTROLLER_IGNORE_DEVICES".to_string(),
                "0x2dc8/0x3106".to_string()
            )));
            assert!(envs.contains(&(
                "SDL_ACCELEROMETER_AS_JOYSTICK".to_string(),
                "0".to_string()
            )));
        }
    }

    #[test]
    fn test_target_env_keeps_the_motion_node_off_joystick_lists() {
        // The uinput backends' native motion rides an accelerometer-class
        // evdev node; the game's SDL must classify it as a sensor, not as
        // a second controller with gyro-shaped axes.
        let envs = target_env_for(VirtualGamepadBackend::XInput, Some(0x2dc8), Some(0x3106), false);
        assert!(envs.contains(&(
            "SDL_ACCELEROMETER_AS_JOYSTICK".to_string(),
            "0".to_string()
        )));
        // The physical pad is hidden from both SDL layers: off the gamepad
        // list and off the joystick list entirely.
        assert!(envs.contains(&(
            "SDL_GAMECONTROLLER_IGNORE_DEVICES".to_string(),
            "0x2dc8/0x3106".to_string()
        )));
        assert!(envs.contains(&(
            "SDL_JOYSTICK_BLACKLIST_DEVICES".to_string(),
            "0x2dc8/0x3106".to_string()
        )));
    }

    #[test]
    fn test_target_env_hides_hyphenated_identity_from_the_joystick_layer() {
        // An 8BitDo speaking the virtual pad's wire protocol is virtualized
        // as XInput; nothing may leak it into the game's SDL.
        let envs = target_env_for(VirtualGamepadBackend::XInput, Some(0x2dc8), Some(0x6012), false);
        assert!(envs.contains(&(
            "SDL_JOYSTICK_BLACKLIST_DEVICES".to_string(),
            "0x2dc8/0x6012".to_string()
        )));
        assert!(envs.contains(&(
            "SDL_JOYSTICK_HIDAPI".to_string(),
            "0".to_string()
        )));
    }

    #[test]
    fn test_passthrough_target_env_stays_empty() {
        let envs =
            target_env_for(VirtualGamepadBackend::SwitchPro, Some(0x057e), Some(0x2009), true);
        assert!(envs.is_empty());
    }

    #[test]
    fn test_steam_input_backend_maps_and_hides_the_physical_pad() {
        // The Steam Input identity exists in no SDL database, so the game
        // maps the pad through the env mapping; the physical pad is hidden
        // because it never shares Valve's identity.
        assert!(sdl_mapping_for_backend(VirtualGamepadBackend::SteamInput)
            .unwrap()
            .starts_with("03000000de280000ff11000000010000,Steam Virtual Gamepad,"));
        assert_eq!(
            ignored_device_for_target(0x2dc8, 0x3106, VirtualGamepadBackend::SteamInput),
            Some("0x2dc8/0x3106".to_string())
        );
        let envs = target_env_for(VirtualGamepadBackend::SteamInput, Some(0x2dc8), Some(0x3106), false);
        assert!(envs.contains(&(
            "SDL_ACCELEROMETER_AS_JOYSTICK".to_string(),
            "0".to_string()
        )));
        assert!(envs.iter().any(
            |(key, value)| key == "SDL_GAMECONTROLLERCONFIG"
                && value.starts_with("03000000de280000ff11000000010000")
        ));
    }

    #[test]
    fn test_sdl_mapping_is_configured_for_switch_pro_backend() {
        assert!(sdl_mapping_for_backend(VirtualGamepadBackend::SwitchPro)
            .unwrap()
            .starts_with("030000007e0500000920000011810000,"));
        assert!(sdl_mapping_for_backend(VirtualGamepadBackend::XInput).is_none());
    }
}