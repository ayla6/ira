use std::path::{Path, PathBuf};
use std::process::Command;

use ira_overlay_ipc::{
    clamp_replay_buffer_seconds, replay_buffer_directory,
    replay_manifest_path as shared_replay_manifest_path, replay_window_size, MappedShm,
    RecordingFormat, RecordingQuality, VideoEncoder, DEFAULT_REPLAY_BUFFER_SECONDS,
    REPLAY_SEGMENT_SECONDS, SHM_MAGIC,
};

#[derive(Clone, Copy)]
pub(super) struct RecordingSettings {
    pub(super) encoder: VideoEncoder,
    pub(super) quality: RecordingQuality,
    pub(super) format: RecordingFormat,
    pub(super) replay_buffer_enabled: bool,
    pub(super) replay_buffer_seconds: u32,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            encoder: VideoEncoder::default(),
            quality: RecordingQuality::default(),
            format: RecordingFormat::default(),
            replay_buffer_enabled: false,
            replay_buffer_seconds: DEFAULT_REPLAY_BUFFER_SECONDS,
        }
    }
}

pub(super) fn recording_settings() -> RecordingSettings {
    let defaults = RecordingSettings::default();
    let Ok(shm_path) = std::env::var("IRA_OVERLAY_SHM") else {
        return defaults;
    };
    let Ok(shm) = MappedShm::open(&shm_path) else {
        eprintln!("ira-overlay: failed to read recording settings from SHM; using defaults");
        return defaults;
    };
    let header = shm.header();
    if header.magic != SHM_MAGIC {
        eprintln!("ira-overlay: invalid SHM header; using default recording settings");
        return defaults;
    }
    RecordingSettings {
        encoder: VideoEncoder::from_u32(header.video_encoder),
        quality: RecordingQuality::from_u32(header.recording_quality),
        format: RecordingFormat::from_u32(header.recording_format),
        replay_buffer_enabled: header.replay_buffer_enabled != 0,
        replay_buffer_seconds: clamp_replay_buffer_seconds(header.replay_buffer_seconds),
    }
}

