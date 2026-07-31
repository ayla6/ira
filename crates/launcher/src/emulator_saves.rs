use std::path::{Path, PathBuf};

/// Convert a Linux absolute path to a Wine UNC path.
/// `/home/user/.local/share/ira` → `Z:\home\user\.local\share\ira`
fn to_wine_unc_path(path: &Path) -> String {
    format!("Z:{}", path.to_string_lossy().replace('/', "\\"))
}

/// Set up centralized save path for GBE by pushing the `GseSavePath` env var.
/// Called at launch time when `trophy_source == Gse`.
///
/// GBE checks this env var first (before `configs.user.ini`). When set, all
/// save data goes to `<centralized>/<appid>/` and the global settings folder
/// is ignored (fine — we put everything in `steam_settings/configs.user.ini`).
pub fn setup_gbe_saves(
    env: &mut Vec<(String, String)>,
    save_dir: &str,
    is_wine: bool,
) {
    let centralized = Path::new(save_dir).join("emulator_saves").join("gbe");
    let _ = std::fs::create_dir_all(&centralized);

    let path_str = if is_wine {
        to_wine_unc_path(&centralized)
    } else {
        centralized.to_string_lossy().into_owned()
    };

    env.retain(|(k, _)| k != "GseSavePath");
    env.push(("GseSavePath".to_string(), path_str));
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

    let users_dir = Path::new(wine_prefix).join("drive_c").join("users");
    let user_dirs = list_wine_users(&users_dir);

    if user_dirs.is_empty() {
        let steamuser = users_dir.join("steamuser");
        let _ = std::fs::create_dir_all(steamuser.join("AppData").join("Roaming"));
        create_nge_symlink(&steamuser, &centralized);
    } else {
        for user_dir in user_dirs {
            create_nge_symlink(&user_dir, &centralized);
        }
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

/// Create the `NemirtingasGalaxyEmu` symlink in a user's AppData/Roaming.
///
/// - If it's already a symlink → skip.
/// - If it's a real directory → migrate contents to centralized path, then
///   replace with symlink.
/// - If it doesn't exist → create symlink.
fn create_nge_symlink(user_dir: &Path, centralized: &Path) {
    let roaming = user_dir.join("AppData").join("Roaming");
    let _ = std::fs::create_dir_all(&roaming);

    let target = roaming.join("NemirtingasGalaxyEmu");

    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            return;
        }
        // Real directory — migrate contents safely before replacing
        eprintln!(
            "Migrating NemirtingasGalaxyEmu saves from {} to centralized path",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_wine_unc_path() {
        let path = Path::new("/home/user/.local/share/ira/emulator_saves/gbe");
        assert_eq!(
            to_wine_unc_path(path),
            "Z:\\home\\user\\.local\\share\\ira\\emulator_saves\\gbe"
        );
    }

    #[test]
    fn test_to_wine_unc_path_root() {
        let path = Path::new("/");
        assert_eq!(to_wine_unc_path(path), "Z:\\");
    }

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
    fn test_create_nge_symlink_creates_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let centralized = tmp.path().join("nge");
        std::fs::create_dir_all(&centralized).unwrap();
        let user_dir = tmp.path().join("steamuser");
        std::fs::create_dir_all(user_dir.join("AppData").join("Roaming")).unwrap();

        create_nge_symlink(&user_dir, &centralized);

        let target = user_dir.join("AppData").join("Roaming").join("NemirtingasGalaxyEmu");
        assert!(target.is_symlink());
        assert_eq!(std::fs::read_link(&target).unwrap(), centralized);
    }

    #[test]
    fn test_create_nge_symlink_migrates_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let centralized = tmp.path().join("nge");
        std::fs::create_dir_all(&centralized).unwrap();
        let user_dir = tmp.path().join("steamuser");
        let roaming = user_dir.join("AppData").join("Roaming");
        let nge_dir = roaming.join("NemirtingasGalaxyEmu");
        std::fs::create_dir_all(&nge_dir).unwrap();
        std::fs::write(nge_dir.join("save.dat"), b"save data").unwrap();

        create_nge_symlink(&user_dir, &centralized);

        // Original is now a symlink
        assert!(nge_dir.is_symlink());
        // Save data was migrated
        assert!(centralized.join("save.dat").exists());
    }

    #[test]
    fn test_create_nge_symlink_skips_existing_symlink() {
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

            create_nge_symlink(&user_dir, &centralized);

            // Symlink should still point to the original target, not overwritten
            assert_eq!(std::fs::read_link(&target).unwrap(), other);
        }
    }

    #[test]
    fn test_setup_gbe_saves_sets_env_var() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let mut env = vec![("FOO".to_string(), "bar".to_string())];

        setup_gbe_saves(&mut env, save_dir, false);

        let gse = env.iter().find(|(k, _)| k == "GseSavePath");
        assert!(gse.is_some());
        let path = gse.unwrap().1.clone();
        assert!(path.ends_with("emulator_saves/gbe"));
    }

    #[test]
    fn test_setup_gbe_saves_wine_uses_unc_path() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let mut env = Vec::new();

        setup_gbe_saves(&mut env, save_dir, true);

        let gse = env.iter().find(|(k, _)| k == "GseSavePath");
        assert!(gse.is_some());
        let path = &gse.unwrap().1;
        assert!(path.starts_with("Z:\\"));
        assert!(path.contains("emulator_saves\\gbe"));
    }
}
