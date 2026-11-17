use ira::desktop::integration;
use ira_models::Game;
use std::path::{Path, PathBuf};

fn shortcut_icon(root: &Path) -> PathBuf {
    let key = glib::KeyFile::new();
    key.load_from_file(
        root.join("applications/com.github.ira.game-42.desktop"),
        glib::KeyFileFlags::NONE,
    )
    .unwrap();
    key.string("Desktop Entry", "Icon").unwrap().as_str().into()
}

fn write_image(path: &Path, color: [u8; 4]) {
    image::RgbaImage::from_pixel(64, 64, image::Rgba(color))
        .save(path)
        .unwrap();
}

#[test]
fn test_create_shortcut_exports_png_and_changes_icon_identity_when_art_changes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("icon.webp");
    write_image(&source, [255, 0, 0, 255]);
    let game = Game {
        db_id: 42,
        name: "Game".into(),
        icon_path: source.to_string_lossy().into_owned(),
        ..Default::default()
    };
    integration::create_shortcut(root.path(), Path::new("/usr/bin/ira"), &game).unwrap();
    let red = shortcut_icon(root.path());
    assert_eq!(red.extension().unwrap(), "png");
    assert!(std::fs::read(&red)
        .unwrap()
        .starts_with(b"\x89PNG\r\n\x1a\n"));
    write_image(&source, [0, 0, 255, 255]);
    integration::remove_shortcut(root.path(), 42, None).unwrap();
    integration::create_shortcut(root.path(), Path::new("/usr/bin/ira"), &game).unwrap();
    let blue = shortcut_icon(root.path());
    assert_ne!(
        red, blue,
        "A desktop icon cache must see a new identity when the image bytes change"
    );
    assert_eq!(
        image::open(blue).unwrap().to_rgba8().get_pixel(0, 0).0,
        [0, 0, 255, 255]
    );
}

#[test]
fn test_create_shortcut_uses_full_resolution_icon_instead_of_sidebar_thumbnail() {
    let root = tempfile::tempdir().unwrap();
    let thumbnail = root.path().join("icon_small.webp");
    write_image(&thumbnail, [255, 0, 0, 255]);
    write_image(&root.path().join("icon.webp"), [0, 255, 0, 255]);
    let game = Game {
        db_id: 42,
        name: "Game".into(),
        icon_path: thumbnail.to_string_lossy().into_owned(),
        ..Default::default()
    };
    integration::create_shortcut(root.path(), Path::new("/usr/bin/ira"), &game).unwrap();
    assert_eq!(
        image::open(shortcut_icon(root.path()))
            .unwrap()
            .to_rgba8()
            .get_pixel(0, 0)
            .0,
        [0, 255, 0, 255]
    );
}

#[test]
fn test_refresh_shortcut_updates_same_path_art_without_changing_launch_command() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("icon.webp");
    write_image(&source, [255, 0, 0, 255]);
    let mut game = Game {
        db_id: 42,
        name: "Game".into(),
        icon_path: source.to_string_lossy().into_owned(),
        ..Default::default()
    };
    assert!(!integration::refresh_shortcut(root.path(), &game).unwrap());
    assert!(!root.path().join("applications").exists());
    integration::create_shortcut(root.path(), Path::new("/home/Managed Ira.AppImage"), &game)
        .unwrap();
    let old = shortcut_icon(root.path());
    assert!(!integration::refresh_shortcut(root.path(), &game).unwrap());
    write_image(&source, [0, 0, 255, 255]);
    game.name = "Renamed game".into();
    assert!(integration::refresh_shortcut(root.path(), &game).unwrap());
    let new = shortcut_icon(root.path());
    assert_ne!(old, new);
    let key = glib::KeyFile::new();
    key.load_from_file(
        root.path()
            .join("applications/com.github.ira.game-42.desktop"),
        glib::KeyFileFlags::NONE,
    )
    .unwrap();
    assert_eq!(
        key.string("Desktop Entry", "Exec").unwrap(),
        "\"/home/Managed Ira.AppImage\" --launch-game 42"
    );
    assert_eq!(key.string("Desktop Entry", "Name").unwrap(), "Renamed game");
    integration::remove_shortcut(root.path(), 42, None).unwrap();
    assert!(!integration::refresh_shortcut(root.path(), &game).unwrap());
}

