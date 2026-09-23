//! Block-compressed NCA payloads (NCZ) inside NSZ/XCZ containers.
//!
//! Layout per the reference compressor (`references/nsz`): each
//! compressed NCA entry keeps its first 0x4000 bytes verbatim (NCA
//! header + section tables, so header decryption never knows the
//! difference), then one `NCZSECTN` header listing the encryption
//! sections, then one `NCZBLOCK` header with the per-block compressed
//! sizes. Blocks map decompressed space from 0x4000 on: block `i`
//! covers `[0x4000 + i * S, 0x4000 + (i + 1) * S)`. A block stored at
//! or above its decompressed size is verbatim, anything smaller is
//! zstd. Solid-compressed files have no per-entry table and read as
//! `None` here; so do plain NCAs and verbatim-stored sections, whose
//! callers fall back to the plain read.
//!
//! Returned bytes are decompressed PLAINTEXT: the compressor reads
//! through the section crypto, so unlike the plain path there is
//! nothing left to decrypt. Callers must skip their CTR pass for
//! these bytes.

use std::io::Read;
use std::path::Path;

/// NCA prefix every NCZ entry keeps verbatim; the block map covers the
/// decompressed bytes from here on.
const VERBATIM_PREFIX: u64 = 0x4000;
/// Block exponents outside this range are corrupt (per the reference).
const MIN_BLOCK_EXPONENT: u8 = 14;
const MAX_BLOCK_EXPONENT: u8 = 32;
/// Refuses absurd block counts before allocating the size table.
const MAX_BLOCKS: u64 = 1 << 20;
/// Refuses absurd single blocks; control sections are megabytes.
const MAX_BLOCK_BYTES: u64 = 64 * 1024 * 1024;

struct BlockMap {
    block_size: u64,
    decompressed_size: u64,
    compressed_sizes: Vec<u32>,
    /// Absolute file offset of the first block's bytes.
    data_start: u64,
}

/// Reads decompressed bytes `[start, start + len)` of one container
/// entry's NCA payload, capped at `max_len`. Bytes below the verbatim
/// prefix come straight from the file; the rest go through the entry's
/// NCZ block map. `None` means "not block-compressed or unreadable" —
/// callers fall back to the plain read, which then either works (plain
/// NCA) or fails its own checks.
pub(super) fn read_range(
    path: &Path,
    entry_offset: u64,
    start: u64,
    len: u64,
    max_len: u64,
) -> Option<Vec<u8>> {
    if len == 0 || len > max_len {
        return None;
    }
    let end = start.checked_add(len)?;
    let mut file = std::fs::File::open(path).ok()?;
    let map = parse_block_map(&mut file, entry_offset)?;
    let mut out = Vec::with_capacity(len as usize);
    if start < VERBATIM_PREFIX {
        let plain = (VERBATIM_PREFIX - start).min(len);
        out.extend_from_slice(&read_at(&mut file, entry_offset + start, plain)?);
    }
    let compressed_start = start.max(VERBATIM_PREFIX);
    if compressed_start < end {
        out.extend_from_slice(&read_compressed(
            &mut file,
            &map,
            compressed_start,
            end - compressed_start,
        )?);
    }
    (out.len() as u64 == len).then_some(out)
}

fn parse_block_map(file: &mut std::fs::File, entry_offset: u64) -> Option<BlockMap> {
    use std::io::{Seek, SeekFrom};

    let mut cursor = entry_offset + VERBATIM_PREFIX;
    let mut magic = [0u8; 8];
    file.seek(SeekFrom::Start(cursor)).ok()?;
    std::io::Read::read_exact(file, &mut magic).ok()?;
    if &magic != b"NCZSECTN" {
        return None;
    }
    cursor += 8;
    let sections = read_u64(file, &mut cursor)?;
    if sections == 0 || sections > 16 {
        return None;
    }
    // Section entries are 64 bytes each; only their presence matters
    // here — keys and counters come from the decrypted NCA header.
    cursor += sections * 64;
    file.seek(SeekFrom::Start(cursor)).ok()?;
    std::io::Read::read_exact(file, &mut magic).ok()?;
    if &magic != b"NCZBLOCK" {
        return None;
    }
    cursor += 8;
    let mut versioned = [0u8; 4];
    file.seek(SeekFrom::Start(cursor)).ok()?;
    std::io::Read::read_exact(file, &mut versioned).ok()?;
    let exponent = versioned[3];
    if !(MIN_BLOCK_EXPONENT..=MAX_BLOCK_EXPONENT).contains(&exponent) {
        return None;
    }
    cursor += 4;
    let blocks = read_u32(file, &mut cursor)? as u64;
    if blocks == 0 || blocks > MAX_BLOCKS {
        return None;
    }
    let decompressed_size = read_u64(file, &mut cursor)?;
    let block_size = 1u64 << exponent;
    if decompressed_size == 0 || decompressed_size > blocks.saturating_mul(block_size) {
        return None;
    }
    let mut compressed_sizes = Vec::with_capacity(blocks.min(1 << 16) as usize);
    for _ in 0..blocks {
        let size = read_u32(file, &mut cursor)?;
        if size == 0 {
            return None;
        }
        compressed_sizes.push(size);
    }
    Some(BlockMap {
        block_size,
        decompressed_size,
        compressed_sizes,
        data_start: cursor,
    })
}

