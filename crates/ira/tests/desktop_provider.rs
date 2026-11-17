use gio::DBusCallFlags;
use glib::variant::ToVariant;
use ira::desktop::{integration, protocol::*};
use ira_models::{GameKind, TrophySource};
use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

#[path = "support/desktop_bus.rs"]
mod desktop_bus;

fn call(
    connection: &gio::DBusConnection,
    method: &str,
    parameters: glib::Variant,
) -> Result<glib::Variant, glib::Error> {
    connection.call_sync(
        Some(PROVIDER_ID),
        PROVIDER_PATH,
        PROVIDER_INTERFACE,
        method,
        Some(&parameters),
        None,
        DBusCallFlags::NONE,
        10_000,
        gio::Cancellable::NONE,
    )
}

fn game(db: &ira_db::DbConn, name: &str, hidden: bool) -> i64 {
    let id = ira_db::add_game(
        db,
        ira_db::NewGame {
            kind: GameKind::Linux,
            trophy_source: TrophySource::Empty,
            steam_id: "",
            trophy_id: "",
            native_id: "",
            platform_id: GameKind::Linux.as_str(),
            title: name,
        },
    )
    .unwrap();
    if hidden {
        ira_db::update_field(db, id, "hidden", &true).unwrap();
    }
    id
}

fn fixture(root: &Path) -> (i64, std::path::PathBuf) {
    let data = root.join("data");
    let config = root.join("config");
    std::fs::create_dir_all(config.join("ira")).unwrap();
    let cfg = ira_config::Config {
        save_dir: root.join("library").to_string_lossy().into_owned(),
        ..Default::default()
    };
    std::fs::write(
        config.join("ira/config.json"),
        serde_json::to_vec(&cfg).unwrap(),
    )
    .unwrap();
    let db = ira_db::init_db(&format!("{}/ira.db", cfg.save_dir));
    let alpha = game(&db, "Pokémon Alpha", false);
    game(&db, "Pokémon Beta", false);
    game(&db, "Pokémon Hidden", true);
    ira_db::checkpoint(&db).unwrap();
    let marker = root.join("activation.txt");
    let image = root.join("Ira's launcher %f.AppImage");
    let binary = env!("CARGO_BIN_EXE_ira").replace('\'', "'\\''");
    let marker_arg = marker.to_string_lossy().replace('\'', "'\\''");
    std::fs::write(&image, format!("#!/bin/sh\nif [ \"$1\" = '--search-provider' ]; then\n exec '{binary}' \"$@\"\nfi\nprintf '%s\\n' \"$@\" > '{marker_arg}'\n")).unwrap();
    std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o755)).unwrap();
    integration::install(&data, &config, &image, "ira.desktop").unwrap();
    (alpha, image)
}

#[test]
fn test_search_provider_activates_headlessly_and_launches_through_ira() {
    let root = tempfile::tempdir().unwrap();
    let (alpha, image) = fixture(root.path());
    let binary = Path::new(env!("CARGO_BIN_EXE_ira"));
    let bus = desktop_bus::PrivateBus::start(
        root.path(),
        &[
            ("APPIMAGE", image.into_os_string()),
            ("APPDIR", binary.parent().unwrap().as_os_str().to_owned()),
        ],
    );
    let connection = bus.connect();
    let response = call(
        &connection,
        "GetInitialResultSet",
        (vec!["pokemon", "alpha"],).to_variant(),
    )
    .unwrap();
    let (ids,) = response.get::<(Vec<String>,)>().unwrap();
    assert_eq!(ids, vec![alpha.to_string()]);
    let response = call(&connection, "GetResultMetas", (ids.clone(),).to_variant()).unwrap();
    let (metas,) = response
        .get::<(Vec<HashMap<String, glib::Variant>>,)>()
        .unwrap();
    assert_eq!(metas[0]["name"].get::<String>().unwrap(), "Pokémon Alpha");
    assert!(metas[0].contains_key("icon"));
    let response = call(
        &connection,
        "GetSubsearchResultSet",
        (ids, vec!["pokemon", "hidden"]).to_variant(),
    )
    .unwrap();
    assert!(response.get::<(Vec<String>,)>().unwrap().0.is_empty());
    assert!(call(
        &connection,
        "ActivateResult",
        ("not-an-id", vec!["pokemon"], 0u32).to_variant()
    )
    .is_err());
    call(
        &connection,
        "ActivateResult",
        (alpha.to_string(), vec!["pokemon"], 0u32).to_variant(),
    )
    .unwrap();
    let marker = root.path().join("activation.txt");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(marker).unwrap(),
        format!("--{LAUNCH_OPTION}\n{alpha}\n")
    );
}

#[test]
fn test_session_override_is_valid_for_the_gnome_shell_user_service() {
    let analyzer = Path::new("/usr/bin/systemd-analyze");
    if !analyzer.is_file() {
        eprintln!("systemd-analyze is not installed; skipping unit syntax validation");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    let unit = root
        .path()
        .join("config/systemd/user/org.gnome.Shell@.service");
    std::fs::write(
        &unit,
        "[Unit]\nDescription=Test GNOME service\n[Service]\nExecStart=/usr/bin/env\n",
    )
    .unwrap();
    let output = Command::new(analyzer)
        .args(["--user", "--man=no", "verify"])
        .arg(unit.with_file_name("org.gnome.Shell@user.service"))
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("HOME", root.path())
        .env("XDG_CONFIG_DIRS", root.path().join("config-dirs"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dropin = std::fs::read_to_string(
        root.path()
            .join("config/systemd/user/org.gnome.Shell@.service.d/70-ira-desktop.conf"),
    )
    .unwrap();
    assert!(dropin.contains(&format!(
        "XDG_DATA_DIRS={}",
        root.path().join("data").display()
    )));
    assert!(dropin.contains(
        "ExecStartPre=-/usr/bin/dbus-update-activation-environment --systemd XDG_DATA_DIRS"
    ));
}
