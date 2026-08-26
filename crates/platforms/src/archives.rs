//! Streaming access to ROMs inside compressed containers: `.zip`, `.7z`
//! archives and single-file `.zst` streams. Callers get one sequential
//! pass over the decompressed entry — enough to hash and capture header
//! ranges without unpacking anything to disk.

use std::io::Read;
use std::path::Path;

/// A ROM container worth opening even though the extension is not a plain
/// ROM extension.
pub fn is_archive_extension(ext: &str) -> bool {
    matches!(ext.to_ascii_lowercase().as_str(), "zip" | "7z" | "zst")
}

/// Runs `f` with a reader over the selected entry of `path`, or over the
/// file itself when it is not an archive. For `.zst` the whole file is the
/// entry, so `pick` is not consulted. Returns whatever `f` returns, or
/// None when the container cannot be opened or holds no matching entry.
pub fn with_entry_reader<T, F>(path: &Path, pick: &dyn Fn(&str) -> bool, mut f: F) -> Option<T>
where
    F: FnMut(&mut dyn Read) -> Option<T>,
{
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "zip" => with_zip_entry(path, pick, &mut f),
        "7z" => with_sevenz_entry(path, pick, &mut f),
        "zst" => {
            let file = std::fs::File::open(path).ok()?;
            let mut decoder = zstd::stream::read::Decoder::new(file).ok()?;
            f(&mut decoder)
        }
        _ => {
            let mut file = std::fs::File::open(path).ok()?;
            f(&mut file)
        }
    }
}

fn with_zip_entry<T, F>(path: &Path, pick: &dyn Fn(&str) -> bool, f: &mut F) -> Option<T>
where
    F: FnMut(&mut dyn Read) -> Option<T>,
{
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let matching: Vec<usize> = (0..archive.len())
        .filter(|&i| {
            archive
                .by_index(i)
                .map(|entry| pick(entry.name()))
                .unwrap_or(false)
        })
        .collect();
    if matching.len() != 1 {
        return None;
    }
    let mut entry = archive.by_index(matching[0]).ok()?;
    f(&mut entry)
}

fn with_sevenz_entry<T, F>(path: &Path, pick: &dyn Fn(&str) -> bool, f: &mut F) -> Option<T>
where
    F: FnMut(&mut dyn Read) -> Option<T>,
{
    let mut reader = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty()).ok()?;
    let mut result = None;
    let mut match_count = 0usize;
    reader
        .for_each_entries(|entry, data| {
            if !entry.is_directory() && pick(entry.name()) {
                match_count += 1;
                if match_count == 1 {
                    result = f(data);
                } else {
                    result = None;
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .ok()?;
    if match_count != 1 {
        return None;
    }
    result
}

/// Captures selected byte ranges from one sequential pass over a stream,
/// alongside a CRC32 of everything that passed through. Ranges may be
/// registered as the stream advances (e.g. after a header was decoded);
/// data before a range starts is ignored.
pub struct RangeCapture {
    ranges: Vec<(u64, usize)>,
    buffers: Vec<Vec<u8>>,
    pos: u64,
}

impl RangeCapture {
    pub fn new(ranges: Vec<(u64, usize)>) -> Self {
        let buffers = ranges.iter().map(|(_, len)| vec![0u8; *len]).collect();
        Self {
            ranges,
            buffers,
            pos: 0,
        }
    }

    /// The captured bytes of range `index` (0-padded when the stream ended
    /// early).
    pub fn captured(&self, index: usize) -> &[u8] {
        &self.buffers[index]
    }

    /// Bytes actually captured so far for range `index`. The buffer is
    /// preallocated to its full length, so progress must be read from here,
    /// never from `captured(..).len()`.
    pub fn filled(&self, index: usize) -> usize {
        let Some((start, len)) = self.ranges.get(index) else {
            return 0;
        };
        let end = *start + *len as u64;
        (self.pos.clamp(*start, end) - *start) as usize
    }

    /// Whether every registered range has been fully captured.
    pub fn all_filled(&self) -> bool {
        self.ranges
            .iter()
            .enumerate()
            .all(|(i, (_, len))| self.filled(i) == *len)
    }

    pub fn feed(&mut self, data: &[u8]) {
        let data_start = self.pos;
        let data_end = self.pos + data.len() as u64;
        for (i, (range_start, range_len)) in self.ranges.iter().enumerate() {
            let range_end = range_start + *range_len as u64;
            if data_end <= *range_start || data_start >= range_end {
                continue;
            }
            let from = (data_start.max(*range_start) - data_start) as usize;
            let to = (data_end.min(range_end) - data_start) as usize;
            let offset = (data_start.max(*range_start) - range_start) as usize;
            let chunk = &data[from..to];
            self.buffers[i][offset..offset + chunk.len()].copy_from_slice(chunk);
        }
        self.pos = data_end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(bytes: &[u8]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        (file.as_file() as &std::fs::File).write_all(bytes).unwrap();
        file
    }

    #[test]
    fn test_with_entry_reader_plain_file_passes_through() {
        let file = temp_file(b"plain rom bytes");
        let read = with_entry_reader(file.path(), &|_| true, |reader| {
            let mut buf = String::new();
            reader.read_to_string(&mut buf).ok()?;
            Some(buf)
        })
        .unwrap();
        assert_eq!(read, "plain rom bytes");
    }

    #[test]
    fn test_with_entry_reader_zst_decompresses() {
        let compressed = zstd::stream::encode_all(&b"zstd rom"[..], 3).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rom.zst");
        std::fs::write(&path, &compressed).unwrap();
        let read = with_entry_reader(&path, &|_| true, |reader| {
            let mut buf = String::new();
            reader.read_to_string(&mut buf).ok()?;
            Some(buf)
        })
        .unwrap();
        assert_eq!(read, "zstd rom");
    }

    #[test]
    fn test_with_entry_reader_zip_picks_matching_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roms.zip");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("readme.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, b"not a rom").unwrap();
        zip.start_file("game.nds", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, b"ds rom bytes").unwrap();
        zip.finish().unwrap();

        let read = with_entry_reader(&path, &|name| name.ends_with(".nds"), |reader| {
            let mut buf = String::new();
            reader.read_to_string(&mut buf).ok()?;
            Some(buf)
        })
        .unwrap();
        assert_eq!(read, "ds rom bytes");
    }

    #[test]
    fn test_range_capture_slices_overlapping_chunks() {
        let mut capture = RangeCapture::new(vec![(2, 3), (8, 4)]);
        assert!(capture.filled(0) == 0 && capture.filled(1) == 0);
        assert!(!capture.all_filled());
        capture.feed(&[0, 1, 2, 3, 4]);
        assert_eq!(capture.filled(0), 3);
        assert_eq!(capture.filled(1), 0);
        capture.feed(&[5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(capture.captured(0), &[2, 3, 4]);
        assert_eq!(capture.captured(1), &[8, 9, 10, 11]);
        assert!(capture.all_filled());
    }

    #[test]
    fn test_range_capture_tracks_partial_second_range() {
        let mut capture = RangeCapture::new(vec![(8, 4)]);
        capture.feed(&[0; 10]);
        assert_eq!(capture.filled(0), 2);
        assert!(!capture.all_filled());
        // The buffer is preallocated to the range length even before it is
        // filled — progress comes from filled(), not captured(..).len().
        assert_eq!(capture.captured(0).len(), 4);
    }
}
