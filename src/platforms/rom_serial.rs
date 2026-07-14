use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const CSO_HEADER_SIZE: u64 = 24;
const ISO_SECTOR_SIZE: u64 = 2048;

enum RomReader {
    Plain(std::fs::File),
    Cso(CsoReader),
}

struct CsoReader {
    file: std::fs::File,
    block_size: u32,
    indices: Vec<u32>,
}

impl CsoReader {
    fn open(mut file: std::fs::File) -> Option<Self> {
        let mut header = [0u8; 24];
        file.read_exact(&mut header).ok()?;
        if &header[0..4] != b"CISO" {
            return None;
        }
        let block_size = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) * ISO_SECTOR_SIZE as u32;
        if block_size == 0 {
            return None;
        }
        let version = header[4];
        let num_blocks = if version == 0 {
            u32::from_le_bytes([header[12], header[13], header[14], header[15]])
        } else {
            // v1 has total bytes in header[12..20]
            let total_bytes = u64::from_le_bytes([
                header[12], header[13], header[14], header[15],
                header[16], header[17], header[18], header[19],
            ]);
            (total_bytes / block_size as u64) as u32 + 1
        };
        let mut indices = Vec::with_capacity(num_blocks as usize + 1);
        for _ in 0..=num_blocks {
            let mut buf = [0u8; 4];
            file.read_exact(&mut buf).ok()?;
            indices.push(u32::from_le_bytes(buf));
        }
        Some(CsoReader { file, block_size, indices })
    }

    fn read_sector(&mut self, lba: u32) -> Option<Vec<u8>> {
        let idx = self.indices.get(lba as usize)?;
        let next_idx = self.indices.get(lba as usize + 1)?;
        let offset = (*idx as u64 & 0x7FFFFFFF) << 2;
        let compressed_size = (*next_idx as u64 & 0x7FFFFFFF) << 2;
        let is_compressed = (*idx >> 31) == 0;
        let size = (compressed_size - offset) as usize;

        self.file.seek(SeekFrom::Start(offset + CSO_HEADER_SIZE)).ok()?;
        let mut data = vec![0u8; size];
        self.file.read_exact(&mut data).ok()?;

        if is_compressed && size < self.block_size as usize {
            use flate2::read::ZlibDecoder;
            let mut decoder = ZlibDecoder::new(&data[..]);
            let mut out = vec![0u8; self.block_size as usize];
            decoder.read_exact(&mut out).ok()?;
            Some(out)
        } else {
            data.truncate(self.block_size as usize);
            Some(data)
        }
    }
}

impl RomReader {
    fn open(path: &Path) -> Option<Self> {
        let file = std::fs::File::open(path).ok()?;
        match path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()).as_deref() {
            Some("cso") => CsoReader::open(file).map(RomReader::Cso),
            _ => Some(RomReader::Plain(file)),
        }
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Option<()> {
        match self {
            RomReader::Plain(f) => {
                f.seek(SeekFrom::Start(offset)).ok()?;
                f.read_exact(buf).ok()?;
                Some(())
            }
            RomReader::Cso(cso) => {
                let sector = offset / ISO_SECTOR_SIZE;
                let within = (offset % ISO_SECTOR_SIZE) as usize;
                let data = cso.read_sector(sector as u32)?;
                let avail = data.len().saturating_sub(within);
                let to_copy = buf.len().min(avail);
                buf[..to_copy].copy_from_slice(&data[within..within + to_copy]);
                Some(())
            }
        }
    }

    fn read_iso_file(&mut self, file_path: &str) -> Option<Vec<u8>> {
        let mut sector_buf = [0u8; ISO_SECTOR_SIZE as usize];
        // Read PVD (Primary Volume Descriptor) at sector 16
        self.read_at(16 * ISO_SECTOR_SIZE, &mut sector_buf)?;
        if &sector_buf[0..5] != b"CD001" || sector_buf[6] != 0x01 {
            return None;
        }
        // Root directory record starts at offset 156, 34 bytes long
        let root_lba = u32::from_le_bytes([
            sector_buf[156], sector_buf[157], sector_buf[158], sector_buf[159],
        ]);
        let root_size = u32::from_le_bytes([
            sector_buf[166], sector_buf[167], sector_buf[168], sector_buf[169],
        ]);
        let root_sectors = (root_size as u64 + ISO_SECTOR_SIZE - 1) / ISO_SECTOR_SIZE;

        // Scan root directory for the target file
        let target_upper = file_path.to_uppercase();
        for sec in 0..root_sectors {
            self.read_at((root_lba as u64 + sec) * ISO_SECTOR_SIZE, &mut sector_buf)?;
            let mut pos = 0;
            while pos < sector_buf.len() {
                let rec_len = sector_buf[pos] as usize;
                if rec_len == 0 {
                    break;
                }
                if rec_len > sector_buf.len() - pos {
                    break;
                }
                let name_len = sector_buf[pos + 32] as usize;
                if name_len > 0 {
                    let name = &sector_buf[pos + 33..pos + 33 + name_len];
                    let name_str = String::from_utf8_lossy(name);
                    let name_part = name_str.split(';').next().unwrap_or(&name_str);
                    if name_part.to_uppercase() == target_upper {
                        let file_lba = u32::from_le_bytes([
                            sector_buf[pos + 2], sector_buf[pos + 3],
                            sector_buf[pos + 4], sector_buf[pos + 5],
                        ]);
                        let file_size = u32::from_le_bytes([
                            sector_buf[pos + 10], sector_buf[pos + 11],
                            sector_buf[pos + 12], sector_buf[pos + 13],
                        ]);
                        return self.read_file_data(file_lba, file_size);
                    }
                }
                pos += rec_len;
            }
        }
        None
    }

