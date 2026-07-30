use std::path::Path;

/// How an installer runs: Windows installers need Wine, Linux installers
/// run natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerType {
    Windows,
    Linux,
}

/// What platform the installed game is for. Usually matches `InstallerType`,
/// but some Linux `.sh` installers (e.g. LinuxRulez YAD-based releases)
/// install Windows games.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePlatform {
    Windows,
    Linux,
}

/// Classify an installer by its file extension.
/// `.exe` / `.msi` → Windows, anything else → Linux.
pub fn installer_type(path: &Path) -> InstallerType {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("msi") {
        InstallerType::Windows
    } else {
        InstallerType::Linux
    }
}

/// Determine the platform of the game an installer installs.
/// Usually matches the installer type, but LinuxRulez `.sh` installers
/// (YAD Simple Installer format) wrap Windows games.
pub fn game_platform(path: &Path, itype: InstallerType) -> GamePlatform {
    if itype == InstallerType::Linux && is_linuxrulez(path) {
        GamePlatform::Windows
    } else {
        match itype {
            InstallerType::Windows => GamePlatform::Windows,
            InstallerType::Linux => GamePlatform::Linux,
        }
    }
}

/// Marker that appears in every GOG makeself/mojosetup installer.
pub(super) const MAKESELF_FILESIZE_MARKER: &[u8] = b"filesizes=\"";
pub(super) const MAKESELF_OFFSET_MARKER: &[u8] = b"offset=`head -n ";

/// Marker for LinuxRulez YAD Simple Installer releases.
/// These are `.sh` installers that install Windows games using a YAD GUI.
const LINUXRULEZ_MARKER: &[u8] = b"YAD Simple Installer";

/// Check whether a Linux `.sh` file is a GOG makeself/mojosetup installer.
/// Searches the first 10 KB for both the `filesizes="` and
/// `offset=\`head -n ` markers that makeself embeds in its header.
pub fn is_gog_makeself(path: &Path) -> bool {
    let mut buf = vec![0u8; 10_240];
    let n = match std::fs::File::open(path).and_then(|mut f| {
        std::io::Read::read(&mut f, &mut buf).or(Ok(0usize))
    }) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let head = &buf[..n];
    find_subsequence(head, MAKESELF_FILESIZE_MARKER).is_some()
        && find_subsequence(head, MAKESELF_OFFSET_MARKER).is_some()
}

/// Inno Setup's `SetupLoaderHeaderMagic` — the 4-byte string "Inno" at
/// offset 0x30 in the PE file. See `references/innoextract/src/loader/offsets.cpp:64`.
const INNO_MAGIC_OFFSET: usize = 0x30;
const INNO_MAGIC: &[u8] = b"Inno";
const INNO_VERSION_STRING: &[u8] = b"Inno Setup Setup Data";

/// Check whether a Windows `.exe` file is an Inno Setup installer.
/// Looks for the "Inno" magic at offset 0x30, and falls back to searching
/// the first 64 KB for the "Inno Setup Setup Data" version string (used by
/// GOG Galaxy-wrapped installers where the header may be embedded deeper).
pub fn is_inno_setup(path: &Path) -> bool {
    let mut buf = vec![0u8; 65_536];
    let n = match std::fs::File::open(path).and_then(|mut f| {
        std::io::Read::read(&mut f, &mut buf).or(Ok(0usize))
    }) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let head = &buf[..n];

    if n >= INNO_MAGIC_OFFSET + INNO_MAGIC.len()
        && &head[INNO_MAGIC_OFFSET..INNO_MAGIC_OFFSET + INNO_MAGIC.len()] == INNO_MAGIC
    {
        return true;
    }

    find_subsequence(head, INNO_VERSION_STRING).is_some()
}

/// Check whether a Linux `.sh` file is a LinuxRulez YAD Simple Installer.
/// These are bash scripts that use YAD for a GUI and extract a zstd-compressed
/// tar archive containing a Windows game. They run natively on Linux but the
/// installed game is a Windows game.
pub fn is_linuxrulez(path: &Path) -> bool {
    let mut buf = vec![0u8; 10_240];
    let n = match std::fs::File::open(path).and_then(|mut f| {
        std::io::Read::read(&mut f, &mut buf).or(Ok(0usize))
    }) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let head = &buf[..n];
    find_subsequence(head, LINUXRULEZ_MARKER).is_some()
}

