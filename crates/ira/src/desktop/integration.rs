use super::{
    entries, environment, files::atomic_write, launcher, paths, protocol::*, search_settings,
    shortcut_icons,
};
use gio::prelude::*;
use ira_models::Game;
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

static SHORTCUT_WRITER: Mutex<()> = Mutex::new(());

fn shortcut_writer() -> Result<MutexGuard<'static, ()>, String> {
    SHORTCUT_WRITER
        .lock()
        .map_err(|_| crate::tr!("Shortcut writer lock poisoned"))
}

fn check_owned(path: &Path, group: &str) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("Refusing to replace a symlink: {}", path.display()));
        }
        Ok(_) => {}
    }
    let file = glib::KeyFile::new();
    file.load_from_file(path, glib::KeyFileFlags::NONE)
        .map_err(|error| error.to_string())?;
    if file.boolean(group, entries::MANAGED_KEY).unwrap_or(false) {
        Ok(())
    } else {
        Err(format!("File is not managed by Ira: {}", path.display()))
    }
}

fn write_managed(path: &Path, group: &str, contents: &str) -> Result<(), String> {
    check_owned(path, group)?;
    atomic_write(path, contents.as_bytes())
}

pub fn install_for_user() -> Result<(), String> {
    let executable = paths::executable()?;
    let desktop_id = launcher::find(&executable)?;
    install(
        &ira_config::data_home(),
        &ira_config::config_home(),
        &executable,
        &desktop_id,
    )?;
    environment::reload_user_units()?;
    search_settings::set_enabled(&desktop_id, true)
}

fn registration_files(root: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut files = vec![
        (
            root.join(format!("dbus-1/services/{PROVIDER_ID}.service")),
            "D-BUS Service",
        ),
        (
            root.join(format!("gnome-shell/search-providers/{APP_ID}.ini")),
            "Shell Search Provider",
        ),
    ];
    let export =
        environment::export_root(root).join(format!("gnome-shell/search-providers/{APP_ID}.ini"));
    // Shell enumerates only its system data roots. Flatpak's user export root
    // is already in that cached list on desktops that configure Flatpak.
    if environment::exports_are_discoverable(root) || export.exists() || export.is_symlink() {
        files.push((export, "Shell Search Provider"));
    }
    files
}

#[derive(Clone, Copy)]
pub struct IntegrationStatus {
    pub enabled: bool,
    pub requires_login: bool,
}

pub fn status_for_user() -> Result<IntegrationStatus, String> {
    let root = ira_config::data_home();
    let mut enabled = true;
    for (path, group) in registration_files(&root) {
        if !path.try_exists().map_err(|error| error.to_string())? {
            enabled = false;
            continue;
        }
        check_owned(&path, group)?;
    }
    if enabled {
        let id = registered_desktop_id(&root)?;
        enabled = !search_settings::disabled(&id)
            && gio::AppInfo::all()
                .iter()
                .any(|app| app.id().is_some_and(|app_id| app_id == id));
    }
    let session_file = environment::path(&ira_config::config_home());
    if !session_file
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        enabled = false;
    }
    check_environment_owned(&session_file)?;
    Ok(IntegrationStatus {
        enabled,
        requires_login: enabled && environment::requires_login(&root)?,
    })
}

fn check_environment_owned(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("Refusing to replace a symlink: {}", path.display()))
        }
        Ok(_) => {}
    }
    let data = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    if data.starts_with(environment::MARKER) {
        Ok(())
    } else {
        Err(format!(
            "Session environment file is not managed by Ira: {}",
            path.display()
        ))
    }
}

