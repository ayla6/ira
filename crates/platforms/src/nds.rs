//! Nintendo DS ROM metadata: banner (icon + title) reading and the
//! RetroAchievements identification hash. Plain ROM files are parsed
//! directly; compressed containers (.zip/.7z/.zst) are streamed when the
//! caller opts into unpacking.

use std::io::Read;
use std::path::Path;

use md5::{Digest, Md5};

use crate::archives::{self, RangeCapture};

/// Metadata read from a DS ROM's header and banner.
pub struct DsRomInfo {
    /// Banner title, preferring English (UTF-16, NUL-trimmed).
    pub title: String,
    /// 32×32 RGBA8 icon, decoded from the banner's 4bpp tile layout.
    /// Color 0 is transparent.
    pub icon: Vec<u8>,
    /// No-Intro-style CRC32 of the whole ROM, lowercase hex.
    pub rom_crc32: String,
    /// RetroAchievements hash (MD5 over header + arm9 + arm7 + icon/title).
    pub rom_hash: String,
}

const HEADER_LEN: u64 = 0x160;
const BANNER_LEN: usize = 0xA00;
const ICON_TILE_BYTES: usize = 512;
const PALETTE_OFFSET: usize = 0x220;
const TITLES_OFFSET: usize = 0x240;
const TITLE_BYTES: usize = 128;

/// ROM file name extensions that hold a plain DS ROM.
pub fn is_ds_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "nds" | "ids" | "srl" | "dsi"
    )
}

fn u32_le(buf: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(buf.try_into().ok()?))
}

/// A SuperCard header prepended to the ROM shifts every header offset by
/// 512 bytes. Detected by its branch instruction at 0 and "DIDN" marker at
/// 0xB0, both of which precede the real ROM's header.
fn supercard_base(lead: &[u8]) -> u64 {
    let is_supercard = lead.len() > 0xB4
        && lead[0] == 0x2E
        && lead[1] == 0x00
        && lead[2] == 0x00
        && lead[3] == 0xEA
        && lead[0xB0] == 0x44
        && lead[0xB1] == 0x46
        && lead[0xB2] == 0x96
        && lead[0xB3] == 0;
    if is_supercard {
        512
    } else {
        0
    }
}

/// UTF-16 banner title, NUL-trimmed.
fn title_from_bytes(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .split('\0')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Decodes the banner's 32×32 4bpp icon: 4×4 tiles of 8×8 pixels, low
/// nibble first, against a BGR555 palette whose entry 0 is transparent.
/// Matches melonDS's `EmuInstance::romIcon`.
fn decode_icon(icon: &[u8], palette: &[u8]) -> Vec<u8> {
    let mut rgba = vec![0u8; 32 * 32 * 4];
    let mut pal = [0u16; 16];
    for (i, entry) in pal.iter_mut().enumerate() {
        let lo = *palette.get(i * 2).unwrap_or(&0);
        let hi = *palette.get(i * 2 + 1).unwrap_or(&0);
        *entry = u16::from_le_bytes([lo, hi]);
    }
    let expand = |v: u16| ((v as u32 * 255 + 15) / 31) as u8;
    let mut count = 0usize;
    for ytile in 0..4usize {
        for xtile in 0..4usize {
            for ypixel in 0..8usize {
                for xpixel in 0..8usize {
                    let byte = icon.get(count / 2).copied().unwrap_or(0);
                    let nibble = if count % 2 == 1 {
                        byte >> 4
                    } else {
                        byte & 0x0F
                    };
                    let entry = pal[nibble as usize % 16];
                    let dst = (ytile * 256 + ypixel * 32 + xtile * 8 + xpixel) * 4;
                    rgba[dst] = expand(entry & 0x1F);
                    rgba[dst + 1] = expand((entry >> 5) & 0x1F);
                    rgba[dst + 2] = expand((entry >> 10) & 0x1F);
                    rgba[dst + 3] = if nibble == 0 { 0 } else { 255 };
                    count += 1;
                }
            }
        }
    }
    rgba
}

/// Reads the banner title and icon from a captured 0xA00 banner block.
fn banner_info(banner: &[u8]) -> Option<(String, Vec<u8>)> {
    let titles_end = TITLES_OFFSET + TITLE_BYTES * 2;
    if banner.len() < titles_end {
        return None;
    }
    // Banner versions 1-3 cover DS and DSi titles; a zeroed or future
    // banner means this is not a readable DS ROM.
    let banner_version = u16::from_le_bytes([banner[0], banner[1]]);
    if !matches!(banner_version, 1..=3) {
        return None;
    }
    // Six title slots start at 0x240: Japanese first, English second.
    let english = title_from_bytes(&banner[TITLES_OFFSET + TITLE_BYTES..titles_end]);
    let title = if english.is_empty() {
        title_from_bytes(&banner[TITLES_OFFSET..TITLES_OFFSET + TITLE_BYTES])
    } else {
        english
    };
    let icon = decode_icon(
        &banner[0x20..0x20 + ICON_TILE_BYTES],
        &banner[PALETTE_OFFSET..],
    );
    Some((title, icon))
}

/// Reads title, icon and identification hashes from a DS ROM. Plain ROM
/// files are read directly; `.zip`/`.7z`/`.zst` containers are streamed
/// only when `unpack_archives` is on. The entry is selected by name for
/// archives; the whole file is the entry for `.zst` and plain ROMs.
pub fn read_rom_info(path: &Path, unpack_archives: bool) -> Option<DsRomInfo> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if archives::is_archive_extension(&ext) && !unpack_archives {
        return None;
    }
    let pick = |name: &str| {
        Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(is_ds_extension)
            .unwrap_or(false)
    };
    archives::with_entry_reader(path, &pick, |reader| read_from_stream(reader))
}

