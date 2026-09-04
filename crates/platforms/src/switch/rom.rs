//! Title id and native icon extraction from Switch ROM files without
//! emulator keys: dump file names, NSP ticket names (plaintext container
//! directory) and homebrew NRO asset blocks.

use std::io::{Read, Seek};
use std::path::Path;

/// Maximum bytes read for container directories and embedded assets; real
/// values stay orders of magnitude below this.
const MAX_READ: u64 = 4 * 1024 * 1024;

/// True for 16 hex digits starting with `01`, the Switch application id
/// shape (base titles end in 000, updates in 800, DLC in 001–7ff).
pub(super) fn is_title_id(s: &str) -> bool {
    s.len() == 16 && s.starts_with("01") && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Updates (base id | 0x800) describe the same game as their base title;
/// DLC keeps its own identity.
fn base_application_id(id: &str) -> Option<String> {
    let value = u64::from_str_radix(id, 16).ok()? & !0x800;
    Some(format!("{value:016x}"))
}

/// True when the id is an update's (`base | 0x800`): it describes the same
/// game as its base title and must never surface as a title of its own.
pub(super) fn is_update_title_id(id: &str) -> bool {
    u64::from_str_radix(id, 16)
        .map(|value| value & 0x800 != 0)
        .unwrap_or(false)
}

/// Finds a title id in a dump file name. Scene dumps bracket it
/// (`Zelda [01007ef00011e000][v0]`); some tools lead with a bare id.
pub fn title_id_from_filename(stem: &str) -> Option<String> {
    let mut inner = stem;
    while let Some(start) = inner.find('[') {
        let Some(end) = inner[start..].find(']').map(|e| start + e) else {
            break;
        };
        let candidate = &inner[start + 1..end];
        if let Some(id) = base_application_id(candidate).filter(|id| is_title_id(id)) {
            return Some(id);
        }
        inner = &inner[end + 1..];
    }
    stem.split_whitespace()
        .next()
        .filter(|token| is_title_id(token))
        .and_then(base_application_id)
}

/// One file inside a PFS0 container, with its absolute offset and size.
pub(super) struct PfsEntry {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

/// Reads the PFS0 file table of an NSP. The header and name table are
/// plaintext, so no decryption keys are needed.
pub(super) fn read_pfs0_entries(path: &Path) -> Option<Vec<PfsEntry>> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 16];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"PFS0" {
        return None;
    }
    let file_count = u32::from_le_bytes(header[4..8].try_into().ok()?) as u64;
    let table_size = u32::from_le_bytes(header[8..12].try_into().ok()?) as u64;
    if file_count == 0 || file_count > 8192 {
        return None;
    }
    let entries_size = file_count.checked_mul(0x18)?;
    // The 16-byte header is already consumed; read what follows it.
    let rest = entries_size
        .checked_add(table_size)?
        .min(MAX_READ.saturating_sub(16));
    let mut buf = vec![0u8; rest as usize];
    file.read_exact(&mut buf).ok()?;

    // Entry offsets are relative to the data section, which starts after
    // the header, the entry table and the name table.
    let data_start = 16 + entries_size + table_size;
    let mut entries = Vec::with_capacity(file_count as usize);
    for entry in 0..file_count {
        let base = (entry * 0x18) as usize;
        let name_offset = u32::from_le_bytes(buf[base + 16..base + 20].try_into().ok()?) as u64;
        let name_start = (entries_size + name_offset) as usize;
        let Some(name) = cstr(buf.get(name_start..)?) else {
            continue;
        };
        entries.push(PfsEntry {
            name: name.to_string(),
            offset: data_start + u64::from_le_bytes(buf[base..base + 8].try_into().ok()?),
            size: u64::from_le_bytes(buf[base + 8..base + 16].try_into().ok()?),
        });
    }
    Some(entries)
}

/// Reads the title id from an NSP's plaintext file-table names: the
/// `<rights id>.tik`/`.cert` tickets, or the `<title id>.cnmt.nca` meta
/// entry repacked NSPs carry. Works without any keys, so every ticketed or
/// repacked dump gets its real id instead of a filename-based identity.
pub fn title_id_from_nsp(path: &Path) -> Option<String> {
    let mut cnmt = None;
    for entry in read_pfs0_entries(path)? {
        match id_from_entry_name(&entry.name) {
            EntryId::RightsId(id) => return Some(id),
            EntryId::Cnmt(id) => {
                // First-come: the table order of meta entries is not
                // meaningful, but a base-game NSP carries exactly one.
                cnmt = cnmt.or(Some(id));
            }
            EntryId::None => {}
        }
    }
    cnmt
}

