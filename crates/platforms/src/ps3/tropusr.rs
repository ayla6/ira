use std::collections::HashMap;
use std::path::Path;

/// TROPUSR.DAT magic (big-endian): 0x818F54AD.
const TROPUSR_MAGIC: u32 = 0x818F54AD;

/// CellRtcTick epoch offset: microseconds from year 1 to Unix epoch (1970-01-01).
/// Matches RPCS3's RTC_MAGIC_OFFSET in cellRtc.h.
const RTC_MAGIC_OFFSET: u64 = 62_135_596_800_000_000;

/// Header size: magic(4) + unk1(4) + tables_count(4) + unk2(4) + reserved(32).
const HEADER_SIZE: usize = 48;

/// Table header size: type(4) + entries_size(4) + unk1(4) + entries_count(4) + offset(8) + reserved(8).
const TABLE_HEADER_SIZE: usize = 32;

/// Per-trophy entry from TROPUSR.DAT table 6.
#[derive(Debug, Clone)]
pub struct TropusrTrophy {
    pub trophy_id: u32,
    pub earned: bool,
    /// Unix timestamp in seconds (0 if not earned).
    pub earned_time: i64,
}

fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_u64_be(data: &[u8], offset: usize) -> Option<u64> {
    data.get(offset..offset + 8)
        .map(|s| u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}

/// Convert a CellRtcTick (microseconds since year 1) to a Unix timestamp in seconds.
/// Returns 0 for ticks below the Unix epoch (shouldn't happen for earned trophies).
fn tick_to_unix_seconds(tick: u64) -> i64 {
    if tick < RTC_MAGIC_OFFSET {
        return 0;
    }
    ((tick - RTC_MAGIC_OFFSET) / 1_000_000) as i64
}

/// Parse TROPUSR.DAT and return a map of trophy_id → (earned, unix_timestamp_seconds).
///
/// TROPUSR.DAT layout (all integers big-endian):
///   header (48 bytes): magic, unk1, tables_count, unk2, reserved[32]
///   table headers (32 bytes each, starting at offset 0x30):
///     type, entries_size, unk1, entries_count, offset, reserved
///   table 6 entries (0x70 bytes each): per-trophy unlock state
///     entry_type(4), entry_size(4), entry_id(4), entry_unk1(4),
///     trophy_id(4), trophy_state(4), unk4(4), unk5(4),
///     timestamp1(8), timestamp2(8), padding[64]
///
/// We only read table 6 (unlock state); table 4 duplicates grade/pid info
/// already present in TROPCONF.SFM.
pub fn parse_tropusr(path: &Path) -> Result<HashMap<u32, (bool, i64)>, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    parse_tropusr_bytes(&data)
}

