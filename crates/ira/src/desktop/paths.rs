use std::path::{Path, PathBuf};

pub(crate) fn executable() -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let appdir = std::env::var_os("APPDIR").map(PathBuf::from);
    let image = std::env::var_os("APPIMAGE").map(PathBuf::from);
    Ok(persistent_executable(
        &current,
        appdir.as_deref(),
        image.as_deref(),
    ))
}

fn persistent_executable(current: &Path, appdir: Option<&Path>, image: Option<&Path>) -> PathBuf {
    if let (Some(appdir), Some(image)) = (appdir, image) {
        if current.starts_with(appdir) && image.is_absolute() {
            return image.to_path_buf();
        }
    }
    current.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_persistent_executable_uses_appimage_outside_mount() {
        let mounted = Path::new("/tmp/.mount_Ira/usr/lib/ira/ira");
        let appdir = Some(Path::new("/tmp/.mount_Ira"));
        let image_path = Path::new("/home/user/My Apps/Ira.AppImage");
        let image = Some(image_path);
        assert_eq!(persistent_executable(mounted, appdir, image), image_path);
        let native = Path::new("/usr/lib/ira/ira");
        assert_eq!(persistent_executable(native, appdir, image), native);
        assert_eq!(persistent_executable(mounted, None, image), mounted);
        assert_eq!(
            persistent_executable(mounted, appdir, Some(Path::new("relative"))),
            mounted
        );
    }
}
