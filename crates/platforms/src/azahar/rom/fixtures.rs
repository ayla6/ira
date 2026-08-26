//! Builders for synthetic ROM files shared by the azahar test modules.

/// Full SMDH: header + 16 titles + flags + small icon + 48×48 large icon.
pub const SMDH_FIXTURE_LEN: usize = 0x36C0;

const LARGE_ICON_OFFSET: usize = 0x24C0;

/// 8×8 Z-order interleave, matching Azahar's `MortonInterleave`.
fn morton_offset(x: u32, y: u32) -> usize {
    const XLUT: [u32; 8] = [0x00, 0x01, 0x04, 0x05, 0x10, 0x11, 0x14, 0x15];
    const YLUT: [u32; 8] = [0x00, 0x02, 0x08, 0x0A, 0x20, 0x22, 0x28, 0x2A];
    (XLUT[(x % 8) as usize] + YLUT[(y % 8) as usize]) as usize * 2
}

pub fn smdh_bytes(english: &str) -> Vec<u8> {
    let mut smdh = vec![0u8; SMDH_FIXTURE_LEN];
    smdh[0..4].copy_from_slice(b"SMDH");
    let mut utf16: Vec<u16> = english.encode_utf16().collect();
    utf16.push(0);
    for (i, unit) in utf16.iter().enumerate() {
        let start = 8 + 0x200 + i * 2; // region 1 = English
        smdh[start..start + 2].copy_from_slice(&unit.to_le_bytes());
    }
    // Distinctive Morton-tiled icon pixels: red at (0,0), green at (0,1),
    // blue at (10,9) — the latter sits in tile row 1, so decoding it
    // correctly requires the per-tile-row stride, catching degenerate
    // layouts where every tile row reads tile row 0 again.
    let icon = LARGE_ICON_OFFSET;
    smdh[icon..icon + 2].copy_from_slice(&0xF800u16.to_le_bytes());
    let green = icon + morton_offset(0, 1);
    smdh[green..green + 2].copy_from_slice(&0x07E0u16.to_le_bytes());
    const BLUE_X: u32 = 10;
    const BLUE_Y: u32 = 9;
    let coarse = ((BLUE_Y & !7) as usize) * 48 + ((BLUE_X & !7) as usize) * 8;
    let blue_px = morton_offset(BLUE_X % 8, BLUE_Y % 8) / 2 + coarse;
    smdh[icon + blue_px * 2..icon + blue_px * 2 + 2].copy_from_slice(&0x001Fu16.to_le_bytes());
    smdh
}

/// Wraps `inner` in a Z3DS container: 0x20-byte header, optional
/// metadata, then the payload as a single zstd frame.
pub fn z3ds_fixture_with_metadata(inner: &[u8], metadata: &[u8]) -> Vec<u8> {
    let compressed = zstd::stream::encode_all(inner, 3).unwrap();
    let mut out = Vec::with_capacity(0x20 + metadata.len() + compressed.len());
    out.extend_from_slice(b"Z3DS");
    out.extend_from_slice(b"NCCH");
    out.push(1);
    out.push(0);
    out.extend_from_slice(&0x20u16.to_le_bytes());
    out.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    out.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    out.extend_from_slice(&(inner.len() as u64).to_le_bytes());
    assert_eq!(out.len(), 0x20);
    out.extend_from_slice(metadata);
    out.extend_from_slice(&compressed);
    out
}

pub fn z3ds_fixture(inner: &[u8]) -> Vec<u8> {
    z3ds_fixture_with_metadata(inner, &[])
}

/// Builds a minimal 3DSX homebrew with an optional embedded SMDH.
pub fn three_dsx_fixture(smdh: Option<&[u8]>) -> Vec<u8> {
    let mut rom = vec![0u8; 0x100];
    rom[0..4].copy_from_slice(b"3DSX");
    rom[4..6].copy_from_slice(&0x2Cu16.to_le_bytes());
    if let Some(smdh) = smdh {
        let offset = rom.len() as u32;
        rom.extend_from_slice(smdh);
        rom[0x20..0x24].copy_from_slice(&offset.to_le_bytes());
        rom[0x24..0x28].copy_from_slice(&(smdh.len() as u32).to_le_bytes());
    }
    rom
}

/// Builds a minimal unencrypted CXI with an SMDH-carrying ExeFS.
pub fn cxi_fixture(title_id: u64) -> Vec<u8> {
    let smdh = smdh_bytes("Test Game");
    let smdh_offset = 0x600usize;
    let mut rom = vec![0u8; smdh_offset + smdh.len()];
    rom[0x100..0x104].copy_from_slice(b"NCCH");
    rom[0x108..0x110].copy_from_slice(&title_id.to_le_bytes());
    // +0x18F crypto flags: bit2 = no crypto, like a decrypted dump.
    rom[0x18F] = 0x04;
    let exefs_offset = 0x400u32; // bytes; media units below
    rom[0x1A0..0x1A4].copy_from_slice(&(exefs_offset / 0x200).to_le_bytes());
    // ExeFS header at 0x400: "icon" entry pointing at offset 0.
    rom[0x400..0x408].copy_from_slice(b"icon\0\0\0\0");
    rom[0x408..0x40C].copy_from_slice(&0u32.to_le_bytes());
    rom[smdh_offset..smdh_offset + smdh.len()].copy_from_slice(&smdh);
    rom
}
