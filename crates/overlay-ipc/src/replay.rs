use std::path::{Path, PathBuf};

pub const DEFAULT_REPLAY_BUFFER_SECONDS: u32 = 5 * 60;
pub const MIN_REPLAY_BUFFER_SECONDS: u32 = 60;
pub const MAX_REPLAY_BUFFER_SECONDS: u32 = 60 * 60;
pub const REPLAY_SEGMENT_SECONDS: u32 = 2;

pub fn clamp_replay_buffer_seconds(seconds: u32) -> u32 {
    seconds.clamp(MIN_REPLAY_BUFFER_SECONDS, MAX_REPLAY_BUFFER_SECONDS)
}

pub fn replay_window_size(buffer_seconds: u32, segment_seconds: u32) -> u32 {
    let segment_seconds = segment_seconds.max(1);
    buffer_seconds
        .max(segment_seconds)
        .div_ceil(segment_seconds)
}

pub fn replay_buffer_directory(data_dir: &Path) -> PathBuf {
    data_dir.join("ira").join("replay")
}

pub fn replay_manifest_path(data_dir: &Path) -> PathBuf {
    replay_buffer_directory(data_dir).join("session.mpd")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        clamp_replay_buffer_seconds, replay_buffer_directory, replay_manifest_path,
        replay_window_size, DEFAULT_REPLAY_BUFFER_SECONDS, MAX_REPLAY_BUFFER_SECONDS,
        MIN_REPLAY_BUFFER_SECONDS, REPLAY_SEGMENT_SECONDS,
    };

    #[test]
    fn test_replay_window_size_uses_five_minutes_by_default() {
        assert_eq!(
            replay_window_size(DEFAULT_REPLAY_BUFFER_SECONDS, REPLAY_SEGMENT_SECONDS),
            150
        );
    }

    #[test]
    fn test_replay_window_size_rounds_up_and_handles_invalid_values() {
        assert_eq!(replay_window_size(301, 2), 151);
        assert_eq!(replay_window_size(0, 0), 1);
    }

    #[test]
    fn test_clamp_replay_buffer_seconds_to_ui_bounds() {
        assert_eq!(clamp_replay_buffer_seconds(0), MIN_REPLAY_BUFFER_SECONDS);
        assert_eq!(
            clamp_replay_buffer_seconds(DEFAULT_REPLAY_BUFFER_SECONDS),
            DEFAULT_REPLAY_BUFFER_SECONDS
        );
        assert_eq!(
            clamp_replay_buffer_seconds(u32::MAX),
            MAX_REPLAY_BUFFER_SECONDS
        );
    }

    #[test]
    fn test_replay_paths_use_session_manifest() {
        let base = Path::new("/tmp/ira-data");
        assert_eq!(
            replay_buffer_directory(base),
            Path::new("/tmp/ira-data/ira/replay")
        );
        assert_eq!(
            replay_manifest_path(base),
            Path::new("/tmp/ira-data/ira/replay/session.mpd")
        );
    }
}
