//! Shared memory protocol — fixed-size C structs for cross-process IPC.
//!
//! Layout of the shared memory region:
//! ```text
//! [0 .. A]                          ShmHeader
//! [A .. A + MAX_ACHIEVEMENTS * B]    AchievementEntry array
//! [A + MAX_ACHIEVEMENTS * B ..]      NotificationEntry ring buffer (MAX_NOTIFICATIONS)
//! ```
//! where A = sizeof(ShmHeader), B = sizeof(AchievementEntry).
//!
//! Single-producer (Ira app) single-consumer (overlay), lock-free.
//! The Ira app writes achievement data and pushes notifications;
//! the overlay polls each frame.

use std::sync::atomic::AtomicU32;

pub const SHM_MAGIC: u32 = 0x4952414F;
pub const SHM_VERSION: u32 = 1;
pub const MAX_ACHIEVEMENTS: usize = 1024;
pub const MAX_NOTIFICATIONS: usize = 16;

/// Total size of the shared memory region.
pub const SHM_SIZE: usize = std::mem::size_of::<ShmHeader>()
    + MAX_ACHIEVEMENTS * std::mem::size_of::<AchievementEntry>()
    + MAX_NOTIFICATIONS * std::mem::size_of::<NotificationEntry>();

/// Fixed-size header at the start of the shared memory region.
#[repr(C)]
pub struct ShmHeader {
    pub magic: u32,
    pub version: u32,
    pub game_db_id: i64,
    pub game_name: [u8; 256],
    pub game_kind: [u8; 32],
    pub cover_image_path: [u8; 512],
    pub total_achievements: u32,
    pub unlocked_achievements: u32,
    pub playtime_seconds: u64,
    /// Ring-buffer write index — incremented by Ira app after writing a notification.
    /// The overlay tracks its own read index and consumes up to this value.
    pub notification_write_index: AtomicU32,
    /// Overlay position (value of OverlayPosition as u32).
    pub overlay_position: u32,
    /// Video encoder (value of VideoEncoder as u32).
    pub video_encoder: u32,
    /// Recording quality (value of RecordingQuality as u32).
    pub recording_quality: u32,
    /// Evdev keycode for toggle hotkey (0 = use default Shift+Tab).
    pub toggle_keysym: u32,
    /// Modifier mask for toggle hotkey (Shift=0x01, Ctrl=0x04, Alt=0x08, Super=0x40).
    pub toggle_mods: u32,
    /// Evdev keycode for screenshot hotkey (0 = use default F12).
    pub screenshot_keysym: u32,
    /// Modifier mask for screenshot hotkey.
    pub screenshot_mods: u32,
    /// Evdev keycode for record hotkey (0 = use default F11).
    pub record_keysym: u32,
    /// Modifier mask for record hotkey.
    pub record_mods: u32,
    /// Cross-process visibility flag for the standalone overlay.
    /// Written by the shim (on hotkey), read by the standalone overlay.
    pub overlay_visible: AtomicU32,
    /// Timestamp (ms since epoch, lower 32 bits) of the last toggle.
    /// Used for cross-process debounce — prevents multiple child processes
    /// from toggling simultaneously on the same key event.
    pub last_toggle_ms: AtomicU32,
    pub padding: [u8; 44],
}

/// One achievement entry in the shared memory array.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AchievementEntry {
    pub earned_time: u64,
    pub global_percent: f32,
    pub display_name: [u8; 128],
    pub description: [u8; 256],
    pub icon_path: [u8; 512],
    pub icon_gray_path: [u8; 512],
    pub earned: u8,
    /// b=bronze, s=silver, g=gold, p=platinum, 0=unknown.
    pub trophy_type: u8,
    pub hidden: u8,
    pub _pad: [u8; 5],
}

/// Ring-buffer entry for achievement unlock / progress notifications.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NotificationEntry {
    pub notification_type: u32,
    pub achievement_index: u32,
    pub timestamp: u64,
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u32)]
pub enum NotificationType {
    AchievementUnlocked = 0,
    AchievementProgress = 1,
}

/// Raw input event — used by the LD_PRELOAD shim to pass captured input
/// to the Vulkan layer via the exported C API (`ira_overlay_poll_events`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct InputEventRaw {
    /// 0=move, 1=button_down, 2=button_up, 3=key_press, 4=key_release
    pub event_type: u32,
    pub x: i32,
    pub y: i32,
    /// 0=left, 1=right, 2=middle (for button events).
    pub button: u32,
    /// X11 keycode (for key events).
    pub keycode: u32,
}

/// Offsets into the shared memory region for each section.
pub const fn header_offset() -> usize {
    0
}

pub const fn achievements_offset() -> usize {
    std::mem::size_of::<ShmHeader>()
}

pub const fn notifications_offset() -> usize {
    std::mem::size_of::<ShmHeader>() + MAX_ACHIEVEMENTS * std::mem::size_of::<AchievementEntry>()
}

#[cfg(test)]
mod tests {
    use super::*;

    const _: () = {
        assert!(SHM_SIZE < 2 * 1024 * 1024);
        assert!(SHM_SIZE > 1024);
    };

    #[test]
    fn test_shm_size_is_reasonable() {
        // Compile-time assertions are in the const block above.
        // This test verifies the size at link time as well.
        let size = SHM_SIZE;
        assert!(size > 1024 && size < 2 * 1024 * 1024);
    }

    #[test]
    fn test_achievement_entry_size() {
        let size = std::mem::size_of::<AchievementEntry>();
        assert!(size > 1400 && size < 1500, "got {size}");
    }

    #[test]
    fn test_offsets_are_consistent() {
        let h = header_offset();
        let a = achievements_offset();
        let n = notifications_offset();
        assert_eq!(h, 0);
        assert!(a > 0);
        assert!(n > a);
        assert_eq!(
            n,
            a + MAX_ACHIEVEMENTS * std::mem::size_of::<AchievementEntry>()
        );
    }

    #[test]
    fn test_input_event_raw_size() {
        let size = std::mem::size_of::<InputEventRaw>();
        assert_eq!(size, 20);
    }
}
