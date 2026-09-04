//! Native Switch metadata from the emulators' own caches, read without
//! decryption keys. Two families are covered:
//!
//! - yuzu-family (Eden, suyu, citron, sudachi, yuzu): Eden caches every
//!   scanned title's native icon (`<title id>.jpeg`) and application name
//!   (`<title id>.appname.txt`) under its `game_list` cache, and it can
//!   install titles to its NAND — its cache is also the installed-titles
//!   source.
//! - Ryujinx-family (Ryubing, Kenji-NX): `games/<title id>/gui/
//!   metadata.json` holds each library title's display name; icons are
//!   never cached on disk. The family cannot install titles, so its
//!   library is enumerated from the ROM files in the folders its
//!   `Config.json` points at — the metadata dirs themselves outlive
//!   deleted files and would resurrect uninstalled titles.
//!
//! ROM files map onto these entries by file-name title id, NSP ticket
//! names, or clean-name match; homebrew NROs carry icon and title inside
//! their asset block. With the user's dumped keys, the ROM's own control
//! NCA is decrypted for its icon and `control.nacp` application title —
//! no emulator cache involved.

mod config;
mod keys;
mod nacp;
mod nca;
mod registered;
mod rom;
mod xci;
mod ryujinx;

pub use config::resolve_rom;
pub use nca::ControlMeta;

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
    /// Application title from an emulator cache, the NRO NACP, or the
    /// ROM's own control NACP (`rom_meta_deep`); empty when unknown (the
    /// clean file name is the fallback).
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
#[derive(Clone)]
pub struct TitleCache {
    by_id: HashMap<String, CacheEntry>,
    by_name: HashMap<String, String>,
}

#[derive(Clone)]
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

/// All native metadata caches on this machine: Ryujinx-family metadata
/// dirs and yuzu-family game-list caches resolve ROM files' titles and
/// icons; only the yuzu-family caches double as the installed-titles
/// source, because that family alone can install titles to its NAND.
pub struct SwitchCaches {
    caches: Vec<TitleCache>,
    installed: Vec<TitleCache>,
}

impl SwitchCaches {
    pub fn load(executable: &str) -> Self {
        let mut caches = ryujinx::title_caches(executable);
        let mut installed = Vec::new();
        for dir in config::game_list_cache_dirs_for(executable) {
            let cache = TitleCache::from_game_list_dir(&dir);
            if !cache.is_empty() {
                installed.push(cache.clone());
                caches.push(cache);
            }
        }
        SwitchCaches { caches, installed }
    }

    fn title_for(&self, title_id: &str) -> Option<&str> {
        self.caches
            .iter()
            .find_map(|cache| cache.title_for(title_id).filter(|t| !t.is_empty()))
    }

    fn icon_for(&self, title_id: &str) -> SwitchIcon {
        self.caches
            .iter()
            .find_map(|cache| match cache.icon_for(title_id) {
                SwitchIcon::None => None,
                icon => Some(icon),
            })
            .unwrap_or(SwitchIcon::None)
    }

    fn id_for_name(&self, name_key: &str) -> Option<&str> {
        self.caches.iter().find_map(|cache| cache.id_for_name(name_key))
    }
}

/// A title of an emulator library rather than a ROM file in the user's
/// own folders: yuzu-family NAND installs surface through the game-list
/// cache, Ryujinx-family titles through the ROM files in the folders its
/// config points at.
pub struct SwitchInstalledGame {
    /// 16 lowercase hex digits, always a base application id: updates
    /// (`…800`) describe the same game as their base title and never
    /// become entries of their own.
    pub title_id: String,
    /// The emulator's application title; installed games always have one.
    pub title: String,
    /// The emulator-cached icon JPEG, when the yuzu family provides it.
    pub icon: Option<PathBuf>,
}