pub fn parse_tropusr_bytes(data: &[u8]) -> Result<HashMap<u32, (bool, i64)>, String> {
    if data.len() < HEADER_SIZE {
        return Err("TROPUSR.DAT too short for header".into());
    }

    let magic = read_u32_be(data, 0).unwrap();
    if magic != TROPUSR_MAGIC {
        return Err(format!("bad TROPUSR magic: 0x{:08X}", magic));
    }

    let tables_count = read_u32_be(data, 8).unwrap() as usize;

    let mut result = HashMap::new();

    let header_start = HEADER_SIZE;
    for i in 0..tables_count {
        let th_off = header_start + i * TABLE_HEADER_SIZE;
        if th_off + TABLE_HEADER_SIZE > data.len() {
            break;
        }

        let table_type = read_u32_be(data, th_off).unwrap();
        let entries_count = read_u32_be(data, th_off + 12).unwrap() as usize;
        let entries_offset = read_u64_be(data, th_off + 16).unwrap() as usize;

        if table_type != 6 {
            continue;
        }

        // Each table-6 entry is 0x70 bytes. Only the first 0x30 bytes carry useful data;
        // the rest is padding.
        const ENTRY6_SIZE: usize = 0x70;
        for j in 0..entries_count {
            let off = entries_offset + j * ENTRY6_SIZE;
            if off + 0x30 > data.len() {
                break;
            }

            let trophy_id = read_u32_be(data, off + 16).unwrap_or(0);
            let trophy_state = read_u32_be(data, off + 20).unwrap_or(0);
            // timestamp2 is the "official" unlock timestamp (per RPCS3's GetTrophyTimestamp).
            let timestamp2 = read_u64_be(data, off + 32).unwrap_or(0);

            let earned = trophy_state != 0;
            let earned_time = if earned {
                tick_to_unix_seconds(timestamp2)
            } else {
                0
            };

            result.insert(trophy_id, (earned, earned_time));
        }
        break;
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_to_unix_seconds_epoch() {
        // Tick at exactly the Unix epoch should be 0.
        assert_eq!(tick_to_unix_seconds(RTC_MAGIC_OFFSET), 0);
    }

    #[test]
    fn test_tick_to_unix_seconds_one_second_after_epoch() {
        assert_eq!(tick_to_unix_seconds(RTC_MAGIC_OFFSET + 1_000_000), 1);
    }

    #[test]
    fn test_tick_to_unix_seconds_before_epoch() {
        assert_eq!(tick_to_unix_seconds(0), 0);
        assert_eq!(tick_to_unix_seconds(RTC_MAGIC_OFFSET - 1), 0);
    }

    #[test]
    fn test_parse_tropusr_bad_magic() {
        let data = [0u8; 64];
        assert!(parse_tropusr_bytes(&data).is_err());
    }

    #[test]
    fn test_parse_tropusr_too_short() {
        let data = [0u8; 10];
        assert!(parse_tropusr_bytes(&data).is_err());
    }

    #[test]
    fn test_parse_tropusr_no_table6() {
        // Header says 0 tables — should return empty map.
        let mut data = vec![0u8; HEADER_SIZE];
        data[0..4].copy_from_slice(&TROPUSR_MAGIC.to_be_bytes());
        data[8..12].copy_from_slice(&0u32.to_be_bytes()); // tables_count = 0
        let result = parse_tropusr_bytes(&data).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_tropusr_single_locked_trophy() {
        // Build a minimal TROPUSR.DAT with one locked trophy in table 6.
        let entries_count: u32 = 1;
        let entries_offset: u64 = (HEADER_SIZE + TABLE_HEADER_SIZE) as u64;
        let entry_off = entries_offset as usize;

        let mut data = vec![0u8; entry_off + 0x70];

        // Header
        data[0..4].copy_from_slice(&TROPUSR_MAGIC.to_be_bytes());
        data[8..12].copy_from_slice(&1u32.to_be_bytes()); // tables_count = 1

        // Table header (type=6, entries_size=0x60, unk1=1, count=1, offset=entries_offset)
        let th = HEADER_SIZE;
        data[th..th + 4].copy_from_slice(&6u32.to_be_bytes());
        data[th + 4..th + 8].copy_from_slice(&0x60u32.to_be_bytes());
        data[th + 8..th + 12].copy_from_slice(&1u32.to_be_bytes());
        data[th + 12..th + 16].copy_from_slice(&entries_count.to_be_bytes());
        data[th + 16..th + 24].copy_from_slice(&entries_offset.to_be_bytes());

        // Entry: trophy_id=5, state=0 (locked)
        data[entry_off + 16..entry_off + 20].copy_from_slice(&5u32.to_be_bytes());
        data[entry_off + 20..entry_off + 24].copy_from_slice(&0u32.to_be_bytes());

        let result = parse_tropusr_bytes(&data).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.get(&5), Some(&(false, 0)));
    }

    #[test]
    fn test_parse_tropusr_unlocked_trophy_with_timestamp() {
        // Trophy 3 unlocked at Unix timestamp 1700000000 seconds.
        let unix_ts: i64 = 1_700_000_000;
        let tick = RTC_MAGIC_OFFSET + (unix_ts as u64) * 1_000_000;

        let entries_offset: u64 = (HEADER_SIZE + TABLE_HEADER_SIZE) as u64;
        let entry_off = entries_offset as usize;
        let mut data = vec![0u8; entry_off + 0x70];

        data[0..4].copy_from_slice(&TROPUSR_MAGIC.to_be_bytes());
        data[8..12].copy_from_slice(&1u32.to_be_bytes());

        let th = HEADER_SIZE;
        data[th..th + 4].copy_from_slice(&6u32.to_be_bytes());
        data[th + 4..th + 8].copy_from_slice(&0x60u32.to_be_bytes());
        data[th + 8..th + 12].copy_from_slice(&1u32.to_be_bytes());
        data[th + 12..th + 16].copy_from_slice(&1u32.to_be_bytes());
        data[th + 16..th + 24].copy_from_slice(&entries_offset.to_be_bytes());

        data[entry_off + 16..entry_off + 20].copy_from_slice(&3u32.to_be_bytes()); // trophy_id=3
        data[entry_off + 20..entry_off + 24].copy_from_slice(&1u32.to_be_bytes()); // state=unlocked
        data[entry_off + 32..entry_off + 40].copy_from_slice(&tick.to_be_bytes()); // timestamp2

        let result = parse_tropusr_bytes(&data).unwrap();
        assert_eq!(result.get(&3), Some(&(true, unix_ts)));
    }

    #[test]
    fn test_parse_tropusr_skips_table4() {
        // Table 4 entries should be skipped — we only care about table 6.
        let entries_offset: u64 = (HEADER_SIZE + 2 * TABLE_HEADER_SIZE) as u64;
        let entry_off = entries_offset as usize;
        let mut data = vec![0u8; entry_off + 0x70];

        data[0..4].copy_from_slice(&TROPUSR_MAGIC.to_be_bytes());
        data[8..12].copy_from_slice(&2u32.to_be_bytes()); // 2 table headers

        // Table 4 header (should be skipped)
        let th4 = HEADER_SIZE;
        data[th4..th4 + 4].copy_from_slice(&4u32.to_be_bytes());
        data[th4 + 12..th4 + 16].copy_from_slice(&1u32.to_be_bytes());
        data[th4 + 16..th4 + 24].copy_from_slice(&(entries_offset / 2).to_be_bytes());

        // Table 6 header
        let th6 = HEADER_SIZE + TABLE_HEADER_SIZE;
        data[th6..th6 + 4].copy_from_slice(&6u32.to_be_bytes());
        data[th6 + 4..th6 + 8].copy_from_slice(&0x60u32.to_be_bytes());
        data[th6 + 8..th6 + 12].copy_from_slice(&1u32.to_be_bytes());
        data[th6 + 12..th6 + 16].copy_from_slice(&1u32.to_be_bytes());
        data[th6 + 16..th6 + 24].copy_from_slice(&entries_offset.to_be_bytes());

        // Table 6 entry: trophy_id=0, unlocked
        data[entry_off + 16..entry_off + 20].copy_from_slice(&0u32.to_be_bytes());
        data[entry_off + 20..entry_off + 24].copy_from_slice(&1u32.to_be_bytes());

        let result = parse_tropusr_bytes(&data).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[&0].0);
    }
}
