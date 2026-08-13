//! Overlay shared memory management — creates the SHM region and writes
//! game data + achievements before launching a game.
//!
//! The SHM persists in `/dev/shm/` after this function returns (the mapping
//! is dropped, but the file remains). The overlay (loaded in the game process)
//! opens it read-only via `IRA_OVERLAY_SHM`. The next launch unlinks and
//! recreates it.

use ira_models::Game;
use ira_overlay_ipc::{
    parse_hotkey, shm_path, MappedShm, OverlaySettings, RecordingQuality, VideoEncoder,
    MAX_ACHIEVEMENTS,
};

/// Creates the shared memory region and writes game data + achievements.
/// Returns the SHM name (e.g. `/ira_overlay_123`) to pass as `IRA_OVERLAY_SHM`,
/// or `None` on failure.
pub fn write_game_shm(
    game: &Game,
    settings: &OverlaySettings,
    encoder: Option<u32>,
    recording_quality: Option<u32>,
) -> Option<String> {
    let mut shm = MappedShm::create(game.db_id).ok()?;
    shm.init_header(game.db_id);

    {
        let hdr = shm.header_mut();
        write_str(&mut hdr.game_name, &game.name);
        write_str(&mut hdr.game_kind, &format!("{}", game.kind));
        write_str(&mut hdr.cover_image_path, &game.icon_path);
        hdr.total_achievements = game.total_count.min(u32::MAX as usize) as u32;
        hdr.unlocked_achievements = game.earned_count.min(u32::MAX as usize) as u32;
        hdr.playtime_seconds = (game.playtime * 3600.0) as u64;

        hdr.overlay_position = settings.position.as_u32();
        hdr.video_encoder = encoder
            .map(VideoEncoder::from_u32)
            .unwrap_or(settings.encoder)
            .as_u32();
        hdr.recording_quality = recording_quality
            .map(RecordingQuality::from_u32)
            .unwrap_or(settings.recording_quality)
            .as_u32();

        // Write hotkey config as (evdev_keycode, modifier_mask).
        let (toggle_kc, toggle_mods) = parse_hotkey(&settings.toggle_hotkey);
        let (screenshot_kc, screenshot_mods) = parse_hotkey(&settings.screenshot_hotkey);
        let (record_kc, record_mods) = parse_hotkey(&settings.record_hotkey);
        hdr.toggle_keysym = toggle_kc;
        hdr.toggle_mods = toggle_mods;
        hdr.screenshot_keysym = screenshot_kc;
        hdr.screenshot_mods = screenshot_mods;
        hdr.record_keysym = record_kc;
        hdr.record_mods = record_mods;
    }

    let achievements = shm.achievements_mut();
    for (i, ach) in game.achievements.iter().take(MAX_ACHIEVEMENTS).enumerate() {
        let entry = &mut achievements[i];
        write_str(&mut entry.display_name, &ach.display_name);
        write_str(&mut entry.description, &ach.description);
        write_str(&mut entry.icon_path, &ach.icon_path);
        write_str(&mut entry.icon_gray_path, &ach.icon_gray_path);
        entry.earned = u8::from(ach.earned);
        entry.earned_time = ach.earned_time.max(0) as u64;
        entry.global_percent = ach.global_percent as f32;
        entry.trophy_type = ach.trophy_type as u8;
        entry.hidden = u8::from(ach.hidden);
    }

    Some(shm_path(game.db_id))
}

/// Pushes an achievement-unlocked notification to the SHM ring buffer.
/// Called by the watcher when it detects a newly unlocked achievement.
pub fn push_achievement_notification(db_id: i64, achievement_index: u32) {
    let Ok(mut shm) = MappedShm::open_rw(&shm_path(db_id)) else {
        return;
    };
    shm.push_notification(ira_overlay_ipc::NotificationEntry {
        notification_type: 0,
        achievement_index,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    });
}

fn write_str(dst: &mut [u8], src: &str) {
    let bytes = src.as_bytes();
    let len = bytes.len().min(dst.len() - 1);
    dst[..len].copy_from_slice(&bytes[..len]);
    dst[len] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_overrides_use_known_values() {
        assert_eq!(VideoEncoder::from_u32(2), VideoEncoder::Nvenc);
        assert_eq!(RecordingQuality::from_u32(2), RecordingQuality::High);
    }
}
