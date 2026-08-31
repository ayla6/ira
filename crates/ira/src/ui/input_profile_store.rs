use ira_input::{GamepadButton, InputProfile, InputSource};
use std::path::{Path, PathBuf};

pub(super) const PROFILE_DIRECTORY: &str = "controller_profiles";
pub(super) const CONTROLLER_DEFAULT_DIRECTORY: &str = "controller_defaults";

#[derive(Clone)]
pub(super) struct StoredProfile {
    pub path: PathBuf,
    pub profile: InputProfile,
}

pub(super) fn profile_directory(save_dir: &str) -> PathBuf {
    Path::new(save_dir).join(PROFILE_DIRECTORY)
}

pub(super) fn controller_default_path(save_dir: &str, key: &str) -> PathBuf {
    Path::new(save_dir)
        .join(CONTROLLER_DEFAULT_DIRECTORY)
        .join(format!("{key}.json"))
}

/// Default layouts written while the virtual backend was still chosen on the
/// device instead of on the layout. Kept so existing files keep resolving;
/// new defaults are a single file per device whose layout owns the backend.
const LEGACY_DEFAULT_SUFFIXES: [&str; 5] =
    ["-directinput", "-switch-pro", "-dualshock4", "-dualsense", "-dsu"];

/// First default layout that already exists for this device, legacy
/// backend-keyed files included. Used when the stored path is missing.
pub(super) fn find_controller_default_profile(save_dir: &str, key: &str) -> Option<PathBuf> {
    let directory = Path::new(save_dir).join(CONTROLLER_DEFAULT_DIRECTORY);
    std::iter::once(String::new())
        .chain(LEGACY_DEFAULT_SUFFIXES.iter().map(|suffix| suffix.to_string()))
        .map(|suffix| directory.join(format!("{key}{suffix}.json")))
        .find(|path| path.is_file())
}

pub(super) fn ensure_controller_default_profile(
    save_dir: &str,
    key: &str,
    name: &str,
    supported_buttons: &[GamepadButton],
) -> Result<PathBuf, String> {
    let path = controller_default_path(save_dir, key);
    if !path.is_file() {
        let mut profile = InputProfile::default_gamepad_for_buttons(supported_buttons);
        profile.name = name.to_string();
        write_profile(&path, &profile)?;
    } else {
        let mut profile = read_profile_data(&path)?;
        let original = profile.clone();
        prune_unsupported_controls(&mut profile, supported_buttons);
        profile.validate()?;
        if profile != original {
            write_profile(&path, &profile)?;
        }
    }
    Ok(path)
}

/// Drop controls the connected device does not report and outputs the virtual
/// backend cannot carry.
fn prune_unsupported_controls(profile: &mut InputProfile, supported_buttons: &[GamepadButton]) {
    let backend = profile.backend;
    for set in &mut profile.action_sets {
        set.inputs.retain(|input| {
            let source_supported = match input.source {
                InputSource::Button(button) => supported_buttons.contains(&button),
                _ => true,
            };
            input.activators.iter().all(|activator| {
                activator
                    .outputs
                    .iter()
                    .all(|output| output.is_supported_by(backend))
            }) && source_supported
        });
    }
}

pub(super) fn list_profiles(save_dir: &str) -> Result<Vec<StoredProfile>, String> {
    let mut profiles = Vec::new();
    for directory in [
        profile_directory(save_dir),
        Path::new(save_dir).join(CONTROLLER_DEFAULT_DIRECTORY),
    ] {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Could not read controller profiles: {error}")),
        };
        for entry in entries {
            let entry = entry
                .map_err(|error| format!("Could not read controller profile entry: {error}"))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json")
                || path
                    .file_stem()
                    .is_some_and(|stem| stem.to_string_lossy().starts_with("resolved-"))
            {
                continue;
            }
            match read_profile(&path) {
                Ok(profile) => profiles.push(StoredProfile { path, profile }),
                Err(error) => eprintln!("Skipping invalid controller profile {:?}: {error}", path),
            }
        }
    }
    profiles.sort_by_cached_key(|profile| profile.profile.name.to_lowercase());
    Ok(profiles)
}

pub(super) fn read_profile(path: &Path) -> Result<InputProfile, String> {
    let profile = read_profile_data(path)?;
    profile.validate()?;
    Ok(profile)
}

