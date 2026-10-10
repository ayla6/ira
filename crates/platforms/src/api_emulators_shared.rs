use std::path::{Path, PathBuf};

/// Compute a relative path from `from_dir` to `to_path` (used for the
/// per-game settings symlinks).
#[cfg(unix)]
pub(crate) fn compute_relative(from_dir: &Path, to_path: &Path) -> PathBuf {
    use std::path::Component;

    let from_components: Vec<_> = from_dir.components().collect();
    let to_components: Vec<_> = to_path.components().collect();

    let mut common = 0;
    while common < from_components.len()
        && common < to_components.len()
        && from_components[common] == to_components[common]
    {
        common += 1;
    }

    let up = from_components.len() - common;
    let mut result = PathBuf::new();
    for _ in 0..up {
        result.push("..");
    }
    for comp in &to_components[common..] {
        if let Component::Normal(s) = comp {
            result.push(s);
        }
    }
    result
}

pub(crate) fn backup_file(path: &Path) -> Result<(), String> {
    let bak = path.with_extension(format!(
        "{}.bak",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    if !bak.exists() && path.exists() {
        std::fs::rename(path, &bak).map_err(|e| format!("backup failed: {}", e))?;
    }
    Ok(())
}

pub(crate) fn find_file_case_insensitive(dir: &Path, name: &str) -> Option<PathBuf> {
    let lower = name.to_lowercase();
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .find(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|entry_name| entry_name.to_lowercase() == lower)
        })
        .map(|entry| entry.path())
}

pub(crate) fn normalize_file_name(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let canonical = dir.join(name);
    if canonical.exists() {
        return Ok(canonical);
    }
    if let Some(actual) = find_file_case_insensitive(dir, name) {
        std::fs::rename(&actual, &canonical)
            .map_err(|e| format!("normalize {}: {e}", actual.display()))?;
    }
    Ok(canonical)
}

pub(crate) fn prepare_original_file(
    dir: &Path,
    source_name: &str,
    destination_name: &str,
) -> Result<Option<PathBuf>, String> {
    let name = if find_file_case_insensitive(dir, destination_name).is_some() {
        destination_name
    } else if find_file_case_insensitive(dir, source_name).is_some() {
        source_name
    } else {
        return Ok(None);
    };
    normalize_file_name(dir, name).map(Some)
}

/// Restore the original file behind `path` from whichever backup variant
/// exists (ours: `.dll.bak`; other tools: `.bak.dll`, `.owo`, `_o.dll`).
/// Our own `.ext.bak` wins when several variants coexist, since it holds
/// what we replaced last.
pub(crate) fn restore_backup(path: &Path) -> Result<(), String> {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Ok(());
    };
    for variant in backup_variants(name) {
        let bak = path.with_file_name(variant);
        if bak.exists() {
            if path.exists() {
                std::fs::remove_file(path).map_err(|e| format!("remove emu file: {}", e))?;
            }
            std::fs::rename(&bak, path).map_err(|e| format!("restore backup: {}", e))?;
            break;
        }
    }
    Ok(())
}

pub(crate) fn copy_file(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::copy(src, dst).map_err(|e| format!("copy {:?} \u{2192} {:?}: {}", src, dst, e))?;
    Ok(())
}

pub fn api_emulators_dir(save_dir: &str) -> PathBuf {
    Path::new(save_dir).join("api_emulators")
}

/// List Denuvo API emulator `.so` files directly under `api_emulators/denuvo/`.
/// Returns sorted filenames; empty when the directory is missing/empty.
pub fn list_denuvo_versions(save_dir: &str) -> Vec<String> {
    let root = api_emulators_dir(save_dir).join("denuvo");
    let mut versions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                && entry.path().extension().and_then(|e| e.to_str()) == Some("so")
            {
                if let Some(name) = entry.file_name().to_str() {
                    versions.push(name.to_string());
                }
            }
        }
    }
    versions.sort();
    versions
}

