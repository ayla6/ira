use std::path::{Path, PathBuf};

use super::vdf;

/// Given a game install folder, return its parent `steamapps` directory if the
/// path matches the `steamapps/common/<game>` layout.
pub fn steamapps_in_path(path: &Path) -> Option<PathBuf> {
    let common = path.parent()?;
    if common.file_name().and_then(|n| n.to_str()) != Some("common") {
        return None;
    }
    let steamapps = common.parent()?;
    if steamapps.file_name().and_then(|n| n.to_str()) == Some("steamapps") {
        Some(steamapps.to_path_buf())
    } else {
        None
    }
}

/// Scan `appmanifest_*.acf` files in `steamapps_dir` and return the
/// `(appid, name)` of the manifest whose `installdir` matches
/// `installdir_name` (case-insensitive).
pub fn find_appid_for_installdir(steamapps_dir: &Path, installdir_name: &str) -> Option<(String, String)> {
    let entries = std::fs::read_dir(steamapps_dir).ok()?;
    let target = installdir_name.to_lowercase();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("appmanifest_") || !name_str.ends_with(".acf") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
        let Some(parsed) = vdf::parse_vdf(&text) else { continue };
        let installdir = vdf::get_str(&parsed, "installdir").unwrap_or("");
        if installdir.to_lowercase() == target {
            let appid = vdf::get_str(&parsed, "appid")?.to_string();
            let name = vdf::get_str(&parsed, "name").unwrap_or("").to_string();
            return Some((appid, name));
        }
    }
    None
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
    fn test_steamapps_in_path_rejects_non_common() {
        assert!(steamapps_in_path(Path::new("/games/MyGame")).is_none());
        let path = Path::new("/x/steamapps/MyGame"); // missing "common" segment
        assert!(steamapps_in_path(path).is_none());
    }

    #[test]
    fn test_find_appid_for_installdir_matches_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        write_acf(dir, "413410", "Danganronpa", "Danganronpa Trigger Happy Havoc");
        write_acf(dir, "1687950", "Persona 5 Royal", "P5R");

        let result = find_appid_for_installdir(dir, "danganronpa trigger happy havoc");
        assert_eq!(result, Some(("413410".to_string(), "Danganronpa".to_string())));
    }

    #[test]
    fn test_find_appid_for_installdir_no_match() {
        let tmp = TempDir::new().unwrap();
        write_acf(tmp.path(), "413410", "Danganronpa", "Danganronpa Trigger Happy Havoc");
        assert!(find_appid_for_installdir(tmp.path(), "Nonexistent").is_none());
    }

    #[test]
    fn test_find_appid_for_installdir_empty_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(find_appid_for_installdir(tmp.path(), "Anything").is_none());
    }
}
