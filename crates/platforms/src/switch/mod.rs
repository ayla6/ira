//! Native Switch metadata from the emulators' own caches, read without
//! decryption keys. Two families are covered:
//!
//! - yuzu-family (Eden, suyu, citron, sudachi, yuzu): Eden caches every
//!   scanned title's native icon (`<title id>.jpeg`) and application name
//!   (`<title id>.appname.txt`) under its `game_list` cache.
//! - Ryujinx-family (Ryubing, Kenji-NX): `games/<title id>/gui/
//!   metadata.json` holds each library title's display name; icons are
//!   never cached on disk.
//!
//! ROM files map onto these entries by file-name title id, NSP ticket
//! names, or clean-name match; homebrew NROs carry icon and title inside
//! their asset block.

mod config;
mod keys;
mod nca;
mod registered;
mod rom;
mod ryujinx;

/// Synthetic encrypted NCA fixtures shared across the switch tests.
#[cfg(test)]
pub(crate) mod synth;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A native Switch icon: an emulator-cached JPEG file, raw image bytes
/// (NRO PNG, decrypted NCA JPEG), or nothing.
#[derive(Debug, Clone)]
pub enum SwitchIcon {
    None,
    File(PathBuf),
    Bytes(Vec<u8>),
}

/// Native metadata for one Switch ROM file.
#[derive(Debug, Clone)]
pub struct SwitchRomMeta {
    /// 16 lowercase hex digits, or empty when the file carries none.
    pub title_id: String,
    /// Application title from an emulator cache or the NRO NACP; empty
    /// when unknown (the clean file name is the fallback).
    pub title: String,
    pub icon: SwitchIcon,
}

impl SwitchRomMeta {
    fn empty() -> Self {
        SwitchRomMeta {
            title_id: String::new(),
            title: String::new(),
            icon: SwitchIcon::None,
        }
    }
}

/// Title names and icons known to one emulator install, keyed by title id
/// and by normalized title.
pub struct TitleCache {
    by_id: HashMap<String, CacheEntry>,
    by_name: HashMap<String, String>,
}

struct CacheEntry {
    title: String,
    icon: Option<PathBuf>,
}

impl TitleCache {
    fn empty() -> Self {
        TitleCache {
            by_id: HashMap::new(),
            by_name: HashMap::new(),
        }
    }

    fn insert(&mut self, title_id: &str, title: String, icon: Option<PathBuf>) {
        let id = title_id.to_lowercase();
        if !title.is_empty() {
            self.by_name.insert(normalize_name(&title), id.clone());
        }
        self.by_id.insert(id, CacheEntry { title, icon });
    }

    fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    fn title_for(&self, title_id: &str) -> Option<&str> {
        self.by_id.get(title_id).map(|e| e.title.as_str())
    }

    fn icon_for(&self, title_id: &str) -> SwitchIcon {
        self.by_id
            .get(title_id)
            .and_then(|e| e.icon.clone())
            .map_or(SwitchIcon::None, SwitchIcon::File)
    }

    fn id_for_name(&self, name_key: &str) -> Option<&str> {
        self.by_name.get(name_key).map(String::as_str)
    }

    /// Reads one emulator's `game_list` cache directory: per-title
    /// `<id>.appname.txt` names with sibling `<id>.jpeg` icons.
    fn from_game_list_dir(dir: &Path) -> Self {
        let mut cache = TitleCache::empty();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return cache;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".appname.txt") else {
                continue;
            };
            if !rom::is_title_id(stem) {
                continue;
            }
            let Ok(title) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let icon = {
                let jpeg = entry.path().with_file_name(format!("{stem}.jpeg"));
                jpeg.is_file().then_some(jpeg)
            };
            cache.insert(stem, title.trim().to_string(), icon);
        }
        cache
    }
}

/// Lowercases and drops separators so `Super_Mario Odyssey [upd]` matches
/// an emulator's `Super Mario Odyssey`. Bracketed tags are removed by the
/// caller.
fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// True when the string is a 16-digit lowercase hex application id.
pub fn is_title_id(s: &str) -> bool {
    rom::is_title_id(s)
}

