//! Icons for NAND-installed titles, decrypted straight from the
//! emulator's own content store: every yuzu-family and Ryujinx-family
//! install keeps installed NCAs under a `Contents/Registered` directory
//! (flat files or one entry directory each), with `.tik` tickets beside
//! them for CDN-format installs. The control NCA is identified by its
//! decrypted header — file names are content ids, not title ids.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::keys::SwitchKeys;
use super::{config, nca, ryujinx};

/// Decrypts the icon of `title_id` out of every detected install's NAND
/// content directories. Returns `None` when the title is not installed
/// there or no keys unlock it.
pub(super) fn installed_icon(executable: &str, title_id: &str) -> Option<Vec<u8>> {
    let keys = SwitchKeys::load(executable)?;
    let mut dirs = config::nand_registered_dirs_for(executable);
    dirs.extend(ryujinx::bis_registered_dirs(executable));
    installed_icon_in(&dirs, title_id, &keys)
}

fn installed_icon_in(dirs: &[PathBuf], title_id: &str, keys: &SwitchKeys) -> Option<Vec<u8>> {
    for dir in dirs {
        for entry_dir in entry_dirs(dir) {
            let (nca_files, tickets) = collect_ncas_and_tickets(&entry_dir);
            for nca_path in nca_files {
                if let Some((found_id, icon)) = nca::icon_from_nca_file(&nca_path, keys, &tickets)
                {
                    if found_id == title_id {
                        return Some(icon);
                    }
                }
            }
        }
    }
    None
}

/// The directories holding one install's NCAs: yuzu-family entries are
/// subdirectories of `Registered`, Ryujinx keeps flat files in it.
fn entry_dirs(registered: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(registered) else {
        return Vec::new();
    };
    let mut out = vec![registered.to_path_buf()];
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            out.push(entry.path());
        }
    }
    out
}

/// One scan level: the `.nca` files of an entry directory plus the
/// encrypted title keys of any `.tik` tickets beside them.
fn collect_ncas_and_tickets(dir: &Path) -> (Vec<PathBuf>, BTreeMap<[u8; 16], [u8; 16]>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (Vec::new(), BTreeMap::new());
    };
    let mut ncas = Vec::new();
    let mut tickets = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".nca") {
            ncas.push(path);
        } else if name.ends_with(".tik") {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Some((rights_id, title_key)) = nca::parse_ticket(&bytes) {
                    tickets.insert(rights_id, title_key);
                }
            }
        }
    }
    (ncas, tickets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_installed_icon_found_by_title_id() {
        let tmp = tempfile::tempdir().unwrap();
        let registered = tmp.path().join("Registered");
        let entry = registered.join("00000000000000000000000000000001");
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(
            entry.join("9c4f2b099c79dedff9426c2722d09b18.nca"),
            crate::switch::synth::synthetic_control_nca(0x0100a9400c9c2000),
        )
        .unwrap();

        let keys_path = tmp.path().join("prod.keys");
        std::fs::write(&keys_path, crate::switch::synth::test_keys_text()).unwrap();
        let keys = SwitchKeys::from_file(&keys_path).unwrap();
        let dirs = std::slice::from_ref(&registered);
        let icon = installed_icon_in(dirs, "0100a9400c9c2000", &keys).expect("icon");
        assert_eq!(icon, b"JPEGDATA");
        // A different title id finds nothing.
        assert!(installed_icon_in(dirs, "0100000000010000", &keys).is_none());
    }
}
