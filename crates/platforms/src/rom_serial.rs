use std::path::{Path, PathBuf};

struct DiscInfo {
    serial: Option<String>,
    title: Option<String>,
}

fn disc_info_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let bin = dir.join("ira-disc-info");
    if bin.is_file() {
        Some(bin)
    } else {
        None
    }
}

fn read_disc_info(path: &Path) -> Option<DiscInfo> {
    let bin = disc_info_binary()?;
    let output = std::process::Command::new(&bin)
        .arg(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout.trim();

    if stdout == "null" || stdout.is_empty() {
        return None;
    }

    let v: serde_json::Value = serde_json::from_str(stdout).ok()?;
    Some(DiscInfo {
        serial: v.get("serial").and_then(|s| s.as_str()).map(|s| s.to_string()),
        title: v.get("title").and_then(|s| s.as_str()).map(|s| s.to_string()),
    })
}

pub fn read_serial(path: &Path) -> Option<String> {
    read_disc_info(path).and_then(|info| info.serial)
}

pub fn read_title(path: &Path) -> Option<String> {
    read_disc_info(path).and_then(|info| info.title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_serial_nonexistent() {
        assert!(read_serial(std::path::Path::new("/nonexistent.chd")).is_none());
    }
}
