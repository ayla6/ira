use std::path::Path;

const NPBIND_MAGIC: u32 = 0xD294A018;

/// Extract NPCommId (e.g. "NPWR11866_00") from npbind.dat.
pub fn parse_npbind(path: &Path) -> Result<Vec<String>, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    if data.len() < 0x88 {
        return Err("npbind.dat too short".into());
    }

    let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    if magic != NPBIND_MAGIC {
        return Err(format!("bad npbind magic: 0x{:08X}", magic));
    }

    let num_entries = u64::from_be_bytes([
        data[0x18], data[0x19], data[0x1a], data[0x1b],
        data[0x1c], data[0x1d], data[0x1e], data[0x1f],
    ]) as usize;
    let mut offset = 0x80usize;
    let mut result = Vec::new();

    for _ in 0..num_entries {
        if offset + 8 > data.len() {
            break;
        }
        // Read 4 TLV entries
        for _ in 0..4 {
            if offset + 4 > data.len() {
                break;
            }
            let tlv_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
            let tlv_size = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
            offset += 4;

            if tlv_type == 0x0010 {
                // NPCommId
                if offset + tlv_size <= data.len() {
                    let slice = &data[offset..offset + tlv_size];
                    let nul = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
                    let id = String::from_utf8_lossy(&slice[..nul]).to_string();
                    if !id.is_empty() {
                        result.push(id);
                    }
                }
            }
            offset += tlv_size;
        }
        // Skip 0x98 bytes of padding
        offset += 0x98;
    }

    Ok(result)
}