pub(crate) fn detect_arch(game_exe: &str) -> &'static str {
    if is_windows(game_exe) {
        let is64 = game_exe.contains("64")
            || std::fs::metadata(game_exe)
                .map(|m| m.len() > 1_500_000)
                .unwrap_or(false);
        if is64 { "x64" } else { "x86" }
    } else if std::env::consts::ARCH == "x86_64" {
        "x64"
    } else {
        "x86"
    }
}

pub(crate) fn is_windows(game_exe: &str) -> bool {
    Path::new(game_exe)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("bat"))
}

/// Detect bitness from the DLLs actually present in `dir`, most specific
/// signal first (file names compared case-insensitively): any `names_64`
/// file → 64-bit; else a 64-marked folder name (`Win64`, `x86_64`, `bin64`)
/// → 64-bit even for unsuffixed names like `steam_api.dll`/`libsteam_api.so`;
/// else any `names_32` file → 32-bit; otherwise `None` (callers fall back to
/// exe heuristics).
pub(crate) fn detect_folder_bitness(
    dir: &Path,
    names_64: &[&str],
    names_32: &[&str],
) -> Option<bool> {
    let files = dir_file_names_lower(dir)?;
    if names_64.iter().any(|n| files.contains(&n.to_lowercase())) {
        return Some(true);
    }
    if folder_name_says_64(dir) {
        return Some(true);
    }
    if names_32.iter().any(|n| files.contains(&n.to_lowercase())) {
        return Some(false);
    }
    None
}

/// Lowercased file names directly inside `dir`, or `None` when unreadable.
fn dir_file_names_lower(dir: &Path) -> Option<std::collections::HashSet<String>> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut names = std::collections::HashSet::new();
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            if let Some(name) = entry.file_name().to_str() {
                names.insert(name.to_lowercase());
            }
        }
    }
    Some(names)
}

/// Whether `dir`'s own name or its parent's name marks it as 64-bit:
/// ends in "64" (`Win64`, `x86_64`, `bin64`) or contains `64bit`.
fn folder_name_says_64(dir: &Path) -> bool {
    [dir.file_name(), dir.parent().and_then(Path::file_name)]
        .into_iter()
        .flatten()
        .any(|c| {
            c.to_str()
                .map(|s| {
                    let s = s.to_lowercase();
                    s.ends_with("64") || s.contains("64bit")
                })
                .unwrap_or(false)
        })
}

pub(crate) fn find_api_emu_dll_folder(game_exe: &str, dll_names: &[&str]) -> Option<PathBuf> {
    let start = Path::new(game_exe).parent()?;
    if dir_has_any_dll(start, dll_names) {
        return Some(start.to_path_buf());
    }
    if let Ok(entries) = std::fs::read_dir(start) {
        for entry in entries.flatten() {
            let sub = entry.path();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && dir_has_any_dll(&sub, dll_names)
            {
                return Some(sub);
            }
        }
    }
    None
}

/// Case-insensitive check whether `dir` directly contains any of `dll_names`.
/// Both sides are lowercased, so mixed-case lists like `Galaxy.dll` match.
pub(crate) fn dir_has_any_dll(dir: &Path, dll_names: &[&str]) -> bool {
    let lower: Vec<String> = dll_names.iter().map(|n| n.to_lowercase()).collect();
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| lower.contains(&name.to_lowercase()))
        })
}

/// Recursively walk `base_folder` and return every directory that directly
/// contains at least one of `dll_names` (case-insensitive). Iterative
/// depth-first search — no recursion limit concerns.
pub fn find_dll_dirs_recursive(base_folder: &Path, dll_names: &[&str]) -> Vec<PathBuf> {
    let lower_names: Vec<String> = dll_names.iter().map(|s| s.to_lowercase()).collect();
    let mut results = Vec::new();
    let mut stack = vec![base_folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut found_here = false;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        stack.push(entry.path());
                    } else if !found_here {
                        if let Some(name) = entry.file_name().to_str() {
                            if lower_names.contains(&name.to_lowercase()) {
                                found_here = true;
                            }
                        }
                    }
                }
            }
        }
        if found_here {
            results.push(dir);
        }
    }
    results
}

