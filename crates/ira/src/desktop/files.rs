use std::{io::Write, path::Path};

pub(super) fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "File has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let result = file
        .write_all(contents)
        .and_then(|_| std::fs::rename(&temporary, path));
    if let Err(error) = result {
        if let Err(cleanup) = std::fs::remove_file(&temporary) {
            eprintln!(
                "Remove incomplete desktop file {}: {cleanup}",
                temporary.display()
            );
        }
        return Err(error.to_string());
    }
    Ok(())
}
