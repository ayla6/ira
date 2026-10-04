//! Host-side input broker wrapping: the broker stays outside Wine, Proton,
//! umu, gamescope, and Flatpak.

use ira_models::ControllerInputMode;

pub(crate) fn input_binary_path() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let candidates = [
        exe_dir.join("ira-input"),
        exe_dir.join("input").join("ira-input"),
    ];
    if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
        return Some(path.to_string_lossy().into_owned());
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join("ira-input"))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

/// Wraps a final game command with the host-side input broker.
/// The broker stays outside Wine, Proton, umu, gamescope, and Flatpak.
pub fn wrap_with_input(
    command: &mut Vec<String>,
    profile: Option<&str>,
    calibration: Option<&str>,
    pause_unfocused: bool,
) -> Result<(), String> {
    let binary = input_binary_path()
        .ok_or_else(|| "input remapping enabled but ira-input was not found".to_string())?;
    wrap_command_with_input(command, &binary, profile, calibration, pause_unfocused);
    Ok(())
}

/// Wraps the command only when input remapping is explicitly enabled; which
/// virtual controller the game sees is decided by the profile itself. A
/// missing ira-input binary degrades to launching unwrapped — remapping is
/// optional garnish, the game itself must start.
pub fn wrap_with_input_mode(
    command: &mut Vec<String>,
    mode: Option<ControllerInputMode>,
    profile: Option<&str>,
    calibration: Option<&str>,
    pause_unfocused: bool,
) {
    if let Some(ControllerInputMode::Enabled) = mode {
        match input_binary_path() {
            Some(binary) => {
                wrap_command_with_input(command, &binary, profile, calibration, pause_unfocused)
            }
            None => eprintln!(
                "launch: input remapping is enabled but ira-input was not found; \
                 launching without input remapping"
            ),
        }
    }
}

#[cfg(test)]
fn wrap_with_input_mode_for_binary(
    command: &mut Vec<String>,
    mode: Option<ControllerInputMode>,
    profile: Option<&str>,
    calibration: Option<&str>,
    binary: &str,
) {
    match mode {
        Some(ControllerInputMode::Enabled) => {
            wrap_command_with_input(command, binary, profile, calibration, true)
        }
        None | Some(ControllerInputMode::Disabled) => {}
    }
}

fn wrap_command_with_input(
    command: &mut Vec<String>,
    binary: &str,
    profile: Option<&str>,
    calibration: Option<&str>,
    pause_unfocused: bool,
) {
    let game_command = std::mem::take(command);
    let mut wrapped = vec![binary.to_string()];
    if let Some(profile) = profile.filter(|profile| !profile.is_empty()) {
        wrapped.push("--profile".to_string());
        wrapped.push(profile.to_string());
    }
    if let Some(calibration) = calibration.filter(|calibration| !calibration.is_empty()) {
        wrapped.push("--calibration".to_string());
        wrapped.push(calibration.to_string());
    }
    if pause_unfocused {
        wrapped.push("--pause-unfocused".to_string());
    }
    wrapped.push("--".to_string());
    wrapped.extend(game_command);
    *command = wrapped;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command() -> Vec<String> {
        vec!["game".to_string(), "--fullscreen".to_string()]
    }

    #[test]
    fn test_input_mode_disabled_does_not_wrap_command() {
        let mut command = command();
        wrap_with_input_mode_for_binary(
            &mut command,
            Some(ControllerInputMode::Disabled),
            None,
            None,
            "/bin/ira-input",
        );
        assert_eq!(command, vec!["game", "--fullscreen"]);
    }

    #[test]
    fn test_input_mode_none_inherits_without_wrapping_command() {
        let mut command = command();
        wrap_with_input_mode_for_binary(
            &mut command,
            None,
            Some("profile"),
            None,
            "/bin/ira-input",
        );
        assert_eq!(command, vec!["game", "--fullscreen"]);
    }

    #[test]
    fn test_input_mode_enabled_wraps_command_with_profile() {
        let mut command = command();
        wrap_with_input_mode_for_binary(
            &mut command,
            Some(ControllerInputMode::Enabled),
            Some("profile"),
            None,
            "/bin/ira-input",
        );
        assert_eq!(
            command,
            vec![
                "/bin/ira-input",
                "--profile",
                "profile",
                "--pause-unfocused",
                "--",
                "game",
                "--fullscreen"
            ]
        );
    }

    #[test]
    fn test_input_mode_enabled_without_profile_omits_flag() {
        let mut command = command();
        wrap_with_input_mode_for_binary(
            &mut command,
            Some(ControllerInputMode::Enabled),
            None,
            None,
            "/bin/ira-input",
        );
        assert_eq!(
            command,
            vec![
                "/bin/ira-input",
                "--pause-unfocused",
                "--",
                "game",
                "--fullscreen"
            ]
        );
    }
}
