//! Switch encryption keys from the user's dumped `prod.keys` (plus
//! optionally `title.keys`) — the same files Eden and Ryujinx read. The
//! keys are used as-is: `header_key` decrypts NCA headers, the
//! `key_area_key_<kind>_<rev>` entries unlock standard-crypto sections,
//! and `titlekek_<rev>` + `title.keys` cover ticket-protected dumps.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::emu_dirs;

#[derive(Default)]
pub struct SwitchKeys {
    master_keys: BTreeMap<u8, [u8; 16]>,
    titlekeks: BTreeMap<u8, [u8; 16]>,
    /// (kind, revision) → key; kind 0 = application, 1 = ocean, 2 = system.
    key_area_keys: BTreeMap<(u8, u8), [u8; 16]>,
    /// rights id → title key, from a `title.keys` file.
    title_keys: BTreeMap<[u8; 16], [u8; 16]>,
    header_key: Option<[u8; 32]>,
}

impl SwitchKeys {
    /// Reads the first `prod.keys` found in the standard dump location,
    /// Ira's own keys directory, and the emulators' key directories; then
    /// merges a sibling `title.keys` when one exists.
    pub fn load(executable: &str) -> Option<SwitchKeys> {
        for path in key_file_paths(executable) {
            if !path.is_file() {
                continue;
            }
            match SwitchKeys::from_file(&path) {
                Some(mut keys) => {
                    let title_keys = path.parent().map(|dir| dir.join("title.keys"));
                    if let Some(title_keys) = title_keys.filter(|p| p.is_file()) {
                        keys.merge_title_keys(&title_keys);
                    }
                    return Some(keys);
                }
                None => eprintln!("Switch keys: failed to parse {}", path.display()),
            }
        }
        None
    }

    pub(crate) fn from_file(path: &Path) -> Option<SwitchKeys> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut keys = SwitchKeys::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with([';', '#']) {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let Ok(value) = hex_to_bytes(value.trim()) else {
                continue;
            };
            keys.ingest(&name, value);
        }
        keys.header_key.is_some().then_some(keys)
    }

    fn ingest(&mut self, name: &str, value: Vec<u8>) {
        if name == "header_key" {
            self.header_key = value.as_slice().try_into().ok();
            return;
        }
        if let Some(rev) = name
            .strip_prefix("master_key_")
            .and_then(|rev| u8::from_str_radix(rev, 16).ok())
        {
            if let Ok(key) = value.as_slice().try_into() {
                self.master_keys.insert(rev, key);
            }
            return;
        }
        if let Some(rev) = name
            .strip_prefix("titlekek_")
            .and_then(|rev| u8::from_str_radix(rev, 16).ok())
        {
            if let Ok(key) = value.as_slice().try_into() {
                self.titlekeks.insert(rev, key);
            }
            return;
        }
        if let Some(rest) = name.strip_prefix("key_area_key_") {
            if let Some((kind, rev)) = rest.rsplit_once('_') {
                if let Some(kind) = key_area_kind(kind) {
                    let rev = u8::from_str_radix(rev, 16).unwrap_or(0xff);
                    if let Ok(key) = value.as_slice().try_into() {
                        self.key_area_keys.insert((kind, rev), key);
                    }
                }
            }
        }
    }

    /// Merge a `title.keys` file: `<rights id hex> = <title key hex>`.
    pub fn merge_title_keys(&mut self, path: &Path) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with([';', '#']) {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let (Ok(rights_id), Ok(title_key)) =
                (hex_to_bytes(name.trim()), hex_to_bytes(value.trim()))
            else {
                continue;
            };
            if let (Ok(rights_id), Ok(title_key)) = (
                rights_id.as_slice().try_into(),
                title_key.as_slice().try_into(),
            ) {
                self.title_keys.insert(rights_id, title_key);
            }
        }
    }

    pub fn header_key(&self) -> Option<&[u8; 32]> {
        self.header_key.as_ref()
    }

    /// The title key for a rights id, from `title.keys` when present.
    pub fn title_key(&self, rights_id: &[u8; 16]) -> Option<[u8; 16]> {
        self.title_keys.get(rights_id).copied()
    }

    /// `titlekek_<rev>`: decrypts the title key stored in a ticket.
    pub fn titlekek(&self, rev: u8) -> Option<&[u8; 16]> {
        self.titlekeks.get(&rev)
    }

    /// `key_area_key_<kind>_<rev>`: unlocks standard-crypto sections.
    pub fn key_area_key(&self, kind: u8, rev: u8) -> Option<&[u8; 16]> {
        self.key_area_keys.get(&(kind, rev))
    }

}

