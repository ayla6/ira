//! XCI gamecard containers: a `HEAD` gamecard header followed by a root
//! HFS0 whose `update`/`normal`/`secure` entries are HFS0 images of their
//! own. The distributed NCAs and tickets live in `secure`. HFS0 is the
//! hashed sibling of PFS0 — 0x40-byte entries and the same string-table
//! scheme — so entries come back with absolute file offsets and the NSP
//! control-NCA pipeline reads them unchanged. Header layout per
//! switchbrew.org/wiki/XCI and /wiki/HFS0; container crypto is not
//! touched (unencrypted partition tables, NCA decryption stays in nca.rs).

use std::path::Path;

use super::rom::PfsEntry;

/// Gamecard header size; the root partition's HFS0 starts right after it.
const ROOT_OFFSET: u64 = 0x1000;
/// Sane caps: gamecards carry a handful of partitions; partitions a few
/// hundred content files.
const MAX_HFS0_FILES: u64 = 8192;
const MAX_STRING_TABLE: u64 = 4 * 1024 * 1024;

/// True when the file carries the gamecard `HEAD` magic.
pub(super) fn is_xci(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    std::io::Read::read_exact(&mut file, &mut magic).is_ok() && &magic == b"HEAD"
}

/// The gamecard's content files (`.nca`, `.tik`), offsets absolute in the
/// file, best partition first: `secure` is the authoritative one, `normal`
/// mirrors it on some dumps, `update` only carries update NCAs.
pub(super) fn content_entries(path: &Path) -> Option<Vec<PfsEntry>> {
    let mut file = std::fs::File::open(path).ok()?;
    let root = read_hfs0(&mut file, ROOT_OFFSET)?;
    for name in ["secure", "normal", "update"] {
        let Some(entry) = root.iter().find(|e| e.name == name) else {
            continue;
        };
        if let Some(entries) = read_hfs0(&mut file, entry.offset) {
            if !entries.is_empty() {
                return Some(entries);
            }
        }
    }
    None
}

/// One HFS0 directory's file table, offsets absolute in `file`.
fn read_hfs0(file: &mut std::fs::File, base: u64) -> Option<Vec<PfsEntry>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut header = [0u8; 16];
    file.seek(SeekFrom::Start(base)).ok()?;
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"HFS0" {
        return None;
    }
    let file_count = u32::from_le_bytes(header[4..8].try_into().ok()?) as u64;
    let table_size = u32::from_le_bytes(header[8..12].try_into().ok()?) as u64;
    if file_count == 0 || file_count > MAX_HFS0_FILES || table_size > MAX_STRING_TABLE {
        return None;
    }
    // HFS0 entries are 0x40 bytes: offset, size, metatable offset and
    // size (u64 each), name offset and hashed flag (u32 each), padding.
    let entries_size = file_count.checked_mul(0x40)?;
    let mut buf = vec![0u8; entries_size.checked_add(table_size)? as usize];
    file.read_exact(&mut buf).ok()?;

    // File data starts after the header, entry table, name table and the
    // per-entry metatables; entry offsets are relative to that point.
    let data_start = base + 16 + entries_size + table_size;
    let mut out = Vec::with_capacity(file_count as usize);
    for i in 0..file_count {
        let b = (i * 0x40) as usize;
        let name_offset = u32::from_le_bytes(buf[b + 32..b + 36].try_into().ok()?) as usize;
        let Some(name) = super::rom::cstr(buf.get(entries_size as usize + name_offset..)?) else {
            continue;
        };
        out.push(PfsEntry {
            name: name.to_string(),
            offset: data_start + u64::from_le_bytes(buf[b..b + 8].try_into().ok()?),
            size: u64::from_le_bytes(buf[b + 8..b + 16].try_into().ok()?),
        });
    }
    Some(out)
}
