//! Overlay IPC protocol — shared memory layout, config types, and encoder selection.
//! Level 0 crate: no deps on other ira crates. Both the Ira app and overlay crates depend on this.

mod config;
mod hotkey;
mod protocol;
mod shm;

pub use config::{
    OverlayPosition, OverlaySettings, RecordingFormat, RecordingQuality, VideoEncoder,
};
pub use hotkey::{
    parse_hotkey, resolve_defaults, DEFAULT_RECORD_KEYCODE, DEFAULT_RECORD_MODS,
    DEFAULT_SCREENSHOT_KEYCODE, DEFAULT_SCREENSHOT_MODS, DEFAULT_TOGGLE_KEYCODE,
    DEFAULT_TOGGLE_MODS, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER, X11_KEYCODE_OFFSET,
};
pub use protocol::{
    AchievementEntry, InputEventRaw, NotificationEntry, NotificationType, ShmHeader,
    MAX_ACHIEVEMENTS, MAX_NOTIFICATIONS, SHM_MAGIC, SHM_VERSION,
};
pub use shm::{shm_file_path, shm_path, MappedShm};
