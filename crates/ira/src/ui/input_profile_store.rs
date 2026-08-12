use ira_input::{GamepadButton, InputProfile, InputSource, VirtualGamepadBackend};
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

pub(super) fn controller_default_path_for_backend(
    save_dir: &str,
    key: &str,
    backend: VirtualGamepadBackend,
) -> PathBuf {
    match backend {
        VirtualGamepadBackend::XInput => controller_default_path(save_dir, key),
        VirtualGamepadBackend::DirectInput => Path::new(save_dir)
            .join(CONTROLLER_DEFAULT_DIRECTORY)
            .join(format!("{key}-directinput.json")),
    }
}

pub(super) fn ensure_controller_default_profile(
    save_dir: &str,
    key: &str,
    name: &str,
    supported_buttons: &[GamepadButton],
    backend: VirtualGamepadBackend,
) -> Result<PathBuf, String> {
    let path = controller_default_path_for_backend(save_dir, key, backend);
    if !path.is_file() {
        let mut profile =
            InputProfile::default_gamepad_for_backend_and_buttons(backend, supported_buttons);
        profile.name = name.to_string();
        write_profile(&path, &profile)?;
    } else {
        let mut profile = read_profile_data(&path)?;
        let original = profile.clone();
        profile.bindings.retain(|binding| {
            let source_supported = match binding.source {
                InputSource::Button(button) => supported_buttons.contains(&button),
                _ => true,
            };
            source_supported && binding.output.is_supported_by(profile.backend)
        });
        profile.validate()?;
        if profile != original {
            write_profile(&path, &profile)?;
        }
    }
    Ok(path)
}

pub(super) fn copy_controller_default(
    save_dir: &str,
    source_key: &str,
    target_key: &str,
    target_name: &str,
    supported_buttons: &[GamepadButton],
) -> Result<PathBuf, String> {
    let source_path = controller_default_path(save_dir, source_key);
    let mut profile = read_profile_data(&source_path)?;
    let target_path = controller_default_path_for_backend(save_dir, target_key, profile.backend);

    profile.bindings.retain(|binding| {
        !matches!(
            binding.source,
            InputSource::Button(button) if !supported_buttons.contains(&button)
        )
    });
    profile.name = target_name.to_string();
    profile.validate()?;
    write_profile(&target_path, &profile)?;
    Ok(target_path)
}