pub(super) fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_fixture(dir: &Path, name: &str, content: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    #[test]
    fn test_installer_type_windows_exe() {
        assert_eq!(
            installer_type(Path::new("setup.exe")),
            InstallerType::Windows
        );
    }

    #[test]
    fn test_installer_type_windows_msi() {
        assert_eq!(
            installer_type(Path::new("setup.msi")),
            InstallerType::Windows
        );
    }

    #[test]
    fn test_installer_type_windows_case_insensitive() {
        assert_eq!(
            installer_type(Path::new("SETUP.EXE")),
            InstallerType::Windows
        );
    }

    #[test]
    fn test_installer_type_linux_sh() {
        assert_eq!(
            installer_type(Path::new("installer.sh")),
            InstallerType::Linux
        );
    }

    #[test]
    fn test_installer_type_linux_bin() {
        assert_eq!(
            installer_type(Path::new("installer.bin")),
            InstallerType::Linux
        );
    }

    #[test]
    fn test_installer_type_linux_no_extension() {
        assert_eq!(
            installer_type(Path::new("installer")),
            InstallerType::Linux
        );
    }

    #[test]
    fn test_is_gog_makeself_true() {
        let tmp = tempfile::tempdir().unwrap();
        let header = b"#!/bin/sh\n\
umask 077\n\
\n\
CRCsum=\"110917346\"\n\
MD5=\"2f936558c83bb8930582b5be21fa4b60\"\n\
\n\
label=\"Hotline Miami (GOG.com)\"\n\
filesizes=\"877150\"\n\
keep=\"n\"\n\
quiet=\"n\"\n\
\n\
offset=`head -n 519 \"$0\" | wc -c | tr -d \" \"\n\
";
        let path = write_fixture(tmp.path(), "installer.sh", header);
        assert!(is_gog_makeself(&path));
    }

    #[test]
    fn test_is_gog_makeself_false_plain_script() {
        let tmp = tempfile::tempdir().unwrap();
        let header = b"#!/bin/sh\necho hello\n";
        let path = write_fixture(tmp.path(), "script.sh", header);
        assert!(!is_gog_makeself(&path));
    }

    #[test]
    fn test_is_gog_makeself_false_only_one_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let header = b"#!/bin/sh\nfilesizes=\"100\"\necho hello\n";
        let path = write_fixture(tmp.path(), "script.sh", header);
        assert!(!is_gog_makeself(&path));
    }

    #[test]
    fn test_is_gog_makeself_false_missing_file() {
        assert!(!is_gog_makeself(Path::new("/nonexistent/path.sh")));
    }

    #[test]
    fn test_is_inno_setup_magic_at_offset_0x30() {
        let tmp = tempfile::tempdir().unwrap();
        let mut buf = vec![0u8; 256];
        buf[0] = b'M';
        buf[1] = b'Z';
        buf[INNO_MAGIC_OFFSET..INNO_MAGIC_OFFSET + INNO_MAGIC.len()]
            .copy_from_slice(INNO_MAGIC);
        let path = write_fixture(tmp.path(), "setup.exe", &buf);
        assert!(is_inno_setup(&path));
    }

    #[test]
    fn test_is_inno_setup_version_string_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let mut buf = vec![0u8; 4096];
        buf[0] = b'M';
        buf[1] = b'Z';
        let msg = b"Inno Setup Setup Data (5.5.0)";
        let pos = 2000;
        buf[pos..pos + msg.len()].copy_from_slice(msg);
        let path = write_fixture(tmp.path(), "setup.exe", &buf);
        assert!(is_inno_setup(&path));
    }

    #[test]
    fn test_is_inno_setup_false_plain_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let mut buf = vec![0u8; 256];
        buf[0] = b'M';
        buf[1] = b'Z';
        let path = write_fixture(tmp.path(), "setup.exe", &buf);
        assert!(!is_inno_setup(&path));
    }

    #[test]
    fn test_is_inno_setup_false_missing_file() {
        assert!(!is_inno_setup(Path::new("/nonexistent/setup.exe")));
    }

    #[test]
    fn test_find_subsequence_basic() {
        assert_eq!(find_subsequence(b"hello world", b"world"), Some(6));
        assert_eq!(find_subsequence(b"hello world", b"xyz"), None);
        assert_eq!(find_subsequence(b"abc", b"abcd"), None);
    }

    #[test]
    fn test_is_linuxrulez_true() {
        let tmp = tempfile::tempdir().unwrap();
        let header = b"#!/usr/bin/env sh\n\
#\n\
# YAD Simple Installer script version 12.01.2021\n\
# by Khryundelek & Kron4ek\n\
\n\
app=\"Metaphor ReFantazio\"\n";
        let path = write_fixture(tmp.path(), "installer.sh", header);
        assert!(is_linuxrulez(&path));
    }

    #[test]
    fn test_is_linuxrulez_false_plain_script() {
        let tmp = tempfile::tempdir().unwrap();
        let header = b"#!/bin/sh\necho hello\n";
        let path = write_fixture(tmp.path(), "script.sh", header);
        assert!(!is_linuxrulez(&path));
    }

    #[test]
    fn test_is_linuxrulez_false_gog_makeself() {
        let tmp = tempfile::tempdir().unwrap();
        let header = b"#!/bin/sh\nfilesizes=\"877150\"\noffset=`head -n 519 \"$0\"`\n";
        let path = write_fixture(tmp.path(), "gog.sh", header);
        assert!(!is_linuxrulez(&path));
    }

    #[test]
    fn test_is_linuxrulez_false_missing_file() {
        assert!(!is_linuxrulez(Path::new("/nonexistent/installer.sh")));
    }

    #[test]
    fn test_game_platform_windows_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fixture(tmp.path(), "setup.exe", b"MZ");
        assert_eq!(
            game_platform(&path, InstallerType::Windows),
            GamePlatform::Windows
        );
    }

    #[test]
    fn test_game_platform_linux_sh() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fixture(tmp.path(), "installer.sh", b"#!/bin/sh\necho hi\n");
        assert_eq!(
            game_platform(&path, InstallerType::Linux),
            GamePlatform::Linux
        );
    }

    #[test]
    fn test_game_platform_linuxrulez_is_windows() {
        let tmp = tempfile::tempdir().unwrap();
        let header = b"#!/usr/bin/env sh\n# YAD Simple Installer script version 12.01.2021\n";
        let path = write_fixture(tmp.path(), "game.sh", header);
        assert_eq!(
            game_platform(&path, InstallerType::Linux),
            GamePlatform::Windows
        );
    }
}
