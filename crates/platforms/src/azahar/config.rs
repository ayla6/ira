use std::path::{Path, PathBuf};

pub const AZAHAR_FLATPAK_ID: &str = "org.azahar_emu.Azahar";

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn app_path_for(executable: &str, suffix: &str) -> PathBuf {
    let base = executable
        .strip_prefix("flatpak:")
        .map(|id| home_dir().join(".var").join("app").join(id))
        .unwrap_or_default();
    if !base.as_os_str().is_empty() {
        return base.join(suffix);
    }
    PathBuf::from(suffix)
}

/// Azahar runs fully portable when a `user` directory sits next to the
/// executable; it then keeps config and data inside it.
fn portable_user_dir_for(executable: &str) -> Option<PathBuf> {
    if executable.is_empty() || executable.starts_with("flatpak:") {
        return None;
    }
    let path = Path::new(executable);
    [path.parent(), Some(path)]
        .into_iter()
        .flatten()
        .map(|root| root.join("user"))
        .find(|path| path.is_dir())
}

pub(crate) fn config_dir_for(executable: &str) -> PathBuf {
    if executable.starts_with("flatpak:") {
        return app_path_for(executable, "config/azahar-emu");
    }
    if let Some(user) = portable_user_dir_for(executable) {
        return user.join("config");
    }
    xdg::BaseDirectories::new()
        .get_config_home()
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("azahar-emu")
}

pub(crate) fn data_dir_for(executable: &str) -> PathBuf {
    if executable.starts_with("flatpak:") {
        return app_path_for(executable, "data/azahar-emu");
    }
    if let Some(user) = portable_user_dir_for(executable) {
        return user;
    }
    xdg::BaseDirectories::new()
        .get_data_home()
        .unwrap_or_else(|| home_dir().join(".local").join("share"))
        .join("azahar-emu")
}

/// Decodes Qt INI percent escapes (`Data%20Storage` → `Data Storage`).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let is_hex = |b: u8| (b as char).is_ascii_hexdigit();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && bytes.len() > i + 2 && is_hex(bytes[i + 1]) && is_hex(bytes[i + 2]) {
            let value = u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'%');
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Minimal Qt INI reader. Section names are percent-decoded; group levels in
/// keys are kept as backslash-separated key paths (`Paths\gamedirs\1\path`).
struct QtIni {
    /// (section, key, value) with section and key lowercased for lookups.
    entries: Vec<(String, String, String)>,
}

impl QtIni {
    fn parse(text: &str) -> Self {
        let mut entries = Vec::new();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with([';', '#']) {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = percent_decode(name).trim().to_lowercase();
            } else if let Some((key, value)) = line.split_once('=') {
                entries.push((
                    section.clone(),
                    percent_decode(key).trim().to_lowercase(),
                    value.trim().to_string(),
                ));
            }
        }
        Self { entries }
    }

    fn get(&self, section: &str, key: &str) -> Option<&str> {
        let section = section.to_lowercase();
        let key = key.to_lowercase();
        self.entries
            .iter()
            .find(|(s, k, _)| *s == section && *k == key)
            .map(|(_, _, v)| v.as_str())
    }
}

/// Game locations and virtual storage roots configured in Azahar.
pub struct AzaharPaths {
    /// (directory, deep scan) pairs from the game list; deep scan searches
    /// subdirectories.
    pub game_dirs: Vec<(PathBuf, bool)>,
    pub nand_dir: PathBuf,
    pub sdmc_dir: PathBuf,
}

pub fn read_paths_for_executable(executable: &str) -> AzaharPaths {
    let data_dir = data_dir_for(executable);
    let ini = std::fs::read_to_string(config_dir_for(executable).join("qt-config.ini"))
        .ok()
        .map(|text| QtIni::parse(&text));
    AzaharPaths {
        game_dirs: ini.as_ref().map(game_dirs).unwrap_or_default(),
        nand_dir: storage_dir(ini.as_ref(), "nand_directory")
            .unwrap_or_else(|| data_dir.join("nand")),
        sdmc_dir: storage_dir(ini.as_ref(), "sdmc_directory")
            .unwrap_or_else(|| data_dir.join("sdmc")),
    }
}

