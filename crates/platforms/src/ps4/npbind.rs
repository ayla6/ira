use std::path::Path;

const NPBIND_MAGIC: u32 = 0xD294A018;

/// Extract NPCommId (e.g. "NPWR11866_00") from npbind.dat.
pub fn parse_npbind(path: &Path) -> Result<Vec<String>, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    parse_npbind_bytes(&data)
}

fn read_u16_be(data: &[u8], offset: usize) -> Option<(u16, u16)> {
    let s = data.get(offset..offset + 4)?;
    Some((
        u16::from_be_bytes([s[0], s[1]]),
        u16::from_be_bytes([s[2], s[3]]),
    ))
}

fn read_npcomm_id(data: &[u8], offset: usize, size: usize) -> Option<String> {
    let slice = data.get(offset..offset + size)?;
    let nul = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let id = String::from_utf8_lossy(&slice[..nul]).to_string();
    (!id.is_empty()).then_some(id)
}

pub fn parse_npbind_bytes(data: &[u8]) -> Result<Vec<String>, String> {
    if data.len() < 0x88 {
        return Err("npbind.dat too short".into());
    }

    let magic = data
        .get(..4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0);
    if magic != NPBIND_MAGIC {
        return Err(format!("bad npbind magic: 0x{:08X}", magic));
    }

    let num_entries = data
        .get(0x18..0x20)
        .map(|s| {
            u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]) as usize
        })
        .unwrap_or(0);
    let mut offset = 0x80usize;
    let mut result = Vec::new();

    for _ in 0..num_entries {
        if offset + 8 > data.len() {
            break;
        }
        // Read 4 TLV entries
        for _ in 0..4 {
            let Some((tlv_type, tlv_size)) = read_u16_be(data, offset) else {
                break;
            };
            let tlv_size = tlv_size as usize;
            offset += 4;

            if tlv_type == 0x0010 {
                // NPCommId
                if let Some(id) = read_npcomm_id(data, offset, tlv_size) {
                    result.push(id);
                }
            }
            offset = offset.saturating_add(tlv_size);
            if offset > data.len() {
                break;
            }
        }
        // Skip 0x98 bytes of padding
        offset = offset.saturating_add(0x98);
    }

    Ok(result)
}
