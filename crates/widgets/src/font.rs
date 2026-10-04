//! The big-picture UI's font: embedded in the binary, registered as a
//! fontconfig *application* font — private to the process, never installed
//! into the user's font directories. Shared so the main app and the
//! overlay render the same type.

/// The family the big picture CSS uses, and its files (Regular first,
/// then any weight companions).
pub const ACTIVE: (&str, &[(&str, &[u8])]) = (
    "M PLUS 2",
    &[(
        "MPLUS2-VariableFont_wght.ttf",
        include_bytes!("../assets/fonts/M_PLUS_2/MPLUS2-VariableFont_wght.ttf"),
    )],
);

mod fc {
    use std::ffi::c_char;

    #[repr(C)]
    pub struct FcConfig {
        _private: [u8; 0],
    }

    #[link(name = "fontconfig")]
    unsafe extern "C" {
        pub fn FcConfigGetCurrent() -> *mut FcConfig;
        pub fn FcConfigAppFontAddFile(config: *mut FcConfig, file: *const c_char) -> i32;
    }
}

/// Register the active font with fontconfig as an application font. Must
/// run before GTK initializes text rendering; the font bytes are cached
/// in the app's own cache directory because fontconfig loads from file
/// paths. Failures are reported but never fatal — the UI falls back to
/// the system font.
pub fn install() {
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|home| home.join(".cache"))
        });
    let Some(dir) = dir.map(|d| d.join("ira/fonts")) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        eprintln!("ira-widgets: could not create the font cache dir {}", dir.display());
        return;
    }
    unsafe {
        let config = fc::FcConfigGetCurrent();
        for (name, bytes) in ACTIVE.1 {
            let path = dir.join(name);
            let stale = std::fs::metadata(&path).is_ok_and(|m| m.len() != bytes.len() as u64);
            if (!path.exists() || stale)
                && let Err(e) = std::fs::write(&path, bytes)
            {
                eprintln!("ira-widgets: could not write the font to {}: {e}", path.display());
                continue;
            }
            let Ok(cpath) = std::ffi::CString::new(path.as_os_str().to_string_lossy().as_bytes())
            else {
                eprintln!("ira-widgets: font path is not NUL-safe: {}", path.display());
                continue;
            };
            if fc::FcConfigAppFontAddFile(config, cpath.as_ptr()) == 0 {
                eprintln!("ira-widgets: fontconfig rejected {}", path.display());
            }
        }
    }
}
