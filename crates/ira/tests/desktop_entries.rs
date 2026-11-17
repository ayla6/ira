use ira::desktop::integration;
use ira_models::Game;

#[test]
fn test_install_registers_search_separately_from_game_shortcuts() {
    let root = tempfile::tempdir().unwrap();
    let executable = std::path::Path::new("/home/My Apps/Ira.AppImage");
    integration::install(
        root.path(),
        &root.path().join("config"),
        executable,
        "ira.desktop",
    )
    .unwrap();
    let provider = root
        .path()
        .join("gnome-shell/search-providers/com.github.ira.ini");
    assert!(provider.is_file());
    let service = std::fs::read_to_string(
        root.path()
            .join("dbus-1/services/com.github.ira.SearchProvider.service"),
    )
    .unwrap();
    assert!(service.contains("\"/home/My Apps/Ira.AppImage\" --search-provider"));
    assert!(!root.path().join("applications").exists());
    assert!(std::fs::read_to_string(provider)
        .unwrap()
        .contains("DesktopId=ira.desktop\n"));
    integration::install(
        root.path(),
        &root.path().join("config"),
        executable,
        "ira.desktop",
    )
    .unwrap();
}

#[test]
fn test_create_and_remove_shortcut_preserve_unrelated_files() {
    let root = tempfile::tempdir().unwrap();
    let game = Game {
        db_id: 42,
        name: "A game\nExec=bad".into(),
        variant_id: Some(3),
        ..Default::default()
    };
    let executable = std::path::Path::new("/home/My Apps/Ira %f.AppImage");
    assert!(!integration::shortcut_exists(root.path(), 42, Some(3)).unwrap());
    integration::create_shortcut(root.path(), executable, &game).unwrap();
    assert!(integration::shortcut_exists(root.path(), 42, Some(3)).unwrap());
    assert!(!integration::shortcut_exists(root.path(), 42, None).unwrap());
    let path = root
        .path()
        .join("applications/com.github.ira.game-42-variant-3.desktop");
    let contents = std::fs::read_to_string(&path).unwrap();
    let info = glib::KeyFile::new();
    info.load_from_file(&path, glib::KeyFileFlags::NONE)
        .unwrap();
    assert_eq!(
        info.string("Desktop Entry", "Name").unwrap(),
        "A game\nExec=bad"
    );
    assert!(contents.contains("--launch-game 42 --variant 3"));
    assert!(!info
        .keys("Desktop Entry")
        .unwrap()
        .iter()
        .any(|key| key == "StartupWMClass"));
    assert!(!info.boolean("Desktop Entry", "StartupNotify").unwrap());
    integration::remove_shortcut(root.path(), 42, Some(3)).unwrap();
    integration::remove_shortcut(root.path(), 42, Some(3)).unwrap();
    assert!(!path.exists());
    assert!(!integration::shortcut_exists(root.path(), 42, Some(3)).unwrap());
    std::fs::write(
        &path,
        "[Desktop Entry]\nName=Other\nType=Application\nExec=other\n",
    )
    .unwrap();
    assert!(integration::create_shortcut(root.path(), executable, &game).is_err());
    assert!(integration::remove_shortcut(root.path(), 42, Some(3)).is_err());
    assert!(integration::shortcut_exists(root.path(), 42, Some(3)).is_err());
    assert!(std::fs::read_to_string(path)
        .unwrap()
        .contains("Name=Other"));
}

#[test]
fn test_shortcuts_reject_symlinks_and_invalid_ids() {
    let root = tempfile::tempdir().unwrap();
    let applications = root.path().join("applications");
    std::fs::create_dir(&applications).unwrap();
    let target = root.path().join("unrelated.desktop");
    std::fs::write(&target, "[Desktop Entry]\nX-Ira-Managed=true\n").unwrap();
    std::os::unix::fs::symlink(&target, applications.join("com.github.ira.game-2.desktop"))
        .unwrap();
    assert!(integration::remove_shortcut(root.path(), 2, None).is_err());
    assert!(integration::shortcut_exists(root.path(), 2, None).is_err());
    assert!(target.exists());
    assert!(integration::remove_shortcut(root.path(), -1, None).is_err());
}