fn key_area_kind(name: &str) -> Option<u8> {
    match name {
        "application" => Some(0),
        "ocean" => Some(1),
        "system" => Some(2),
        _ => None,
    }
}

fn hex_to_bytes(value: &str) -> Result<Vec<u8>, ()> {
    let cleaned: String = value.chars().filter(|c| !c.is_whitespace()).collect();
    if !cleaned.len().is_multiple_of(2) || cleaned.is_empty() {
        return Err(());
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(|_| ()))
        .collect()
}

/// `prod.keys` locations, best first: the standard dump directory, Ira's
/// own keys folder, then the emulators' (portable layouts first, then the
/// XDG/flatpak config dirs of the forks that share the layout).
fn key_file_paths(executable: &str) -> Vec<PathBuf> {
    let home = emu_dirs::home_dir();
    let mut paths = vec![home.join(".switch/prod.keys")];
    paths.push(home.join(".config/ira/keys/prod.keys"));

    if !executable.is_empty() && !executable.starts_with("flatpak:") {
        let exe = Path::new(executable);
        for root in [exe.parent(), Some(exe)].into_iter().flatten() {
            paths.push(root.join("system/prod.keys"));
        }
    }
    let mut config_roots = vec![emu_dirs::config_home()];
    if let Some(app) = emu_dirs::flatpak_app_dir(executable) {
        config_roots.push(app.join("config"));
    }
    for base in ["Ryubing", "Ryujinx", "Kenji-NX", "kenji-nx", "eden"] {
        for root in &config_roots {
            paths.push(root.join(base).join("system/prod.keys"));
            paths.push(root.join(base).join("keys/prod.keys"));
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keys_ingest_standard_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("prod.keys");
        std::fs::write(
            &path,
            "; comment\r\n\
             master_key_00 = 000102030405060708090a0b0c0d0e0f\r\n\
             master_key_02 = 100102030405060708090a0b0c0d0e0f\r\n\
             header_key = 200102030405060708090a0b0c0d0e0f110102030405060708090a0b0c0d0e0f\r\n\
             key_area_key_application_00 = 300102030405060708090a0b0c0d0e0f\r\n\
             titlekek_00 = 400102030405060708090a0b0c0d0e0f\r\n\
             unrelated_key = deadbeef\r\n",
        )
        .unwrap();
        let keys = SwitchKeys::from_file(&path).unwrap();
        assert_eq!(keys.header_key().unwrap()[0], 0x20);
        assert!(keys.key_area_key(0, 0).is_some());
        assert!(keys.key_area_key(1, 0).is_none());
        assert_eq!(keys.titlekek(0).unwrap()[0], 0x40);
    }

    #[test]
    fn test_title_keys_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let prod = tmp.path().join("prod.keys");
        std::fs::write(
            &prod,
            "header_key = 200102030405060708090a0b0c0d0e0f110102030405060708090a0b0c0d0e0f\n",
        )
        .unwrap();
        let title = tmp.path().join("title.keys");
        std::fs::write(
            &title,
            "0100a9400c9c20000000000000000008 = 6d4bb868cd3864bf375b13a48d679594\n",
        )
        .unwrap();

        let mut keys = SwitchKeys::from_file(&prod).unwrap();
        keys.merge_title_keys(&title);
        let rights_id: [u8; 16] = [
            0x01, 0x00, 0xa9, 0x40, 0x0c, 0x9c, 0x20, 0x00, 0, 0, 0, 0, 0, 0, 0, 0x08,
        ];
        assert_eq!(keys.title_key(&rights_id).unwrap()[0], 0x6d);
    }

    #[test]
    fn test_hex_to_bytes_accepts_spaced_hex() {
        assert_eq!(
            hex_to_bytes("AA BB 01 02").unwrap(),
            vec![0xaa, 0xbb, 0x01, 0x02]
        );
        assert!(hex_to_bytes("abc").is_err());
    }
}