impl SwitchCaches {
    /// Enumerates the yuzu family's NAND-installed titles: every cached
    /// name, deduplicated by title id, update ids skipped, sorted so
    /// scans are reproducible.
    fn installed_games(&self) -> Vec<SwitchInstalledGame> {
        let mut out: Vec<SwitchInstalledGame> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for cache in &self.installed {
            for (id, entry) in cache.by_id.iter() {
                if entry.title.is_empty()
                    || rom::is_update_title_id(id)
                    || !seen.insert(id.clone())
                {
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

/// Enumerates the titles of every detected yuzu-family and Ryujinx-family
/// install — both at once, no matter which emulator the user configured
/// for launching ROM files.
pub fn discover_installed_games(executable: &str) -> Vec<SwitchInstalledGame> {
    let caches = SwitchCaches::load(executable);
    let mut games = caches.installed_games();
    let mut seen: std::collections::HashSet<String> = games
        .iter()
        .map(|game| game.title_id.clone())
        .collect();
    for game in ryujinx::library_games(executable) {
        if seen.insert(game.title_id.clone()) {
            games.push(game);
        }
    }
    games.sort_by(|a, b| a.title_id.cmp(&b.title_id));
    games
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

/// Resolves a ROM file's native metadata with the icon and application
/// title decrypted straight from the ROM's control NCA (see
/// `native_control_meta`); the emulator caches fill in whatever the ROM
/// does not carry.
pub fn rom_meta_deep(rom: &Path, caches: &SwitchCaches, executable: &str) -> SwitchRomMeta {
    let mut meta = rom_meta(rom, caches);
    let control = native_control_meta(rom, executable);
    if let Some(icon) = control.as_ref().and_then(|m| m.icon.clone()) {
        meta.icon = SwitchIcon::Bytes(icon);
    }
    if meta.title.is_empty() {
        if let Some(title) = control.and_then(|m| m.title) {
            meta.title = title;
        }
    }
    meta
}

/// A ROM's native icon, straight from the source: the ROM file's control
/// NCA is decrypted with the user's dumped keys — the same bytes the
/// emulators show. The emulators' cached icon JPEGs only serve as a
/// fallback for containers Ira cannot decrypt (XCI) or when no keys are
/// installed.
pub fn native_icon(rom: &Path, caches: &SwitchCaches, executable: &str) -> SwitchIcon {
    if let Some(icon) = native_control_meta(rom, executable).and_then(|meta| meta.icon) {
        return SwitchIcon::Bytes(icon);
    }
    rom_meta(rom, caches).icon
}

/// The ROM's control NCA metadata (application title and icon), decoded
/// in one pass when the user's dumped keys are available. `None` for
/// containers Ira cannot decrypt (XCI) or without keys.
fn native_control_meta(rom: &Path, executable: &str) -> Option<nca::ControlMeta> {
    let keys = keys::SwitchKeys::load(executable)?;
    nca::extract_control_meta(rom, &keys)
}

/// The application title and icon of a NAND-installed title, decrypted
/// from the emulator's own `Contents/Registered` NCAs — no emulator
/// cache involved.
pub fn extract_installed_meta(executable: &str, title_id: &str) -> Option<ControlMeta> {
    registered::installed_meta(executable, title_id)
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
        let mut eden = TitleCache::empty();
        eden.insert(
            "01007ef00011e000",
            "The Legend of Zelda".to_string(),
            Some(PathBuf::from("/cache/01007EF00011E000.jpeg")),
        );
        // A title with no name (an unreadable metadata entry) is skipped.
        eden.insert("0100000000010000", String::new(), None);
        // The same title cached twice dedupes to one entry.
        let mut eden_portable = TitleCache::empty();
        eden_portable.insert(
            "01007ef00011e000",
            "The Legend of Zelda".to_string(),
            None,
        );

        let caches = SwitchCaches {
            caches: vec![eden.clone(), eden_portable.clone()],
            installed: vec![eden, eden_portable],
        };
        let installed = caches.installed_games();
        assert_eq!(
            installed
                .iter()
                .map(|g| (g.title_id.as_str(), g.title.as_str()))
                .collect::<Vec<_>>(),
            vec![("01007ef00011e000", "The Legend of Zelda")]
        );
        assert_eq!(
            installed[0].icon,
            Some(PathBuf::from("/cache/01007EF00011E000.jpeg"))
        );
    }

    #[test]
    fn test_installed_games_skip_update_ids_and_ryujinx_dirs() {
        let mut eden = TitleCache::empty();
        // An update id cached on its own (its base title is gone) never
        // becomes a game of its own.
        eden.insert("010051f0207b2800", "Tomodachi Life".to_string(), None);
        eden.insert("010051f0207b2000", "Real Base Title".to_string(), None);
        // Ryujinx metadata dirs are a name source only.
        let mut ryujinx = TitleCache::empty();
        ryujinx.insert("0100000000010000", "Super Mario Odyssey".to_string(), None);

        let caches = SwitchCaches {
            caches: vec![ryujinx, eden.clone()],
            installed: vec![eden],
        };
        assert_eq!(
            caches
                .installed_games()
                .iter()
                .map(|g| g.title_id.as_str())
                .collect::<Vec<_>>(),
            vec!["010051f0207b2000"]
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