pub(super) fn recording_args(
    source_width: u32,
    source_height: u32,
    encoder: VideoEncoder,
    quality: RecordingQuality,
    format: RecordingFormat,
    output: &Path,
) -> Vec<String> {
    let (width, height, fps, bitrate) = quality.params();
    let vaapi = encoder == VideoEncoder::Vaapi && format != RecordingFormat::Webm;
    let codec = if format == RecordingFormat::Webm {
        "libvpx-vp9"
    } else {
        encoder.ffmpeg_codec()
    };
    let mut filter = format!(
        "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"
    );
    if vaapi {
        filter.push_str(",format=nv12,hwupload");
    }

    let mut args = vec!["-y".to_string()];
    if vaapi {
        args.extend([
            "-vaapi_device".to_string(),
            vaapi_device().to_string_lossy().into_owned(),
        ]);
    }
    args.extend([
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pixel_format".to_string(),
        "rgba".to_string(),
        "-video_size".to_string(),
        format!("{source_width}x{source_height}"),
        "-use_wallclock_as_timestamps".to_string(),
        "1".to_string(),
        "-i".to_string(),
        "-".to_string(),
        "-vf".to_string(),
        filter,
        "-c:v".to_string(),
        codec.to_string(),
        "-b:v".to_string(),
        format!("{bitrate}M"),
        "-r".to_string(),
        fps.to_string(),
        "-f".to_string(),
        ffmpeg_muxer(format).to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
    ]);
    if !vaapi {
        args.extend(["-pix_fmt".to_string(), "yuv420p".to_string()]);
    }
    if format == RecordingFormat::Mp4 {
        args.extend(["-movflags".to_string(), "+faststart".to_string()]);
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

pub(super) fn replay_args(
    source_width: u32,
    source_height: u32,
    encoder: VideoEncoder,
    quality: RecordingQuality,
    replay_buffer_seconds: u32,
    output: &Path,
) -> Vec<String> {
    let (width, height, fps, bitrate) = quality.params();
    let vaapi = encoder == VideoEncoder::Vaapi;
    let mut filter = format!(
        "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"
    );
    if vaapi {
        filter.push_str(",format=nv12,hwupload");
    }

    let mut args = vec!["-y".to_string()];
    if vaapi {
        args.extend([
            "-vaapi_device".to_string(),
            vaapi_device().to_string_lossy().into_owned(),
        ]);
    }
    args.extend([
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pixel_format".to_string(),
        "rgba".to_string(),
        "-video_size".to_string(),
        format!("{source_width}x{source_height}"),
        "-use_wallclock_as_timestamps".to_string(),
        "1".to_string(),
        "-i".to_string(),
        "-".to_string(),
        "-vf".to_string(),
        filter,
        "-c:v".to_string(),
        encoder.ffmpeg_codec().to_string(),
        "-b:v".to_string(),
        format!("{bitrate}M"),
        "-r".to_string(),
        fps.to_string(),
        "-g".to_string(),
        (fps * REPLAY_SEGMENT_SECONDS).to_string(),
        "-keyint_min".to_string(),
        (fps * REPLAY_SEGMENT_SECONDS).to_string(),
        "-sc_threshold".to_string(),
        "0".to_string(),
        "-f".to_string(),
        "dash".to_string(),
        "-seg_duration".to_string(),
        REPLAY_SEGMENT_SECONDS.to_string(),
        "-window_size".to_string(),
        replay_window_size(replay_buffer_seconds, REPLAY_SEGMENT_SECONDS).to_string(),
        "-extra_window_size".to_string(),
        "0".to_string(),
        "-remove_at_exit".to_string(),
        "0".to_string(),
        "-use_template".to_string(),
        "1".to_string(),
        "-use_timeline".to_string(),
        "1".to_string(),
        "-init_seg_name".to_string(),
        "init.mp4".to_string(),
        "-media_seg_name".to_string(),
        "segment-$Number%05d$.m4s".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
    ]);
    if !vaapi {
        args.extend(["-pix_fmt".to_string(), "yuv420p".to_string()]);
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

pub(super) fn replay_manifest_path() -> PathBuf {
    shared_replay_manifest_path(&data_dir())
}

pub(super) fn prepare_replay_directory() -> Result<PathBuf, String> {
    let dir = replay_buffer_directory(&data_dir());
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create replay directory: {error}"))?;
    let entries = std::fs::read_dir(&dir)
        .map_err(|error| format!("failed to read replay directory: {error}"))?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!("ira-overlay: failed to inspect replay artifact: {error}");
                continue;
            }
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_segment = name.starts_with("segment-") && name.ends_with(".m4s");
        if name == "session.mpd" || name == "init.mp4" || is_segment {
            if let Err(error) = std::fs::remove_file(entry.path()) {
                eprintln!("ira-overlay: failed to remove replay artifact {name}: {error}");
            }
        }
    }
    Ok(dir)
}

fn ffmpeg_muxer(format: RecordingFormat) -> &'static str {
    match format {
        RecordingFormat::Mp4 => "mp4",
        RecordingFormat::Mkv => "matroska",
        RecordingFormat::Webm => "webm",
    }
}

pub(super) fn resolve_encoder(requested: VideoEncoder, format: RecordingFormat) -> VideoEncoder {
    if format == RecordingFormat::Webm || requested != VideoEncoder::Auto {
        return if format == RecordingFormat::Webm {
            VideoEncoder::Software
        } else {
            requested
        };
    }
    let encoders = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            let mut encoders = String::from_utf8_lossy(&output.stdout).into_owned();
            encoders.push_str(&String::from_utf8_lossy(&output.stderr));
            encoders
        })
        .unwrap_or_default();
    if vaapi_device().exists() && encoders.contains("h264_vaapi") {
        VideoEncoder::Vaapi
    } else if encoders.contains("h264_nvenc") {
        VideoEncoder::Nvenc
    } else {
        VideoEncoder::Software
    }
}

fn vaapi_device() -> PathBuf {
    std::fs::read_dir("/dev/dri")
        .ok()
        .and_then(|entries| {
            entries.flatten().map(|entry| entry.path()).find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("renderD"))
            })
        })
        .unwrap_or_else(|| PathBuf::from("/dev/dri/renderD128"))
}

pub(super) fn screenshot_path() -> PathBuf {
    let base = data_dir();
    let dir = base.join("ira").join("screenshots");
    let _ = std::fs::create_dir_all(&dir);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    dir.join(format!("screenshot_{millis}.webp"))
}

pub(super) fn video_path(format: RecordingFormat) -> PathBuf {
    let base = data_dir();
    let dir = base.join("ira").join("videos");
    let _ = std::fs::create_dir_all(&dir);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let extension = format.extension();
    let mut path = dir.join(format!("video_{millis}.{extension}"));
    let mut suffix = 1;
    while path.exists() {
        path = dir.join(format!("video_{millis}_{suffix}.{extension}"));
        suffix += 1;
    }
    path
}

