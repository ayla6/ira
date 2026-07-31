use std::path::{Path, PathBuf};

/// Set up centralized save path for GBE by symlinking its save directories
/// inside the Wine prefix. Called at launch time when `trophy_source == Gse`.
///
/// GBE saves to `%APPDATA%\GSE Saves\` (legacy name: `Goldberg SteamEmu Saves`).
/// We replace those directories with symlinks to `<save_dir>/emulator_saves/gbe/`
/// so saves persist across prefix resets.
///
/// The previous approach passed `GseSavePath` as an env var to redirect GBE's
/// save root. That broke Denuvo/reflex hypervisor games (P5T, P5R): with the
/// env var set, GBE ignores its existing settings folder under the default save
/// root, and the game refuses to start. Symlinks keep GBE on its default path,
/// so the game can't tell saves were redirected.
pub fn setup_gbe_saves(wine_prefix: &str, save_dir: &str) {
    let centralized = Path::new(save_dir).join("emulator_saves").join("gbe");
    let _ = std::fs::create_dir_all(&centralized);

    for user_dir in prefix_user_dirs(wine_prefix) {
        create_roaming_symlink(&user_dir, &centralized, "GSE Saves");
        create_roaming_symlink(&user_dir, &centralized, "Goldberg SteamEmu Saves");
    }
}
/// Set up centralized save path for NGE by creating symlinks in the Wine prefix.
/// Called at launch time when `trophy_source == Nge` and Wine is enabled.
///
/// NGE saves to `%APPDATA%\NemirtingasGalaxyEmu\`. We replace that directory
/// with a symlink to `<save_dir>/emulator_saves/nge/` so saves persist across
/// prefix resets. Multiple GOG games in the same prefix share the symlink —
/// NGE separates by `productid` internally.
pub fn setup_nge_saves(wine_prefix: &str, save_dir: &str) {
    let centralized = Path::new(save_dir).join("emulator_saves").join("nge");
    let _ = std::fs::create_dir_all(&centralized);

    for user_dir in prefix_user_dirs(wine_prefix) {
        create_roaming_symlink(&user_dir, &centralized, "NemirtingasGalaxyEmu");
    }
}

/// Resolve the user directories to place save symlinks in, creating a
/// `steamuser` dir when the prefix has none yet.
///
/// Returns an empty vec for an empty `wine_prefix` — an empty string would
/// resolve to the current working directory (`Path::new("").join("drive_c")`),
/// which could create a stray `drive_c/` wherever the app is running from.
fn prefix_user_dirs(wine_prefix: &str) -> Vec<PathBuf> {
    if wine_prefix.is_empty() {
        return Vec::new();
    }
    let users_dir = Path::new(wine_prefix).join("drive_c").join("users");
    let user_dirs = list_wine_users(&users_dir);

    if user_dirs.is_empty() {
        let steamuser = users_dir.join("steamuser");
        let _ = std::fs::create_dir_all(steamuser.join("AppData").join("Roaming"));
        vec![steamuser]
    } else {
        user_dirs
    }
}

/// List real user directories under `drive_c/users/`.
///
/// If `$USER` is a symlink to `steamuser` (common Proton behavior), it is
/// skipped — `steamuser` already covers it. Only `steamuser` and real (non-
/// symlinked) user directories are returned.
fn list_wine_users(users_dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let entries = match std::fs::read_dir(users_dir) {
        Ok(e) => e,
        Err(_) => return result,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };

        if name == "steamuser" {
            result.push(path);
            continue;
        }

        // Skip symlinks to steamuser (Proton redirects $USER → steamuser)
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() {
                if let Ok(target) = std::fs::read_link(&path) {
                    if target.ends_with("steamuser") || target == Path::new("steamuser") {
                        continue;
                    }
                }
            }
        }

        result.push(path);
    }

    result
}

