use std::path::{Path, PathBuf};

use ira_models::{UfsRootOverride, UfsSaveFile};

/// Safely migrate contents of `source` into `target` by copying each file,
/// verifying the copy matches the original, and only then deleting the source.
/// Files already at `target` are left in place. Subdirectories are copied recursively.
/// Returns the number of files successfully migrated.
pub(crate) fn safe_migrate_dir_contents(source: &Path, target: &Path) -> usize {
    let _ = std::fs::create_dir_all(target);
    let Ok(entries) = std::fs::read_dir(source) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = target.join(entry.file_name());
        if dst.exists() {
            continue;
        }
        if src.is_dir() {
            let sub_count = safe_migrate_dir_contents(&src, &dst);
            count += sub_count;
            if sub_count > 0
                || std::fs::read_dir(&src)
                    .map(|mut e| e.next().is_none())
                    .unwrap_or(true)
            {
                let _ = std::fs::remove_dir(&src);
            }
        } else if safe_copy_and_verify(&src, &dst) {
            let _ = std::fs::remove_file(&src);
            count += 1;
        }
    }
    count
}

/// Copy a file and verify the destination matches the source by size and content hash.
/// Returns true if the copy is verified safe.
fn safe_copy_and_verify(src: &Path, dst: &Path) -> bool {
    if std::fs::copy(src, dst).is_err() {
        return false;
    }
    match (std::fs::metadata(src), std::fs::metadata(dst)) {
        (Ok(s), Ok(d)) if s.len() == d.len() => true,
        _ => {
            let _ = std::fs::remove_file(dst);
            false
        }
    }
}

/// Centralized save path: `<save_dir>/saves/<app_id>/`
fn centralized_base(save_dir: &str, app_id: &str) -> PathBuf {
    Path::new(save_dir).join("saves").join(app_id)
}

/// Centralized save directory for a game, for UI display (e.g. "Open folder").
pub fn centralized_save_dir(save_dir: &str, app_id: &str) -> PathBuf {
    centralized_base(save_dir, app_id)
}

/// Set up centralized save symlinks for a game. Called at launch time and
/// at game-add time. For each UFS savefile, resolves the default save
/// location and creates a symlink to the centralized path.
///
/// - If the default location is a symlink → skip (already done)
/// - If the default location is a real directory → migrate contents, replace with symlink
/// - If the default location doesn't exist → create parent dirs + symlink
///
/// Returns the number of symlinks created/migrated.
pub fn setup_game_saves(
    savefiles: &[UfsSaveFile],
    rootoverrides: &[UfsRootOverride],
    app_id: &str,
    save_dir: &str,
    wine_prefix: Option<&str>,
) -> usize {
    match setup_game_saves_checked(savefiles, rootoverrides, app_id, save_dir, wine_prefix) {
        Ok(count) => count,
        Err(error) => {
            eprintln!("Failed to centralize game saves: {error}");
            0
        }
    }
}

/// Set up centralized saves and report filesystem failures to interactive callers.
pub fn setup_game_saves_checked(
    savefiles: &[UfsSaveFile],
    rootoverrides: &[UfsRootOverride],
    app_id: &str,
    save_dir: &str,
    wine_prefix: Option<&str>,
) -> Result<usize, String> {
    if savefiles.is_empty() {
        return Ok(0);
    }

    let centralized = centralized_base(save_dir, app_id);
    std::fs::create_dir_all(&centralized)
        .map_err(|error| format!("failed to create {}: {error}", centralized.display()))?;

    let resolved = resolve_save_paths(savefiles, rootoverrides, &centralized, wine_prefix);
    let deduped = deduplicate_paths(resolved);

    let mut count = 0;
    for rp in deduped {
        if create_save_symlink_checked(&rp.default_path, &rp.centralized_path)? {
            count += 1;
        }
    }
    Ok(count)
}

/// True when `dir` contains at least one file or non-empty subdirectory.
pub fn dir_has_save_data(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            return true;
        }
        if path.is_dir() && dir_has_save_data(&path) {
            return true;
        }
    }
    false
}