/// All native metadata caches on this machine, best source first:
/// Ryujinx-family installs ahead of the yuzu-family, portable layouts
/// ahead of XDG ones.
pub struct SwitchCaches(Vec<TitleCache>);

impl SwitchCaches {
    pub fn load(executable: &str) -> Self {
        let mut caches = ryujinx::title_caches(executable);
        for dir in config::game_list_cache_dirs_for(executable) {
            let cache = TitleCache::from_game_list_dir(&dir);
            if !cache.is_empty() {
                caches.push(cache);
            }
        }
        SwitchCaches(caches)
    }

    fn title_for(&self, title_id: &str) -> Option<&str> {
        self.0
            .iter()
            .find_map(|cache| cache.title_for(title_id).filter(|t| !t.is_empty()))
    }

    fn icon_for(&self, title_id: &str) -> SwitchIcon {
        self.0
            .iter()
            .find_map(|cache| match cache.icon_for(title_id) {
                SwitchIcon::None => None,
                icon => Some(icon),
            })
            .unwrap_or(SwitchIcon::None)
    }

    fn id_for_name(&self, name_key: &str) -> Option<&str> {
        self.0.iter().find_map(|cache| cache.id_for_name(name_key))
    }
}

/// A title installed in an emulator's NAND or library rather than present
/// as a ROM file: yuzu-family NAND installs surface through the game-list
/// cache, Ryujinx-family library entries through their metadata dirs.
pub struct SwitchInstalledGame {
    /// 16 lowercase hex digits.
    pub title_id: String,
    /// The emulator's application title; installed games always have one.
    pub title: String,
    /// The emulator-cached icon JPEG, when the yuzu family provides it.
    pub icon: Option<PathBuf>,
}

impl SwitchCaches {
    /// Enumerates every installed title across all known installs,
    /// deduplicated by title id with the Ryujinx family's metadata
    /// winning, sorted by id so scans are reproducible.
    fn installed_games(&self) -> Vec<SwitchInstalledGame> {
        let mut out: Vec<SwitchInstalledGame> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for cache in &self.0 {
            for (id, entry) in cache.by_id.iter() {
                if entry.title.is_empty() || !seen.insert(id.clone()) {
                    continue;
                }
                out.push(SwitchInstalledGame {
                    title_id: id.clone(),
                    title: entry.title.clone(),
                    icon: entry.icon.clone(),
                });
            }
        }
        out.sort_by(|a, b| a.title_id.cmp(&b.title_id));
        out
    }
}

/// Enumerates the titles installed in every detected yuzu-family and
/// Ryujinx-family install — both at once, no matter which emulator the
/// user configured for launching ROM files.
pub fn discover_installed_games(executable: &str) -> Vec<SwitchInstalledGame> {
    SwitchCaches::load(executable).installed_games()
}

/// Resolves a ROM file's native metadata: title id from the file name or
/// its NSP ticket, title and icon from the emulator caches, or the NRO's
/// own asset block for homebrew.
pub fn rom_meta(rom: &Path, caches: &SwitchCaches) -> SwitchRomMeta {
    let stem = rom
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    if let Some(title_id) =
        rom::title_id_from_nsp(rom).or_else(|| rom::title_id_from_filename(&stem))
    {
        let mut meta = SwitchRomMeta::empty();
        meta.title_id = title_id.clone();
        meta.title = caches.title_for(&title_id).unwrap_or_default().to_string();
        meta.icon = caches.icon_for(&title_id);
        return meta;
    }

    if let Some(extension) = rom.extension().and_then(|e| e.to_str()) {
        if extension.eq_ignore_ascii_case("nro") {
            if let Some((icon, title)) = rom::read_nro_asset(rom) {
                let mut meta = SwitchRomMeta::empty();
                meta.title = title;
                meta.icon = SwitchIcon::Bytes(icon);
                return meta;
            }
        }
    }

    // A clean dump file name matches a cached application title, which
    // supplies the id (and through it icon and title).
    let name_key = normalize_name(&strip_tags(&stem));
    if let Some(title_id) = caches.id_for_name(&name_key) {
        let mut meta = SwitchRomMeta::empty();
        meta.title_id = title_id.to_string();
        meta.title = caches.title_for(title_id).unwrap_or_default().to_string();
        meta.icon = caches.icon_for(title_id);
        return meta;
    }

    SwitchRomMeta::empty()
}

