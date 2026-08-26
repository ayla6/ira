use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::title::{three_dsx_key, title_from_filename};
use super::AzaharGame;

#[cfg(test)]
pub(super) mod fixtures;

/// Logical bytes read from the start of a ROM before parsing headers; large
/// enough to cover cartridge partition 0, its ExeFS and SMDH. Z3DS frames
/// are at least 256 KiB (larger for CIA-derived content), so the first
/// frame suffices.
const ROM_PREFIX_LEN: usize = 64 * 1024;

fn read_at(file: &mut std::fs::File, offset: u64, buf: &mut [u8]) -> Option<()> {
    file.seek(SeekFrom::Start(offset)).ok()?;
    file.read_exact(buf).ok()
}

fn u32_le(buf: &[u8]) -> u32 {
    u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]])
}

/// Reads logical (decompressed) bytes from a ROM, transparently unwrapping
/// Z3DS containers. Layout per Azahar's `Z3DSFileHeader`: a 0x20 header
/// (`Z3DS` magic, version 1), an optional metadata blob of
/// `header_size + metadata_size` bytes, then seekable-zstd frames.
struct RomSource {
    inner: RomInner,
    logical_pos: u64,
}

enum RomInner {
    Plain(std::fs::File),
    Z3ds(zstd::stream::read::Decoder<'static, std::io::BufReader<std::fs::File>>),
}

impl RomSource {
    fn open(mut file: std::fs::File) -> Option<Self> {
        let mut header = [0u8; 0x20];
        read_at(&mut file, 0, &mut header)?;
        let inner = if &header[0..4] == b"Z3DS" && header[8] == 1 {
            let frames_at = u16::from_le_bytes([header[0x0A], header[0x0B]]) as u64
                + u32_le(&header[0x0C..0x10]) as u64;
            file.seek(SeekFrom::Start(frames_at)).ok()?;
            // Decoder buffers internally and handles concatenated frames.
            let decoder = zstd::stream::read::Decoder::new(file).ok()?;
            RomInner::Z3ds(decoder)
        } else {
            // Rewind past the header peek so sequential reads start at 0.
            file.seek(SeekFrom::Start(0)).ok()?;
            RomInner::Plain(file)
        };
        Some(RomSource {
            inner,
            logical_pos: 0,
        })
    }

    /// Reads the next `len` logical bytes; a short result means end of data.
    fn read(&mut self, len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        let mut filled = 0;
        while filled < len {
            let result = match &mut self.inner {
                RomInner::Z3ds(decoder) => decoder.read(&mut buf[filled..]),
                RomInner::Plain(file) => file.read(&mut buf[filled..]),
            };
            match result {
                Ok(0) | Err(_) => break,
                Ok(n) => filled += n,
            }
        }
        self.logical_pos += filled as u64;
        buf.truncate(filled);
        buf
    }