/// Find the directory containing one of `dll_names` inside a game install.
///
/// Tries the shallow exe-relative scan first (fast path for games whose DLLs
/// sit next to or one level under the exe), then falls back to a recursive
/// scan of `game_folder`. Prefers the deepest match — for nested installs
/// (e.g. Unreal Engine games) the API DLLs live several levels below the exe.
pub(crate) fn find_game_dll_folder(
    game_exe: &str,
    game_folder: &str,
    dll_names: &[&str],
) -> Option<PathBuf> {
    let extension = if is_windows(game_exe) { ".dll" } else { ".so" };
    let platform_names: Vec<&str> = dll_names
        .iter()
        .copied()
        .filter(|name| name.ends_with(extension))
        .collect();
    let names = if platform_names.is_empty() {
        dll_names
    } else {
        &platform_names
    };
    if let Some(folder) = find_api_emu_dll_folder(game_exe, names) {
        return Some(folder);
    }
    if game_folder.is_empty() {
        return None;
    }
    let mut dirs = find_dll_dirs_recursive(Path::new(game_folder), names);
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    dirs.into_iter().next()
}

/// Backup filename variants for a given DLL name — our own backups plus the
/// rename conventions other tools use for the stashed original.
/// For `steam_api64.dll` returns:
///   `steam_api64.dll.bak`, `steam_api64.bak.dll`,
///   `steam_api64.owo.dll`, `steam_api64.dll.owo`,
///   `steam_api64_o.dll`
fn backup_variants(dll_name: &str) -> Vec<String> {
    let (stem, ext) = match dll_name.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => return Vec::new(),
    };
    vec![
        format!("{}.{}.bak", stem, ext),
        format!("{}.bak.{}", stem, ext),
        format!("{}.owo.{}", stem, ext),
        format!("{}.{}.owo", stem, ext),
        format!("{}_o.{}", stem, ext),
    ]
}

/// Check whether `dir` contains any emulator backup file for the given DLL
/// names. Returns true if the API setup appears to already be patched —
/// either by ira (originals backed up as `.dll.bak`, `.bak.dll`, `.owo.dll`,
/// `.dll.owo`) or by another tool that renamed originals to `_o.dll`.
/// Such directories are treated as already handled: no new install is
/// attempted, and uninstall restores the stashed original.
pub fn has_emulator_backups(dir: &Path, dll_names: &[&str]) -> bool {
    for dll in dll_names {
        for variant in backup_variants(dll) {
            if dir.join(&variant).exists() {
                return true;
            }
        }
    }
    false
}

pub fn ensure_skeleton(save_dir: &str) {
    let root = api_emulators_dir(save_dir);
    let dirs = ["steam", "gog"];
    for d in &dirs {
        let _ = std::fs::create_dir_all(root.join(d));
    }
}

/// Sorted names of subdirectories directly under `root`, skipping any in
/// `exclude`. Shared by the GSE/NGE/Denuvo version listers.
pub(crate) fn list_version_dirs(root: &Path, exclude: &[&str]) -> Vec<String> {
    let mut versions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    if !exclude.contains(&name) {
                        versions.push(name.to_string());
                    }
                }
            }
        }
    }
    versions.sort();
    versions
}

/// Resolve an emulator version directory under
/// `<save_dir>/api_emulators/<subdir>/`. An empty `version` picks the first
/// (sorted) version available.
pub(crate) fn resolve_emu_version(
    save_dir: &str,
    subdir: &str,
    version: &str,
    label: &str,
) -> Result<PathBuf, String> {
    let root = api_emulators_dir(save_dir).join(subdir);
    if version.is_empty() {
        let versions = list_version_dirs(&root, &[]);
        let v = versions
            .first()
            .ok_or_else(|| format!("No {label} versions found at {root:?}"))?;
        Ok(root.join(v))
    } else {
        let dir = root.join(version);
        if !dir.is_dir() {
            return Err(format!("{label} version '{version}' not found at {dir:?}"));
        }
        Ok(dir)
    }
}