/// Resolves a ROM file's native metadata with the icon decrypted straight
/// from the ROM's control NCA (see `native_icon`); titles and ids still
/// come from the emulator caches.
pub fn rom_meta_deep(rom: &Path, caches: &SwitchCaches, executable: &str) -> SwitchRomMeta {
    let mut meta = rom_meta(rom, caches);
    meta.icon = native_icon(rom, caches, executable);
    meta
}

/// A ROM's native icon, straight from the source: the ROM file's control
/// NCA is decrypted with the user's dumped keys — the same bytes the
/// emulators show. The emulators' cached icon JPEGs only serve as a
/// fallback for containers Ira cannot decrypt (XCI) or when no keys are
/// installed.
pub fn native_icon(rom: &Path, caches: &SwitchCaches, executable: &str) -> SwitchIcon {
    if let Some(keys) = keys::SwitchKeys::load(executable) {
        if let Some(bytes) = nca::extract_icon(rom, &keys) {
            return SwitchIcon::Bytes(bytes);
        }
    }
    rom_meta(rom, caches).icon
}

/// The icon of a NAND-installed title, decrypted from the emulator's own
/// `Contents/Registered` NCAs — no emulator cache involved.
pub fn extract_installed_icon(executable: &str, title_id: &str) -> Option<Vec<u8>> {
    registered::installed_icon(executable, title_id)
}