/// Create a symlink `<user_dir>/AppData/Roaming/<name>` → `centralized`.
fn create_roaming_symlink(user_dir: &Path, centralized: &Path, name: &str) {
    let roaming = user_dir.join("AppData").join("Roaming");
    let _ = std::fs::create_dir_all(&roaming);
    create_save_symlink(&roaming, name, centralized);
}

/// Create a symlink `<base>/<name>` → `centralized`.
///
/// - If it's already a symlink → skip.
/// - If it's a real directory → migrate contents to centralized path, then
///   replace with symlink.
/// - If it doesn't exist → create symlink.
fn create_save_symlink(base: &Path, name: &str, centralized: &Path) {
    let target = base.join(name);

    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            return;
        }
        // Real directory — migrate contents safely before replacing
        eprintln!(
            "Migrating {} saves from {} to centralized path",
            name,
            target.display()
        );
        crate::game_saves::safe_migrate_dir_contents(&target, centralized);
        let _ = std::fs::remove_dir(&target);
    }

    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(centralized, &target);
    }
}

/// GBE's native Linux save root: `$XDG_DATA_HOME`, falling back to
/// `~/.local/share`.
fn native_saves_base() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| Path::new(&h).join(".local/share"))
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
        })
}

/// Set up centralized save path for GBE for a native Linux game by symlinking
/// GBE's native save directories (`$XDG_DATA_HOME/GSE Saves` and legacy
/// `Goldberg SteamEmu Saves`) to `<save_dir>/emulator_saves/gbe/`.
pub fn setup_gbe_saves_native(save_dir: &str) {
    let centralized = Path::new(save_dir).join("emulator_saves").join("gbe");
    let _ = std::fs::create_dir_all(&centralized);

    let base = native_saves_base();
    let _ = std::fs::create_dir_all(&base);
    create_save_symlink(&base, "GSE Saves", &centralized);
    create_save_symlink(&base, "Goldberg SteamEmu Saves", &centralized);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_wine_users_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let users = tmp.path().join("users");
        std::fs::create_dir_all(&users).unwrap();
        assert!(list_wine_users(&users).is_empty());
    }

    #[test]
    fn test_list_wine_users_steamuser_only() {
        let tmp = tempfile::tempdir().unwrap();
        let users = tmp.path().join("users");
        std::fs::create_dir_all(users.join("steamuser")).unwrap();
        let result = list_wine_users(&users);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], users.join("steamuser"));
    }

    #[test]
    fn test_list_wine_users_skips_symlink_to_steamuser() {
        let tmp = tempfile::tempdir().unwrap();
        let users = tmp.path().join("users");
        std::fs::create_dir_all(users.join("steamuser")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("steamuser", users.join("myuser")).unwrap();

        let result = list_wine_users(&users);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], users.join("steamuser"));
    }

    #[test]
    fn test_list_wine_users_includes_real_user() {
        let tmp = tempfile::tempdir().unwrap();
        let users = tmp.path().join("users");
        std::fs::create_dir_all(users.join("steamuser")).unwrap();
        std::fs::create_dir_all(users.join("myuser")).unwrap();

        let result = list_wine_users(&users);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_create_roaming_symlink_creates_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let centralized = tmp.path().join("nge");
        std::fs::create_dir_all(&centralized).unwrap();
        let user_dir = tmp.path().join("steamuser");
        std::fs::create_dir_all(user_dir.join("AppData").join("Roaming")).unwrap();

        create_roaming_symlink(&user_dir, &centralized, "NemirtingasGalaxyEmu");

        let target = user_dir.join("AppData").join("Roaming").join("NemirtingasGalaxyEmu");
        assert!(target.is_symlink());
        assert_eq!(std::fs::read_link(&target).unwrap(), centralized);
    }

    #[test]
    fn test_create_roaming_symlink_migrates_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let centralized = tmp.path().join("nge");
        std::fs::create_dir_all(&centralized).unwrap();
        let user_dir = tmp.path().join("steamuser");
        let roaming = user_dir.join("AppData").join("Roaming");
        let nge_dir = roaming.join("NemirtingasGalaxyEmu");
        std::fs::create_dir_all(&nge_dir).unwrap();
        std::fs::write(nge_dir.join("save.dat"), b"save data").unwrap();

        create_roaming_symlink(&user_dir, &centralized, "NemirtingasGalaxyEmu");

        // Original is now a symlink
        assert!(nge_dir.is_symlink());
        // Save data was migrated
        assert!(centralized.join("save.dat").exists());
    }

    #[test]
    fn test_create_roaming_symlink_skips_existing_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let centralized = tmp.path().join("nge");
        std::fs::create_dir_all(&centralized).unwrap();
        let user_dir = tmp.path().join("steamuser");
        let roaming = user_dir.join("AppData").join("Roaming");
        std::fs::create_dir_all(&roaming).unwrap();
        let target = roaming.join("NemirtingasGalaxyEmu");

        #[cfg(unix)]
        {
            // Create a symlink to some other location
            let other = tmp.path().join("other");
            std::fs::create_dir_all(&other).unwrap();
            std::os::unix::fs::symlink(&other, &target).unwrap();

            create_roaming_symlink(&user_dir, &centralized, "NemirtingasGalaxyEmu");

            // Symlink should still point to the original target, not overwritten
            assert_eq!(std::fs::read_link(&target).unwrap(), other);
        }
    }

    #[test]
    fn test_setup_gbe_saves_creates_both_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let prefix = tmp.path().join("prefix");
        let user_dir = prefix.join("drive_c").join("users").join("steamuser");
        std::fs::create_dir_all(user_dir.join("AppData").join("Roaming")).unwrap();

        setup_gbe_saves(prefix.to_str().unwrap(), save_dir);

        let roaming = user_dir.join("AppData").join("Roaming");
        let gse = roaming.join("GSE Saves");
        let legacy = roaming.join("Goldberg SteamEmu Saves");
        assert!(gse.is_symlink());
        assert!(legacy.is_symlink());
        assert_eq!(
            std::fs::read_link(&gse).unwrap(),
            Path::new(save_dir).join("emulator_saves").join("gbe")
        );
        assert_eq!(std::fs::read_link(&legacy).unwrap(), gse.read_link().unwrap());
    }

    #[test]
    fn test_setup_gbe_saves_skips_existing_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let prefix = tmp.path().join("prefix");
        let user_dir = prefix.join("drive_c").join("users").join("steamuser");
        let roaming = user_dir.join("AppData").join("Roaming");
        std::fs::create_dir_all(&roaming).unwrap();

        let manual = tmp.path().join("manual_saves");
        std::fs::create_dir_all(&manual).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&manual, roaming.join("GSE Saves")).unwrap();

        setup_gbe_saves(prefix.to_str().unwrap(), save_dir);

        // Pre-existing symlink is left alone, pointing at the manual location
        assert_eq!(std::fs::read_link(roaming.join("GSE Saves")).unwrap(), manual);
    }

    #[test]
    fn test_prefix_user_dirs_empty_prefix_creates_nothing() {
        let cwd = std::env::current_dir().unwrap();
        let users = prefix_user_dirs("");
        assert!(users.is_empty());
        assert!(!cwd.join("drive_c").exists());
    }

    #[test]
    fn test_setup_gbe_saves_native_creates_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().join("ira").to_str().unwrap().to_string();
        let data_home = tmp.path().join("data_home");
        std::fs::create_dir_all(&data_home).unwrap();

        // Redirect XDG_DATA_HOME for the test
        std::env::set_var("XDG_DATA_HOME", &data_home);
        std::env::remove_var("HOME");

        setup_gbe_saves_native(&save_dir);

        std::env::remove_var("XDG_DATA_HOME");

        let gse = data_home.join("GSE Saves");
        let legacy = data_home.join("Goldberg SteamEmu Saves");
        assert!(gse.is_symlink());
        assert!(legacy.is_symlink());
        assert_eq!(
            std::fs::read_link(&gse).unwrap(),
            Path::new(&save_dir).join("emulator_saves").join("gbe")
        );
    }
}
