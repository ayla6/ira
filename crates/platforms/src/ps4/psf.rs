use std::collections::HashMap;
use std::path::Path;

const PSF_MAGIC: u32 = 0x00505346;

#[derive(Debug, Clone)]
pub enum PsfValue {
    Text(String),
    Integer(i32),
    Binary(Vec<u8>),
}

fn read_u32_le(b: &[u8]) -> Option<u32> {
    b.get(..4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn read_u16_le(b: &[u8]) -> Option<u16> {
    b.get(..2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

/// Parse a param.sfo file, returning a map of key → value.
pub fn parse_psf(path: &Path) -> Result<HashMap<String, PsfValue>, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    parse_psf_bytes(&data)
}

pub fn parse_psf_bytes(data: &[u8]) -> Result<HashMap<String, PsfValue>, String> {
    if data.len() < 20 {
        return Err("param.sfo too short".into());
    }

    let magic = data
        .get(..4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0);
    if magic != PSF_MAGIC {
        return Err(format!("bad PSF magic: 0x{:08X}", magic));
    }

    let key_table_offset = read_u32_le(data.get(8..12).unwrap_or(&[])).unwrap_or(0) as usize;
    let data_table_offset = read_u32_le(data.get(12..16).unwrap_or(&[])).unwrap_or(0) as usize;
    let num_entries = read_u32_le(data.get(16..20).unwrap_or(&[])).unwrap_or(0) as usize;

    let mut result = HashMap::new();

    for i in 0..num_entries {
        let off = 20 + i * 16;
        if off + 16 > data.len() {
            break;
        }
        let entry = &data[off..off + 16];
        let key_off = read_u16_le(&entry[..2]).unwrap_or(0) as usize;
        let param_fmt = read_u16_le(&entry[2..4]).unwrap_or(0);
        let param_len = read_u32_le(&entry[4..8]).unwrap_or(0) as usize;
        let data_off = read_u32_le(&entry[12..16]).unwrap_or(0) as usize;

        // Read key string (null-terminated); skip corrupt entries.
        let key_start = key_table_offset.saturating_add(key_off);
        let Some(key_rest) = data.get(key_start..) else {
            break;
        };
        let key_end = key_rest
            .iter()
            .position(|&b| b == 0)
            .map(|p| key_start + p)
            .unwrap_or(data.len().min(key_start + 64));
        let Some(key_bytes) = data.get(key_start..key_end) else {
            break;
        };
        let key = String::from_utf8_lossy(key_bytes).to_string();

        // Read data; truncated values fall back to empty/zero.
        let data_start = data_table_offset.saturating_add(data_off);
        let data_end = data_start.saturating_add(param_len);
        let slice = if data_end <= data.len() {
            &data[data_start..data_end]
        } else {
            &[]
        };

        let value = match param_fmt {
            0x0204 => {
                // Text (UTF-8, null-terminated)
                let nul = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
                PsfValue::Text(String::from_utf8_lossy(&slice[..nul]).to_string())
            }
            0x0404 => {
                // Integer (i32 LE); short values read as zero.
                let val = slice
                    .get(..4)
                    .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
                    .unwrap_or(0);
                PsfValue::Integer(val)
            }
            _ => PsfValue::Binary(slice.to_vec()),
        };

        result.insert(key, value);
    }

    Ok(result)
}

/// Extract TITLE and TITLE_ID from a parsed PSF.
pub fn psf_get_title(psf: &HashMap<String, PsfValue>) -> String {
    match psf.get("TITLE") {
        Some(PsfValue::Text(s)) => s.clone(),
        _ => String::new(),
    }
}

pub fn psf_get_title_id(psf: &HashMap<String, PsfValue>) -> String {
    match psf.get("TITLE_ID") {
        Some(PsfValue::Text(s)) => s.clone(),
        _ => String::new(),
    }
}