/// The title id an entry name carries, if any.
enum EntryId {
    /// `<rights id>.tik`/`.cert` — the most specific source.
    RightsId(String),
    /// `<title id>.cnmt.nca` — repacked NSPs rename the meta NCA.
    Cnmt(String),
    None,
}

fn id_from_entry_name(name: &str) -> EntryId {
    if let Some((id, kind)) = name.rsplit_once('.') {
        if matches!(kind, "tik" | "cert") {
            if let Some(title_id) = base_application_id(&id[..16.min(id.len())]) {
                if is_title_id(&title_id) {
                    return EntryId::RightsId(title_id);
                }
            }
        }
    }
    match name.strip_suffix(".cnmt.nca") {
        // Only the exact 16-hex shape: a hash-named (original card dump)
        // meta NCA has no readable title id in its name.
        Some(stem) if is_title_id(stem) => EntryId::Cnmt(base_application_id(stem).unwrap_or(stem.to_string())),
        _ => EntryId::None,
    }
}

pub(super) fn cstr(bytes: &[u8]) -> Option<&str> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end]).ok()
}

struct AssetSection {
    offset: u64,
    size: u64,
}

/// Reads an NRO's asset block: homebrew executables carry an unencrypted
/// `ASET` section after the code, holding the icon (PNG) and the NACP with
/// the application title.
pub fn read_nro_asset(path: &Path) -> Option<(Vec<u8>, String)> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 0x80];
    file.read_exact(&mut header).ok()?;
    if &header[0x10..0x14] != b"NRO0" {
        return None;
    }
    let asset_offset = u32::from_le_bytes(header[0x18..0x1c].try_into().ok()?) as u64;

    let mut asset = [0u8; 0x38];
    file.read_exact(&mut asset).ok()?;
    if &asset[0..4] != b"ASET" {
        return None;
    }
    let section = |at: usize| -> Option<AssetSection> {
        Some(AssetSection {
            offset: u64::from_le_bytes(asset[at..at + 8].try_into().ok()?),
            size: u64::from_le_bytes(asset[at + 8..at + 16].try_into().ok()?),
        })
    };
    let icon = section(8)?;
    let nacp = section(0x18)?;

    let icon = read_at(&mut file, asset_offset + icon.offset, icon.size.min(MAX_READ))?;
    let title = read_at(
        &mut file,
        asset_offset + nacp.offset,
        nacp.size.min(0x40000),
    )
    .and_then(nacp_title)
    .unwrap_or_default();
    Some((icon, title))
}

