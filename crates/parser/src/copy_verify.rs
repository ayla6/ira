use std::path::Path;

/// Safely migrate contents of `source` into `target` by copying each file,
/// verifying the copy matches the original, and only then deleting the source.
/// Files already at `target` are left in place. Subdirectories are copied recursively.
/// Returns the number of files successfully migrated.
pub fn safe_migrate_dir_contents(source: &Path, target: &Path) -> usize {
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
    fn test_safe_migrate_moves_and_removes_source_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.sav"), b"hello").unwrap();
        std::fs::write(src.join("sub").join("b.sav"), b"world").unwrap();

        let count = safe_migrate_dir_contents(&src, &dst);

        assert_eq!(count, 2);
        assert_eq!(std::fs::read(dst.join("a.sav")).unwrap(), b"hello");
        assert_eq!(std::fs::read(dst.join("sub").join("b.sav")).unwrap(), b"world");
        assert!(!src.join("a.sav").exists());
        assert!(!src.join("sub").join("b.sav").exists());
    }

    #[test]
    fn test_safe_migrate_leaves_existing_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.sav"), b"new").unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("a.sav"), b"old").unwrap();

        let count = safe_migrate_dir_contents(&src, &dst);

        assert_eq!(count, 0);
        assert_eq!(std::fs::read(dst.join("a.sav")).unwrap(), b"old");
        assert!(src.join("a.sav").exists());
    }

    #[test]
    fn test_safe_migrate_missing_source_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let count = safe_migrate_dir_contents(&tmp.path().join("nope"), &tmp.path().join("dst"));
        assert_eq!(count, 0);
    }
}