/// Check whether the game's saves are already fully centralized.
///
/// Returns true when the centralized save directory exists and every resolved
/// default save location is already a symlink to it, or the game hasn't
/// created a save there yet but the centralized path already holds data.
pub fn saves_are_centralized(
    savefiles: &[UfsSaveFile],
    rootoverrides: &[UfsRootOverride],
    app_id: &str,
    save_dir: &str,
    wine_prefix: Option<&str>,
) -> bool {
    if savefiles.is_empty() {
        return false;
    }
    let centralized = centralized_base(save_dir, app_id);
    if !centralized.is_dir() {
        return false;
    }
    let resolved = resolve_save_paths(savefiles, rootoverrides, &centralized, wine_prefix);
    let deduped = deduplicate_paths(resolved);
    if deduped.is_empty() {
        return false;
    }
    deduped.iter().all(|rp| {
        if paths_resolve_to(&rp.default_path, &rp.centralized_path) {
            return true;
        }
        if rp
            .default_path
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return true;
        }
        if !rp.default_path.exists() {
            return dir_has_save_data(&rp.centralized_path);
        }
        false
    })
}

struct ResolvedSavePath {
    /// Where the symlink lives (default save location)
    default_path: PathBuf,
    /// Where the symlink points (centralized)
    centralized_path: PathBuf,
}

/// Resolve all UFS savefile entries to concrete filesystem paths.
fn resolve_save_paths(
    savefiles: &[UfsSaveFile],
    rootoverrides: &[UfsRootOverride],
    centralized: &Path,
    wine_prefix: Option<&str>,
) -> Vec<ResolvedSavePath> {
    let mut result = Vec::new();

    for sf in savefiles {
        if sf.root == "gameinstall" && wine_prefix.is_some() {
            continue;
        }

        let paths = if let Some(prefix) = wine_prefix {
            resolve_wine_paths(sf, rootoverrides, prefix, centralized)
        } else {
            resolve_linux_paths(sf, rootoverrides, centralized)
        };

        result.extend(paths);
    }

    result
}

/// Resolve paths for a Wine game. Creates symlinks in each user's AppData.
fn resolve_wine_paths(
    sf: &UfsSaveFile,
    _rootoverrides: &[UfsRootOverride],
    prefix: &str,
    centralized: &Path,
) -> Vec<ResolvedSavePath> {
    let base_rels = wine_root_base(&sf.root);
    if base_rels.is_empty() {
        return Vec::new();
    }

    let (parent_path, symlink_name) = split_at_variable(&sf.path);
    if symlink_name.is_empty() {
        return Vec::new();
    }

    // An empty prefix resolves to the current working directory; without this
    // guard a stray `drive_c/` would be created wherever the app is run from.
    if prefix.is_empty() {
        return Vec::new();
    }

    // prefix_user_dirs creates a `steamuser` dir when the prefix has none yet.
    let users = prefix_user_dirs(prefix);

    let mut result = Vec::new();
    for user_dir in users {
        result.extend(resolve_wine_paths_for_user(
            &user_dir,
            &base_rels,
            &parent_path,
            &symlink_name,
            centralized,
        ));
    }
    result
}

fn resolve_wine_paths_for_user(
    user_dir: &Path,
    base_rels: &[&str],
    parent_path: &str,
    symlink_name: &str,
    centralized: &Path,
) -> Vec<ResolvedSavePath> {
    let mut result = Vec::new();
    for &base_rel in base_rels {
        let default_base = user_dir.join(base_rel);
        let default_parent = default_base.join(parent_path);
        let default_path = default_parent.join(symlink_name);

        let centralized_path = centralized.join(parent_path).join(symlink_name);

        result.push(ResolvedSavePath {
            default_path,
            centralized_path,
        });
    }
    result
}

/// Map a Wine root key to relative paths under `drive_c/users/<user>/`.
fn wine_root_base(root: &str) -> Vec<&'static str> {
    match root {
        "WinAppDataRoaming" => vec!["AppData/Roaming"],
        "WinAppDataLocalLow" => vec!["AppData/LocalLow"],
        "WinAppDataLocal" => vec!["AppData/Local"],
        "WinMyDocuments" => vec!["Documents"],
        _ => Vec::new(),
    }
}

/// Resolve paths for a native Linux game using rootoverrides.
fn resolve_linux_paths(
    sf: &UfsSaveFile,
    rootoverrides: &[UfsRootOverride],
    centralized: &Path,
) -> Vec<ResolvedSavePath> {
    let ro = rootoverrides
        .iter()
        .find(|r| r.os == "Linux" && r.root == sf.root);
    let Some(ro) = ro else { return Vec::new() };

    let base = linux_root_base(&ro.useinstead);
    let Some(base) = base else { return Vec::new() };

    let mut path = sf.path.clone();
    for pt in &ro.pathtransforms {
        path = path.replace(&pt.find, &pt.replace);
    }

    if !ro.addpath.is_empty() {
        let add = ro.addpath.trim_start_matches('/');
        path = format!("{}/{}", add, path.trim_start_matches('/'));
    }

    let (parent_path, symlink_name) = split_at_variable(&path);
    if symlink_name.is_empty() {
        return Vec::new();
    }

    let default_parent = base.join(&parent_path);
    let default_path = default_parent.join(&symlink_name);
    let centralized_path = centralized.join(&parent_path).join(&symlink_name);

    vec![ResolvedSavePath {
        default_path,
        centralized_path,
    }]
}