#[test]
fn test_refresh_shortcut_preserves_unrelated_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root
        .path()
        .join("applications/com.github.ira.game-42.desktop");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = "[Desktop Entry]\nType=Application\nName=Other app\nExec=other\n";
    std::fs::write(&path, original).unwrap();
    let game = Game {
        db_id: 42,
        ..Default::default()
    };
    assert!(integration::refresh_shortcut(root.path(), &game).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

#[test]
fn test_create_shortcut_bounds_export_and_rejects_broken_art() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("icon.webp");
    image::RgbaImage::from_pixel(512, 128, image::Rgba([0u8, 255, 0, 255]))
        .save(&source)
        .unwrap();
    let game = Game {
        db_id: 42,
        icon_path: source.to_string_lossy().into_owned(),
        ..Default::default()
    };
    integration::create_shortcut(root.path(), Path::new("/usr/bin/ira"), &game).unwrap();
    let icon = image::open(shortcut_icon(root.path())).unwrap();
    assert_eq!((icon.width(), icon.height()), (256, 64));
    integration::remove_shortcut(root.path(), 42, None).unwrap();
    std::fs::write(source, b"invalid image").unwrap();
    assert!(integration::create_shortcut(root.path(), Path::new("/usr/bin/ira"), &game).is_err());
    assert!(!integration::shortcut_exists(root.path(), 42, None).unwrap());
}

#[test]
fn test_refresh_shortcut_keeps_variant_and_base_icons_independent() {
    let root = tempfile::tempdir().unwrap();
    let base_source = root.path().join("base.webp");
    let variant_source = root.path().join("variant.webp");
    write_image(&base_source, [255, 0, 0, 255]);
    write_image(&variant_source, [0, 0, 255, 255]);
    let base = Game {
        db_id: 42,
        icon_path: base_source.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let variant = Game {
        variant_id: Some(3),
        icon_path: variant_source.to_string_lossy().into_owned(),
        ..base.clone()
    };
    integration::create_shortcut(root.path(), Path::new("/usr/bin/ira"), &base).unwrap();
    integration::create_shortcut(root.path(), Path::new("/usr/bin/ira"), &variant).unwrap();
    let base_icon = shortcut_icon(root.path());
    write_image(&variant_source, [0, 255, 0, 255]);
    assert!(integration::refresh_shortcut(root.path(), &variant).unwrap());
    assert_eq!(shortcut_icon(root.path()), base_icon);
    let key = glib::KeyFile::new();
    key.load_from_file(
        root.path()
            .join("applications/com.github.ira.game-42-variant-3.desktop"),
        glib::KeyFileFlags::NONE,
    )
    .unwrap();
    let icon = key.string("Desktop Entry", "Icon").unwrap();
    assert_eq!(
        image::open(icon.as_str())
            .unwrap()
            .to_rgba8()
            .get_pixel(0, 0)
            .0,
        [0, 255, 0, 255]
    );
}

#[test]
fn test_refresh_shortcut_clears_window_ownership_without_artwork_changes() {
    let root = tempfile::tempdir().unwrap();
    let game = Game {
        db_id: 42,
        ..Default::default()
    };
    integration::create_shortcut(root.path(), Path::new("/usr/bin/ira"), &game).unwrap();
    let path = root
        .path()
        .join("applications/com.github.ira.game-42.desktop");
    let key = glib::KeyFile::new();
    key.load_from_file(&path, glib::KeyFileFlags::NONE).unwrap();
    key.set_string(
        "Desktop Entry",
        "StartupWMClass",
        ira::desktop::protocol::APP_ID,
    );
    key.set_boolean("Desktop Entry", "StartupNotify", true);
    std::fs::write(&path, key.to_data()).unwrap();
    let original_icon = shortcut_icon(root.path());
    assert!(integration::refresh_shortcut(root.path(), &game).unwrap());
    key.load_from_file(&path, glib::KeyFileFlags::NONE).unwrap();
    assert!(!key
        .keys("Desktop Entry")
        .unwrap()
        .iter()
        .any(|key| key == "StartupWMClass"));
    assert!(!key.boolean("Desktop Entry", "StartupNotify").unwrap());
    assert_eq!(shortcut_icon(root.path()), original_icon);
    assert!(!integration::refresh_shortcut(root.path(), &game).unwrap());
}
