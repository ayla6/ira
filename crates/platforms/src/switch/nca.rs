//! Native Switch control-NCA extraction: decrypts the control NCA inside
//! an NSP with the user's dumped keys and pulls the 256×256 icon PNG and
//! the `control.nacp` application title out of its RomFS — the same
//! bytes and names the emulators show.
//!
//! Pipeline (offsets per switchbrew, crypto conventions per nsz/LibHac):
//! PFS0 file table → NCA header (AES-XTS with `header_key`, magic
//! "NCA3" at 0x200 after the two signatures) → section 0 (AES-CTR; the
//! body key comes from key-area slot 2 or, for ticket-protected NCAs
//! with a rights id, from the `.tik` title key under `titlekek`) →
//! scan for the RomFS image → `icon_<Language>.dat` (a JPEG) and
//! `control.nacp` (16 language title slots).

use std::collections::BTreeMap;
use std::path::Path;

use aes::cipher::{BlockCipherDecrypt, KeyInit, KeyIvInit, StreamCipher};
use ctr::Ctr128BE;
use xts_mode::Xts128;

use super::keys::SwitchKeys;
use super::nacp;
use super::rom::{read_pfs0_entries, PfsEntry};

/// NCA headers and section tables span 0xC00 bytes (12 XTS sectors).
pub(super) const HEADER_SIZE: usize = 0xC00;
pub(super) const SECTOR_SIZE: usize = 0x200;
/// Control NCAs are a few MiB; refuse to buffer anything larger.
const MAX_CONTROL_SECTION: u64 = 64 * 1024 * 1024;
/// Tickets are a few hundred bytes; refuse to buffer anything larger.
const MAX_READ: u64 = 64 * 1024;
/// A control NCA's `control.nacp` is exactly this big; larger reads are
/// garbage and capped away.
const NACP_SIZE: u64 = 0x4000;

/// A control NCA's application title and icon, decrypted in one pass.
#[derive(Debug, Default)]
pub struct ControlMeta {
    /// Application title from the NACP's language table.
    pub title: Option<String>,
    /// The 256×256 icon image.
    pub icon: Option<Vec<u8>>,
}

/// Decrypts the control NCA of a ROM container (NSP; XCIs that carry
/// keys also work) and reads its application title and icon in one pass,
/// or `None` when no key/section yields either.
pub fn extract_control_meta(path: &Path, keys: &SwitchKeys) -> Option<ControlMeta> {
    let entries = read_pfs0_entries(path)?;
    let tickets = inline_tickets(path, &entries);
    for nca in entries.iter().filter(|entry| entry.name.ends_with(".nca")) {
        if let Some((_, meta)) = control_meta_from_entry(path, nca, keys, &tickets) {
            return Some(meta);
        }
    }
    None
}

/// Encrypted title keys from the container's `.tik` files, keyed by
/// rights id — the in-container equivalent of a `title.keys` file.
fn inline_tickets(path: &Path, entries: &[PfsEntry]) -> BTreeMap<[u8; 16], [u8; 16]> {
    let mut out = BTreeMap::new();
    let Ok(mut file) = std::fs::File::open(path) else {
        return out;
    };
    for entry in entries.iter().filter(|entry| entry.name.ends_with(".tik")) {
        let mut buf = vec![0u8; entry.size.min(MAX_READ) as usize];
        if std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(entry.offset)).is_err() {
            continue;
        }
        if std::io::Read::read_exact(&mut file, &mut buf).is_err() {
            continue;
        }
        if let Some((rights_id, title_key)) = parse_ticket(&buf) {
            out.insert(rights_id, title_key);
        }
    }
    out
}

/// Pulls `(rights id, encrypted title key)` from ticket bytes. Layout
/// per switchbrew.org/wiki/Ticket: a variable-size signature block (the
/// type word picks its length) padded to 0x40, then the ticket data with
/// the title key at +0x40 and the rights id at +0x160.
pub(super) fn parse_ticket(buf: &[u8]) -> Option<([u8; 16], [u8; 16])> {
    let signature_type = u32::from_le_bytes(buf.first_chunk::<4>()?.to_owned());
    let signature_size = match signature_type {
        0x10000 | 0x10003 => 0x200,
        0x10001 | 0x10004 => 0x100,
        0x10002 | 0x10005 => 0x3C,
        _ => return None,
    };
    let after_signature = 4 + signature_size;
    let data_at = after_signature + (0x40 - after_signature % 0x40);
    let title_key = buf.get(data_at + 0x40..data_at + 0x50)?.try_into().ok()?;
    let rights_id = buf.get(data_at + 0x160..data_at + 0x170)?.try_into().ok()?;
    Some((rights_id, title_key))
}