fn read_compressed(
    file: &mut std::fs::File,
    map: &BlockMap,
    start: u64,
    len: u64,
) -> Option<Vec<u8>> {
    // Block ids live in decompressed space relative to the verbatim
    // prefix, exactly like the reference reader.
    let relative = start.checked_sub(VERBATIM_PREFIX)?;
    let end = relative.checked_add(len)?;
    if end > map.decompressed_size {
        return None;
    }
    let first = relative / map.block_size;
    let last = (end - 1) / map.block_size;
    let block_count = map.compressed_sizes.len() as u64;
    if last >= block_count {
        return None;
    }
    let mut compressed_at = map.data_start;
    for size in map.compressed_sizes.iter().take(first as usize) {
        compressed_at += u64::from(*size);
    }
    let mut out = Vec::with_capacity(len as usize);
    for id in first..=last {
        let compressed_size = u64::from(map.compressed_sizes[id as usize]);
        let decompressed_size = if id == block_count - 1 {
            match map.decompressed_size % map.block_size {
                0 => map.block_size,
                remainder => remainder,
            }
        } else {
            map.block_size
        };
        let raw = read_at(file, compressed_at, compressed_size)?;
        if compressed_size < decompressed_size {
            out.extend_from_slice(&zstd_decode(&raw, decompressed_size)?);
        } else {
            if raw.len() as u64 != decompressed_size {
                return None;
            }
            out.extend_from_slice(&raw);
        }
        compressed_at += compressed_size;
    }
    let skip = (relative % map.block_size) as usize;
    out.truncate(skip + len as usize);
    out.drain(..skip);
    Some(out)
}

fn zstd_decode(raw: &[u8], expected: u64) -> Option<Vec<u8>> {
    if expected == 0 || expected > MAX_BLOCK_BYTES {
        return None;
    }
    let mut decoder = zstd::stream::read::Decoder::new(raw).ok()?;
    let mut out = Vec::with_capacity(expected.min(1 << 20) as usize);
    decoder.read_to_end(&mut out).ok()?;
    (out.len() as u64 == expected).then_some(out)
}

fn read_at(file: &mut std::fs::File, offset: u64, len: u64) -> Option<Vec<u8>> {
    use std::io::{Seek, SeekFrom};

    if len == 0 || len > MAX_BLOCK_BYTES {
        return None;
    }
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len as usize];
    std::io::Read::read_exact(file, &mut buf).ok()?;
    Some(buf)
}