/// Recursively search for a settings directory called `name` under `root`.
pub(crate) fn find_settings_recursive(root: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().and_then(|n| n.to_str()) == Some(name) {
                        return Some(path);
                    }
                    stack.push(path);
                }
            }
        }
    }
    None
}

pub(crate) fn find_settings_from_exe(
    game_exe: &Path,
    game_root: &Path,
    name: &str,
) -> Option<PathBuf> {
    let root = if game_root.as_os_str().is_empty() {
        game_exe.parent()?
    } else {
        game_root
    };
    let mut current = game_exe.parent()?.to_path_buf();
    while current.starts_with(root) {
        let candidate = current.join(name);
        if candidate.is_dir() {
            return Some(candidate);
        }
        if current == root || !current.pop() {
            break;
        }
    }
    find_settings_recursive(root, name)
}

#[cfg(unix)]
fn merge_settings_contents(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| format!("create settings target: {e}"))?;
    for entry in std::fs::read_dir(source)
        .map_err(|e| format!("read settings directory {}: {e}", source.display()))?
    {
        let entry = entry.map_err(|e| format!("read settings entry: {e}"))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let source_meta = std::fs::symlink_metadata(&source_path)
            .map_err(|e| format!("stat settings entry {}: {e}", source_path.display()))?;
        let target_meta = std::fs::symlink_metadata(&target_path).ok();
        match target_meta {
            None => std::fs::rename(&source_path, &target_path)
                .map_err(|e| format!("move settings entry {}: {e}", source_path.display()))?,
            Some(target_meta) if source_meta.is_dir() && target_meta.is_dir() => {
                merge_settings_contents(&source_path, &target_path)?;
                if std::fs::read_dir(&source_path)
                    .map_err(|e| format!("read merged settings: {e}"))?
                    .next()
                    .is_none()
                {
                    std::fs::remove_dir(&source_path)
                        .map_err(|e| format!("remove merged settings: {e}"))?;
                }
            }
            Some(_) => remove_settings_path(&source_path)?,
        }
    }
    Ok(())
}