pub(super) fn list_profiles(save_dir: &str) -> Result<Vec<StoredProfile>, String> {
    let directory = profile_directory(save_dir);
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read controller profiles: {error}")),
    };
    let mut profiles = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("Could not read controller profile entry: {error}"))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        match read_profile(&path) {
            Ok(profile) => profiles.push(StoredProfile { path, profile }),
            Err(error) => eprintln!("Skipping invalid controller profile {:?}: {error}", path),
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
    serde_json::from_str(&text).map_err(|error| format!("Invalid profile JSON: {error}"))
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
        add_game_compatibility, copy_controller_default, ensure_controller_default_profile,
        managed_profile_path, new_managed_profile_path, profile_matches_game,
    };
    use ira_input::{
        Binding, GamepadButton, GyroCalibration, InputProfile, InputSource, OutputAction,
        VirtualGamepadBackend,
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
            bindings: (1..=8)
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
                    Binding::new(
                        InputSource::Button(button),
                        OutputAction::GamepadButton(output),
                    )
                })
                .collect(),
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
            VirtualGamepadBackend::XInput,
        )
        .unwrap();

        let saved = super::read_profile(&path).unwrap();
        assert_eq!(saved.bindings.len(), 4);
        assert!(saved
            .bindings
            .iter()
            .all(|binding| binding.output.is_xinput_compatible()));
        assert!(!saved.bindings.iter().any(|binding| {
            matches!(
                binding.source,
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
                bindings: vec![Binding::new(
                    InputSource::Button(GamepadButton::A),
                    OutputAction::Keyboard { keycode: 57 },
                )],
                ..InputProfile::default()
            },
        )
        .unwrap();

        ensure_controller_default_profile(
            tmp.path().to_str().unwrap(),
            "pad",
            "Pad",
            &[GamepadButton::A],
            VirtualGamepadBackend::XInput,
        )
        .unwrap();

        let saved = super::read_profile(&path).unwrap();
        assert_eq!(
            saved.bindings[0].output,
            OutputAction::Keyboard { keycode: 57 }
        );
    }

    #[test]
    fn test_controller_default_preserves_gyro_calibration() {
        let tmp = tempfile::tempdir().unwrap();
        let calibration = GyroCalibration {
            x: 0.1,
            y: 0.2,
            z: 0.3,
        };
        let path = super::controller_default_path(tmp.path().to_str().unwrap(), "pad");
        super::write_profile(
            &path,
            &InputProfile {
                gyro_calibration: calibration,
                ..InputProfile::default()
            },
        )
        .unwrap();
        ensure_controller_default_profile(
            tmp.path().to_str().unwrap(),
            "pad",
            "Pad",
            &[],
            VirtualGamepadBackend::XInput,
        )
        .unwrap();
        assert_eq!(
            super::read_profile(&path).unwrap().gyro_calibration,
            calibration
        );
    }

    #[test]
    fn test_copy_controller_default_creates_independent_target_with_filtered_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let source = super::controller_default_path(save_dir, "source");
        let profile = InputProfile {
            name: "Source".to_string(),
            compatible_game_ids: vec![42],
            gyro_calibration: GyroCalibration {
                x: 0.1,
                y: 0.2,
                z: 0.3,
            },
            bindings: vec![
                Binding::new(
                    InputSource::Button(GamepadButton::A),
                    OutputAction::Keyboard { keycode: 30 },
                ),
                Binding::new(
                    InputSource::Button(GamepadButton::Paddle1),
                    OutputAction::GamepadButton(GamepadButton::B),
                ),
            ],
            ..InputProfile::default()
        };
        super::write_profile(&source, &profile).unwrap();

        let target =
            copy_controller_default(save_dir, "source", "target", "Target", &[GamepadButton::A])
                .unwrap();
        let copied = super::read_profile(&target).unwrap();
        assert_eq!(copied.name, "Target");
        assert_eq!(copied.compatible_game_ids, vec![42]);
        assert_eq!(copied.gyro_calibration, profile.gyro_calibration);
        assert_eq!(copied.bindings.len(), 1);
        assert_eq!(
            copied.bindings[0].output,
            OutputAction::Keyboard { keycode: 30 }
        );

        assert_eq!(super::read_profile(&source).unwrap(), profile);
        assert_ne!(source, target);
        assert_ne!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&target).unwrap()
        );
    }

    #[test]
    fn test_copy_controller_default_validates_target_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let source = super::controller_default_path(save_dir, "source");
        let mut profile = InputProfile::default();
        profile.bindings.push(Binding {
            source: InputSource::Button(GamepadButton::A),
            output: OutputAction::GamepadButton(GamepadButton::B),
            transform: ira_input::AxisTransform {
                dead_zone: 1.0,
                ..ira_input::AxisTransform::default()
            },
            ..Binding::new(
                InputSource::Button(GamepadButton::A),
                OutputAction::GamepadButton(GamepadButton::B),
            )
        });
        super::write_profile(&source, &profile).unwrap();

        let error =
            copy_controller_default(save_dir, "source", "target", "Target", &[GamepadButton::A])
                .unwrap_err();
        assert!(error.contains("dead_zone"));
        assert!(!super::controller_default_path(save_dir, "target").exists());
    }

    #[test]
    fn test_copy_controller_default_preserves_direct_input_paddle_output() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let source = super::controller_default_path(save_dir, "source");
        super::write_profile(
            &source,
            &InputProfile {
                backend: VirtualGamepadBackend::DirectInput,
                bindings: vec![Binding::new(
                    InputSource::Button(GamepadButton::A),
                    OutputAction::GamepadButton(GamepadButton::Paddle1),
                )],
                ..InputProfile::default()
            },
        )
        .unwrap();

        let target = copy_controller_default(
            save_dir,
            "source",
            "target",
            "DirectInput target",
            &[GamepadButton::A],
        )
        .unwrap();
        let copied = super::read_profile(&target).unwrap();
        assert_eq!(copied.backend, VirtualGamepadBackend::DirectInput);
        assert_eq!(
            copied.bindings[0].output,
            OutputAction::GamepadButton(GamepadButton::Paddle1)
        );
    }
}
