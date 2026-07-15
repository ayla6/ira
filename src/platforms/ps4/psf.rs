use std::collections::HashMap;
use std::path::Path;

const PSF_MAGIC: u32 = 0x00505346;

#[derive(Debug, Clone)]
pub enum PsfValue {
    Text(String),
    Integer(i32),
    Binary(Vec<u8>),
}

fn read_u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn read_u16_le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
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

    let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    if magic != PSF_MAGIC {
        return Err(format!("bad PSF magic: 0x{:08X}", magic));
    }

    let key_table_offset = read_u32_le(&data[8..12]) as usize;
    let data_table_offset = read_u32_le(&data[12..16]) as usize;
    let num_entries = read_u32_le(&data[16..20]) as usize;

    let mut result = HashMap::new();

    for i in 0..num_entries {
        let off = 20 + i * 16;
        if off + 16 > data.len() {
            break;
        }
        let key_off = read_u16_le(&data[off..off + 2]) as usize;
        let param_fmt = read_u16_le(&data[off + 2..off + 4]);
        let param_len = read_u32_le(&data[off + 4..off + 8]) as usize;
        let data_off = read_u32_le(&data[off + 12..off + 16]) as usize;

        // Read key string (null-terminated)
        let key_start = key_table_offset + key_off;
        let key_end = data[key_start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| key_start + p)
            .unwrap_or(data.len());
        let key = String::from_utf8_lossy(&data[key_start..key_end]).to_string();

        // Read data
        let data_start = data_table_offset + data_off;
        let data_end = data_start + param_len;

        let value = match param_fmt {
            0x0204 => {
                // Text (UTF-8, null-terminated)
                let s = if data_end <= data.len() {
                    let slice = &data[data_start..data_end];
                    let nul = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
                    String::from_utf8_lossy(&slice[..nul]).to_string()
                } else {
                    String::new()
                };
                PsfValue::Text(s)
            }
            0x0404 => {
                // Integer (i32 LE)
                let val = if data_end <= data.len() {
                    i32::from_le_bytes([
                        data[data_start],
                        data[data_start + 1],
                        data[data_start + 2],
                        data[data_start + 3],
                    ])
                } else {
                    0
                };
                PsfValue::Integer(val)
            }
            _ => {
                // Binary
                let bytes = if data_end <= data.len() {
                    data[data_start..data_end].to_vec()
                } else {
                    Vec::new()
                };
                PsfValue::Binary(bytes)
            }
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