fn read_profile_data(path: &Path) -> Result<InputProfile, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read controller profile: {error}"))?;
    InputProfile::from_json(&text)
}

pub(super) fn write_profile(path: &Path, profile: &InputProfile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create controller profile folder: {error}"))?;
    }
    let text = serde_json::to_string_pretty(profile)
        .map_err(|error| format!("Could not encode controller profile: {error}"))?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|error| format!("Could not save controller profile: {error}"))
}

pub(super) fn add_game_compatibility(path: &Path, game_id: i64) -> Result<(), String> {
    let mut profile = read_profile(path)?;
    if profile.compatible_game_ids.is_empty() || profile.compatible_game_ids.contains(&game_id) {
        return Ok(());
    }
    profile.compatible_game_ids.push(game_id);
    write_profile(path, &profile)
}

pub(super) fn profile_matches_game(profile: &InputProfile, game_id: i64) -> bool {
    profile.compatible_game_ids.is_empty() || profile.compatible_game_ids.contains(&game_id)
}

/// A profile scoped to platforms shows only on those platforms; unscoped
/// profiles stay global. PC games carry an empty platform id, so they see
/// exactly the global pool.
pub(super) fn profile_matches_platform(profile: &InputProfile, platform_id: &str) -> bool {
    profile.compatible_platform_ids.is_empty()
        || profile
            .compatible_platform_ids
            .iter()
            .any(|candidate| candidate == platform_id)
}

pub(super) fn delete_profile(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|error| format!("Could not delete controller profile: {error}"))
}

pub(super) fn managed_profile_path(save_dir: &str, name: &str) -> PathBuf {
    profile_directory(save_dir).join(format!("{}.json", profile_slug(name)))
}

pub(super) fn new_managed_profile_path(save_dir: &str, name: &str) -> PathBuf {
    let base = managed_profile_path(save_dir, name);
    if !base.exists() {
        return base;
    }
    let stem = base
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("controller-profile");
    (2..)
        .map(|index| profile_directory(save_dir).join(format!("{stem}-{index}.json")))
        .find(|path| !path.exists())
        .expect("unbounded profile path search must find a free path")
}