/// Extracts `(title id, application title + icon)` from a standalone
/// `.nca` file — the layout NAND installs use inside
/// `Contents/Registered`. Returns `None` for anything that is not a
/// decryptable control NCA.
pub(super) fn control_meta_from_nca_file(
    path: &Path,
    keys: &SwitchKeys,
    tickets: &BTreeMap<[u8; 16], [u8; 16]>,
) -> Option<(String, ControlMeta)> {
    let size = std::fs::metadata(path).ok()?.len();
    let name = path.file_name()?.to_string_lossy().into_owned();
    control_meta_from_entry(
        path,
        &PfsEntry {
            name,
            offset: 0,
            size,
        },
        keys,
        tickets,
    )
}

/// Reads and decrypts one NCA's 0xC00 header (AES-XTS with `header_key`)
/// and checks it is an NCA3 control NCA (content type 2).
fn decrypted_control_header(
    path: &Path,
    nca: &PfsEntry,
    keys: &SwitchKeys,
) -> Option<[u8; HEADER_SIZE]> {
    let mut raw = [0u8; HEADER_SIZE];
    let mut file = std::fs::File::open(path).ok()?;
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(nca.offset)).ok()?;
    std::io::Read::read_exact(&mut file, &mut raw).ok()?;

    let header_key = keys.header_key()?;
    // Switch tweaks the sector number big-endian, opposite IEEE 1619.
    let sector_tweak =
        |sector: u128| -> xts_mode::Array<u8, aes::cipher::consts::U16> {
            xts_mode::Array(sector.to_be_bytes())
        };
    Xts128::<aes::Aes128>::new(
        aes::Aes128::new_from_slice(&header_key[..16]).ok()?,
        aes::Aes128::new_from_slice(&header_key[16..]).ok()?,
    )
    .decrypt_area(&mut raw, SECTOR_SIZE, 0, sector_tweak);

    // The magic sits at 0x200, after the two RSA signatures; content
    // type 2 marks the control NCA. Anything else is skipped.
    if raw.get(0x200..0x204) != Some(&b"NCA3"[..]) || raw.get(0x205).copied() != Some(2) {
        return None;
    }
    Some(raw)
}

/// The AES-CTR key of the control NCA's RomFS section. Ticket-protected
/// NCAs (non-zero rights id) unlock the `.tik`/`title.keys` title key
/// with the revision's titlekek; standard-crypto NCAs keep the section
/// key in key-area slot 2, wrapped with the revision's key area key.
fn section_body_key(
    raw: &[u8; HEADER_SIZE],
    keys: &SwitchKeys,
    tickets: &BTreeMap<[u8; 16], [u8; 16]>,
    rights_id: [u8; 16],
    master_index: u8,
) -> Option<[u8; 16]> {
    if rights_id.iter().any(|byte| *byte != 0) {
        let encrypted_title_key = match tickets.get(&rights_id) {
            Some(key) => *key,
            None => keys.title_key(&rights_id)?,
        };
        let titlekek = keys.titlekek(master_index)?;
        let mut block = aes::Block::default();
        block.copy_from_slice(&encrypted_title_key);
        aes::Aes128::new_from_slice(titlekek)
            .ok()?
            .decrypt_block(&mut block);
        let mut out = [0u8; 16];
        out.copy_from_slice(&block);
        Some(out)
    } else {
        let key_index = *raw.get(0x207)?;
        let encrypted_area = raw.get(0x300..0x340)?;
        let kak = keys.key_area_key(key_index, master_index)?;
        let mut kak_block = aes::Block::default();
        kak_block.copy_from_slice(kak);
        let kak_cipher = aes::Aes128::new(&kak_block);
        let mut key = aes::Block::default();
        key.copy_from_slice(encrypted_area.get(32..48)?);
        kak_cipher.decrypt_block(&mut key);
        let mut out = [0u8; 16];
        out.copy_from_slice(&key);
        Some(out)
    }
}

