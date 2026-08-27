//! Free-space lookups and human-readable sizes for folder pickers.

/// Available bytes on the filesystem containing `path`, or None when the
/// nearest existing ancestor cannot be stat'ed.
pub fn available_bytes(path: &std::path::Path) -> Option<u64> {
    let mut probe = Some(path);
    while let Some(dir) = probe {
        if dir.exists() {
            return statvfs_available(dir);
        }
        probe = dir.parent();
    }
    None
}

fn statvfs_available(path: &std::path::Path) -> Option<u64> {
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) };
    if result != 0 || stats.f_frsize == 0 {
        return None;
    }
    Some(stats.f_bavail as u64 * stats.f_frsize as u64)
}

/// Human-readable size, decimal units with one decimal (e.g. "12.5 GB").
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "kB", "MB", "GB", "TB", "PB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} {}", UNITS[0]);
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(7), "7 B");
        assert_eq!(format_size(999), "999 B");
    }

    #[test]
    fn test_format_size_larger_units() {
        assert_eq!(format_size(1000), "1.0 kB");
        assert_eq!(format_size(4_200_000), "4.2 MB");
        assert_eq!(format_size(82_400_000_000), "82.4 GB");
    }

    #[test]
    fn test_format_size_saturates_at_last_unit() {
        assert_eq!(format_size(9_000_000_000_000_000_000), "9000.0 PB");
    }

    #[test]
    fn test_available_bytes_on_existing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = available_bytes(tmp.path());
        assert!(bytes.is_some());
    }

    #[test]
    fn test_available_bytes_on_missing_child() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("no/such/dir");
        let bytes = available_bytes(&missing);
        assert!(bytes.is_some());
    }
}
