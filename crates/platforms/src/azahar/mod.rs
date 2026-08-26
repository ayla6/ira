//! Azahar (Nintendo 3DS emulator) integration: reads its `qt-config.ini`
//! for game folders and the NAND/SDMC roots, and lists games from them.

mod config;
mod rom;
mod title;

pub use self::config::{read_paths_for_executable, AzaharPaths, AZAHAR_FLATPAK_ID};

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use self::rom::{scan_installed_content, scan_rom_file};

/// ROM extensions Azahar's own game list recognizes, minus `elf`/`axf`
/// executables; `zcci`/`zcxi`/`z3dsx` are Z3DS-compressed counterparts.
const ROM_EXTENSIONS: &[&str] = &["3ds", "cci", "cxi", "app", "zcci", "zcxi", "z3dsx", "3dsx"];
/// NAND/SDMC title category that holds user applications. System titles
/// (home menu, system settings, applets, data archives) live in other
/// categories such as 00040010 and are deliberately not listed.
const APPLICATIONS_CATEGORY: &str = "00040000";
const MAX_SCAN_DEPTH: u32 = 8;

#[derive(Debug, Clone)]
pub struct AzaharGame {
    /// 16 lowercase hex digits for retail titles, e.g. `00040000000e5c00`;
    /// `3dsx-<name>` for homebrew, which carries no title ID.
    pub title_id: String,
    /// SMDH short description, a cleaned dump filename, or empty for
    /// installed titles whose metadata is encrypted.
    pub title: String,
    /// Launchable file: the ROM itself (including Z3DS-compressed) or the
    /// installed title's main content file.
    pub game_path: PathBuf,
    /// 48×48 RGB565 icon from the SMDH, when the ExeFS is readable.
    pub icon: Option<Vec<u8>>,
}

/// Collects ROM file paths below `path`; parsing is deferred so it can run
/// concurrently.
fn collect_rom_paths(path: &Path, depth_left: u32, out: &mut Vec<PathBuf>) {
    if depth_left == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            collect_rom_paths(&entry.path(), depth_left - 1, out);
        } else if entry
            .path()
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ROM_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        {
            out.push(entry.path());
        }
    }
}

/// Scans files concurrently; Z3DS containers decompress forward from the
/// start for every metadata read, which made large libraries take minutes
/// when scanned one file at a time. `par_iter` output order matches the
/// input, so dedup priority stays deterministic.
fn scan_rom_files(
    paths: &[PathBuf],
    parse: fn(&Path) -> Option<AzaharGame>,
) -> Vec<Option<AzaharGame>> {
    use rayon::prelude::*;
    paths.par_iter().map(|path| parse(path)).collect()
}

/// Collects the `title` directory roots below a NAND or SDMC root:
/// NAND keeps titles at `<root>/<id0>/title`, the SDMC at
/// `<root>/Nintendo 3DS/<id0>/<id1>/title`.
fn installed_title_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let direct = root.join("title");
    if direct.is_dir() {
        roots.push(direct);
    }
    let base = root.join("Nintendo 3DS");
    let base = if base.is_dir() {
        base
    } else {
        root.to_path_buf()
    };
    let Ok(ids) = std::fs::read_dir(base) else {
        return roots;
    };
    for id0 in ids
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
    {
        let title_dir = id0.path().join("title");
        if title_dir.is_dir() {
            roots.push(title_dir);
            continue;
        }
        let Ok(id1s) = std::fs::read_dir(id0.path()) else {
            continue;
        };
        for id1 in id1s
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        {
            let title_dir = id1.path().join("title");
            if title_dir.is_dir() {
                roots.push(title_dir);
            }
        }
    }
    roots
}

/// The main content is the largest non-metadata file in `content/`; the
/// `.tmd` title metadata and manual/DLC contents are smaller than the game.
fn main_content_file(content_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(content_dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|entry| entry.path().extension().is_none_or(|ext| ext != "tmd"))
        .max_by_key(|entry| entry.metadata().map(|m| m.len()).unwrap_or(0))
        .map(|entry| entry.path())
}

/// One installed title awaiting its metadata scan: the main content file
/// plus the low title ID used to assemble the full `title_id`.
struct InstalledTitle {
    content: PathBuf,
    low_id: String,
}

