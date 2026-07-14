use std::path::Path;

pub fn read_serial(path: &Path) -> Option<String> {
    let info = opticaldiscs::detect::DiscImageInfo::open(path).ok()?;
    info.game
        .as_ref()
        .and_then(|g| g.serial.clone())
        .filter(|s| !s.is_empty())
}

pub fn read_title(path: &Path) -> Option<String> {
    let info = opticaldiscs::detect::DiscImageInfo::open(path).ok()?;
    info.game
        .as_ref()
        .and_then(|g| g.title.clone())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_serial_nonexistent() {
        assert!(read_serial(std::path::Path::new("/nonexistent.chd")).is_none());
    }
}