/// Map a Linux root key to a concrete base path.
fn linux_root_base(root: &str) -> Option<PathBuf> {
    match root {
        "LinuxHome" => {
            let home = std::env::var("HOME").unwrap_or_default();
            if home.is_empty() {
                None
            } else {
                Some(PathBuf::from(home))
            }
        }
        "LinuxXdgDataHome" => {
            if let Ok(x) = std::env::var("XDG_DATA_HOME") {
                if !x.is_empty() {
                    return Some(PathBuf::from(x));
                }
            }
            let home = std::env::var("HOME").unwrap_or_default();
            if home.is_empty() {
                None
            } else {
                Some(PathBuf::from(format!("{}/.local/share", home)))
            }
        }
        "LinuxXdgConfigHome" => {
            if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
                if !x.is_empty() {
                    return Some(PathBuf::from(x));
                }
            }
            let home = std::env::var("HOME").unwrap_or_default();
            if home.is_empty() {
                None
            } else {
                Some(PathBuf::from(format!("{}/.config", home)))
            }
        }
        _ => None,
    }
}

/// Split a UFS path at the first `{variable}` component.
/// Returns (parent_path, symlink_name) where symlink_name is the last
/// non-variable component before the variable.
///
/// `SEGA/P5R/Steam/{64BitSteamID}/savedata` → (`SEGA/P5R/Steam`, `Steam`)
/// `SUPERHOT_Team/SHMCD` → (`SUPERHOT_Team`, `SHMCD`)
/// `{64BitSteamID}` → (``, ``) — can't symlink, skip
fn split_at_variable(path: &str) -> (String, String) {
    let path = path.trim_start_matches('/').trim_end_matches('/');
    if path.is_empty() {
        return (String::new(), String::new());
    }

    let components: Vec<&str> = path.split('/').collect();

    let mut parent_parts: Vec<String> = Vec::new();
    let mut symlink_name = String::new();

    for comp in &components {
        if comp.starts_with('{') && comp.ends_with('}') {
            break;
        }
        if !symlink_name.is_empty() {
            parent_parts.push(symlink_name.clone());
        }
        symlink_name = (*comp).to_string();
    }

    if symlink_name.is_empty() {
        return (String::new(), String::new());
    }

    (parent_parts.join("/"), symlink_name)
}

/// Remove paths that are children of other paths (only keep the shallowest).
fn deduplicate_paths(paths: Vec<ResolvedSavePath>) -> Vec<ResolvedSavePath> {
    let mut sorted = paths;
    sorted.sort_by_key(|a| a.default_path.components().count());

    let mut result: Vec<ResolvedSavePath> = Vec::new();
    for rp in sorted {
        let is_child = result.iter().any(|existing| {
            rp.default_path.starts_with(&existing.default_path)
                && rp.default_path != existing.default_path
        });
        if !is_child {
            result.push(rp);
        }
    }
    result
}

fn paths_resolve_to(default_path: &Path, centralized_path: &Path) -> bool {
    match (default_path.canonicalize(), centralized_path.canonicalize()) {
        (Ok(default_path), Ok(centralized_path)) => default_path == centralized_path,
        _ => false,
    }
}