/// Removes only global Ira registrations; individual game shortcuts remain.
pub fn uninstall(root: &Path, config_home: &Path) -> Result<(), String> {
    let files = registration_files(root);
    for (path, group) in &files {
        check_owned(path, group)?;
    }
    let session_file = environment::path(config_home);
    check_environment_owned(&session_file)?;
    for path in files
        .into_iter()
        .map(|(path, _)| path)
        .chain([session_file])
    {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

pub fn uninstall_for_user() -> Result<(), String> {
    let root = ira_config::data_home();
    let provider = root.join(format!("gnome-shell/search-providers/{APP_ID}.ini"));
    if provider.try_exists().map_err(|error| error.to_string())? {
        check_owned(&provider, "Shell Search Provider")?;
        search_settings::set_enabled(&registered_desktop_id(&root)?, false)?;
    }
    uninstall(&root, &ira_config::config_home())?;
    environment::reload_user_units()
}

fn registered_desktop_id(root: &Path) -> Result<String, String> {
    let file = glib::KeyFile::new();
    file.load_from_file(
        root.join(format!("gnome-shell/search-providers/{APP_ID}.ini")),
        glib::KeyFileFlags::NONE,
    )
    .map_err(|error| error.to_string())?;
    file.string("Shell Search Provider", "DesktopId")
        .map(|id| id.to_string())
        .map_err(|error| error.to_string())
}

/// `root` and `config_home` are XDG homes. Existing unrelated files are never replaced.
pub fn install(
    root: &Path,
    config_home: &Path,
    executable: &Path,
    desktop_id: &str,
) -> Result<(), String> {
    if !desktop_id.ends_with(".desktop") || desktop_id.contains(['/', '\\', '\n', '\r', '\0']) {
        return Err("Invalid installed launcher desktop ID".into());
    }
    let executable = executable
        .to_str()
        .ok_or_else(|| "Executable path is not UTF-8".to_string())?;
    let session_file = environment::path(config_home);
    check_environment_owned(&session_file)?;
    let session = environment::entry(root)?;
    let files = registration_files(root);
    for (path, group) in &files {
        check_owned(path, group)?;
    }
    let service = entries::service_entry(executable)?;
    let provider = entries::provider_entry(desktop_id);
    for (path, group) in files {
        let contents = if group == "D-BUS Service" {
            &service
        } else {
            &provider
        };
        write_managed(&path, group, contents)?;
    }
    atomic_write(&session_file, session.as_bytes())
}

fn shortcut_path(root: &Path, game: i64, variant: Option<i64>) -> Result<PathBuf, String> {
    if game <= 0 || variant.is_some_and(|id| id <= 0) {
        return Err("Invalid game or variant ID".into());
    }
    let variant = variant
        .map(|id| format!("-variant-{id}"))
        .unwrap_or_default();
    Ok(root.join(format!(
        "applications/{APP_ID}.game-{game}{variant}.desktop"
    )))
}

pub fn create_shortcut(root: &Path, executable: &Path, game: &Game) -> Result<(), String> {
    let _writer = shortcut_writer()?;
    let path = shortcut_path(root, game.db_id, game.variant_id)?;
    check_owned(&path, "Desktop Entry")?;
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| crate::tr!("Invalid shortcut filename"))?;
    let icon = shortcut_icons::export(root, stem, game.display_icon())?;
    let executable = executable
        .to_str()
        .ok_or_else(|| "Executable path is not UTF-8".to_string())?;
    let contents = entries::game_entry(
        executable,
        &game.name,
        &icon.to_string_lossy(),
        game.db_id,
        game.variant_id,
    )?;
    write_managed(&path, "Desktop Entry", &contents)
}

/// Refreshes artwork and title for an existing Ira-owned shortcut, preserving its launch command.
pub fn refresh_shortcut(root: &Path, game: &Game) -> Result<bool, String> {
    let _writer = shortcut_writer()?;
    if !shortcut_exists(root, game.db_id, game.variant_id)? {
        return Ok(false);
    }
    let path = shortcut_path(root, game.db_id, game.variant_id)?;
    let key = glib::KeyFile::new();
    key.load_from_file(&path, glib::KeyFileFlags::NONE)
        .map_err(|error| error.to_string())?;
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| crate::tr!("Invalid shortcut filename"))?;
    let icon = shortcut_icons::export(root, stem, game.display_icon())?;
    let icon = icon.to_string_lossy();
    let has_window_class = key
        .has_key("Desktop Entry", "StartupWMClass")
        .unwrap_or(false);
    let startup_notify = key
        .boolean("Desktop Entry", "StartupNotify")
        .unwrap_or(true);
    if key
        .string("Desktop Entry", "Icon")
        .map(|old| old.as_str() == icon)
        .unwrap_or(false)
        && key
            .string("Desktop Entry", "Name")
            .map(|old| old.as_str() == game.name)
            .unwrap_or(false)
        && !has_window_class
        && !startup_notify
    {
        return Ok(false);
    }
    key.set_string("Desktop Entry", "Icon", &icon);
    key.set_string("Desktop Entry", "Name", &game.name);
    if has_window_class {
        key.remove_key("Desktop Entry", "StartupWMClass")
            .map_err(|error| error.to_string())?;
    }
    key.set_boolean("Desktop Entry", "StartupNotify", false);
    atomic_write(&path, key.to_data().as_bytes())?;
    Ok(true)
}

pub fn remove_shortcut(root: &Path, game: i64, variant: Option<i64>) -> Result<(), String> {
    let _writer = shortcut_writer()?;
    let path = shortcut_path(root, game, variant)?;
    check_owned(&path, "Desktop Entry")?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn shortcut_exists(root: &Path, game: i64, variant: Option<i64>) -> Result<bool, String> {
    let path = shortcut_path(root, game, variant)?;
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
        Ok(_) => {
            check_owned(&path, "Desktop Entry")?;
            Ok(true)
        }
    }
}

pub(crate) fn shortcut_for_game(game: &Game, create: bool) -> Result<(), String> {
    let root = ira_config::data_home();
    if create {
        create_shortcut(&root, &paths::executable()?, game)
    } else {
        remove_shortcut(&root, game.db_id, game.variant_id)
    }
}
