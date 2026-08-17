use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};

use ira_overlay_ipc::{
    replay_buffer_directory, replay_manifest_path as shared_replay_manifest_path,
    replay_window_size, RecordingFormat, RecordingQuality, VideoEncoder, REPLAY_SEGMENT_SECONDS,
};

pub(crate) fn recording_args(
    source_width: u32,
    source_height: u32,
    encoder: VideoEncoder,
    quality: RecordingQuality,
    format: RecordingFormat,
    output: &Path,
) -> Vec<String> {
    let (width, height, fps, bitrate) = quality.params();
    let codec = if format == RecordingFormat::Webm {
        "libvpx-vp9"
    } else {
        encoder.ffmpeg_codec()
    };
    let mut filter = format!(
        "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"
    );
    if encoder == VideoEncoder::Vaapi && format != RecordingFormat::Webm {
        filter.push_str(",format=nv12,hwupload");
    }

    let mut args = vec![
        "-y".to_string(),
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
        muxer(format).to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
    ];
    if encoder == VideoEncoder::Vaapi && format != RecordingFormat::Webm {
        args.splice(
            1..1,
            [
                "-vaapi_device".to_string(),
                vaapi_device().to_string_lossy().into_owned(),
            ],
        );
    }
    if encoder != VideoEncoder::Vaapi || format == RecordingFormat::Webm {
        args.extend(["-pix_fmt".to_string(), "yuv420p".to_string()]);
    }
    if format == RecordingFormat::Mp4 {
        args.extend(["-movflags".to_string(), "+faststart".to_string()]);
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

pub(crate) fn replay_args(
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

fn muxer(format: RecordingFormat) -> &'static str {
    match format {
        RecordingFormat::Mp4 => "mp4",
        RecordingFormat::Mkv => "matroska",
        RecordingFormat::Webm => "webm",
    }
}

fn screenshot_args(width: u32, height: u32, output: &Path) -> Vec<String> {
    vec![
        "-y".to_string(),
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pixel_format".to_string(),
        "rgba".to_string(),
        "-video_size".to_string(),
        format!("{width}x{height}"),
        "-i".to_string(),
        "-".to_string(),
        "-frames:v".to_string(),
        "1".to_string(),
        "-c:v".to_string(),
        "libwebp".to_string(),
        "-lossless".to_string(),
        "1".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        output.to_string_lossy().into_owned(),
    ]
}

pub(crate) fn start_recording(
    width: u32,
    height: u32,
    encoder: VideoEncoder,
    quality: RecordingQuality,
    format: RecordingFormat,
) -> Result<ChildStdin, String> {
    let encoder = resolve_encoder(encoder, format);
    let output = recording_path(format);
    let mut child = Command::new("ffmpeg");
    child
        .args(recording_args(
            width, height, encoder, quality, format, &output,
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = child
        .spawn()
        .map_err(|error| format!("failed to start ffmpeg recording: {error}"))?;
    let stdin = child.stdin.take().ok_or_else(|| {
        let _ = child.kill();
        let _ = child.wait();
        "ffmpeg recording has no stdin".to_string()
    })?;
    eprintln!("ira-overlay-standalone: recording to {}", output.display());
    std::thread::spawn(move || match child.wait() {
        Ok(status) if !status.success() => {
            eprintln!("ira-overlay-standalone: ffmpeg exited with {status}");
        }
        Err(error) => eprintln!("ira-overlay-standalone: failed waiting for ffmpeg: {error}"),
        _ => {}
    });
    Ok(stdin)
}

pub(crate) fn start_replay_buffer(
    width: u32,
    height: u32,
    encoder: VideoEncoder,
    quality: RecordingQuality,
    replay_buffer_seconds: u32,
) -> Result<ChildStdin, String> {
    prepare_replay_directory()?;
    let output = replay_manifest_path();
    let encoder = resolve_encoder(encoder, RecordingFormat::Mp4);
    let mut child = Command::new("ffmpeg")
        .args(replay_args(
            width,
            height,
            encoder,
            quality,
            replay_buffer_seconds,
            &output,
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to start ffmpeg replay buffer: {error}"))?;
    let stdin = child.stdin.take().ok_or_else(|| {
        let _ = child.kill();
        let _ = child.wait();
        "ffmpeg replay buffer has no stdin".to_string()
    })?;
    eprintln!(
        "ira-overlay-standalone: replay buffer writing to {}",
        output.display()
    );
    std::thread::spawn(move || match child.wait() {
        Ok(status) if !status.success() => {
            eprintln!("ira-overlay-standalone: replay ffmpeg exited with {status}");
        }
        Err(error) => {
            eprintln!("ira-overlay-standalone: failed waiting for replay ffmpeg: {error}")
        }
        _ => {}
    });
    Ok(stdin)
}

fn resolve_encoder(requested: VideoEncoder, format: RecordingFormat) -> VideoEncoder {
    if format == RecordingFormat::Webm || requested != VideoEncoder::Auto {
        return requested;
    }
    let encoders = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
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

pub(crate) fn save_screenshot(rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    let output = screenshot_path();
    let mut child = Command::new("ffmpeg")
        .args(screenshot_args(width, height, &output))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to start ffmpeg screenshot: {error}"))?;
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("ffmpeg screenshot has no stdin".to_string());
    };
    if let Err(error) = stdin.write_all(rgba) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("failed to send screenshot to ffmpeg: {error}"));
    }
    drop(stdin);
    let status = child
        .wait()
        .map_err(|error| format!("failed waiting for screenshot ffmpeg: {error}"))?;
    if !status.success() {
        return Err(format!("ffmpeg screenshot exited with {status}"));
    }
    eprintln!(
        "ira-overlay-standalone: saved screenshot to {}",
        output.display()
    );
    Ok(())
}

fn recording_path(format: RecordingFormat) -> PathBuf {
    let dir = data_dir().join("ira").join("videos");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("video_{}.{}", timestamp(), format.extension()))
}

fn replay_manifest_path() -> PathBuf {
    shared_replay_manifest_path(&data_dir())
}

fn prepare_replay_directory() -> Result<PathBuf, String> {
    let dir = replay_buffer_directory(&data_dir());
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create replay directory: {error}"))?;
    let entries = std::fs::read_dir(&dir)
        .map_err(|error| format!("failed to read replay directory: {error}"))?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!("ira-overlay-standalone: failed to inspect replay artifact: {error}");
                continue;
            }
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_segment = name.starts_with("segment-") && name.ends_with(".m4s");
        if name == "session.mpd" || name == "init.mp4" || is_segment {
            if let Err(error) = std::fs::remove_file(entry.path()) {
                eprintln!(
                    "ira-overlay-standalone: failed to remove replay artifact {name}: {error}"
                );
            }
        }
    }
    Ok(dir)
}

fn screenshot_path() -> PathBuf {
    let dir = data_dir().join("ira").join("screenshots");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("screenshot_{}.webp", timestamp()))
}

fn timestamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
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
    fn test_recording_args_use_quality_encoder_and_format() {
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
            args[args.iter().position(|arg| arg == "-c:v").unwrap() + 1],
            "h264_nvenc"
        );
        assert_eq!(
            args[args.iter().position(|arg| arg == "-r").unwrap() + 1],
            "60"
        );
        assert_eq!(
            args[args.iter().position(|arg| arg == "-b:v").unwrap() + 1],
            "8M"
        );
        assert_eq!(args[args.len() - 1], "/tmp/ira capture.mkv");
    }

    #[test]
    fn test_recording_args_use_vp9_for_webm() {
        let args = recording_args(
            1920,
            1080,
            VideoEncoder::Software,
            RecordingQuality::Medium,
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
    }
}