/// Removes bracketed tags and a leading bare title id from a dump file
/// name, leaving the game's name for matching.
fn strip_tags(stem: &str) -> String {
    let stem = match stem.split_once(' ') {
        Some((token, rest)) if rom::is_title_id(token) => rest,
        _ => stem,
    };
    let mut out = String::new();
    let mut depth = 0i32;
    for c in stem.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::rom;

    pub(crate) fn nro_fixture(title: &str, icon: &[u8]) -> Vec<u8> {
        rom::nro_fixture(title, icon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Portable Eden layout rooted at a fake executable, whose game_list
    /// cache holds one title with an icon.
    fn eden_fixture(tmp: &Path) -> String {
        let exe = tmp.join("eden.AppImage");
        std::fs::write(&exe, b"").unwrap();
        let dir = tmp.join("user/cache/game_list");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("01007EF00011E000.appname.txt"),
            "The Legend of Zelda\n",
        )
        .unwrap();
        std::fs::write(dir.join("01007EF00011E000.jpeg"), b"JPEGDATA").unwrap();
        exe.to_string_lossy().into_owned()
    }

    /// Portable Ryubing layout next to the same executable, naming a
    /// different title — the family the lookup prefers.
    fn ryujinx_fixture(tmp: &Path) {
        let dir = tmp.join("portable/games/0100000000010000/gui");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("metadata.json"), r#"{"title": "Super Mario Odyssey"}"#).unwrap();
    }

    #[test]
    fn test_rom_meta_from_filename_id_uses_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = eden_fixture(tmp.path());
        let caches = SwitchCaches::load(&exe);
        let meta = rom_meta(Path::new("/roms/Zelda [01007EF00011E000].nsp"), &caches);
        assert_eq!(meta.title_id, "01007ef00011e000");
        assert_eq!(meta.title, "The Legend of Zelda");
        assert!(matches!(meta.icon, SwitchIcon::File(_)));
    }

    #[test]
    fn test_rom_meta_matches_cached_app_name() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = eden_fixture(tmp.path());
        let caches = SwitchCaches::load(&exe);
        let meta = rom_meta(Path::new("/roms/The Legend of Zelda [upd].xci"), &caches);
        assert_eq!(meta.title_id, "01007ef00011e000");
        assert_eq!(meta.title, "The Legend of Zelda");
    }

    #[test]
    fn test_rom_meta_prefers_ryujinx_then_eden() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = eden_fixture(tmp.path());
        ryujinx_fixture(tmp.path());
        let caches = SwitchCaches::load(&exe);

        // Ryujinx's title and id win…
        let meta = rom_meta(Path::new("/roms/Super Mario Odyssey.xci"), &caches);
        assert_eq!(meta.title_id, "0100000000010000");
        assert_eq!(meta.title, "Super Mario Odyssey");
        assert!(matches!(meta.icon, SwitchIcon::None));

        // …while an id from the file name takes Ryujinx's title (unknown
        // here) but Eden's icon.
        let meta = rom_meta(Path::new("/roms/game [01007EF00011E000].nsp"), &caches);
        assert_eq!(meta.title, "The Legend of Zelda");
        assert!(matches!(meta.icon, SwitchIcon::File(_)));
    }

    #[test]
    fn test_rom_meta_unknown_file_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = eden_fixture(tmp.path());
        let caches = SwitchCaches::load(&exe);
        let meta = rom_meta(Path::new("/roms/Unknown Game.nsp"), &caches);
        assert_eq!(meta.title_id, "");
        assert_eq!(meta.title, "");
        assert!(matches!(meta.icon, SwitchIcon::None));
    }

    #[test]
    fn test_rom_meta_nro_carries_icon_and_title() {
        let tmp = tempfile::tempdir().unwrap();
        let rom = tmp.path().join("homebrew.nro");
        std::fs::write(&rom, fixtures::nro_fixture("Homebrew Title", b"PNG")).unwrap();
        let caches = SwitchCaches::load("");
        let meta = rom_meta(&rom, &caches);
        assert_eq!(meta.title, "Homebrew Title");
        assert!(matches!(meta.icon, SwitchIcon::Bytes(data) if data == b"PNG"));
    }

    #[test]
    fn test_installed_games_dedupes_by_title_id() {
        let mut ryujinx = TitleCache::empty();
        ryujinx.insert(
            "0100000000010000",
            "Super Mario Odyssey".to_string(),
            None,
        );
        let mut eden = TitleCache::empty();
        // The same title known to both: the Ryujinx entry wins.
        eden.insert("0100000000010000", "Older Name".to_string(), None);
        eden.insert(
            "01007ef00011e000",
            "The Legend of Zelda".to_string(),
            Some(PathBuf::from("/cache/01007EF00011E000.jpeg")),
        );
        // A title with no name (an unreadable metadata entry) is skipped.
        eden.insert("0100000000010800", String::new(), None);

        let caches = SwitchCaches(vec![ryujinx, eden]);
        let installed = caches.installed_games();
        assert_eq!(
            installed
                .iter()
                .map(|g| (g.title_id.as_str(), g.title.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("0100000000010000", "Super Mario Odyssey"),
                ("01007ef00011e000", "The Legend of Zelda"),
            ]
        );
        assert_eq!(
            installed[1].icon,
            Some(PathBuf::from("/cache/01007EF00011E000.jpeg"))
        );
    }

    #[test]
    fn test_normalize_name_drops_separators() {
        assert_eq!(
            normalize_name("Super_Mario Odyssey 3D World"),
            "supermarioodyssey3dworld"
        );
    }

    #[test]
    fn test_strip_tags_removes_brackets_and_leading_id() {
        assert_eq!(strip_tags("Game [DLC] [v0]"), "Game");
        assert_eq!(strip_tags("0100000000010000 Game Name"), "Game Name");
    }

    #[test]
    fn test_is_title_id_checks_shape() {
        assert!(is_title_id("01007ef00011e000"));
        assert!(!is_title_id("1234"));
        assert!(!is_title_id("zzzzzzzzzzzzzzzz"));
    }
}
