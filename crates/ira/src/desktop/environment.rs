use std::path::{Path, PathBuf};

pub(super) const MARKER: &str = "# Managed by Ira desktop integration\n";

pub(super) fn path(config_home: &Path) -> PathBuf {
    config_home.join("systemd/user/org.gnome.Shell@.service.d/70-ira-desktop.conf")
}

pub(super) fn entry(data_home: &Path) -> Result<String, String> {
    entry_with_dirs(data_home, &session_data_dirs())
}

fn session_data_dirs() -> std::ffi::OsString {
    // AppRun preserves the host paths before adding its temporary mount and
    // loader cache. Session startup must never retain either temporary path.
    std::env::var_os("IRA_HOST_DATA_DIRS")
        .or_else(|| std::env::var_os("XDG_DATA_DIRS"))
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into())
}

fn entry_with_dirs(data_home: &Path, data_dirs: &std::ffi::OsStr) -> Result<String, String> {
    let root = data_home
        .to_str()
        .ok_or_else(|| "XDG data path is not UTF-8".to_string())?;
    if !data_home.is_absolute() || root.contains(['\0', '\n', '\r', ':', '$']) {
        return Err(
            "XDG data path cannot be represented in the session data directory list".into(),
        );
    }
    let roots = std::iter::once(data_home.to_path_buf())
        .chain(std::env::split_paths(data_dirs).filter(|path| path != data_home));
    let joined = std::env::join_paths(roots).map_err(|error| error.to_string())?;
    let value = joined.to_str().ok_or("Session data paths are not UTF-8")?;
    if value.contains(['\0', '\n', '\r']) {
        return Err("Session data paths contain control characters".into());
    }
    let value = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    // GNOME imports its login environment after environment.d runs. A service
    // override wins over that import; the pre-start propagates it to D-Bus apps.
    // D-Bus environment sync is best-effort; Shell still gets its own override.
    Ok(format!("{MARKER}[Service]\nEnvironment=\"XDG_DATA_DIRS={value}\"\nExecStartPre=-/usr/bin/dbus-update-activation-environment --systemd XDG_DATA_DIRS\n"))
}

pub(super) fn requires_login(root: &Path) -> Result<bool, String> {
    use glib::variant::ToVariant;
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|error| error.to_string())?;
    let reply = connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetConnectionUnixProcessID",
            Some(&("org.gnome.Shell",).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .map_err(|error| {
            crate::tr!("Read GNOME Shell session: {}").replacen("{}", &error.to_string(), 1)
        })?;
    let (pid,) = reply
        .get::<(u32,)>()
        .ok_or_else(|| crate::tr!("Invalid GNOME Shell process ID"))?;
    let environment = std::fs::read(format!("/proc/{pid}/environ")).map_err(|error| {
        crate::tr!("Read GNOME Shell data directories: {}").replacen("{}", &error.to_string(), 1)
    })?;
    // AppImage's own data roots can differ from the running desktop's roots.
    // Discovery status belongs to the Shell process that loads the providers.
    Ok(!root_in_environment(&environment, root))
}

fn root_in_environment(environment: &[u8], root: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let value = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"XDG_DATA_DIRS="))
        .filter(|value| !value.is_empty())
        .unwrap_or(b"/usr/local/share:/usr/share");
    std::env::split_paths(std::ffi::OsStr::from_bytes(value)).any(|path| path == root)
}

pub(super) fn reload_user_units() -> Result<(), String> {
    let output = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .map_err(|error| format!("Reload user session services: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Reload user session services: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_preserves_existing_session_data_paths() {
        let entry = entry_with_dirs(
            Path::new("/home/My Apps/data"),
            std::ffi::OsStr::new("/known/system/share:/usr/share"),
        )
        .unwrap();
        assert!(entry.starts_with(MARKER));
        assert!(entry.contains(
            "Environment=\"XDG_DATA_DIRS=/home/My Apps/data:/known/system/share:/usr/share\""
        ));
        assert!(entry.contains("[Service]\n"));
        assert!(entry.contains(
            "ExecStartPre=-/usr/bin/dbus-update-activation-environment --systemd XDG_DATA_DIRS"
        ));
        assert!(super::entry(Path::new("relative")).is_err());
        assert!(super::entry(Path::new("/home/line\nbreak")).is_err());
    }

    #[test]
    fn test_entry_with_dirs_escapes_specifiers_and_deduplicates_user_root() {
        let entry = entry_with_dirs(
            Path::new("/home/100%/data"),
            std::ffi::OsStr::new("/usr/share:/home/100%/data"),
        )
        .unwrap();
        assert!(entry.contains("XDG_DATA_DIRS=/home/100%%/data:/usr/share\""));
        assert_eq!(entry.matches("/home/100%%/data").count(), 1);
    }

    #[test]
    fn test_root_in_environment_uses_the_shells_paths_and_handles_default_roots() {
        let root = Path::new("/home/user/.local/share");
        assert!(!root_in_environment(
            b"HOME=/home/user\0XDG_DATA_DIRS=/usr/share\0",
            root
        ));
        assert!(root_in_environment(
            b"XDG_DATA_DIRS=/home/user/.local/share/:/usr/share\0",
            root
        ));
        assert!(root_in_environment(
            b"HOME=/home/user\0",
            Path::new("/usr/share")
        ));
        assert!(root_in_environment(
            b"XDG_DATA_DIRS=\0",
            Path::new("/usr/local/share")
        ));
    }
}