fn storage_dir(ini: Option<&QtIni>, key: &str) -> Option<PathBuf> {
    ini.and_then(|ini| ini.get("Data Storage", key))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn game_dirs(ini: &QtIni) -> Vec<(PathBuf, bool)> {
    // Azahar stores the list under `UI\Paths\gamedirs\…`; match on the
    // `gamedirs\…` suffix so the section/group prefix may vary between
    // versions.
    let value = |suffix: &str| -> Option<&str> {
        ini.entries
            .iter()
            .find(|(_, key, _)| key.strip_prefix("paths\\").unwrap_or(key) == suffix)
            .map(|(_, _, value)| value.as_str())
    };
    let size: usize = value("gamedirs\\size")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    // `size` comes from a hand-editable INI; never iterate beyond the
    // number of keys actually present.
    let size = size.min(ini.entries.len());
    (1..=size)
        .filter_map(|i| {
            // `INSTALLED` and `SYSTEM` are Azahar's built-in game list
            // entries, not locations on disk.
            let path = value(&format!("gamedirs\\{i}\\path"))?;
            if !Path::new(path).is_absolute() {
                return None;
            }
            let deep_scan = value(&format!("gamedirs\\{i}\\deep_scan"))
                .is_some_and(|value| value.eq_ignore_ascii_case("true"));
            Some((PathBuf::from(path), deep_scan))
        })
        .filter(|(path, _)| path.is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_decode_unescapes_sections() {
        assert_eq!(percent_decode("Data%20Storage"), "Data Storage");
        assert_eq!(percent_decode("UI"), "UI");
        assert_eq!(percent_decode("bad%2"), "bad%2");
        assert_eq!(percent_decode("bad%zz"), "bad%zz");
    }

    #[test]
    fn test_qt_ini_reads_gamedirs_and_storage() {
        let roms = tempfile::tempdir().unwrap();
        let roms_path = roms.path().join("3ds");
        std::fs::create_dir_all(&roms_path).unwrap();
        let ini = QtIni::parse(&format!(
            "[Data%20Storage]\n\
             nand_directory=/tmp/nand/\n\
             nand_directory\\default=false\n\
             sdmc_directory=/tmp/sdmc/\n\
             [UI]\n\
             Paths\\gamedirs\\1\\deep_scan=false\n\
             Paths\\gamedirs\\1\\path={0}\n\
             Paths\\gamedirs\\2\\path=INSTALLED\n\
             Paths\\gamedirs\\3\\path=SYSTEM\n\
             Paths\\gamedirs\\size=3\n",
            roms_path.display()
        ));
        assert_eq!(
            ini.get("Data Storage", "nand_directory"),
            Some("/tmp/nand/")
        );
        let dirs = game_dirs(&ini);
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].0, roms_path);
        assert!(!dirs[0].1);
    }

    #[test]
    fn test_game_dirs_skip_default_suffixed_keys() {
        let roms = tempfile::tempdir().unwrap();
        let roms_path = roms.path().join("3ds");
        std::fs::create_dir_all(&roms_path).unwrap();
        let ini = QtIni::parse(&format!(
            "[UI]\n\
             Paths\\gamedirs\\1\\path={0}\n\
             Paths\\gamedirs\\1\\path\\default=true\n\
             Paths\\gamedirs\\size=1\n\
             Paths\\gamedirs\\size\\default=true\n",
            roms_path.display()
        ));
        let dirs = game_dirs(&ini);
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].0, roms_path);
    }

    #[test]
    fn test_flatpak_config_and_data_dirs() {
        let executable = format!("flatpak:{AZAHAR_FLATPAK_ID}");
        assert!(config_dir_for(&executable)
            .ends_with(format!(".var/app/{AZAHAR_FLATPAK_ID}/config/azahar-emu")));
        assert!(data_dir_for(&executable)
            .ends_with(format!(".var/app/{AZAHAR_FLATPAK_ID}/data/azahar-emu")));
    }
}