fn read_at(file: &mut std::fs::File, offset: u64, size: u64) -> Option<Vec<u8>> {
    if size == 0 {
        return None;
    }
    file.seek(std::io::SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; size as usize];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// The NACP starts with 16 localized application titles; the first one
/// holds the name as a NUL-terminated UTF-16LE string.
fn nacp_title(nacp: Vec<u8>) -> Option<String> {
    let units: Vec<u16> = nacp[..nacp.len().min(0x200)]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let len = units.iter().position(|u| *u == 0).unwrap_or(units.len());
    let title: String = String::from_utf16_lossy(&units[..len]).trim().into();
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// Fixture NRO: 0x80 header, then an ASET block with icon + NACP.
#[cfg(test)]
pub(crate) fn nro_fixture(title: &str, icon: &[u8]) -> Vec<u8> {
    let mut nacp = Vec::new();
    for unit in title.encode_utf16() {
        nacp.extend_from_slice(&unit.to_le_bytes());
    }
    nacp.resize(0x400, 0);

    let mut out = vec![0u8; 0x80];
    out[0x10..0x14].copy_from_slice(b"NRO0");
    out[0x18..0x1c].copy_from_slice(&0x80u32.to_le_bytes());

    let icon_offset = 0x38u64;
    let nacp_offset = icon_offset + icon.len() as u64;
    out.extend_from_slice(b"ASET");
    out.extend_from_slice(&0u32.to_le_bytes()); // format version
    out.extend_from_slice(&icon_offset.to_le_bytes());
    out.extend_from_slice(&(icon.len() as u64).to_le_bytes());
    out.extend_from_slice(&nacp_offset.to_le_bytes());
    out.extend_from_slice(&(nacp.len() as u64).to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(icon);
    out.extend_from_slice(&nacp);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_id_from_filename_bracket() {
        assert_eq!(
            title_id_from_filename("Zelda [01007EF00011E000][v0]"),
            Some("01007ef00011e000".to_string())
        );
        assert_eq!(
            title_id_from_filename("0100000000010800 Game"),
            Some("0100000000010000".to_string())
        );
        assert_eq!(title_id_from_filename("Just A Game"), None);
        // Brackets that are not ids keep the scan going.
        assert_eq!(
            title_id_from_filename("Game [DLC] [0100000000010800]"),
            Some("0100000000010000".to_string())
        );
    }

    #[test]
    fn test_title_id_ignores_non_application_ids() {
        assert_eq!(title_id_from_filename("[1234567890abcdef] Game"), None);
        assert_eq!(title_id_from_filename("[0100] Game"), None);
    }

    #[test]
    fn test_is_update_title_id_checks_low_bits() {
        assert!(is_update_title_id("010051f0207b2800"));
        assert!(!is_update_title_id("010051f0207b2000"));
        // DLC ids (001–7ff) are not updates.
        assert!(!is_update_title_id("010051f0207b2001"));
        assert!(!is_update_title_id("garbage"));
    }

    /// Fixture NSP: PFS0 header, two entries whose names live in the string
    /// table; the ticket name carries the rights id.
    fn nsp_fixture(names: &[&str]) -> Vec<u8> {
        let mut table = Vec::new();
        let offsets: Vec<u32> = names
            .iter()
            .map(|name| {
                let offset = table.len() as u32;
                table.extend_from_slice(name.as_bytes());
                table.push(0);
                offset
            })
            .collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"PFS0");
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&(table.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for (i, _) in names.iter().enumerate() {
            out.extend_from_slice(&0u64.to_le_bytes()); // offset
            out.extend_from_slice(&0u64.to_le_bytes()); // size
            out.extend_from_slice(&offsets[i].to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out.extend_from_slice(&table);
        out
    }

    #[test]
    fn test_title_id_from_nsp_ticket_name() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("game.nsp");
        std::fs::write(
            &path,
            nsp_fixture(&[
                "0100000000010000.cnmt.nca",
                "01000000000108000000000000000003.tik",
            ]),
        )
        .unwrap();
        assert_eq!(
            title_id_from_nsp(&path),
            Some("0100000000010000".to_string())
        );
    }

    #[test]
    fn test_title_id_from_nsp_cnmt_name_without_ticket() {
        // Ticket-less repacked dumps carry only the renamed meta NCA.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("game.nsp");
        std::fs::write(
            &path,
            nsp_fixture(&[
                "01000ab001234800.cnmt.nca",
                "9c4f2b099c79dedff9426c2722d09b18.nca",
            ]),
        )
        .unwrap();
        assert_eq!(
            title_id_from_nsp(&path),
            Some("01000ab001234000".to_string())
        );
    }

    #[test]
    fn test_title_id_from_nsp_ignores_hash_named_cnmt() {
        // Original card dumps name the meta NCA by content hash — no id
        // is readable there.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("game.nsp");
        std::fs::write(
            &path,
            nsp_fixture(&["9c4f2b099c79dedff9426c2722d09b18.cnmt.nca"]),
        )
        .unwrap();
        assert_eq!(title_id_from_nsp(&path), None);
    }

    #[test]
    fn test_title_id_from_nsp_ticket_beats_cnmt() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("game.nsp");
        std::fs::write(
            &path,
            nsp_fixture(&[
                "0100000000010800.cnmt.nca",
                "01000000000100000000000000000016.tik",
            ]),
        )
        .unwrap();
        assert_eq!(
            title_id_from_nsp(&path),
            Some("0100000000010000".to_string())
        );
    }

    #[test]
    fn test_title_id_from_nsp_rejects_non_nsp() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("game.nsp");
        std::fs::write(&path, b"not an nsp at all").unwrap();
        assert_eq!(title_id_from_nsp(&path), None);
    }

    #[test]
    fn test_read_nro_asset_icon_and_title() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("homebrew.nro");
        std::fs::write(&path, nro_fixture("Test Homebrew", b"PNGDATA")).unwrap();
        let (icon, title) = read_nro_asset(&path).unwrap();
        assert_eq!(icon, b"PNGDATA");
        assert_eq!(title, "Test Homebrew");
    }

    #[test]
    fn test_read_nro_asset_rejects_non_nro() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("game.nro");
        std::fs::write(&path, vec![0u8; 0x100]).unwrap();
        assert!(read_nro_asset(&path).is_none());
    }
}
