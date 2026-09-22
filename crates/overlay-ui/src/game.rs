//! Game data snapshot read from the Ira app's SHM region.
//!
//! The host only reads: achievements, counts, playtime, and the
//! notification write index (for change detection). All parsing is pure and
//! unit-tested; GTK lives in `ui.rs`.

use std::sync::atomic::Ordering;

use ira_overlay_ipc::{MappedShm, MAX_ACHIEVEMENTS};

#[derive(Clone)]
pub struct AchData {
    pub name: String,
    pub desc: String,
    pub icon: String,
    pub icon_gray: String,
    pub earned: bool,
    pub hidden: bool,
}

#[derive(Clone)]
pub struct GameData {
    pub name: String,
    pub kind: String,
    pub total: u32,
    pub unlocked: u32,
    pub playtime_seconds: u64,
    pub achievements: Vec<AchData>,
}

/// Change-detection fingerprint: totals + playtime + notification index.
/// The host rebuilds the widget tree only when this changes.
pub type Fingerprint = (u32, u32, u32, u64);

pub fn fingerprint(shm: &MappedShm) -> Fingerprint {
    let hdr = shm.header();
    (
        hdr.total_achievements,
        hdr.unlocked_achievements,
        hdr.notification_write_index.load(Ordering::SeqCst),
        hdr.playtime_seconds,
    )
}

pub fn load(shm: &MappedShm) -> GameData {
    let hdr = shm.header();
    let total = hdr.total_achievements as usize;
    let achievements = shm
        .achievements()
        .iter()
        .take(total.min(MAX_ACHIEVEMENTS))
        .map(|e| AchData {
            name: cstr(&e.display_name),
            desc: cstr(&e.description),
            icon: cstr(&e.icon_path),
            icon_gray: cstr(&e.icon_gray_path),
            earned: e.earned != 0,
            hidden: e.hidden != 0,
        })
        .collect();
    GameData {
        name: cstr(&hdr.game_name),
        kind: cstr(&hdr.game_kind),
        total: hdr.total_achievements,
        unlocked: hdr.unlocked_achievements,
        playtime_seconds: hdr.playtime_seconds,
        achievements,
    }
}

impl GameData {
    pub fn progress_line(&self) -> String {
        let h = self.playtime_seconds / 3600;
        let m = (self.playtime_seconds % 3600) / 60;
        format!("{} • {} / {} — {h}h {m}m", self.kind, self.unlocked, self.total)
    }

    /// Mock data for `--mock` runs (no game SHM). Mirrors the spike fixture.
    pub fn mock() -> Self {
        Self {
            name: "Mock Game".to_string(),
            kind: "gbe_steam".to_string(),
            total: 8,
            unlocked: 3,
            playtime_seconds: 5 * 3600 + 42 * 60,
            achievements: (0..8)
                .map(|i| AchData {
                    name: format!("Achievement {i}"),
                    desc: "Defeat the boss without taking damage on hard mode".to_string(),
                    icon: String::new(),
                    icon_gray: String::new(),
                    earned: i < 3,
                    hidden: false,
                })
                .collect(),
        }
    }
}

fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn write_str(dst: &mut [u8], src: &str) {
        let bytes = src.as_bytes();
        let len = bytes.len().min(dst.len() - 1);
        dst[..len].copy_from_slice(&bytes[..len]);
        dst[len] = 0;
    }

    fn fixture(db_id: i64) -> MappedShm {
        let mut shm = MappedShm::create(db_id).unwrap();
        shm.init_header(db_id);
        let hdr = shm.header_mut();
        write_str(&mut hdr.game_name, "Test Game");
        hdr.total_achievements = 2;
        hdr.unlocked_achievements = 1;
        hdr.playtime_seconds = 3700;
        let achs = shm.achievements_mut();
        write_str(&mut achs[0].display_name, "First");
        achs[0].earned = 1;
        write_str(&mut achs[1].display_name, "Second");
        achs[1].earned = 0;
        achs[1].hidden = 1;
        shm
    }

    fn cleanup(db_id: i64) {
        let path = ira_overlay_ipc::shm_path(db_id);
        let c_path = CString::new(path).unwrap();
        unsafe { libc::shm_unlink(c_path.as_ptr()) };
    }

    #[test]
    fn test_load_reads_game_and_achievements() {
        let db_id = 20260201;
        let shm = fixture(db_id);
        let data = load(&shm);
        assert_eq!(data.name, "Test Game");
        assert_eq!((data.total, data.unlocked), (2, 1));
        assert_eq!(data.achievements.len(), 2);
        assert_eq!(data.achievements[0].name, "First");
        assert!(data.achievements[0].earned);
        assert!(data.achievements[1].hidden);
        cleanup(db_id);
    }

    #[test]
    fn test_fingerprint_changes_on_notification() {
        let db_id = 20260202;
        let mut shm = fixture(db_id);
        let before = fingerprint(&shm);
        shm.push_notification(ira_overlay_ipc::NotificationEntry {
            notification_type: 0,
            achievement_index: 0,
            timestamp: 1,
        });
        assert_ne!(fingerprint(&shm), before);
        cleanup(db_id);
    }

    #[test]
    fn test_progress_line_format() {
        let data = GameData::mock();
        assert_eq!(data.progress_line(), "gbe_steam • 3 / 8 — 5h 42m");
    }

    #[test]
    fn test_cstr_edge_cases() {
        assert_eq!(cstr(&[]), "");
        assert_eq!(cstr(&[0, 0]), "");
        assert_eq!(cstr(b"abc"), "abc");
        assert_eq!(cstr(b"ab\x00cd"), "ab");
        assert_eq!(cstr(&[0xff, 0xfe, 0]), "��");
    }
}