#[test]
fn test_uninstall_removes_registration_but_preserves_game_shortcuts() {
    let root = tempfile::tempdir().unwrap();
    let executable = std::path::Path::new("/usr/bin/ira");
    let launcher = root.path().join("applications/ira.desktop");
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    let original =
        "[Desktop Entry]\nName=Ira\nType=Application\nExec=/usr/bin/ira\nX-AppHerder=true\n";
    std::fs::write(&launcher, original).unwrap();
    integration::install(
        root.path(),
        &root.path().join("config"),
        executable,
        "ira.desktop",
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&launcher).unwrap(), original);
    let game = Game {
        db_id: 42,
        name: "Game".into(),
        ..Default::default()
    };
    integration::create_shortcut(root.path(), executable, &game).unwrap();
    integration::uninstall(root.path(), &root.path().join("config")).unwrap();
    assert_eq!(std::fs::read_to_string(&launcher).unwrap(), original);
    assert!(!root
        .path()
        .join("applications/com.github.ira.desktop")
        .exists());
    assert!(!root
        .path()
        .join("gnome-shell/search-providers/com.github.ira.ini")
        .exists());
    assert!(!root
        .path()
        .join("dbus-1/services/com.github.ira.SearchProvider.service")
        .exists());
    assert!(root
        .path()
        .join("applications/com.github.ira.game-42.desktop")
        .is_file());
    integration::uninstall(root.path(), &root.path().join("config")).unwrap();
}

#[test]
fn test_uninstall_checks_ownership_before_removing_any_file() {
    let root = tempfile::tempdir().unwrap();
    integration::install(
        root.path(),
        &root.path().join("config"),
        std::path::Path::new("/usr/bin/ira"),
        "ira.desktop",
    )
    .unwrap();
    let service = root
        .path()
        .join("dbus-1/services/com.github.ira.SearchProvider.service");
    std::fs::write(&service, "[D-BUS Service]\nName=SomeoneElse\n").unwrap();
    assert!(integration::uninstall(root.path(), &root.path().join("config")).is_err());
    assert!(root
        .path()
        .join("gnome-shell/search-providers/com.github.ira.ini")
        .is_file());
    assert!(service.is_file());
}

#[test]
fn test_cli_search_registration_reuses_manager_launcher() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let config = root.path().join("config");
    let applications = data.join("applications");
    std::fs::create_dir_all(&applications).unwrap();
    let tools = root.path().join("bin");
    std::fs::create_dir_all(&tools).unwrap();
    let systemctl = tools.join("systemctl");
    std::fs::write(
        &systemctl,
        "#!/bin/sh\n[ \"$1\" = --user ] && [ \"$2\" = daemon-reload ]\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&systemctl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(std::iter::once(tools).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    let binary = std::path::Path::new(env!("CARGO_BIN_EXE_ira"));
    let launcher = applications.join("ira.desktop");
    let original = format!("[Desktop Entry]\nName=Ira\nType=Application\nExec=env DESKTOPINTEGRATION=1 {}\nX-AppHerder=true\n", binary.display());
    std::fs::write(&launcher, &original).unwrap();
    for mode in [
        "--install-desktop-integration",
        "--uninstall-desktop-integration",
    ] {
        let result = std::process::Command::new(binary)
            .env("PATH", &path)
            .arg(mode)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_DATA_DIRS", root.path().join("system"))
            .env("GSETTINGS_BACKEND", "memory")
            .env_remove("APPIMAGE")
            .env_remove("APPDIR")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read_to_string(&launcher).unwrap(), original);
        assert_eq!(std::fs::read_dir(&applications).unwrap().count(), 1);
        if mode == "--install-desktop-integration" {
            let provider = std::fs::read_to_string(
                data.join("gnome-shell/search-providers/com.github.ira.ini"),
            )
            .unwrap();
            assert!(provider.contains("DesktopId=ira.desktop\n"));
        }
    }
}