/// Decrypts one NCA entry's header and, when it is the control NCA, its
/// RomFS section to read the application title and icon it carries,
/// together with the title id from the header.
fn control_meta_from_entry(
    path: &Path,
    nca: &PfsEntry,
    keys: &SwitchKeys,
    tickets: &BTreeMap<[u8; 16], [u8; 16]>,
) -> Option<(String, ControlMeta)> {
    let raw = decrypted_control_header(path, nca, keys)?;
    let key_generation_new = *raw.get(0x220)?;
    let key_generation_old = *raw.get(0x206)?;
    let rights_id: [u8; 16] = raw.get(0x230..0x240)?.try_into().ok()?;
    let title_id = format!("{:016x}", u64::from_le_bytes(raw.get(0x210..0x218)?.try_into().ok()?));

    // Firmware 3.0.0+ keeps the master key revision at 0x220; older NCAs
    // zero it and store the value at 0x206. Both are one-based.
    let master_index = if key_generation_new > 2 {
        key_generation_new
    } else {
        key_generation_old
    }
    .saturating_sub(1);

    let body_key = section_body_key(&raw, keys, tickets, rights_id, master_index)?;

    for section_id in 0..4usize {
        let entry_at = 0x240 + section_id * 0x10;
        let entry = raw.get(entry_at..entry_at + 8)?;
        let start_sector = u32::from_le_bytes(entry[0..4].try_into().ok()?) as u64;
        let end_sector = u32::from_le_bytes(entry[4..8].try_into().ok()?) as u64;
        if end_sector <= start_sector {
            continue;
        }
        let fs_at = 0x400 + section_id * 0x200;
        let fs = raw.get(fs_at..fs_at + 0x150)?;
        let encryption_type = *fs.get(0x4)?;
        if encryption_type != 3 {
            // 1 = none, 2 = XTS, 4 = BKTR (updates); RomFS sections of
            // base titles use AesCtr.
            continue;
        }
        let section_offset = start_sector * SECTOR_SIZE as u64;
        let size = (end_sector - start_sector) * SECTOR_SIZE as u64;
        if size > MAX_CONTROL_SECTION {
            continue;
        }
        let mut body = vec![0u8; size as usize];
        let mut file = std::fs::File::open(path).ok()?;
        std::io::Seek::seek(
            &mut file,
            std::io::SeekFrom::Start(nca.offset + section_offset),
        )
        .ok()?;
        std::io::Read::read_exact(&mut file, &mut body).ok()?;

        // Counter layout: section_ctr_high BE || section_ctr_low BE ||
        // (offset into the NCA / 16) BE.
        let ctr_low = u32::from_le_bytes(fs.get(0x140..0x144)?.try_into().ok()?);
        let ctr_high = u32::from_le_bytes(fs.get(0x144..0x148)?.try_into().ok()?);
        let mut counter = [0u8; 16];
        counter[0..4].copy_from_slice(&ctr_high.to_be_bytes());
        counter[4..8].copy_from_slice(&ctr_low.to_be_bytes());
        counter[8..16].copy_from_slice(&(section_offset / 16).to_be_bytes());
        Ctr128BE::<aes::Aes128>::new_from_slices(&body_key, &counter)
            .ok()?
            .apply_keystream(&mut body);

        // No magic to check up front: the IVFC superblock that precedes
        // the RomFS image varies in size, so control_meta_from_romfs
        // scans for the RomFS header itself.
        let meta = control_meta_from_romfs(&body);
        if meta.icon.is_some() || meta.title.is_some() {
            return Some((title_id, meta));
        }
    }
    None
}

/// Walks the decrypted section and reads the `icon_*.dat` icon and the
/// `control.nacp` application-title table from the RomFS image it
/// contains, in one pass.
fn control_meta_from_romfs(section: &[u8]) -> ControlMeta {
    walk_romfs_files(section).unwrap_or_default()
}

