//! The couch UI's font under test: embedded in the binary, registered as
//! a fontconfig *application* font — private to this process, never
//! installed into the user's font directories. Swap the `ACTIVE` block to
//! try another candidate.

/// The family the couch CSS currently uses, and its files (Regular first,
/// then any weight companions). Candidates waiting their turn live in
/// `assets/fonts/` unembedded: IBM_Plex_Sans_JP, Murecho, M_PLUS_2,
/// M_PLUS_1.
pub const ACTIVE: (&str, &[(&str, &[u8])]) = (
    "M PLUS 2",
    &[(
        "MPLUS2-VariableFont_wght.ttf",
        include_bytes!("../../assets/fonts/M_PLUS_2/MPLUS2-VariableFont_wght.ttf"),
    )],
);

mod fc {
    use std::ffi::c_char;

    #[repr(C)]
    pub struct FcConfig {
        _private: [u8; 0],
    }

    #[link(name = "fontconfig")]
    extern "C" {
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
        eprintln!("ira: could not create the font cache dir {}", dir.display());
        return;
    }
    unsafe {
        let config = fc::FcConfigGetCurrent();
        for (name, bytes) in ACTIVE.1 {
            let path = dir.join(name);
            if !path.exists() && std::fs::write(&path, bytes).is_err() {
                eprintln!("ira: could not write the font to {}", path.display());
                continue;
            }
            let Ok(cpath) = std::ffi::CString::new(path.as_os_str().to_string_lossy().as_bytes())
            else {
                continue;
            };
            if fc::FcConfigAppFontAddFile(config, cpath.as_ptr()) == 0 {
                eprintln!("ira: fontconfig rejected {}", path.display());
            }
        }
    }
}