    /// Reads `len` logical bytes at `offset`. Compressed sources can only
    /// move forward through the zstd stream.
    fn read_range(&mut self, offset: u64, len: usize) -> Option<Vec<u8>> {
        match &mut self.inner {
            RomInner::Plain(file) => {
                file.seek(SeekFrom::Start(offset)).ok()?;
                self.logical_pos = offset;
            }
            RomInner::Z3ds(_) => {}
        }
        if offset < self.logical_pos {
            return None;
        }
        while self.logical_pos < offset {
            let want = ((offset - self.logical_pos) as usize).min(8192);
            if self.read(want).len() < want {
                return None;
            }
        }
        let buf = self.read(len);
        (buf.len() == len).then_some(buf)
    }
}

/// Validates an SMDH block and extracts its short title and large icon.
/// The magic gates against encrypted ExeFS contents reading as garbage.
struct Smdh {
    title: String,
    /// 48×48 linear RGB565 pixels (0x1200 bytes) from the large-icon slot.
    icon: Vec<u8>,
}

/// Full SMDH size (see Azahar's `Loader::SMDH`): header, 16 titles, ratings
/// and flags, the 24×24 small icon, then the 48×48 large icon at 0x24C0.
const SMDH_LEN: usize = 0x36C0;
const LARGE_ICON_OFFSET: usize = 0x24C0;
const ICON_PIXELS: usize = 48 * 48;

/// Undo the 8×8 Z-order (Morton) tiling 3DS icon textures use, matching
/// Azahar's `MortonInterleave` + `GetMortonOffset`.
fn decode_large_icon(raw: &[u8]) -> Vec<u8> {
    const XLUT: [u32; 8] = [0x00, 0x01, 0x04, 0x05, 0x10, 0x11, 0x14, 0x15];
    const YLUT: [u32; 8] = [0x00, 0x02, 0x08, 0x0A, 0x20, 0x22, 0x28, 0x2A];
    let mut linear = vec![0u8; ICON_PIXELS * 2];
    for y in 0..48u32 {
        for x in 0..48u32 {
            let morton = (XLUT[(x % 8) as usize] + YLUT[(y % 8) as usize]) as usize;
            let src = (morton + (x & !7) as usize * 8) * 2;
            let dst = (y as usize * 48 + x as usize) * 2;
            linear[dst..dst + 2].copy_from_slice(&raw[src..src + 2]);
        }
    }
    linear
}

fn smdh_from_bytes(bytes: &[u8]) -> Option<Smdh> {
    if bytes.len() < SMDH_LEN || &bytes[0..4] != b"SMDH" {
        return None;
    }
    let bytes = &bytes[..SMDH_LEN];
    smdh_short_title(bytes.try_into().ok()?).map(|title| Smdh {
        title,
        icon: decode_large_icon(&bytes[LARGE_ICON_OFFSET..]),
    })
}

/// SMDH stores 16 regional titles at offset 8, each 0x200 bytes starting
/// with the UTF-16 short description. Prefer English, then any other.
fn smdh_short_title(smdh: &[u8; SMDH_LEN]) -> Option<String> {
    for region in [1usize, 0, 2, 3, 4, 5] {
        let start = 8 + region * 0x200;
        let units: Vec<u16> = smdh[start..start + 0x80]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let title = String::from_utf16_lossy(&units);
        let title = title.split('\0').next().unwrap_or("").trim();
        if !title.is_empty() {
            return Some(title.to_string());
        }
    }
    None
}

/// NCSD (cartridge dumps, .3ds/.cci) wraps the game NCCH partition; NCCH
/// is the format of .cxi/.app contents. Both keep the title ID at header
/// offset 0x108.
struct ParsedRom {
    title_id: u64,
    ncch_offset: usize,
}

fn parse_ncsd_ncch(prefix: &[u8]) -> Option<ParsedRom> {
    let magic = prefix.get(0x100..0x104)?;
    let title_id = u64::from_le_bytes(prefix.get(0x108..0x110)?.try_into().ok()?);
    if magic == b"NCSD" {
        Some(ParsedRom {
            title_id,
            ncch_offset: u32_le(prefix.get(0x120..0x124)?) as usize * 0x200,
        })
    } else if magic == b"NCCH" {
        Some(ParsedRom {
            title_id,
            ncch_offset: 0,
        })
    } else {
        None
    }
}

/// Reads `len` logical bytes at `offset`, serving from the already-read
/// prefix when possible. Z3DS sources can only move forward, so a region
/// straddling the prefix boundary combines the prefix tail with fresh reads.
fn read_slice(source: &mut RomSource, prefix: &[u8], start: usize, len: usize) -> Option<Vec<u8>> {
    if let Some(bytes) = prefix.get(start..start + len) {
        return Some(bytes.to_vec());
    }
    let mut out = Vec::with_capacity(len);
    if start < prefix.len() {
        out.extend_from_slice(&prefix[start..]);
        out.extend_from_slice(&source.read_range(prefix.len() as u64, len - out.len())?);
    } else {
        out.extend_from_slice(&source.read_range(start as u64, len)?);
    }
    (out.len() == len).then_some(out)
}

/// Reads the game name and icon from the ExeFS `icon` (SMDH) of an NCCH
/// partition. Field offsets follow Azahar's `NCCH_Header`: the ExeFS
/// location lives at +0x1A0, and +0x18F holds the crypto flags. Encrypted
/// titles use title-key AES-CTR for ExeFS contents, so this only succeeds
/// on unencrypted/fixed-key dumps — which is all Azahar itself accepts.
fn read_smdh_for_ncch(source: &mut RomSource, prefix: &[u8], ncch_offset: usize) -> Option<Smdh> {
    let ncch = prefix.get(ncch_offset..ncch_offset + 0x200)?;
    if ncch.get(0x100..0x104) != Some(b"NCCH") {
        return None;
    }
    let exefs_offset = ncch_offset + u32_le(ncch.get(0x1A0..0x1A4)?) as usize * 0x200;
    let exefs = read_slice(source, prefix, exefs_offset, 0x200)?;
    for entry in 0..8 {
        let fields = exefs.get(entry * 0x10..entry * 0x10 + 0x10)?;
        if fields.get(0..8) != Some(b"icon\0\0\0\0") {
            continue;
        }
        let icon_offset = u32_le(fields.get(8..12)?) as usize;
        let smdh_start = exefs_offset + 0x200 + icon_offset;
        return read_slice(source, prefix, smdh_start, SMDH_LEN).and_then(|b| smdh_from_bytes(&b));
    }
    None
}

/// 3DSX homebrew header: the SMDH offset/size live at 0x20/0x24 and are
/// absolute from file start (see Azahar's `THREEDSX_Header`); both are zero
/// when the homebrew embeds no SMDH.
fn parse_3dsx_smdh(prefix: &[u8]) -> Option<(u64, usize)> {
    if prefix.get(0..4) != Some(b"3DSX") {
        return None;
    }
    let offset = u32_le(prefix.get(0x20..0x24)?) as u64;
    let size = u32_le(prefix.get(0x24..0x28)?) as usize;
    (offset != 0 && size >= SMDH_LEN).then_some((offset, SMDH_LEN))
}

pub(super) fn scan_rom_file(path: &Path) -> Option<AzaharGame> {
    let mut source = RomSource::open(std::fs::File::open(path).ok()?)?;
    let prefix = source.read(ROM_PREFIX_LEN);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let fallback_title = title_from_filename(&stem);

    // 3DSX homebrew carries an SMDH but no title ID; the fields are
    // optional and absent (zero) in headerless homebrew.
    if prefix.get(0..4) == Some(b"3DSX") {
        let (title, icon) = parse_3dsx_smdh(&prefix)
            .and_then(|(smdh_offset, _)| {
                read_slice(&mut source, &prefix, smdh_offset as usize, SMDH_LEN)
                    .and_then(|bytes| smdh_from_bytes(&bytes))
            })
            .map(|smdh| (smdh.title, Some(smdh.icon)))
            .unwrap_or_else(|| (fallback_title.clone(), None));
        return Some(AzaharGame {
            title_id: three_dsx_key(&fallback_title),
            title,
            icon,
            game_path: path.to_path_buf(),
        });
    }

    let parsed = parse_ncsd_ncch(&prefix)?;
    let (title, icon) = match read_smdh_for_ncch(&mut source, &prefix, parsed.ncch_offset) {
        Some(smdh) => (smdh.title, Some(smdh.icon)),
        None => (fallback_title, None),
    };
    Some(AzaharGame {
        title_id: format!("{:016x}", parsed.title_id),
        title,
        icon,
        game_path: path.to_path_buf(),
    })
}

/// Scans an installed title's main content file (an NCCH, optionally
/// Z3DS-compressed). Unlike ROM dumps the file name is a content hash, so a
/// missing SMDH leaves both title and icon empty.
pub(super) fn scan_installed_content(path: &Path) -> Option<AzaharGame> {
    let mut source = RomSource::open(std::fs::File::open(path).ok()?)?;
    let prefix = source.read(ROM_PREFIX_LEN);
    let parsed = parse_ncsd_ncch(&prefix)?;
    let (title, icon) = match read_smdh_for_ncch(&mut source, &prefix, parsed.ncch_offset) {
        Some(smdh) => (smdh.title, Some(smdh.icon)),
        None => (String::new(), None),
    };
    Some(AzaharGame {
        title_id: format!("{:016x}", parsed.title_id),
        title,
        icon,
        game_path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::fixtures::{
        cxi_fixture, smdh_bytes, three_dsx_fixture, z3ds_fixture, z3ds_fixture_with_metadata,
    };
    use super::*;

    #[test]
    fn test_scan_rom_file_reads_smdh_title() {
        let tmp = tempfile::tempdir().unwrap();
        let rom = tmp.path().join("game.cxi");
        std::fs::write(&rom, cxi_fixture(0x00040000000E5C00)).unwrap();

        let game = scan_rom_file(&rom).unwrap();
        assert_eq!(game.title_id, "00040000000e5c00");
        assert_eq!(game.title, "Test Game");
        let icon = game.icon.unwrap();
        assert_eq!(icon.len(), 48 * 48 * 2);
        // The fixture icon is Morton-tiled: red at (0,0), green at (0,1).
        assert_eq!(&icon[0..2], &0xF800u16.to_le_bytes());
        assert_eq!(&icon[48 * 2..48 * 2 + 2], &0x07E0u16.to_le_bytes());
    }

    #[test]
    fn test_scan_rom_file_ncsd_wraps_partition() {
        let tmp = tempfile::tempdir().unwrap();
        let cxi = cxi_fixture(0x00040000000CE000);
        let mut rom = vec![0u8; 0x400 + cxi.len()];
        rom[0x100..0x104].copy_from_slice(b"NCSD");
        rom[0x108..0x110].copy_from_slice(&0x00040000000CE000u64.to_le_bytes());
        // Partition 0 starts at media offset 2 (= 0x400).
        rom[0x120..0x124].copy_from_slice(&2u32.to_le_bytes());
        rom[0x400..0x400 + cxi.len()].copy_from_slice(&cxi);

        std::fs::write(tmp.path().join("dump.cci"), &rom).unwrap();
        let game = scan_rom_file(&tmp.path().join("dump.cci")).unwrap();
        assert_eq!(game.title_id, "00040000000ce000");
        assert_eq!(game.title, "Test Game");
    }

    #[test]
    fn test_scan_rom_file_encrypted_falls_back_to_filename() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rom = cxi_fixture(0x00040000000E5C00);
        // Clobber the ExeFS with garbage, as an encrypted dump would read.
        for byte in &mut rom[0x400..] {
            *byte = 0xA5;
        }
        let path = tmp.path().join("00040000000E5C00 Shin Game (U).cci");
        std::fs::write(&path, &rom).unwrap();

        let game = scan_rom_file(&path).unwrap();
        assert_eq!(game.title_id, "00040000000e5c00");
        assert_eq!(game.title, "Shin Game");
    }

    #[test]
    fn test_scan_rom_file_rejects_unknown_header() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("junk.3ds");
        std::fs::write(&path, vec![0u8; 0x1000]).unwrap();
        assert!(scan_rom_file(&path).is_none());
    }

    #[test]
    fn test_scan_zcci_reads_title_through_zstd() {
        let tmp = tempfile::tempdir().unwrap();
        let rom = tmp.path().join("game.zcci");
        std::fs::write(&rom, z3ds_fixture(&cxi_fixture(0x00040000000E5C00))).unwrap();

        let game = scan_rom_file(&rom).unwrap();
        assert_eq!(game.title_id, "00040000000e5c00");
        assert_eq!(game.title, "Test Game");
    }

    #[test]
    fn test_scan_zcci_skips_metadata_block() {
        let tmp = tempfile::tempdir().unwrap();
        let rom = tmp.path().join("game.zcci");
        let inner = cxi_fixture(0x00040000000CE000);
        std::fs::write(&rom, z3ds_fixture_with_metadata(&inner, &[0xAA; 8])).unwrap();

        let game = scan_rom_file(&rom).unwrap();
        assert_eq!(game.title_id, "00040000000ce000");
        assert_eq!(game.title, "Test Game");
    }

    #[test]
    fn test_scan_3dsx_reads_smdh() {
        let tmp = tempfile::tempdir().unwrap();
        let rom = tmp.path().join("Game Homebrew.3dsx");
        std::fs::write(&rom, three_dsx_fixture(Some(&smdh_bytes("Test Game")))).unwrap();

        let game = scan_rom_file(&rom).unwrap();
        assert_eq!(game.title_id, "3dsx-game-homebrew");
        assert_eq!(game.title, "Test Game");
    }

    #[test]
    fn test_scan_3dsx_without_smdh_uses_filename() {
        let tmp = tempfile::tempdir().unwrap();
        let rom = tmp.path().join("Fruit Punch [v1.2].3dsx");
        std::fs::write(&rom, three_dsx_fixture(None)).unwrap();

        let game = scan_rom_file(&rom).unwrap();
        assert_eq!(game.title_id, "3dsx-fruit-punch");
        assert_eq!(game.title, "Fruit Punch");
    }

    #[test]
    fn test_scan_z3dsx_homebrew() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = three_dsx_fixture(Some(&smdh_bytes("Test Game")));
        let rom = tmp.path().join("tool.z3dsx");
        std::fs::write(&rom, z3ds_fixture(&inner)).unwrap();

        let game = scan_rom_file(&rom).unwrap();
        assert_eq!(game.title_id, "3dsx-tool");
        assert_eq!(game.title, "Test Game");
    }
}
