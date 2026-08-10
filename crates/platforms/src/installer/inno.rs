use std::fs;
use std::path::Path;
use std::process::Command;

/// Check whether `innoextract` is installed and available on `PATH`.
pub fn innoextract_available() -> bool {
    which::which("innoextract").is_ok()
}

/// Extract an Inno Setup installer using `innoextract`.
///
/// Runs `innoextract --gog --exclude-temp --output-dir <dest> <installer>`
/// and captures stdout/stderr. If `dest/app/` exists after extraction
/// (some GOG Windows installers nest everything under `app/`), its
/// contents are moved up to `dest/`.
pub fn extract_inno(installer: &Path, dest: &Path) -> Result<String, String> {
    let output = Command::new("innoextract")
        .args(["--gog", "--exclude-temp", "--output-dir"])
        .arg(dest)
        .arg(installer)
        .output()
        .map_err(|e| format!("run innoextract: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(format!(
            "innoextract failed ({}):\n{stdout}\n{stderr}",
            output.status
        ));
    }

    flatten_app_subdir(dest)?;

    Ok(format!("{stdout}\n{stderr}").trim().to_string())
}

/// If `dest/app/` exists, move its contents up to `dest/`.
/// This matches wyvern's behavior for GOG Windows installers that nest
/// everything under `app/`.
fn flatten_app_subdir(dest: &Path) -> Result<(), String> {
    let app_dir = dest.join("app");
    if !app_dir.is_dir() {
        return Ok(());
    }

    let entries = fs::read_dir(&app_dir).map_err(|e| format!("read app dir: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let target = dest.join(&name);
        fs::rename(&path, &target)
            .map_err(|e| format!("move {:?} \u{2192} {:?}: {e}", path, target))?;
    }

    fs::remove_dir(&app_dir).map_err(|e| format!("remove app dir: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flatten_app_subdir_moves_contents_up() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path();
        let app = dest.join("app");
        fs::create_dir_all(&app).unwrap();
        fs::write(app.join("game.exe"), b"EXE").unwrap();
        fs::write(app.join("data.bin"), b"BIN").unwrap();

        flatten_app_subdir(dest).unwrap();

        assert!(!app.exists());
        assert!(dest.join("game.exe").exists());
        assert!(dest.join("data.bin").exists());
    }

    #[test]
    fn test_flatten_app_subdir_no_app_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path();
        fs::write(dest.join("game.exe"), b"EXE").unwrap();

        flatten_app_subdir(dest).unwrap();

        assert!(dest.join("game.exe").exists());
    }

    #[test]
    fn test_flatten_app_subdir_empty_app_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path();
        let app = dest.join("app");
        fs::create_dir_all(&app).unwrap();

        flatten_app_subdir(dest).unwrap();

        assert!(!app.exists());
    }
}
