use super::files::atomic_write;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    io::Cursor,
    path::{Path, PathBuf},
};

const FALLBACK: &[u8] = include_bytes!("../../../../contrib/com.github.ira.png");

pub(super) fn export(root: &Path, stem: &str, source: &str) -> Result<PathBuf, String> {
    let full = ira_parser::full_image_path(source);
    let bytes = if full.is_empty() || !Path::new(&full).is_file() {
        FALLBACK.to_vec()
    } else {
        std::fs::read(&full).map_err(|error| {
            crate::tr!("Read shortcut icon: {}").replacen("{}", &format!("{full}: {error}"), 1)
        })?
    };
    let image = image::load_from_memory(&bytes).map_err(|error| {
        crate::tr!("Decode shortcut icon: {}").replacen("{}", &error.to_string(), 1)
    })?;
    let mut png = Cursor::new(Vec::new());
    image
        .thumbnail(256, 256)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| {
            crate::tr!("Encode shortcut icon: {}").replacen("{}", &error.to_string(), 1)
        })?;
    let png = png.into_inner();
    let mut hash = DefaultHasher::new();
    png.hash(&mut hash);
    // Desktop caches key textures by GIcon identity. Immutable PNG paths make
    // changed artwork a new icon even when Ira overwrites the same source path.
    let path = root
        .join("ira/desktop/icons")
        .join(format!("{stem}-{:016x}.png", hash.finish()));
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(crate::tr!("Shortcut icon is a symlink: {}").replacen(
                "{}",
                &path.to_string_lossy(),
                1,
            ))
        }
        Ok(_) => {
            if std::fs::read(&path).map_err(|error| error.to_string())? != png {
                return Err(
                    crate::tr!("Shortcut icon content does not match its identity: {}").replacen(
                        "{}",
                        &path.to_string_lossy(),
                        1,
                    ),
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => atomic_write(&path, &png)?,
        Err(error) => return Err(error.to_string()),
    }
    Ok(path)
}