/// Reads only the banner icon from a DS ROM, without hashing. Always
/// handles archives — the NDS button must work even when bulk extraction is
/// off — and stops decompressing after the banner.
pub fn read_icon(path: &Path) -> Option<Vec<u8>> {
    let pick = |name: &str| {
        Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(is_ds_extension)
            .unwrap_or(false)
    };
    // Plain files: try direct seek first (fast, no streaming)
    if let Some(icon) = read_icon_plain(path) {
        return Some(icon);
    }
    archives::with_entry_reader(path, &pick, |reader| read_icon_from_stream(reader))
}

fn read_icon_plain(path: &Path) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    // Read past 0xB0 so a prepended SuperCard header can be detected; the
    // DS header itself is then parsed from `base`.
    let mut head = vec![0u8; 0xB4 + 0x100];
    file.read_exact(&mut head).ok()?;
    let base = supercard_base(&head);
    let header = head.get(base as usize..)?;
    let icon_addr = u32_le(header.get(0x68..0x6C)?)? as u64 + base;
    file.seek(SeekFrom::Start(icon_addr)).ok()?;
    let mut banner = vec![0u8; BANNER_LEN];
    file.read_exact(&mut banner).ok()?;
    let (_, icon) = banner_info(&banner)?;
    Some(icon)
}

/// Reads only the banner icon: decompresses just far enough to cover the
/// banner range, so archive entries cost at most a few hundred KiB of
/// streaming regardless of ROM size.
fn read_icon_from_stream(reader: &mut dyn Read) -> Option<Vec<u8>> {
    const LEAD_LEN: usize = 8192;
    let mut lead = vec![0u8; LEAD_LEN];
    reader.read_exact(&mut lead).ok()?;
    let base = supercard_base(&lead);
    let header = lead.get(base as usize..)?;
    let icon_addr = u32_le(header.get(0x68..0x6C)?)? as u64 + base;

    let mut capture = RangeCapture::new(vec![(icon_addr, BANNER_LEN)]);
    capture.feed(&lead);
    let mut chunk = [0u8; 64 * 1024];
    while !capture.all_filled() {
        let n = reader.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        capture.feed(&chunk[..n]);
    }
    let (_, icon) = banner_info(capture.captured(0))?;
    Some(icon)
}

/// Reads the four-character game code from the ROM header at 0x0C (e.g.
/// "AQQE") — the serial every DS title ships with. Works through
/// containers: only the first bytes of the entry are read.
pub fn read_serial(path: &Path) -> Option<String> {
    let pick = |name: &str| {
        Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(is_ds_extension)
            .unwrap_or(false)
    };
    archives::with_entry_reader(path, &pick, |reader| {
        let mut lead = vec![0u8; 512 + 0x10];
        reader.read_exact(&mut lead).ok()?;
        let base = supercard_base(&lead) as usize;
        serial_from_header(lead.get(base..)?)
    })
}