/// Enumerates installed application titles under a NAND/SDMC root without
/// parsing their contents.
fn collect_installed_titles(root: &Path) -> Vec<InstalledTitle> {
    let mut titles = Vec::new();
    for title_root in installed_title_roots(root) {
        let apps = title_root.join(APPLICATIONS_CATEGORY);
        let Ok(entries) = std::fs::read_dir(apps) else {
            continue;
        };
        for entry in entries.flatten() {
            let low_id = entry.file_name().to_string_lossy().into_owned();
            if low_id.len() != 8 || !low_id.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let Some(content) = main_content_file(&entry.path().join("content")) else {
                continue;
            };
            titles.push(InstalledTitle { content, low_id });
        }
    }
    titles
}

fn scan_installed_titles(root: &Path, results: &mut Vec<AzaharGame>) {
    let titles = collect_installed_titles(root);
    let contents: Vec<PathBuf> = titles.iter().map(|t| t.content.clone()).collect();
    // Content file names are hashes with no usable title, so installed
    // titles scan without the filename fallback that ROM dumps get.
    for (title, game) in titles
        .into_iter()
        .zip(scan_rom_files(&contents, scan_installed_content))
    {
        if let Some(mut game) = game {
            game.title_id = format!("{APPLICATIONS_CATEGORY}{}", title.low_id.to_lowercase());
            results.push(game);
        }
    }
}

pub fn discover_games() -> Vec<AzaharGame> {
    discover_games_for_executable("")
}

/// Extracts the 48×48 linear RGB565 SMDH icon from a ROM dump or installed
/// title content file (both optionally Z3DS-compressed).
pub fn read_icon(path: &Path) -> Option<Vec<u8>> {
    scan_rom_file(path).and_then(|game| game.icon)
}

pub fn discover_games_for_executable(executable: &str) -> Vec<AzaharGame> {
    // A missing native executable must not stop discovery: game locations
    // live in Azahar's own config (XDG or portable), which is readable even
    // when the launch command is stale (renamed AppImage, wrapper script…).
    if !executable.is_empty()
        && !executable.starts_with("flatpak:")
        && executable.contains('/')
        && !Path::new(executable).is_file()
    {
        eprintln!(
            "Azahar executable not found, scanning its config anyway: {}",
            executable
        );
    }
    let paths = read_paths_for_executable(executable);
    let mut games = Vec::new();
    for (dir, deep_scan) in paths.game_dirs {
        let depth = if deep_scan { MAX_SCAN_DEPTH } else { 1 };
        let mut rom_paths = Vec::new();
        collect_rom_paths(&dir, depth, &mut rom_paths);
        games.extend(scan_rom_files(&rom_paths, scan_rom_file).into_iter().flatten());
    }
    scan_installed_titles(&paths.nand_dir, &mut games);
    scan_installed_titles(&paths.sdmc_dir, &mut games);

    // The same title can be reachable both as a ROM dump in a configured
    // game folder and installed on the NAND/SDMC; configured folders are
    // scanned first so the user-chosen location wins.
    let mut seen = HashSet::new();
    games.retain(|game| seen.insert(game.title_id.clone()));
    games
}

#[cfg(test)]
mod tests {
    use super::rom::fixtures::{cxi_fixture, z3ds_fixture};
    use super::*;

    fn write_installed_title(root: &Path, category: &str, low_id: &str, size: usize) {
        let content = root.join(category).join(low_id).join("content");
        std::fs::create_dir_all(&content).unwrap();
        std::fs::write(content.join("00000000.tmd"), vec![0u8; 16]).unwrap();
        // The main content is a real NCCH; pad it so it stays the largest
        // non-tmd file even next to bigger tmds.
        let mut app = cxi_fixture(0x00040000000E5C00);
        app.resize(app.len() + size, 0);
        std::fs::write(content.join("7926fc67.app"), app).unwrap();
    }