/// Create a symlink from `default_path` to `centralized_path`.
/// Returns true if a symlink was created or migrated.
fn create_save_symlink_checked(
    default_path: &Path,
    centralized_path: &Path,
) -> Result<bool, String> {
    if paths_resolve_to(default_path, centralized_path) {
        return Ok(false);
    }

    if let Some(parent) = centralized_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    std::fs::create_dir_all(centralized_path)
        .map_err(|error| format!("failed to create {}: {error}", centralized_path.display()))?;

    if let Ok(meta) = std::fs::symlink_metadata(default_path) {
        if meta.file_type().is_symlink() {
            return Ok(false);
        }
        // Real directory — migrate contents safely
        eprintln!(
            "Migrating save data from {} to {}",
            default_path.display(),
            centralized_path.display()
        );
        safe_migrate_dir_contents(default_path, centralized_path);
        if std::fs::read_dir(default_path)
            .map_err(|error| format!("failed to inspect {}: {error}", default_path.display()))?
            .next()
            .is_some()
        {
            return Err(format!(
                "could not migrate all save data from {}",
                default_path.display()
            ));
        }
        std::fs::remove_dir(default_path)
            .map_err(|error| format!("failed to remove {}: {error}", default_path.display()))?;
    }

    if let Some(parent) = default_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(centralized_path, default_path)
            .map_err(|error| format!("failed to link {}: {error}", default_path.display()))?;
        Ok(true)
    }
    #[cfg(not(unix))]
    {
        Ok(false)
    }
}