/// Header bytes 0..0x0C hold the internal game title (ASCII, NUL-padded)
/// and 0x0C..0x10 the game code (ASCII), which gates against reading
/// serials out of non-DS files.
fn serial_from_header(header: &[u8]) -> Option<String> {
    let title_ok = |byte: &u8| byte.is_ascii_graphic() || *byte == b' ' || *byte == 0;
    if !header.get(..0x0C)?.iter().all(title_ok) {
        return None;
    }
    let code = header.get(0x0C..0x10)?;
    if !code.iter().all(|byte| byte.is_ascii_graphic()) {
        return None;
    }
    let serial = String::from_utf8_lossy(code).trim().to_string();
    (!serial.is_empty()).then_some(serial)
}

/// One sequential pass over a ROM image: hashes the ranges rcheevos's
/// `rc_hash_nintendo_ds` uses and captures the banner.
fn read_from_stream(reader: &mut dyn Read) -> Option<DsRomInfo> {
    let mut lead = vec![0u8; 1024];
    reader.read_exact(&mut lead).ok()?;
    let base = supercard_base(&lead);
    let header = &lead[base as usize..base as usize + 512];
    let arm9_addr = u32_le(&header[0x20..0x24])? as u64 + base as u64;
    let arm9_size = u32_le(&header[0x2C..0x30])? as usize;
    let arm7_addr = u32_le(&header[0x30..0x34])? as u64 + base as u64;
    let arm7_size = u32_le(&header[0x3C..0x40])? as usize;
    let icon_addr = u32_le(&header[0x68..0x6C])? as u64 + base as u64;
    if arm9_size + arm7_size > 16 * 1024 * 1024 {
        // rcheevos's sanity check — not a DS ROM.
        return None;
    }

    let mut md5 = Md5::new();
    md5.update(&header[..HEADER_LEN as usize]);

    let mut capture = RangeCapture::new(vec![
        (arm9_addr, arm9_size),
        (arm7_addr, arm7_size),
        (icon_addr, BANNER_LEN),
    ]);
    capture.feed(&lead);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut chunk).unwrap_or(0);
        if read == 0 {
            break;
        }
        capture.feed(&chunk[..read]);
    }
    md5.update(capture.captured(0));
    md5.update(capture.captured(1));
    let (title, icon) = banner_info(capture.captured(2))?;
    md5.update(capture.captured(2));

    Some(DsRomInfo {
        title,
        icon,
        rom_crc32: capture.crc32_hex(),
        rom_hash: format!("{:x}", md5.finalize()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const ARM9_ADDR: usize = 0x4000;
    const ARM7_ADDR: usize = 0x5000;
    const ICON_ADDR: usize = 0x6000;

    /// Synthetic DS ROM: header, arm9/arm7 blocks, banner with a white
    /// palette entry and an English title. Matches rcheevos' header layout.
    fn nds_fixture() -> Vec<u8> {
        nds_fixture_with_banner_at(ICON_ADDR)
    }

    /// Same ROM with the banner placed elsewhere; large addresses force
    /// archive readers to stream several chunks before capturing it.
    fn nds_fixture_with_banner_at(icon_addr: usize) -> Vec<u8> {
        let mut rom = vec![0u8; icon_addr + BANNER_LEN];
        let arm9 = vec![0xAAu8; 0x100];
        let arm7 = vec![0xBBu8; 0x80];
        rom[0x20..0x24].copy_from_slice(&(ARM9_ADDR as u32).to_le_bytes());
        rom[0x2C..0x30].copy_from_slice(&(arm9.len() as u32).to_le_bytes());
        rom[0x30..0x34].copy_from_slice(&(ARM7_ADDR as u32).to_le_bytes());
        rom[0x3C..0x40].copy_from_slice(&(arm7.len() as u32).to_le_bytes());
        rom[0x68..0x6C].copy_from_slice(&(icon_addr as u32).to_le_bytes());
        rom[ARM9_ADDR..ARM9_ADDR + arm9.len()].copy_from_slice(&arm9);
        rom[ARM7_ADDR..ARM7_ADDR + arm7.len()].copy_from_slice(&arm7);

        let mut banner = vec![0u8; BANNER_LEN];
        banner[0..2].copy_from_slice(&1u16.to_le_bytes());
        banner[0x20] = 0x21;
        banner[PALETTE_OFFSET + 2..PALETTE_OFFSET + 4].copy_from_slice(&0x7FFEu16.to_le_bytes());
        let english: Vec<u8> = "Test DS Game"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        banner[TITLES_OFFSET + TITLE_BYTES..TITLES_OFFSET + TITLE_BYTES + english.len()]
            .copy_from_slice(&english);
        rom[icon_addr..icon_addr + banner.len()].copy_from_slice(&banner);
        rom
    }

    fn write_zip(name: &str, contents: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("roms.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, contents).unwrap();
        zip.finish().unwrap();
        (dir, zip_path)
    }

    fn write_fixture(rom: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(rom).unwrap();
        file
    }

    #[test]
    fn test_read_rom_info_reads_title_icon_and_hashes() {
        let rom = nds_fixture();
        let file = write_fixture(&rom);
        let info = read_rom_info(file.path(), false).unwrap();
        assert_eq!(info.title, "Test DS Game");
        assert_eq!(&info.icon[0..4], &[247, 255, 255, 255]);
        assert_eq!(&info.icon[4..8], &[0, 0, 0, 255]);
        assert_eq!(info.rom_crc32, format!("{:08x}", crc32fast::hash(&rom)));
    }

    #[test]
    fn test_ra_hash_matches_rcheevos_construction() {
        let rom = nds_fixture();
        let file = write_fixture(&rom);
        let mut md5 = Md5::new();
        md5.update(&rom[..HEADER_LEN as usize]);
        md5.update(&rom[ARM9_ADDR..ARM9_ADDR + 0x100]);
        md5.update(&rom[ARM7_ADDR..ARM7_ADDR + 0x80]);
        md5.update(&rom[ICON_ADDR..ICON_ADDR + BANNER_LEN]);
        let expected = format!("{:x}", md5.finalize());
        let info = read_rom_info(file.path(), false).unwrap();
        assert_eq!(info.rom_hash, expected);
    }

    #[test]
    fn test_ra_hash_skips_supercard_header() {
        let rom = nds_fixture();
        let plain_file = write_fixture(&rom);
        let mut wrapped = vec![0u8; 512];
        wrapped[0..4].copy_from_slice(&[0x2E, 0x00, 0x00, 0xEA]);
        wrapped[0xB0..0xB4].copy_from_slice(&[0x44, 0x46, 0x96, 0x00]);
        wrapped.extend_from_slice(&rom);
        let wrapped_file = write_fixture(&wrapped);
        let plain_hash = read_rom_info(plain_file.path(), false).unwrap().rom_hash;
        let wrapped_hash = read_rom_info(wrapped_file.path(), false).unwrap().rom_hash;
        assert_eq!(plain_hash, wrapped_hash);
    }

    #[test]
    fn test_read_rom_info_rejects_non_ds_file() {
        let file = write_fixture(&vec![0u8; 0x8000]);
        assert!(read_rom_info(file.path(), false).is_none());
    }

    #[test]
    fn test_read_rom_info_skips_archives_without_unpack_flag() {
        // The ZIP must be ignored entirely when unpacking is off: enriching
        // plain ROMs during scans must not decompress archives.
        let (_dir, zip_path) = write_zip("game.nds", &nds_fixture());
        assert!(read_rom_info(&zip_path, false).is_none());
        assert_eq!(
            read_rom_info(&zip_path, true).unwrap().title,
            "Test DS Game"
        );
    }

    /// The NDS icon button reads banners from archives regardless of the
    /// unpack setting, and a banner far into the ROM must still arrive:
    /// it needs more streamed chunks than lead + one buffer.
    #[test]
    fn test_read_icon_from_zip_beyond_first_chunk() {
        let rom = nds_fixture();
        let plain_file = write_fixture(&rom);
        let expected = read_icon(plain_file.path()).unwrap();

        // Far enough that 8 KiB lead + one 64 KiB chunk cannot cover it.
        let far_banner = 8192 + 64 * 1024 + 4096;
        let archived = nds_fixture_with_banner_at(far_banner);
        let (_dir, zip_path) = write_zip("game.nds", &archived);

        assert_eq!(read_icon(&zip_path), Some(expected));
    }

    #[test]
    fn test_read_serial_reads_header_game_code() {
        let mut rom = nds_fixture();
        rom[0..12].copy_from_slice(b"TEST GAME\0\0\0");
        rom[0x0C..0x10].copy_from_slice(b"AQQE");
        let file = write_fixture(&rom);
        assert_eq!(read_serial(file.path()).as_deref(), Some("AQQE"));

        // Same through an archive — the header is all a serial needs.
        let (_dir, zip_path) = write_zip("game.nds", &rom);
        assert_eq!(read_serial(&zip_path).as_deref(), Some("AQQE"));
    }

    #[test]
    fn test_read_serial_rejects_non_ds_header() {
        let file = write_fixture(&[0u8; 0x8000]);
        assert!(read_serial(file.path()).is_none());
    }
}