/// Locates the RomFS image inside the decrypted section by scanning for
/// the header signature (header size `0x50` as u64 LE) at 8-byte-aligned
/// offsets: the IVFC superblock that precedes the image varies in size.
fn romfs_image_at(section: &[u8]) -> Option<usize> {
    let pattern = 0x50u64.to_le_bytes();
    let last = section.len().checked_sub(0x50)?;
    (0..=last)
        .step_by(8)
        .find(|at| section[*at..*at + 8] == pattern)
}

/// One walk over the RomFS file-entry table capturing both targets.
/// Table ordering varies between build tools, so nothing beyond the
/// RomFS magic is assumed; the entry walk validates the candidate.
fn walk_romfs_files(section: &[u8]) -> Option<ControlMeta> {
    let romfs_at = romfs_image_at(section)?;
    let romfs = section.get(romfs_at..)?;

    let u64_at = |at: usize| -> Option<u64> {
        Some(u64::from_le_bytes(romfs.get(at..at + 8)?.try_into().ok()?))
    };
    let file_meta_offset = u64_at(0x38)? as usize;
    let file_meta_size = u64_at(0x40)? as usize;
    let file_data_offset = u64_at(0x48)? as usize;
    let meta = romfs.get(file_meta_offset..file_meta_offset.checked_add(file_meta_size)?)?;

    let mut found = ControlMeta::default();
    // File entries: parent(4) sibling(4) data_offset(8) data_size(8)
    // next_hash(4) name_len(4) name…; the name is inline and padded to 4.
    let mut offset = 0usize;
    while offset + 0x20 <= meta.len() && (found.icon.is_none() || found.title.is_none()) {
        let name_len =
            u32::from_le_bytes(meta[offset + 28..offset + 32].try_into().ok()?) as usize;
        let data_offset = u64::from_le_bytes(meta[offset + 8..offset + 16].try_into().ok()?);
        let data_size = u64::from_le_bytes(meta[offset + 16..offset + 24].try_into().ok()?);
        let name_at = offset + 0x20;
        let Some(name_end) = name_at.checked_add(name_len) else {
            break;
        };
        let Some(name) = meta
            .get(name_at..name_end)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
        else {
            break;
        };
        let want = if found.icon.is_none() && name.starts_with("icon_") {
            Some(data_size)
        } else if found.title.is_none() && name == "control.nacp" {
            Some(data_size.min(NACP_SIZE))
        } else {
            None
        };
        if let Some(size) = want
            .and_then(|size| usize::try_from(size).ok())
            .filter(|size| *size > 0)
        {
            let at = romfs_at
                .checked_add(file_data_offset)
                .and_then(|base| base.checked_add(usize::try_from(data_offset).unwrap_or(0)));
            if let Some(bytes) = at
                .and_then(|at| section.get(at..))
                .and_then(|rest| rest.get(..size))
            {
                if name.starts_with("icon_") {
                    found.icon = Some(bytes.to_vec());
                } else {
                    found.title = nacp::display_title(bytes);
                }
            }
        }
        offset = name_end.div_ceil(4) * 4;
    }
    Some(found)
}

#[cfg(test)]
mod tests {
    use crate::switch::synth::*;
    use super::*;

    /// Builds a full NSP around `synthetic_control_nca`.
    #[test]
    fn test_extract_control_meta_round_trips_synthetic_nsp() {
        let nca = synthetic_control_nca(0x0100a9400c9c2000);

        // PFS0 wrap: one entry, name in the string table.
        let name = b"9c4f2b099c79dedff9426c2722d09b18.nca";
        let mut table = Vec::new();
        let name_offset = table.len() as u32;
        table.extend_from_slice(name);
        table.push(0);
        while table.len() % 16 != 0 {
            table.push(0);
        }
        let mut nsp = Vec::new();
        nsp.extend_from_slice(b"PFS0");
        nsp.extend_from_slice(&1u32.to_le_bytes());
        nsp.extend_from_slice(&(table.len() as u32).to_le_bytes());
        nsp.extend_from_slice(&0u32.to_le_bytes());
        nsp.extend_from_slice(&0u64.to_le_bytes()); // entry data offset
        nsp.extend_from_slice(&(nca.len() as u64).to_le_bytes());
        nsp.extend_from_slice(&name_offset.to_le_bytes());
        nsp.extend_from_slice(&0u32.to_le_bytes());
        nsp.extend_from_slice(&table);
        nsp.extend_from_slice(&nca);

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("game.nsp");
        std::fs::write(&path, &nsp).unwrap();
        let keys_path = tmp.path().join("prod.keys");
        std::fs::write(&keys_path, test_keys_text()).unwrap();
        let keys = SwitchKeys::from_file(&keys_path).unwrap();

        let meta = extract_control_meta(&path, &keys).expect("control meta extracted");
        assert_eq!(meta.icon.as_deref(), Some(b"JPEGDATA".as_slice()));
        assert_eq!(meta.title.as_deref(), Some(SYNTH_TITLE));
    }