fn profile_slug(name: &str) -> String {
    let mut slug = String::new();
    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "controller-profile".to_string()
    } else {
        slug.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        add_game_compatibility, ensure_controller_default_profile, find_controller_default_profile,
        managed_profile_path, new_managed_profile_path, profile_matches_game,
    };
    use ira_input::{
        ActionSet, ControllerCalibration, GamepadButton, InputMapping, InputProfile, InputSource,
        OutputAction,
    };

    #[test]
    fn test_new_managed_profile_path_uses_unique_name() {
        let tmp = tempfile::tempdir().unwrap();
        let first = managed_profile_path(tmp.path().to_str().unwrap(), "");
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::write(&first, "{}").unwrap();
        let next = new_managed_profile_path(tmp.path().to_str().unwrap(), "");
        assert_eq!(next.file_name().unwrap(), "controller-profile-2.json");
    }

    #[test]
    fn test_profile_matches_specific_game() {
        let profile = InputProfile {
            compatible_game_ids: vec![42],
            ..InputProfile::default()
        };
        assert!(profile_matches_game(&profile, 42));
        assert!(!profile_matches_game(&profile, 43));
    }

    #[test]
    fn test_add_game_compatibility_persists_game_id() {
        let tmp = tempfile::tempdir().unwrap();
        let path = managed_profile_path(tmp.path().to_str().unwrap(), "Test");
        let profile = InputProfile {
            name: "Test".to_string(),
            ..InputProfile::default()
        };
        super::write_profile(&path, &profile).unwrap();
        add_game_compatibility(&path, 42).unwrap();
        assert!(profile_matches_game(
            &super::read_profile(&path).unwrap(),
            42
        ));
    }

    #[test]
    fn test_controller_default_removes_unreported_paddles() {
        let tmp = tempfile::tempdir().unwrap();
        let path = super::controller_default_path(tmp.path().to_str().unwrap(), "pad");
        let profile = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: (1..=8)
                    .map(|number| {
                        let button = [
                            GamepadButton::Paddle1,
                            GamepadButton::Paddle2,
                            GamepadButton::Paddle3,
                            GamepadButton::Paddle4,
                            GamepadButton::Paddle5,
                            GamepadButton::Paddle6,
                            GamepadButton::Paddle7,
                            GamepadButton::Paddle8,
                        ][number - 1];
                        let output = if number <= 4 {
                            GamepadButton::A
                        } else {
                            button
                        };
                        InputMapping::simple(
                            InputSource::Button(button),
                            OutputAction::GamepadButton(output),
                        )
                    })
                    .collect(),
            }],
            ..InputProfile::default()
        };
        super::write_profile(&path, &profile).unwrap();

        ensure_controller_default_profile(
            tmp.path().to_str().unwrap(),
            "pad",
            "Pad",
            &[
                GamepadButton::Paddle1,
                GamepadButton::Paddle2,
                GamepadButton::Paddle3,
                GamepadButton::Paddle4,
            ],
        )
        .unwrap();

        let saved = super::read_profile(&path).unwrap();
        // The four reported paddles survive, the unreported ones do not.
        let inputs = &saved.action_sets[0].inputs;
        assert_eq!(inputs.len(), 4);
        assert!(inputs.iter().all(|input| {
            input.activators.iter().all(|activator| {
                activator.outputs.iter().all(|output| match output {
                    OutputAction::GamepadButton(button) => button.is_xinput(),
                    _ => true,
                })
            })
        }));
        assert!(!inputs.iter().any(|input| {
            matches!(
                input.source,
                InputSource::Button(
                    GamepadButton::Paddle5
                        | GamepadButton::Paddle6
                        | GamepadButton::Paddle7
                        | GamepadButton::Paddle8
                )
            )
        }));
    }

    #[test]
    fn test_controller_default_preserves_keyboard_output() {
        let tmp = tempfile::tempdir().unwrap();
        let path = super::controller_default_path(tmp.path().to_str().unwrap(), "pad");
        super::write_profile(
            &path,
            &InputProfile {
                action_sets: vec![ActionSet {
                    name: "Default".to_string(),
                    inputs: vec![InputMapping::simple(
                        InputSource::Button(GamepadButton::A),
                        OutputAction::Keyboard { keycode: 57 },
                    )],
                }],
                ..InputProfile::default()
            },
        )
        .unwrap();

        ensure_controller_default_profile(
            tmp.path().to_str().unwrap(),
            "pad",
            "Pad",
            &[GamepadButton::A],
        )
        .unwrap();

        let saved = super::read_profile(&path).unwrap();
        assert_eq!(saved.action_sets.len(), 1);
        assert_eq!(
            saved.action_sets[0].inputs[0].activators[0].outputs,
            vec![OutputAction::Keyboard { keycode: 57 }]
        );
    }

    #[test]
    fn test_controller_default_preserves_gyro_calibration() {
        let tmp = tempfile::tempdir().unwrap();
        let calibration = ControllerCalibration {
            x: 0.1,
            y: 0.2,
            z: 0.3,
            stick_deadzone_left: 0.0,
            stick_deadzone_right: 0.0,
            nintendo_layout: false,
        };
        let path = super::controller_default_path(tmp.path().to_str().unwrap(), "pad");
        super::write_profile(
            &path,
            &InputProfile {
                controller_calibration: calibration,
                ..InputProfile::default()
            },
        )
        .unwrap();
        ensure_controller_default_profile(tmp.path().to_str().unwrap(), "pad", "Pad", &[]).unwrap();
        assert_eq!(
            super::read_profile(&path).unwrap().controller_calibration,
            calibration
        );
    }

    #[test]
    fn test_find_controller_default_profile_falls_back_to_legacy_files() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        // Nothing on disk yet.
        assert_eq!(find_controller_default_profile(save_dir, "pad"), None);
        // A legacy backend-keyed default still resolves.
        let legacy = super::Path::new(save_dir)
            .join(super::CONTROLLER_DEFAULT_DIRECTORY)
            .join("pad-directinput.json");
        super::write_profile(&legacy, &InputProfile::default()).unwrap();
        assert_eq!(
            find_controller_default_profile(save_dir, "pad"),
            Some(legacy.clone())
        );
        // The plain per-device default wins once it exists.
        let plain = super::controller_default_path(save_dir, "pad");
        super::write_profile(&plain, &InputProfile::default()).unwrap();
        assert_eq!(find_controller_default_profile(save_dir, "pad"), Some(plain));
    }
}
