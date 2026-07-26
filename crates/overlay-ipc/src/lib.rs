//! Overlay IPC protocol — shared memory layout, config types, and encoder selection.
//! Level 0 crate: no deps on other ira crates. Both the Ira app and overlay crates depend on this.

mod config;
mod protocol;
mod shm;

pub use config::{
    OverlayPosition, OverlaySettings, RecordingFormat, RecordingQuality, VideoEncoder,
};
pub use protocol::{
    AchievementEntry, InputEventRaw, NotificationEntry, NotificationType, ShmHeader,
    MAX_ACHIEVEMENTS, MAX_NOTIFICATIONS, SHM_MAGIC, SHM_VERSION,
};
pub use shm::{shm_file_path, shm_path, MappedShm};
