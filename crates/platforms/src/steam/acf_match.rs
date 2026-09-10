use std::path::{Path, PathBuf};

use super::vdf;

/// Whether `node`'s final segment names `name`, case-insensitively: Steam
/// libraries created on Windows or copied around carry `SteamApps`/`Common`
/// spellings just as often as the lowercase ones.
fn segment_is(node: &Path, name: &str) -> bool {
    node.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(name))
}

/// Given a game install folder, return its parent `steamapps` directory if
/// the path matches the `steamapps/common/<game>` layout (any casing).
pub fn steamapps_in_path(path: &Path) -> Option<PathBuf> {
    let common = path.parent()?;
    if !segment_is(common, "common") {
        return None;
    }
    let steamapps = common.parent()?;
    if segment_is(steamapps, "steamapps") {
        Some(steamapps.to_path_buf())
    } else {
        None
    }
}

/// Lowercased alphanumeric skeleton of a name, so folder names that differ
/// from a manifest only in spacing or punctuation still compare equal.
fn normalize_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// Scan `appmanifest_*.acf` files in `steamapps_dir` and return the
/// `(appid, name)` of the manifest best matching `installdir_name`.
/// An exact (case-insensitive) `installdir` match wins; when the folder was
/// renamed relative to the manifest — release-style names, trimmed
/// punctuation — a normalized comparison of the installdir and then the
/// manifest's own name still identifies it.
pub fn find_appid_for_installdir(
    steamapps_dir: &Path,
    installdir_name: &str,
) -> Option<(String, String)> {
    let entries = std::fs::read_dir(steamapps_dir).ok()?;
    let target = installdir_name.to_lowercase();
    let normalized_target = normalize_name(installdir_name);
    let mut loose: Option<(String, String)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("appmanifest_") || !name_str.ends_with(".acf") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Some(parsed) = vdf::parse_vdf(&text) else {
            continue;
        };
        let appid = vdf::get_str(&parsed, "appid").map(str::to_string);
        let Some(appid) = appid else { continue };
        let title = vdf::get_str(&parsed, "name").unwrap_or("").to_string();
        let installdir = vdf::get_str(&parsed, "installdir").unwrap_or("");
        if installdir.to_lowercase() == target {
            return Some((appid, title));
        }
        if loose.is_none() {
            let matched = normalize_name(installdir) == normalized_target
                || normalize_name(&title) == normalized_target;
            if matched {
                loose = Some((appid, title));
            }
        }
    }
    loose
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_acf(dir: &Path, appid: &str, name: &str, installdir: &str) {
        let content = format!(
            r#""AppState"
{{
    "appid"		"{appid}"
    "name"		"{name}"
    "installdir"		"{installdir}"
    "StateFlags"		"4"
}}"#
        );
        std::fs::write(dir.join(format!("appmanifest_{appid}.acf")), content).unwrap();
    }

    #[test]
    fn test_steamapps_in_path_detects_layout() {
        let path = Path::new("/home/me/steam/steamapps/common/Danganronpa");
        assert_eq!(
            steamapps_in_path(path),
            Some(Path::new("/home/me/steam/steamapps").to_path_buf())
        );
    }

    #[test]
    fn test_steamapps_in_path_accepts_windows_casing() {
        let path = Path::new("/games/SteamApps/Common/Danganronpa");
        assert_eq!(
            steamapps_in_path(path),
            Some(Path::new("/games/SteamApps").to_path_buf())
        );
    }

    #[test]
    fn test_steamapps_in_path_rejects_non_common() {
        assert!(steamapps_in_path(Path::new("/games/MyGame")).is_none());
        let path = Path::new("/x/steamapps/MyGame"); // missing "common" segment
        assert!(steamapps_in_path(path).is_none());
    }

    #[test]
    fn test_find_appid_for_installdir_matches_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        write_acf(
            dir,
            "413410",
            "Danganronpa",
            "Danganronpa Trigger Happy Havoc",
        );
        write_acf(dir, "1687950", "Persona 5 Royal", "P5R");

        let result = find_appid_for_installdir(dir, "danganronpa trigger happy havoc");
        assert_eq!(
            result,
            Some(("413410".to_string(), "Danganronpa".to_string()))
        );
    }

    #[test]
    fn test_find_appid_for_installdir_matches_renamed_folder_normalized() {
        // The folder was renamed ("." and spacing differ from the manifest's
        // installdir): the normalized comparison still identifies it.
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        write_acf(dir, "413410", "Danganronpa", "Danganronpa.Trigger.Happy.Havoc");
        write_acf(dir, "1687950", "Persona 5 Royal", "P5R");

        let result = find_appid_for_installdir(dir, "Danganronpa Trigger Happy Havoc");
        assert_eq!(
            result,
            Some(("413410".to_string(), "Danganronpa".to_string()))
        );
    }

    #[test]
    fn test_find_appid_for_installdir_matches_by_manifest_name() {
        // No installdir similarity at all, but the folder is named after
        // the game itself.
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        write_acf(dir, "413410", "Danganronpa: Trigger Happy Havoc", "DR");
        write_acf(dir, "1687950", "Persona 5 Royal", "P5R");

        let result = find_appid_for_installdir(dir, "Danganronpa Trigger Happy Havoc");
        assert_eq!(
            result,
            Some(("413410".to_string(), "Danganronpa: Trigger Happy Havoc".to_string()))
        );
    }

    #[test]
    fn test_find_appid_for_installdir_exact_beats_loose() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        write_acf(dir, "111", "First", "Target Game");
        write_acf(dir, "222", "Second", "Target.Game");

        let result = find_appid_for_installdir(dir, "target game");
        assert_eq!(result, Some(("111".to_string(), "First".to_string())));
    }

    #[test]
    fn test_find_appid_for_installdir_no_match() {
        let tmp = TempDir::new().unwrap();
        write_acf(
            tmp.path(),
            "413410",
            "Danganronpa",
            "Danganronpa Trigger Happy Havoc",
        );
        assert!(find_appid_for_installdir(tmp.path(), "Nonexistent").is_none());
    }

    #[test]
    fn test_find_appid_for_installdir_empty_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(find_appid_for_installdir(tmp.path(), "Anything").is_none());
    }
}