#[cfg(unix)]
fn remove_settings_path(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(path).map_err(|e| format!("remove {}: {e}", path.display()))?;
    } else if metadata.is_dir() {
        std::fs::remove_dir_all(path).map_err(|e| format!("remove {}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn replace_settings_with_link(link: &Path, target: &Path) -> Result<(), String> {
    if link == target {
        return Ok(());
    }
    if let Ok(metadata) = std::fs::symlink_metadata(link) {
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            merge_settings_contents(link, target)?;
        }
        remove_settings_path(link)?;
    }
    let parent = link
        .parent()
        .ok_or_else(|| format!("settings link has no parent: {}", link.display()))?;
    let relative = compute_relative(parent, target);
    std::os::unix::fs::symlink(relative, link)
        .map_err(|e| format!("link {} to {}: {e}", link.display(), target.display()))
}

/// Point every directory containing one of `dll_names` under `root` at the
/// canonical settings directory.
pub(crate) fn symlink_dll_dirs_to_settings(
    root: &Path,
    settings_dir: &Path,
    settings_name: &str,
    dll_names: &[&str],
) -> Result<(), String> {
    for dir in find_dll_dirs_recursive(root, dll_names) {
        let link = dir.join(settings_name);
        if link == settings_dir {
            continue;
        }
        #[cfg(unix)]
        replace_settings_with_link(&link, settings_dir)?;
    }
    Ok(())
}

/// Shared body of `centralize_steam_settings` / `centralize_galaxy_settings`:
/// move discovered settings to the game root, link the old location back to
/// the root, then point every DLL directory at the same canonical directory.
pub(crate) fn centralize_settings(
    game_folder: &str,
    settings_name: &str,
    dll_names: &[&str],
    game_exe: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let root = Path::new(game_folder);
    let root_settings = root.join(settings_name);

    if root_settings.is_dir() {
        symlink_dll_dirs_to_settings(root, &root_settings, settings_name, dll_names)?;
        return Ok(Some(root_settings));
    }

    let found_dir = game_exe
        .and_then(|exe| find_settings_from_exe(exe, root, settings_name))
        .filter(|path| path.starts_with(root))
        .or_else(|| find_settings_recursive(root, settings_name));
    if let Some(found_dir) = found_dir {
        #[cfg(unix)]
        {
            let target = if std::fs::symlink_metadata(&found_dir)
                .map_err(|e| format!("stat settings directory: {e}"))?
                .file_type()
                .is_symlink()
            {
                std::fs::canonicalize(&found_dir)
                    .map_err(|e| format!("resolve settings directory: {e}"))?
            } else {
                std::fs::rename(&found_dir, &root_settings)
                    .map_err(|e| format!("move settings to root: {e}"))?;
                root_settings.clone()
            };
            if target != root_settings {
                replace_settings_with_link(&root_settings, &target)?;
            }
            if found_dir != root_settings {
                replace_settings_with_link(&found_dir, &root_settings)?;
            }
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&root_settings)
            .map_err(|e| format!("create {settings_name} at root: {e}"))?;
        symlink_dll_dirs_to_settings(root, &root_settings, settings_name, dll_names)?;
        return Ok(Some(root_settings));
    }

    std::fs::create_dir_all(&root_settings)
        .map_err(|e| format!("create {settings_name} at root: {e}"))?;
    symlink_dll_dirs_to_settings(root, &root_settings, settings_name, dll_names)?;
    Ok(Some(root_settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_variants_dll() {
        let v = backup_variants("steam_api64.dll");
        assert_eq!(
            v,
            vec![
                "steam_api64.dll.bak".to_string(),
                "steam_api64.bak.dll".to_string(),
                "steam_api64.owo.dll".to_string(),
                "steam_api64.dll.owo".to_string(),
                "steam_api64_o.dll".to_string(),
            ]
        );
    }

    #[test]
    fn test_backup_variants_so() {
        let v = backup_variants("libsteam_api.so");
        assert_eq!(
            v,
            vec![
                "libsteam_api.so.bak".to_string(),
                "libsteam_api.bak.so".to_string(),
                "libsteam_api.owo.so".to_string(),
                "libsteam_api.so.owo".to_string(),
                "libsteam_api_o.so".to_string(),
            ]
        );
    }

    #[test]
    fn test_backup_variants_no_extension() {
        let v = backup_variants("noext");
        assert!(v.is_empty());
    }

    #[test]
    fn test_has_emulator_backups_detects_all_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        for name in &[
            "steam_api64.dll.bak",
            "steam_api64.bak.dll",
            "steam_api64.owo.dll",
            "steam_api64_o.dll",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
            assert!(
                has_emulator_backups(dir, &["steam_api64.dll"]),
                "failed for {}",
                name
            );
            std::fs::remove_file(dir.join(name)).unwrap();
        }
        assert!(!has_emulator_backups(dir, &["steam_api64.dll"]));
    }

    #[test]
    fn test_restore_backup_restores_o_variant() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("steam_api64_o.dll"), b"original").unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"emulator").unwrap();

        restore_backup(&dir.join("steam_api64.dll")).unwrap();

        assert!(!dir.join("steam_api64_o.dll").exists());
        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn test_restore_backup_prefers_own_bak_over_o() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // GSE installed on top of a crack that had renamed the original to _o:
        // our .bak holds the crack wrapper, the _o file must stay untouched.
        std::fs::write(dir.join("steam_api64_o.dll"), b"crack-original").unwrap();
        std::fs::write(dir.join("steam_api64.dll.bak"), b"wrapper").unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"emulator").unwrap();

        restore_backup(&dir.join("steam_api64.dll")).unwrap();

        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"wrapper"
        );
        assert!(dir.join("steam_api64_o.dll").exists());
    }

    #[test]
    fn test_restore_backup_noop_without_backups() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("steam_api64.dll"), b"emulator").unwrap();

        restore_backup(&dir.join("steam_api64.dll")).unwrap();

        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"emulator"
        );
    }

    #[test]
    fn test_detect_folder_bitness_prefers_64_when_both_present() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("steam_api.dll"), b"x").unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"x").unwrap();
        assert_eq!(
            detect_folder_bitness(
                dir,
                &["steam_api64.dll", "libsteam_api64.so"],
                &["steam_api.dll", "libsteam_api.so"]
            ),
            Some(true)
        );
    }

    #[test]
    fn test_detect_folder_bitness_32() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("steam_api.dll"), b"x").unwrap();
        assert_eq!(
            detect_folder_bitness(dir, &["steam_api64.dll"], &["steam_api.dll"]),
            Some(false)
        );
    }

    #[test]
    fn test_detect_folder_bitness_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            detect_folder_bitness(tmp.path(), &["steam_api64.dll"], &["steam_api.dll"]),
            None
        );
    }

    #[test]
    fn test_detect_folder_bitness_uses_folder_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("win64");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_api.dll"), b"x").unwrap();
        assert_eq!(
            detect_folder_bitness(&dir, &["steam_api64.dll"], &["steam_api.dll"]),
            Some(true)
        );
    }

    #[test]
    fn test_detect_folder_bitness_uses_parent_folder_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("x86_64").join("bin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("libsteam_api.so"), b"x").unwrap();
        assert_eq!(
            detect_folder_bitness(&dir, &["libsteam_api64.so"], &["libsteam_api.so"]),
            Some(true)
        );
    }

    #[test]
    fn test_detect_folder_bitness_folder_name_beats_unsuffixed_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("win64");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_api.dll"), b"x").unwrap();
        assert_eq!(
            detect_folder_bitness(
                &dir,
                &["steam_api64.dll", "libsteam_api64.so"],
                &["steam_api.dll", "libsteam_api.so"]
            ),
            Some(true)
        );
    }

    #[test]
    fn test_detect_folder_bitness_32_folder_stays_32() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("win32");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_api.dll"), b"x").unwrap();
        assert_eq!(
            detect_folder_bitness(&dir, &["steam_api64.dll"], &["steam_api.dll"]),
            Some(false)
        );
    }

    #[test]
    fn test_find_dll_dirs_recursive_finds_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // bin/win64/steam_api64.dll
        let nested = root.join("bin").join("win64");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("steam_api64.dll"), b"x").unwrap();
        // root also has a libsteam_api.so
        std::fs::write(root.join("libsteam_api.so"), b"x").unwrap();

        let found = find_dll_dirs_recursive(root, &["steam_api64.dll", "libsteam_api.so"]);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&root.to_path_buf()));
        assert!(found.contains(&nested));
    }

    #[test]
    fn test_find_dll_dirs_recursive_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("Steam_API64.DLL"), b"x").unwrap();
        let found = find_dll_dirs_recursive(root, &["steam_api64.dll"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], root);
    }

    #[test]
    fn test_normalize_file_name_renames_mixed_case_file() {
        let tmp = tempfile::tempdir().unwrap();
        let actual = tmp.path().join("GALAXY64.DLL");
        let canonical = tmp.path().join("Galaxy64.dll");
        std::fs::write(&actual, b"original").unwrap();

        assert_eq!(
            normalize_file_name(tmp.path(), "Galaxy64.dll").unwrap(),
            canonical
        );
        assert!(!actual.exists());
        assert_eq!(std::fs::read(canonical).unwrap(), b"original");
    }

    #[test]
    fn test_find_dll_dirs_recursive_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let found = find_dll_dirs_recursive(tmp.path(), &["steam_api64.dll"]);
        assert!(found.is_empty());
    }

    #[test]
    fn test_list_denuvo_versions_lists_only_so_files() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path();
        let root = api_emulators_dir(&save_dir.to_string_lossy()).join("denuvo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("denuvo-1.so"), b"x").unwrap();
        std::fs::write(root.join("denuvo-2.so"), b"x").unwrap();
        std::fs::write(root.join("walton.dll"), b"x").unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();

        let versions = list_denuvo_versions(&save_dir.to_string_lossy());
        assert_eq!(
            versions,
            vec!["denuvo-1.so".to_string(), "denuvo-2.so".to_string()]
        );
    }

    #[test]
    fn test_list_denuvo_versions_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let versions = list_denuvo_versions(&tmp.path().to_string_lossy());
        assert!(versions.is_empty());
    }

    #[test]
    fn test_is_windows_by_extension() {
        assert!(is_windows("game.exe"));
        assert!(is_windows("setup.bat"));
        assert!(is_windows("GAME.EXE"));
        assert!(is_windows("SETUP.BAT"));
        assert!(!is_windows("game.x86_64"));
        assert!(!is_windows("game.sh"));
    }

    #[test]
    fn test_detect_arch_windows_heuristic() {
        assert_eq!(detect_arch("game64.exe"), "x64");
        assert_eq!(detect_arch("game.exe"), "x86");
        assert_eq!(detect_arch("GAME64.EXE"), "x64");
    }

    #[test]
    fn test_backup_and_copy_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("emu.dll");
        std::fs::write(&src, b"emu").unwrap();
        let dir = tmp.path().join("game");
        std::fs::create_dir_all(&dir).unwrap();
        let dst = dir.join("steam_api64.dll");
        std::fs::write(&dst, b"orig").unwrap();

        copy_file(&src, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"emu");

        backup_file(&dst).unwrap();
        assert!(dir.join("steam_api64.dll.bak").exists());
    }

    #[test]
    fn test_find_game_dll_folder_prefers_deepest_match() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let shallow = root.join("bin");
        let deep = root.join("game").join("Binaries").join("Win64");
        std::fs::create_dir_all(&shallow).unwrap();
        std::fs::create_dir_all(&deep).unwrap();
        let exe = root.join("game").join("launcher.exe");
        std::fs::write(&exe, b"x").unwrap();
        std::fs::write(shallow.join("steam_api.dll"), b"x").unwrap();
        std::fs::write(deep.join("steam_api64.dll"), b"x").unwrap();

        let found = find_game_dll_folder(
            &exe.to_string_lossy(),
            &root.to_string_lossy(),
            &["steam_api.dll", "steam_api64.dll"],
        )
        .unwrap();
        assert_eq!(found, deep);
    }

    #[test]
    fn test_find_game_dll_folder_matches_executable_platform() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let windows = root.join("0-Windows");
        let linux = root.join("1-Linux");
        std::fs::create_dir_all(&windows).unwrap();
        std::fs::create_dir_all(&linux).unwrap();
        std::fs::write(windows.join("steam_api64.dll"), b"x").unwrap();
        std::fs::write(linux.join("libsteam_api.so"), b"x").unwrap();
        let exe = root.join("money_for_girls.sh");
        std::fs::write(&exe, b"x").unwrap();

        let found = find_game_dll_folder(
            &exe.to_string_lossy(),
            &root.to_string_lossy(),
            &["steam_api64.dll", "libsteam_api.so"],
        )
        .unwrap();

        assert_eq!(found, linux);
    }

    #[test]
    fn test_find_game_dll_folder_accepts_uppercase_windows_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let dll_dir = root.join("Windows");
        std::fs::create_dir_all(&dll_dir).unwrap();
        std::fs::write(dll_dir.join("steam_api64.dll"), b"x").unwrap();
        let exe = root.join("Game.EXE");
        std::fs::write(&exe, b"x").unwrap();

        let found = find_game_dll_folder(
            &exe.to_string_lossy(),
            &root.to_string_lossy(),
            &["steam_api64.dll", "libsteam_api.so"],
        )
        .unwrap();

        assert_eq!(found, dll_dir);
    }

    #[test]
    fn test_find_api_emu_dll_folder_matches_mixed_case_list() {
        // NGE lists carry `Galaxy.dll` casing; a lowercase file on disk
        // must still match.
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("game.exe");
        std::fs::write(&exe, b"x").unwrap();
        std::fs::write(tmp.path().join("galaxy.dll"), b"x").unwrap();
        let found =
            find_api_emu_dll_folder(&exe.to_string_lossy(), &["Galaxy.dll", "Galaxy64.dll"]);
        assert_eq!(found, Some(tmp.path().to_path_buf()));
    }

    #[test]
    fn test_list_version_dirs_skips_excluded() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["b", "a", "generate_interfaces"] {
            std::fs::create_dir_all(tmp.path().join(name)).unwrap();
        }
        assert_eq!(
            list_version_dirs(tmp.path(), &["generate_interfaces"]),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn test_resolve_emu_version_picks_first_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let save = tmp.path().join("save");
        let root = api_emulators_dir(&save.to_string_lossy()).join("steam");
        std::fs::create_dir_all(root.join("v2")).unwrap();
        std::fs::create_dir_all(root.join("v1")).unwrap();
        let save_str = save.to_string_lossy().into_owned();
        assert_eq!(
            resolve_emu_version(&save_str, "steam", "", "GSE").unwrap(),
            root.join("v1")
        );
        assert_eq!(
            resolve_emu_version(&save_str, "steam", "v2", "GSE").unwrap(),
            root.join("v2")
        );
        assert!(resolve_emu_version(&save_str, "steam", "missing", "GSE").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_compute_relative_sibling_dirs() {
        let from = Path::new("/games/title/bin");
        let to = Path::new("/games/title/steam_settings");
        assert_eq!(
            compute_relative(from, to),
            PathBuf::from("../steam_settings")
        );
    }

    #[test]
    fn test_find_settings_recursive_finds_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("steam_settings");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            find_settings_recursive(tmp.path(), "steam_settings"),
            Some(nested)
        );
        assert_eq!(find_settings_recursive(tmp.path(), "missing"), None);
    }

    #[cfg(unix)]
    #[test]
    fn test_centralize_settings_moves_nested_config_to_root_and_links_all_dll_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("game");
        let linux = root.join("Linux");
        let windows = root.join("Windows");
        let old_settings = windows.join("steam_settings");
        std::fs::create_dir_all(&linux).unwrap();
        std::fs::create_dir_all(&old_settings).unwrap();
        std::fs::write(linux.join("libsteam_api.so"), b"x").unwrap();
        std::fs::write(windows.join("steam_api64.dll"), b"x").unwrap();
        std::fs::write(old_settings.join("steam_appid.txt"), b"123").unwrap();
        let exe = linux.join("game.sh");
        std::fs::write(&exe, b"x").unwrap();

        let result = centralize_settings(
            &root.to_string_lossy(),
            "steam_settings",
            &["steam_api64.dll", "libsteam_api.so"],
            Some(&exe),
        )
        .unwrap();

        let root_settings = root.join("steam_settings");
        assert_eq!(result, Some(root_settings.clone()));
        assert!(root_settings.is_dir());
        assert!(!std::fs::symlink_metadata(&root_settings)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read(root_settings.join("steam_appid.txt")).unwrap(),
            b"123"
        );
        assert!(std::fs::symlink_metadata(&old_settings)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::symlink_metadata(linux.join("steam_settings"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::symlink_metadata(windows.join("steam_settings"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn test_centralize_settings_replaces_stale_dll_dir_settings_link() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("game");
        let dll_dir = root.join("bin");
        let stale = dll_dir.join("steam_settings");
        let root_settings = root.join("steam_settings");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::create_dir_all(&root_settings).unwrap();
        std::fs::write(root_settings.join("steam_appid.txt"), b"123").unwrap();
        std::fs::write(dll_dir.join("steam_api64.dll"), b"x").unwrap();
        std::fs::write(stale.join("steam_appid.txt"), b"456").unwrap();

        centralize_settings(
            &root.to_string_lossy(),
            "steam_settings",
            &["steam_api64.dll"],
            Some(&dll_dir.join("game.exe")),
        )
        .unwrap();

        assert!(std::fs::symlink_metadata(&stale)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(stale.join("steam_appid.txt")).unwrap(), b"123");
    }
}
