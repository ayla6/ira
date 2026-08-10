use std::path::{Path, PathBuf};

/// Migrate existing GBE saves to `<save_dir>/emulator_saves/gbe/<appid>/`.
pub fn migrate_gbe_saves(save_dir: &str, app_id: &str, wine_prefix: Option<&str>) {
    let centralized = centralized_gbe_path(save_dir);
    let _ = std::fs::create_dir_all(&centralized);
    let target = centralized.join(app_id);

    for folder_name in &["GSE Saves", "Goldberg SteamEmu Saves"] {
        if let Some(base) = xdg_save_base(folder_name) {
            let source = base.join(app_id);
            migrate_dir(&source, &target);
        }
    }

    if let Some(prefix) = wine_prefix {
        for user_dir in wine_user_dirs(prefix) {
            let roaming = user_dir.join("AppData").join("Roaming");
            for folder_name in &["GSE Saves", "Goldberg SteamEmu Saves"] {
                let source = roaming.join(folder_name).join(app_id);
                migrate_dir(&source, &target);
            }
        }
    }
}

/// Migrate existing NGE saves to `<save_dir>/emulator_saves/nge/`.
pub fn migrate_nge_saves(save_dir: &str, wine_prefix: Option<&str>) {
    let centralized = centralized_nge_path(save_dir);
    let _ = std::fs::create_dir_all(&centralized);

    if let Some(prefix) = wine_prefix {
        for user_dir in wine_user_dirs(prefix) {
            let roaming = user_dir.join("AppData").join("Roaming");
            let source = roaming.join("NemirtingasGalaxyEmu");
            migrate_dir_contents(&source, &centralized);
        }
    }
}

/// Convenience: run both GBE and NGE migration based on trophy source.
pub fn migrate_emulator_saves(
    save_dir: &str,
    trophy_source: ira_models::TrophySource,
    app_id: &str,
    wine_prefix: Option<&str>,
) {
    match trophy_source {
        ira_models::TrophySource::Gse => {
            migrate_gbe_saves(save_dir, app_id, wine_prefix);
        }
        ira_models::TrophySource::Nge => {
            migrate_nge_saves(save_dir, wine_prefix);
        }
        _ => {}
    }
}

fn centralized_gbe_path(save_dir: &str) -> PathBuf {
    Path::new(save_dir).join("emulator_saves").join("gbe")
}

fn centralized_nge_path(save_dir: &str) -> PathBuf {
    Path::new(save_dir).join("emulator_saves").join("nge")
}

fn xdg_save_base(folder_name: &str) -> Option<PathBuf> {
    let xdg = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty());
    let home = std::env::var("HOME").unwrap_or_default();
    let base = match xdg {
        Some(x) => Path::new(&x).join(folder_name),
        None => Path::new(&home).join(".local/share").join(folder_name),
    };
    if base.is_dir() {
        Some(base)
    } else {
        None
    }
}

fn wine_user_dirs(prefix: &str) -> Vec<PathBuf> {
    let users_dir = Path::new(prefix).join("drive_c").join("users");
    let mut result = Vec::new();
    let Ok(entries) = std::fs::read_dir(&users_dir) else {
        return result;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            result.push(path);
        }
    }
    result
}

/// Move a directory to `target` if `source` is a real directory
/// (not a symlink). Files that already exist at `target` are left in place.
/// Uses copy+verify+delete for safety across filesystems.
fn migrate_dir(source: &Path, target: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(source) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    if !source.is_dir() {
        return;
    }
    if target.exists() {
        return;
    }
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let count = safe_migrate_dir_contents(source, target);
    if count > 0 {
        let _ = std::fs::remove_dir_all(source);
    }
}

/// Move contents of `source` into `target` if `source` is a real directory
/// (not a symlink). Files already at `target` are not overwritten.
/// Uses copy+verify+delete for safety.
fn migrate_dir_contents(source: &Path, target: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(source) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    if !source.is_dir() {
        return;
    }
    safe_migrate_dir_contents(source, target);
}

/// Safely copy directory contents recursively, verifying each file copy
/// before deleting the source. Returns number of files migrated.
fn safe_migrate_dir_contents(source: &Path, target: &Path) -> usize {
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
        } else {
            if safe_copy_and_verify(&src, &dst) {
                let _ = std::fs::remove_file(&src);
                count += 1;
            }
        }
    }
    count
}

/// Copy a file and verify the destination matches the source by size.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_dir_moves_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        let target = tmp.path().join("target");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("save.txt"), b"data").unwrap();

        migrate_dir(&source, &target);

        assert!(!source.exists());
        assert!(target.is_dir());
        assert!(target.join("save.txt").exists());
    }

    #[test]
    fn test_migrate_dir_skips_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        let symlink = tmp.path().join("source");
        let target = tmp.path().join("target");
        std::fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &symlink).unwrap();

        migrate_dir(&symlink, &target);

        assert!(symlink.is_symlink());
        assert!(!target.exists());
    }

    #[test]
    fn test_migrate_dir_skips_if_target_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        let target = tmp.path().join("target");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("old.txt"), b"old").unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("existing.txt"), b"existing").unwrap();

        migrate_dir(&source, &target);

        assert!(source.exists());
        assert!(target.join("existing.txt").exists());
    }

    #[test]
    fn test_migrate_dir_skips_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("nonexistent");
        let target = tmp.path().join("target");
        migrate_dir(&source, &target);
        assert!(!target.exists());
    }

    #[test]
    fn test_migrate_dir_contents_moves_files() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("NGE");
        let target = tmp.path().join("centralized");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("file1"), b"1").unwrap();
        std::fs::write(source.join("file2"), b"2").unwrap();

        migrate_dir_contents(&source, &target);

        assert!(target.join("file1").exists());
        assert!(target.join("file2").exists());
        assert!(!source.join("file1").exists());
    }

    #[test]
    fn test_migrate_dir_contents_skips_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        let symlink = tmp.path().join("NGE");
        let target = tmp.path().join("centralized");
        std::fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &symlink).unwrap();

        migrate_dir_contents(&symlink, &target);

        assert!(symlink.is_symlink());
        assert!(!target.exists());
    }

    #[test]
    fn test_migrate_dir_contents_preserves_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("NGE");
        let target = tmp.path().join("centralized");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(source.join("new.txt"), b"new").unwrap();
        std::fs::write(target.join("existing.txt"), b"existing").unwrap();

        migrate_dir_contents(&source, &target);

        assert!(target.join("existing.txt").exists());
        assert!(target.join("new.txt").exists());
    }
}