    #[test]
    fn test_scan_installed_titles_lists_only_applications() {
        let tmp = tempfile::tempdir().unwrap();
        let nand = tmp
            .path()
            .join("nand/00000000000000000000000000000000/title");
        write_installed_title(&nand, "00040000", "0f800100", 1024);
        write_installed_title(&nand, "00040010", "00021000", 4096); // home menu
        write_installed_title(&nand, "0004001b", "00018000", 4096); // system data

        let mut games = Vec::new();
        scan_installed_titles(&tmp.path().join("nand"), &mut games);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title_id, "000400000f800100");
        assert!(games[0].game_path.ends_with("7926fc67.app"));
        assert_eq!(games[0].title, "Test Game");
        assert_eq!(games[0].icon.as_deref().map(<[u8]>::len), Some(0x1200));
    }

    /// Installed contents are often Z3DS-compressed on disk; metadata must
    /// be readable through the container.
    #[test]
    fn test_scan_installed_content_through_z3ds() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("8e9c626d.app");
        std::fs::write(&app, z3ds_fixture(&cxi_fixture(0x000400000D40D200))).unwrap();

        let game = super::rom::scan_installed_content(&app).unwrap();
        assert_eq!(game.title_id, "000400000d40d200");
        assert_eq!(game.title, "Test Game");
        assert!(game.icon.is_some());
    }

    #[test]
    fn test_scan_installed_titles_reads_sdmc_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let titles = tmp.path().join("sdmc/Nintendo 3DS/00000000000000000000000000000000/00000000000000000000000000000000/title");
        write_installed_title(&titles, "00040000", "0d40d200", 512);

        let mut games = Vec::new();
        scan_installed_titles(&tmp.path().join("sdmc"), &mut games);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title_id, "000400000d40d200");
    }

    /// Fixture: a portable Azahar whose game list points at a ROM folder and
    /// whose NAND also holds the same title installed.
    fn azahar_fixture_with_duplicated_title() -> (tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("azahar");
        std::fs::write(&exe, b"").unwrap();
        let user = tmp.path().join("user");
        std::fs::create_dir_all(user.join("config")).unwrap();
        let roms = tmp.path().join("roms");
        std::fs::create_dir_all(&roms).unwrap();
        std::fs::write(
            user.join("config/qt-config.ini"),
            format!(
                "[Data%20Storage]\nnand_directory={nand}/\n[UI]\n\
                 Paths\\gamedirs\\1\\deep_scan=false\nPaths\\gamedirs\\1\\path={roms}\n\
                 Paths\\gamedirs\\2\\path=INSTALLED\nPaths\\gamedirs\\size=2\n",
                nand = user.join("nand").display(),
                roms = roms.display()
            ),
        )
        .unwrap();
        std::fs::create_dir_all(user.join("nand").join("title")).unwrap();
        write_installed_title(&user.join("nand/title"), "00040000", "0f800100", 128);
        // Same title id as the installed one above.
        std::fs::write(
            roms.join("000400000F800100 (v1.0) (J).3ds"),
            cxi_fixture(0x000400000F800100),
        )
        .unwrap();
        (tmp, exe.to_string_lossy().into_owned())
    }

    #[test]
    fn test_discover_azahar_dedupes_rom_and_installed_title() {
        let (_tmp, exe) = azahar_fixture_with_duplicated_title();

        let games = discover_games_for_executable(&exe);

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title_id, "000400000f800100");
        assert_eq!(games[0].title, "Test Game");
        assert!(games[0].game_path.to_string_lossy().ends_with(".3ds"));
    }

    #[test]
    fn test_discover_azahar_deep_scan_finds_nested_compressed_roms() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("azahar");
        std::fs::write(&exe, b"").unwrap();
        let user = tmp.path().join("user");
        std::fs::create_dir_all(user.join("config")).unwrap();
        let roms = tmp.path().join("roms/regional");
        std::fs::create_dir_all(&roms).unwrap();
        std::fs::write(
            user.join("config/qt-config.ini"),
            format!(
                "[UI]\nPaths\\gamedirs\\1\\deep_scan=true\n\
                 Paths\\gamedirs\\1\\path={}\nPaths\\gamedirs\\size=1\n",
                tmp.path().join("roms").display()
            ),
        )
        .unwrap();
        std::fs::write(roms.join("game.zcci"), z3ds_fixture(&cxi_fixture(1))).unwrap();

        let games = discover_games_for_executable(&exe.to_string_lossy());

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].game_path, roms.join("game.zcci"));
    }

    /// A stale launch command (renamed AppImage, moved binary) must not hide
    /// games: discovery falls back to the portable user dir next to it.
    #[test]
    fn test_discover_azahar_stale_executable_still_scans_config() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user");
        std::fs::create_dir_all(user.join("config")).unwrap();
        let roms = tmp.path().join("roms");
        std::fs::create_dir_all(&roms).unwrap();
        std::fs::write(
            user.join("config/qt-config.ini"),
            format!(
                "[UI]\nPaths\\gamedirs\\1\\path={}\nPaths\\gamedirs\\size=1\n",
                roms.display()
            ),
        )
        .unwrap();
        std::fs::write(roms.join("game.cxi"), cxi_fixture(0x00040000000E5C00)).unwrap();

        // The configured executable itself does not exist.
        let stale_exe = tmp.path().join("azahar");
        assert!(!stale_exe.exists());

        let games = discover_games_for_executable(&stale_exe.to_string_lossy());

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title_id, "00040000000e5c00");
    }
}