    fn read_file_data(&mut self, lba: u32, size: u32) -> Option<Vec<u8>> {
        let mut data = vec![0u8; size as usize];
        let mut remaining = size as usize;
        let mut offset = 0usize;
        let mut sec = lba as u64;
        while remaining > 0 {
            let mut sector_buf = [0u8; ISO_SECTOR_SIZE as usize];
            self.read_at(sec * ISO_SECTOR_SIZE, &mut sector_buf)?;
            let to_copy = remaining.min(ISO_SECTOR_SIZE as usize);
            data[offset..offset + to_copy].copy_from_slice(&sector_buf[..to_copy]);
            offset += to_copy;
            remaining -= to_copy;
            sec += 1;
        }
        Some(data)
    }
}

fn normalize_serial(raw: &str) -> String {
    let trimmed = raw.trim();
    // Strip path prefixes like "cdrom:\", "cdrom0:\", "cdrom:"
    let after_colon = trimmed.rsplit('\\').next().or_else(|| trimmed.rsplit(':').next()).unwrap_or(trimmed);
    let cleaned = after_colon
        .replace('_', "-")
        .replace('.', "");
    let result = cleaned.split(';').next().unwrap_or(&cleaned);
    result.to_uppercase()
}

pub fn read_ps1_serial(path: &Path) -> Option<String> {
    let mut reader = RomReader::open(path)?;
    let cnf = reader.read_iso_file("SYSTEM.CNF")?;
    let cnf_str = String::from_utf8_lossy(&cnf);
    for line in cnf_str.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("BOOT").and_then(|s| s.strip_prefix(|c: char| c == '=' || c == ' ')) {
            return Some(normalize_serial(val));
        }
    }
    None
}

pub fn read_ps2_serial(path: &Path) -> Option<String> {
    let mut reader = RomReader::open(path)?;
    let cnf = reader.read_iso_file("SYSTEM.CNF")?;
    let cnf_str = String::from_utf8_lossy(&cnf);
    for line in cnf_str.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("BOOT2").and_then(|s| s.strip_prefix(|c: char| c == '=' || c == ' ')) {
            return Some(normalize_serial(val));
        }
    }
    None
}

pub fn read_psp_serial(path: &Path) -> Option<String> {
    let mut reader = RomReader::open(path)?;
    let sfo_data = reader.read_iso_file("PSP_GAME/PARAM.SFO")?;
    crate::platforms::ps4::psf::parse_psf_bytes(&sfo_data)
        .ok()
        .and_then(|map| match map.get("DISC_ID") {
            Some(crate::platforms::ps4::psf::PsfValue::Text(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        })
}

pub fn read_serial(path: &Path) -> Option<String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase())?;
    match ext.as_str() {
        "iso" | "cso" => {
            // Try PSP first, then PS2, then PS1
            read_psp_serial(path)
                .or_else(|| read_ps2_serial(path))
                .or_else(|| read_ps1_serial(path))
        }
        "bin" => {
            // PS1 .bin files are raw CD images with 2352-byte sectors
            // For now, only handle .iso/.cso
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_serial_ps1() {
        assert_eq!(normalize_serial("cdrom:\\SCES_123.45;1"), "SCES-12345");
    }

    #[test]
    fn test_normalize_serial_ps2() {
        assert_eq!(normalize_serial("cdrom0:\\SLUS_214.89;1"), "SLUS-21489");
    }

    #[test]
    fn test_normalize_serial_already_clean() {
        assert_eq!(normalize_serial("ULUS-12345"), "ULUS-12345");
    }
}
