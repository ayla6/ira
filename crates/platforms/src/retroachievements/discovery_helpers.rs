use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[cfg(test)]
use super::api_types::RaGameEntry;

pub(super) fn normalize_name(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_brackets = 0i32;
    let mut prev_space = true;

    for c in s.chars() {
        match c {
            '(' | '[' => in_brackets += 1,
            ')' | ']' => {
                if in_brackets > 0 {
                    in_brackets -= 1;
                }
            }
            _ if in_brackets > 0 => {}
            _ => {
                let c = if c == '_' || c == '.' { ' ' } else { c };
                for lc in c.to_lowercase() {
                    if lc.is_alphanumeric() {
                        result.push(lc);
                        prev_space = false;
                    } else if !prev_space {
                        result.push(' ');
                        prev_space = true;
                    }
                }
            }
        }
    }

    while result.ends_with(' ') {
        result.pop();
    }
    result
}

fn remove_version_tags(s: &str) -> String {
    let mut result = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ if depth == 0 => result.push(c),
            _ => {}
        }
    }
    result.trim().to_string()
}

pub(super) fn strip_disc_pattern(name: &str) -> Option<(String, i32)> {
    let lower = name.to_lowercase();
    let keywords = ["disc ", "disk ", "cd "];

    for kw in keywords {
        if let Some(kw_start) = lower.find(kw) {
            let bracket_start = name[..kw_start].rfind(['(', '['])?;
            let after_kw = kw_start + kw.len();
            let closing = name[after_kw..].find([')', ']'])?;
            let num_str = name[after_kw..after_kw + closing].trim();
            if let Ok(n) = num_str.parse::<i32>() {
                let base = format!(
                    "{} {}",
                    name[..bracket_start].trim(),
                    name[after_kw + closing + 1..].trim()
                );
                let base = remove_version_tags(base.trim()).to_string();
                return Some((base, n));
            }
        }
    }

    None
}

pub(super) struct DiscGroup {
    pub(super) roms: Vec<(String, PathBuf, Option<i32>)>,
    pub(super) serial: Option<String>,
}

pub(super) fn group_multi_disc_roms(
    conn: &ira_db::DbConn,
    roms: Vec<(String, PathBuf)>,
) -> Vec<DiscGroup> {
    let mut by_pattern: HashMap<String, Vec<(String, PathBuf, Option<i32>)>> = HashMap::new();
    let mut ungrouped: Vec<(String, PathBuf, Option<i32>)> = Vec::new();

    for (name, path) in roms {
        match strip_disc_pattern(&name) {
            Some((base, disc_num)) => {
                by_pattern
                    .entry(base)
                    .or_default()
                    .push((name, path, Some(disc_num)));
            }
            None => {
                ungrouped.push((name, path, None));
            }
        }
    }

    let mut groups: Vec<DiscGroup> = Vec::new();

    for (_, mut rom_list) in by_pattern {
        if rom_list.len() >= 2 {
            rom_list.sort_by_key(|(_, _, disc)| disc.unwrap_or(0));
        }
        groups.push(DiscGroup {
            roms: rom_list,
            serial: None,
        });
    }

    let mut by_serial: HashMap<String, Vec<(String, PathBuf, Option<i32>)>> = HashMap::new();
    for (name, path, _) in ungrouped {
        let serial = crate::rom_serial::read_serial_cached(conn, &path);
        if let Some(ref s) = serial {
            by_serial
                .entry(s.clone())
                .or_default()
                .push((name, path, None));
        } else {
            groups.push(DiscGroup {
                roms: vec![(name, path, None)],
                serial: None,
            });
        }
    }

    for (s, mut rom_list) in by_serial {
        if rom_list.len() >= 2 {
            rom_list.sort_by_key(|(name, _, _)| name.clone());
            for (i, entry) in rom_list.iter_mut().enumerate() {
                entry.2 = Some((i + 1) as i32);
            }
        }
        groups.push(DiscGroup {
            roms: rom_list,
            serial: Some(s),
        });
    }

    groups
}

pub(super) fn scan_roms(folder: &str, extensions: &[&str]) -> Option<Vec<(String, PathBuf)>> {
    let mut roms = Vec::new();
    let path = Path::new(folder);
    if !path.is_dir() {
        return None;
    }
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return None,
    };
    for entry in entries.flatten() {
        let file_path = entry.path();
        if !file_path.is_file() {
            continue;
        }
        if let Some(ext) = file_path.extension().and_then(|e| e.to_str()) {
            if extensions.iter().any(|&e| e.eq_ignore_ascii_case(ext)) {
                let name = file_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                roms.push((name, file_path));
            }
        }
    }
    Some(roms)
}

#[cfg(test)]
pub(super) fn match_rom_to_game(rom_name: &str, games: &[RaGameEntry]) -> Option<u32> {
    let rom_norm = normalize_name(rom_name);
    if rom_norm.is_empty() {
        return None;
    }

    for g in games {
        if normalize_name(&g.title) == rom_norm {
            return Some(g.id);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_roms_missing_folder_is_not_successful() {
        assert!(scan_roms("/does/not/exist", &["gba"]).is_none());
    }

    #[test]
    fn test_scan_roms_empty_folder_is_successful() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            scan_roms(dir.path().to_str().unwrap(), &["gba"])
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn test_group_multi_disc_ignores_release_tags() {
        let roms = vec![
            (
                "Devil Summoner - Soul Hackers (Japan) (Disc 1)".to_string(),
                PathBuf::from("/games/disc1.chd"),
            ),
            (
                "Devil Summoner - Soul Hackers (Japan) (Disc 2)".to_string(),
                PathBuf::from("/games/disc2.chd"),
            ),
            (
                "Devil Summoner - Soul Hackers (Extra Dungeon) (Japan) (3M) (Disc 3)".to_string(),
                PathBuf::from("/games/disc3.chd"),
            ),
        ];

        let groups = group_multi_disc_roms(&test_db(), roms);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].roms.len(), 3);
        assert_eq!(
            groups[0]
                .roms
                .iter()
                .map(|(_, _, disc)| *disc)
                .collect::<Vec<_>>(),
            vec![Some(1), Some(2), Some(3)]
        );
    }

    fn test_db() -> ira_db::DbConn {
        let tmp = tempfile::tempdir().unwrap();
        let conn = ira_db::init_db(tmp.path().join("ira.db").to_str().unwrap());
        std::mem::forget(tmp);
        conn
    }
}