    /// A standalone control NCA — the NAND `Contents/Registered` shape —
    /// yields icon, application title, and the title id from its header.
    #[test]
    fn test_control_meta_from_nca_file_reports_all() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("9c4f2b099c79dedff9426c2722d09b18.nca");
        std::fs::write(&path, synthetic_control_nca(0x0100a9400c9c2000)).unwrap();
        let keys_path = tmp.path().join("prod.keys");
        std::fs::write(&keys_path, test_keys_text()).unwrap();
        let keys = SwitchKeys::from_file(&keys_path).unwrap();

        let (title_id, meta) = control_meta_from_nca_file(&path, &keys, &BTreeMap::new()).unwrap();
        assert_eq!(title_id, "0100a9400c9c2000");
        assert_eq!(meta.icon.as_deref(), Some(b"JPEGDATA".as_slice()));
        assert_eq!(meta.title.as_deref(), Some(SYNTH_TITLE));
    }

    /// Sector 1 must decrypt differently from sector 0 under the same
    /// key: the big-endian tweak has to reach the cipher.
    #[test]
    fn test_xts_tweak_reaches_second_sector() {
        let key = [0u8; 32];
        let mut unit = [0u8; SECTOR_SIZE];
        let sector_tweak =
            |sector: u128| -> xts_mode::Array<u8, aes::cipher::consts::U16> {
                xts_mode::Array(sector.to_be_bytes())
            };
        let xts = Xts128::<aes::Aes128>::new(
            aes::Aes128::new_from_slice(&key[..16]).unwrap(),
            aes::Aes128::new_from_slice(&key[16..]).unwrap(),
        );
        xts.decrypt_area(&mut unit, SECTOR_SIZE, 0, sector_tweak);
        let zero_sector = unit;
        let mut unit = [0u8; SECTOR_SIZE];
        xts.decrypt_area(&mut unit, SECTOR_SIZE, 1, sector_tweak);
        assert_ne!(&unit[..16], &zero_sector[..16]);
    }

    /// Wraps a RomFS image in a section, behind an IVFC superblock-sized
    /// gap like a real control NCA's.
    fn section_with_romfs(romfs: &[u8]) -> Vec<u8> {
        let mut section = vec![0u8; 0x100 + romfs.len()];
        section[0..4].copy_from_slice(b"IVFC");
        section[0x10..0x14].copy_from_slice(&4u32.to_le_bytes());
        let levels_at = 0x14 + 2 * 0x20;
        section[levels_at..levels_at + 8].copy_from_slice(&0x100u64.to_le_bytes());
        section[0x100..].copy_from_slice(romfs);
        section
    }

    /// RomFS image with two root files; `first` comes first in the
    /// file-entry table.
    fn two_file_romfs(first: (&[u8], &[u8]), second: (&[u8], &[u8])) -> Vec<u8> {
        let mut meta = Vec::new();
        let mut data = Vec::new();
        for (name, contents) in [first, second] {
            let mut entry = [0u8; 0x20];
            entry[8..16].copy_from_slice(&(data.len() as u64).to_le_bytes());
            entry[16..24].copy_from_slice(&(contents.len() as u64).to_le_bytes());
            entry[28..32].copy_from_slice(&(name.len() as u32).to_le_bytes());
            meta.extend_from_slice(&entry);
            meta.extend_from_slice(name);
            while meta.len() % 4 != 0 {
                meta.push(0);
            }
            data.extend_from_slice(contents);
        }
        let mut romfs = vec![0u8; 0x50 + meta.len() + data.len()];
        romfs[0..8].copy_from_slice(&0x50u64.to_le_bytes());
        romfs[0x38..0x40].copy_from_slice(&0x50u64.to_le_bytes());
        romfs[0x40..0x48].copy_from_slice(&(meta.len() as u64).to_le_bytes());
        romfs[0x48..0x50].copy_from_slice(&((0x50 + meta.len()) as u64).to_le_bytes());
        romfs[0x50..0x50 + meta.len()].copy_from_slice(&meta);
        romfs[0x50 + meta.len()..].copy_from_slice(&data);
        romfs
    }

    /// File-entry parsing: the name is inline and padded to 4 bytes, and
    /// the RomFS image is found by scanning past an IVFC superblock.
    #[test]
    fn test_control_meta_from_romfs_reads_inline_names() {
        // Build a minimal RomFS image: 0x50 header, one file entry with a
        // 5-byte name, name padded to 8, then the data block.
        let mut romfs = vec![0u8; 0x100];
        romfs[0..8].copy_from_slice(&0x50u64.to_le_bytes());
        romfs[0x38..0x40].copy_from_slice(&0x50u64.to_le_bytes()); // meta at 0x50
        romfs[0x40..0x48].copy_from_slice(&0x28u64.to_le_bytes()); // meta size (entry + name)
        romfs[0x48..0x50].copy_from_slice(&0x80u64.to_le_bytes()); // data at 0x80

        let mut entry = vec![0u8; 0x20];
        entry[8..16].copy_from_slice(&0u64.to_le_bytes()); // data offset
        entry[16..24].copy_from_slice(&4u64.to_le_bytes()); // data size
        entry[28..32].copy_from_slice(&5u32.to_le_bytes()); // name length
        entry.extend_from_slice(b"icon_en\0\0\0");
        romfs[0x50..0x50 + entry.len()].copy_from_slice(&entry);
        romfs[0x80..0x84].copy_from_slice(b"PNG!");

        let meta = control_meta_from_romfs(&section_with_romfs(&romfs));
        assert_eq!(meta.icon.as_deref(), Some(b"PNG!".as_slice()));
        assert_eq!(meta.title, None);
    }

    /// Both targets come out of one walk even when `control.nacp` sits
    /// ahead of every `icon_*` entry in the table.
    #[test]
    fn test_control_meta_from_romfs_captures_nacp_before_icon() {
        let nacp_bytes = crate::switch::nacp::test_table(&[(0, "Table Order")]);
        let romfs = two_file_romfs(
            (b"control.nacp", &nacp_bytes),
            (b"icon_en.dat", b"PNG!"),
        );

        let meta = control_meta_from_romfs(&section_with_romfs(&romfs));
        assert_eq!(meta.title.as_deref(), Some("Table Order"));
        assert_eq!(meta.icon.as_deref(), Some(b"PNG!".as_slice()));
    }

    /// A retail-shaped ticket (RSA-2048-SHA256): the 0x100-byte signature
    /// block plus padding puts the ticket data at 0x140, with the title
    /// key at +0x40 and the rights id at +0x160.
    #[test]
    fn test_parse_ticket_reads_rsa2048_ticket() {
        let data_at = 0x140usize;
        let mut buf = vec![0u8; data_at + 0x180];
        buf[0..4].copy_from_slice(&0x10004u32.to_le_bytes());
        buf[data_at + 0x40..data_at + 0x50].copy_from_slice(b"ENCRYPTEDTITLEKY");
        buf[data_at + 0x160..data_at + 0x170].copy_from_slice(b"RIGHTSID12345678");

        let (rights_id, title_key) = parse_ticket(&buf).unwrap();
        assert_eq!(&rights_id, b"RIGHTSID12345678");
        assert_eq!(&title_key, b"ENCRYPTEDTITLEKY");
    }

    #[test]
    fn test_parse_ticket_rejects_unknown_signature() {
        assert!(parse_ticket(&[0u8; 0x300]).is_none());
    }
}
