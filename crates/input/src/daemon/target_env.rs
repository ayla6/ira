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

pub(crate) fn inject_flatpak_target_env(
    program: &str,
    args: &mut Vec<String>,
    backend: VirtualGamepadBackend,
    vendor: Option<u16>,
    product: Option<u16>,
) {
    inject_flatpak_env(program, args, "SDL_JOYSTICK_HIDAPI", "0");
    if let Some(mapping) = sdl_mapping_for_backend(backend) {
        inject_flatpak_env(program, args, "SDL_GAMECONTROLLERCONFIG", &mapping);
    }
    if let (Some(vendor), Some(product)) = (vendor, product) {
        if let Some(ignored_device) = ignored_device_for_target(vendor, product, backend) {
            inject_flatpak_env(
                program,
                args,
                "SDL_GAMECONTROLLER_IGNORE_DEVICES",
                &ignored_device,
            );
        }
    }
}

pub(crate) fn sdl_mapping_for_backend(backend: VirtualGamepadBackend) -> Option<String> {
    match backend {
        VirtualGamepadBackend::XInput => None,
        VirtualGamepadBackend::DirectInput => Some(VirtualGamepad::direct_input_sdl_mapping()),
        VirtualGamepadBackend::SwitchPro => Some(VirtualGamepad::switch_pro_sdl_mapping()),
        VirtualGamepadBackend::DualShock4 => Some(VirtualGamepad::dual_shock_4_sdl_mapping()),
        VirtualGamepadBackend::DualSense => Some(VirtualGamepad::dual_sense_sdl_mapping()),
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
        let mut args = vec!["run".to_string(), "com.example.Game".to_string()];
        inject_flatpak_target_env(
            "/usr/bin/flatpak",
            &mut args,
            VirtualGamepadBackend::SwitchPro,
            Some(SWITCH_PRO_VENDOR),
            Some(SWITCH_PRO_PRODUCT),
        );

        assert!(args.contains(&"--env=SDL_JOYSTICK_HIDAPI=0".to_string()));
        assert!(args.iter().any(|argument| {
            argument.starts_with("--env=SDL_GAMECONTROLLERCONFIG=030000007e0500000920000011810000,")
        }));
        assert!(!args
            .iter()
            .any(|argument| argument.starts_with("--env=SDL_GAMECONTROLLER_IGNORE_DEVICES=")));
    }

    #[test]
    fn test_sdl_mapping_is_configured_for_switch_pro_backend() {
        assert!(sdl_mapping_for_backend(VirtualGamepadBackend::SwitchPro)
            .unwrap()
            .starts_with("030000007e0500000920000011810000,"));
        assert!(sdl_mapping_for_backend(VirtualGamepadBackend::XInput).is_none());
    }
}