fn data_dir() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".local/share")
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ira_overlay_ipc::{
        RecordingFormat, RecordingQuality, VideoEncoder, DEFAULT_REPLAY_BUFFER_SECONDS,
    };

    use super::{recording_args, replay_args};

    #[test]
    fn test_replay_args_use_dash_window_and_manifest() {
        let args = replay_args(
            1920,
            1080,
            VideoEncoder::Software,
            RecordingQuality::Medium,
            DEFAULT_REPLAY_BUFFER_SECONDS,
            Path::new("/tmp/ira/replay/session.mpd"),
        );

        assert_eq!(
            args[args.iter().position(|arg| arg == "-c:v").unwrap() + 1],
            "libx264"
        );
        assert_eq!(
            args[args.iter().position(|arg| arg == "-f").unwrap() + 1],
            "rawvideo"
        );
        assert!(args.windows(2).any(|pair| pair == ["-f", "dash"]));
        assert!(args.windows(2).any(|pair| pair == ["-seg_duration", "2"]));
        assert!(args.windows(2).any(|pair| pair == ["-window_size", "150"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-extra_window_size", "0"]));
        assert_eq!(args.last().unwrap(), "/tmp/ira/replay/session.mpd");
    }

    #[test]
    fn test_replay_args_use_configured_window() {
        let args = replay_args(
            1920,
            1080,
            VideoEncoder::Software,
            RecordingQuality::Medium,
            10 * 60,
            Path::new("/tmp/ira/replay/session.mpd"),
        );

        assert!(args.windows(2).any(|pair| pair == ["-window_size", "300"]));
    }

    #[test]
    fn test_recording_args_use_encoder_quality_and_mkv() {
        let args = recording_args(
            2560,
            1440,
            VideoEncoder::Nvenc,
            RecordingQuality::High,
            RecordingFormat::Mkv,
            Path::new("/tmp/ira capture.mkv"),
        );

        assert_eq!(
            args[args.iter().position(|arg| arg == "-video_size").unwrap() + 1],
            "2560x1440"
        );
        assert_eq!(
            args[args.iter().position(|arg| arg == "-vf").unwrap() + 1],
            "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2"
        );
        assert_eq!(
            args[args.iter().position(|arg| arg == "-c:v").unwrap() + 1],
            "h264_nvenc"
        );
        assert_eq!(
            args[args.iter().position(|arg| arg == "-b:v").unwrap() + 1],
            "8M"
        );
        assert_eq!(
            args[args.iter().position(|arg| arg == "-r").unwrap() + 1],
            "60"
        );
        assert_eq!(
            args[args.iter().rposition(|arg| arg == "-f").unwrap() + 1],
            "matroska"
        );
        assert_eq!(args[args.len() - 1], "/tmp/ira capture.mkv");
    }

    #[test]
    fn test_recording_args_use_vaapi_device_and_upload_filter() {
        let args = recording_args(
            1920,
            1080,
            VideoEncoder::Vaapi,
            RecordingQuality::Medium,
            RecordingFormat::Mp4,
            Path::new("capture.mp4"),
        );

        assert_eq!(args[1], "-vaapi_device");
        assert!(args[2].starts_with("/dev/dri/renderD"));
        assert_eq!(
            args[args.iter().position(|arg| arg == "-vf").unwrap() + 1],
            "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,format=nv12,hwupload"
        );
        assert_eq!(
            args[args.iter().position(|arg| arg == "-c:v").unwrap() + 1],
            "h264_vaapi"
        );
        assert!(!args.iter().any(|arg| arg == "-pix_fmt"));
    }

    #[test]
    fn test_recording_args_use_vp9_for_webm() {
        let args = recording_args(
            1920,
            1080,
            VideoEncoder::Software,
            RecordingQuality::Low,
            RecordingFormat::Webm,
            Path::new("capture.webm"),
        );

        assert_eq!(
            args[args.iter().position(|arg| arg == "-c:v").unwrap() + 1],
            "libvpx-vp9"
        );
        assert_eq!(
            args[args.iter().rposition(|arg| arg == "-f").unwrap() + 1],
            "webm"
        );
        assert_eq!(
            args[args.iter().position(|arg| arg == "-b:v").unwrap() + 1],
            "2M"
        );
        assert!(args.windows(2).any(|pair| pair == ["-pix_fmt", "yuv420p"]));
    }
}