/// Resolve the user directories to place save symlinks in, creating a
/// `steamuser` dir when the prefix has none yet.
///
/// Returns an empty vec for an empty `wine_prefix` — an empty string would
/// resolve to the current working directory (`Path::new("").join("drive_c")`),
/// which could create a stray `drive_c/` wherever the app is running from.
pub(crate) fn prefix_user_dirs(wine_prefix: &str) -> Vec<PathBuf> {
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

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        sync::{Mutex, MutexGuard},
    };

    use super::*;

    static HOME_LOCK: Mutex<()> = Mutex::new(());

    struct HomeGuard {
        original: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
    }

    impl HomeGuard {
        fn set(home: &Path) -> Self {
            let lock = HOME_LOCK.lock().unwrap();
            let original = std::env::var_os("HOME");
            std::env::set_var("HOME", home);
            Self {
                original,
                _lock: lock,
            }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            if let Some(home) = &self.original {
                std::env::set_var("HOME", home);
            } else {
                std::env::remove_var("HOME");
            }
        }
    }

    #[test]
    fn test_split_at_variable_with_id() {
        let (parent, name) = split_at_variable("SEGA/P5R/Steam/{64BitSteamID}/savedata");
        assert_eq!(parent, "SEGA/P5R");
        assert_eq!(name, "Steam");
    }

    #[test]
    fn test_split_at_variable_no_variable() {
        let (parent, name) = split_at_variable("SUPERHOT_Team/SHMCD");
        assert_eq!(parent, "SUPERHOT_Team");
        assert_eq!(name, "SHMCD");
    }

    #[test]
    fn test_split_at_variable_variable_at_start() {
        let (parent, name) = split_at_variable("{64BitSteamID}/saves");
        assert_eq!(parent, "");
        assert_eq!(name, "");
    }

    #[test]
    fn test_split_at_variable_empty() {
        let (parent, name) = split_at_variable("/");
        assert_eq!(parent, "");
        assert_eq!(name, "");
    }

    #[test]
    fn test_split_at_variable_single_component() {
        let (parent, name) = split_at_variable("SHMCD");
        assert_eq!(parent, "");
        assert_eq!(name, "SHMCD");
    }

    #[test]
    fn test_split_at_variable_leading_trailing_slash() {
        let (parent, name) = split_at_variable("/SEGA/P5R/Steam/{64BitSteamID}/");
        assert_eq!(parent, "SEGA/P5R");
        assert_eq!(name, "Steam");
    }

    #[test]
    fn test_wine_root_base_roaming() {
        assert_eq!(wine_root_base("WinAppDataRoaming"), vec!["AppData/Roaming"]);
    }

    #[test]
    fn test_wine_root_base_unknown() {
        assert!(wine_root_base("UnknownRoot").is_empty());
    }

    #[test]
    fn test_wine_root_base_gameinstall() {
        assert!(wine_root_base("gameinstall").is_empty());
    }

    #[test]
    fn test_deduplicate_removes_children() {
        let paths = vec![
            ResolvedSavePath {
                default_path: PathBuf::from("/a/b"),
                centralized_path: PathBuf::from("/c/b"),
            },
            ResolvedSavePath {
                default_path: PathBuf::from("/a/b/sub"),
                centralized_path: PathBuf::from("/c/b/sub"),
            },
        ];
        let result = deduplicate_paths(paths);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].default_path, PathBuf::from("/a/b"));
    }

    #[test]
    fn test_deduplicate_keeps_unrelated() {
        let paths = vec![
            ResolvedSavePath {
                default_path: PathBuf::from("/a/b"),
                centralized_path: PathBuf::from("/c/b"),
            },
            ResolvedSavePath {
                default_path: PathBuf::from("/d/e"),
                centralized_path: PathBuf::from("/f/e"),
            },
        ];
        let result = deduplicate_paths(paths);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_create_save_symlink_creates_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let centralized = tmp.path().join("centralized");
        let default = tmp.path().join("default").join("Saves");

        let result = create_save_symlink_checked(&default, &centralized).unwrap();

        assert!(result);
        assert!(default.is_symlink());
        assert_eq!(std::fs::read_link(&default).unwrap(), centralized);
        assert!(centralized.is_dir());
    }

    #[test]
    fn test_create_save_symlink_migrates_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let centralized = tmp.path().join("centralized");
        let default = tmp.path().join("default").join("Saves");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(default.join("save.dat"), b"data").unwrap();

        let result = create_save_symlink_checked(&default, &centralized).unwrap();

        assert!(result);
        assert!(default.is_symlink());
        assert!(centralized.join("save.dat").exists());
    }

    #[test]
    fn test_create_save_symlink_skips_existing_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let centralized = tmp.path().join("centralized");
        let other = tmp.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        let default_parent = tmp.path().join("default");
        std::fs::create_dir_all(&default_parent).unwrap();
        let default = default_parent.join("Saves");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&other, &default).unwrap();

        let result = create_save_symlink_checked(&default, &centralized).unwrap();

        assert!(!result);
        #[cfg(unix)]
        assert_eq!(std::fs::read_link(&default).unwrap(), other);
    }

    #[test]
    fn test_setup_game_saves_empty_savefiles() {
        let result = setup_game_saves(&[], &[], "123", "/tmp", None);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_setup_game_saves_wine_creates_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().join("saves_dir");
        let prefix = tmp.path().join("prefix");
        let steamuser = prefix.join("drive_c/users/steamuser/AppData/Roaming");
        std::fs::create_dir_all(&steamuser).unwrap();

        let savefiles = vec![UfsSaveFile {
            path: "SEGA/P5R/Steam/{64BitSteamID}".to_string(),
            root: "WinAppDataRoaming".to_string(),
            recursive: false,
        }];

        let count = setup_game_saves(
            &savefiles,
            &[],
            "123456",
            save_dir.to_str().unwrap(),
            Some(prefix.to_str().unwrap()),
        );

        assert_eq!(count, 1);
        let symlink = steamuser.join("SEGA/P5R/Steam");
        assert!(symlink.is_symlink());
        let target = std::fs::read_link(&symlink).unwrap();
        assert!(target.ends_with("saves/123456/SEGA/P5R/Steam"));
    }

    #[test]
    fn test_setup_game_saves_linux_with_rootoverride() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().join("saves_dir");
        std::fs::create_dir_all(&save_dir).unwrap();

        let savefiles = vec![UfsSaveFile {
            path: "SUPERHOT_Team/SHMCD".to_string(),
            root: "WinAppDataLocalLow".to_string(),
            recursive: false,
        }];

        let rootoverrides = vec![UfsRootOverride {
            os: "Linux".to_string(),
            root: "WinAppDataLocalLow".to_string(),
            useinstead: "LinuxHome".to_string(),
            addpath: String::new(),
            pathtransforms: vec![ira_models::UfsPathTransform {
                find: "SUPERHOT_Team/SHMCD".to_string(),
                replace: ".config/unity3d/SUPERHOT_Team/SHMCD".to_string(),
            }],
        }];

        let _home = HomeGuard::set(tmp.path());

        let count = setup_game_saves(
            &savefiles,
            &rootoverrides,
            "123456",
            save_dir.to_str().unwrap(),
            None,
        );

        assert_eq!(count, 1);
        let symlink = tmp.path().join(".config/unity3d/SUPERHOT_Team/SHMCD");
        assert!(symlink.is_symlink());
    }

    #[test]
    fn test_setup_game_saves_skips_gameinstall_for_wine() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().join("saves_dir");
        let prefix = tmp.path().join("prefix");

        let savefiles = vec![UfsSaveFile {
            path: "/".to_string(),
            root: "gameinstall".to_string(),
            recursive: false,
        }];

        let count = setup_game_saves(
            &savefiles,
            &[],
            "123456",
            save_dir.to_str().unwrap(),
            Some(prefix.to_str().unwrap()),
        );

        assert_eq!(count, 0);
    }

    #[test]
    fn test_saves_are_centralized_false_when_no_centralized_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let savefiles = vec![UfsSaveFile {
            path: "SUPERHOT_Team/SHMCD".to_string(),
            root: "WinAppDataLocalLow".to_string(),
            recursive: false,
        }];
        assert!(!saves_are_centralized(
            &savefiles,
            &[],
            "123456",
            tmp.path().to_str().unwrap(),
            None,
        ));
    }

    #[test]
    fn test_saves_are_centralized_true_after_setup() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().join("saves_dir");
        let prefix = tmp.path().join("prefix");
        let steamuser = prefix.join("drive_c/users/steamuser/AppData/Roaming");
        std::fs::create_dir_all(&steamuser).unwrap();

        let savefiles = vec![UfsSaveFile {
            path: "SEGA/P5R/Steam/{64BitSteamID}".to_string(),
            root: "WinAppDataRoaming".to_string(),
            recursive: false,
        }];

        let save_dir_str = save_dir.to_str().unwrap();
        let prefix_str = prefix.to_str().unwrap();
        let count = setup_game_saves(&savefiles, &[], "123456", save_dir_str, Some(prefix_str));
        assert_eq!(count, 1);

        assert!(saves_are_centralized(
            &savefiles,
            &[],
            "123456",
            save_dir_str,
            Some(prefix_str)
        ));
    }

    #[test]
    fn test_saves_are_centralized_false_when_default_is_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().join("saves_dir");
        let default = tmp.path().join("default").join("Saves");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(default.join("save.dat"), b"data").unwrap();

        // Centralized dir exists with data too, but the default path is a
        // real directory (not a symlink) — not centralized yet.
        let centralized = save_dir
            .join("saves")
            .join("123456")
            .join("default")
            .join("Saves");
        std::fs::create_dir_all(&centralized).unwrap();
        std::fs::write(centralized.join("save.dat"), b"data").unwrap();

        let savefiles = vec![UfsSaveFile {
            path: "default/Saves".to_string(),
            root: "gameinstall".to_string(),
            recursive: false,
        }];

        let rootoverrides = vec![UfsRootOverride {
            os: "Linux".to_string(),
            root: "gameinstall".to_string(),
            useinstead: "LinuxHome".to_string(),
            addpath: String::new(),
            pathtransforms: vec![ira_models::UfsPathTransform {
                find: "default/Saves".to_string(),
                replace: "default/Saves".to_string(),
            }],
        }];

        let _home = HomeGuard::set(tmp.path());

        assert!(!saves_are_centralized(
            &savefiles,
            &rootoverrides,
            "123456",
            save_dir.to_str().unwrap(),
            None,
        ));
    }

    #[test]
    fn test_saves_are_centralized_true_when_data_in_centralized_but_default_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().join("saves_dir");
        let centralized = save_dir
            .join("saves")
            .join("123456")
            .join("Game")
            .join("Saves");
        std::fs::create_dir_all(&centralized).unwrap();
        std::fs::write(centralized.join("save.dat"), b"data").unwrap();

        let savefiles = vec![UfsSaveFile {
            path: "Game/Saves".to_string(),
            root: "gameinstall".to_string(),
            recursive: false,
        }];

        let rootoverrides = vec![UfsRootOverride {
            os: "Linux".to_string(),
            root: "gameinstall".to_string(),
            useinstead: "LinuxHome".to_string(),
            addpath: String::new(),
            pathtransforms: vec![ira_models::UfsPathTransform {
                find: "Game/Saves".to_string(),
                replace: "Game/Saves".to_string(),
            }],
        }];

        let _home = HomeGuard::set(tmp.path());

        assert!(saves_are_centralized(
            &savefiles,
            &rootoverrides,
            "123456",
            save_dir.to_str().unwrap(),
            None,
        ));
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
    fn test_prefix_user_dirs_empty_prefix_creates_nothing() {
        let cwd = std::env::current_dir().unwrap();
        let users = prefix_user_dirs("");
        assert!(users.is_empty());
        assert!(!cwd.join("drive_c").exists());
    }

    #[test]
    fn test_prefix_user_dirs_fallback_creates_steamuser() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("prefix");
        std::fs::create_dir_all(prefix.join("drive_c/users")).unwrap();

        let users = prefix_user_dirs(prefix.to_str().unwrap());

        assert_eq!(users.len(), 1);
        let steamuser = &users[0];
        assert!(steamuser.ends_with("steamuser"));
        assert!(steamuser.is_dir());
        assert!(steamuser.join("AppData/Roaming").is_dir());
    }
}