fn read_u32(file: &mut std::fs::File, cursor: &mut u64) -> Option<u32> {
    let bytes = read_at(file, *cursor, 4)?;
    *cursor += 4;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u64(file: &mut std::fs::File, cursor: &mut u64) -> Option<u64> {
    let bytes = read_at(file, *cursor, 8)?;
    *cursor += 8;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture NCZ entry: verbatim prefix, one section entry, block map
    /// over zstd-compressed 16 KiB blocks of `payload` (the decompressed
    /// bytes past 0x4000). Set `raw_block` to store one block verbatim
    /// instead of compressed.
    fn ncz_fixture(payload: &[u8], raw_block: Option<usize>) -> Vec<u8> {
        const EXPONENT: u8 = 14;
        const BLOCK: usize = 1 << EXPONENT;
        let mut out = vec![0xABu8; VERBATIM_PREFIX as usize];
        out.extend_from_slice(b"NCZSECTN");
        out.extend_from_slice(&1u64.to_le_bytes());
        out.extend_from_slice(&VERBATIM_PREFIX.to_le_bytes()); // offset
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes()); // size
        out.extend_from_slice(&3u64.to_le_bytes()); // cryptoType
        out.extend_from_slice(&[0u8; 8]); // padding
        out.extend_from_slice(&[0u8; 16]); // key
        out.extend_from_slice(&[0u8; 16]); // counter
        out.extend_from_slice(b"NCZBLOCK");
        out.extend_from_slice(&[2u8, 1u8, 0u8, EXPONENT]);
        let blocks = payload.chunks(BLOCK).collect::<Vec<_>>();
        out.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        let sizes_at = out.len();
        out.extend_from_slice(&vec![0u8; blocks.len() * 4]);
        let mut sizes = Vec::new();
        for (i, chunk) in blocks.iter().enumerate() {
            if raw_block == Some(i) {
                sizes.push(chunk.len() as u32);
                out.extend_from_slice(chunk);
            } else {
                let compressed = zstd::stream::encode_all(&chunk[..], 3).unwrap();
                sizes.push(compressed.len() as u32);
                out.extend_from_slice(&compressed);
            }
        }
        for (i, size) in sizes.iter().enumerate() {
            out[sizes_at + i * 4..sizes_at + i * 4 + 4]
                .copy_from_slice(&size.to_le_bytes());
        }
        out
    }

    fn payload(size: usize) -> Vec<u8> {
        // Compressible but not trivially: repeated 251-byte phrase.
        let phrase = b"The quick brown fox jumps over the lazy dog. ";
        phrase
            .iter()
            .cycle()
            .take(size)
            .copied()
            .collect()
    }

    fn write_fixture(data: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("control.ncz");
        std::fs::write(&path, data).unwrap();
        (tmp, path)
    }

    #[test]
    fn test_read_range_round_trips_compressed_blocks() {
        let payload = payload(40_000);
        let (_tmp, path) = write_fixture(&ncz_fixture(&payload, None));
        let back = read_range(&path, 0, VERBATIM_PREFIX, payload.len() as u64, 1 << 26)
            .expect("NCZ range reads");
        assert_eq!(back, payload);
    }

    #[test]
    fn test_read_range_partial_cross_block_span() {
        let payload = payload(40_000);
        let (_tmp, path) = write_fixture(&ncz_fixture(&payload, None));
        // Starts mid-block 0, ends mid-block 2.
        let back = read_range(&path, 0, VERBATIM_PREFIX + 10_000, 20_000, 1 << 26)
            .expect("NCZ partial range reads");
        assert_eq!(back, payload[10_000..30_000]);
    }

    #[test]
    fn test_read_range_serves_verbatim_prefix_from_file() {
        let payload = payload(100);
        let (_tmp, path) = write_fixture(&ncz_fixture(&payload, None));
        // Straddles the prefix boundary: verbatim bytes, then payload.
        let back = read_range(&path, 0, 0x3FF0, 0x20, 1 << 26).expect("prefix reads");
        let mut expected = vec![0xABu8; 0x10];
        expected.extend_from_slice(&payload[..0x10]);
        assert_eq!(back, expected);
    }

    #[test]
    fn test_read_range_reads_raw_stored_block() {
        let payload = payload(40_000);
        let (_tmp, path) = write_fixture(&ncz_fixture(&payload, Some(1)));
        let back = read_range(&path, 0, VERBATIM_PREFIX, payload.len() as u64, 1 << 26)
            .expect("NCZ with raw block reads");
        assert_eq!(back, payload);
    }

    #[test]
    fn test_read_range_rejects_plain_data() {
        let (_tmp, path) = write_fixture(&vec![0u8; 0x5000]);
        assert!(read_range(&path, 0, VERBATIM_PREFIX, 100, 1 << 26).is_none());
    }

    #[test]
    fn test_read_range_rejects_truncated_table() {
        let mut fixture = ncz_fixture(&payload(100), None);
        fixture.truncate(VERBATIM_PREFIX as usize + 20);
        let (_tmp, path) = write_fixture(&fixture);
        assert!(read_range(&path, 0, VERBATIM_PREFIX, 100, 1 << 26).is_none());
    }

    #[test]
    fn test_read_range_rejects_bad_exponent() {        let mut fixture = ncz_fixture(&payload(100), None);
        // Block-size exponent lives 3 bytes into the NCZBLOCK header.
        let at = fixture
            .windows(8)
            .position(|w| w == b"NCZBLOCK")
            .unwrap()
            + 8
            + 3;
        fixture[at] = 40;
        let (_tmp, path) = write_fixture(&fixture);
        assert!(read_range(&path, 0, VERBATIM_PREFIX, 100, 1 << 26).is_none());
    }

    #[test]
    fn test_read_range_respects_max_len() {
        let payload = payload(100);
        let (_tmp, path) = write_fixture(&ncz_fixture(&payload, None));
        assert!(read_range(&path, 0, VERBATIM_PREFIX, 100, 99).is_none());
        assert!(read_range(&path, 0, VERBATIM_PREFIX, 0, 1 << 26).is_none());
    }
}
