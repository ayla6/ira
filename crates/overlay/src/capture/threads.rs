//! Background threads + encoder pipes (readback, encode, replay).
//!
//! Two threads own the slow work off the Vulkan present path: readback maps
//! staged frames to RGBA, encode feeds ffmpeg pipes and screenshots. Both
//! pump `recv_timeout(10ms)` so shutdown joins promptly.

use std::io::Write;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::time::Duration;

use ash::vk;

use super::{EncodeFrame, PendingFrame, StagingBuffer};
use crate::capture_ffmpeg::{
    recording_args, recording_settings, replay_args, replay_manifest_path, resolve_encoder,
    video_path,
};

pub(super) fn readback_thread(
    ready_rx: Receiver<PendingFrame>,
    free_tx: Sender<StagingBuffer>,
    encode_tx: SyncSender<EncodeFrame>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Relaxed) {
        match ready_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(frame) => {
                let rgba = unsafe { super::convert::read_pixels(&frame) };
                let _ = free_tx.send(frame.staging);
                if rgba.is_empty() {
                    continue;
                }
                let _ = encode_tx.try_send(EncodeFrame {
                    rgba,
                    is_screenshot: frame.is_screenshot,
                    width: frame.width,
                    height: frame.height,
                });
            }
            Err(_) => continue,
        }
    }
}

pub(super) fn encode_thread(encode_rx: Receiver<EncodeFrame>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Relaxed) {
        match encode_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(frame) => {
                if super::is_recording()
                    && write_frame_to_pipe(&super::FFMPEG_PIPE, &frame.rgba, "recording")
                {
                    super::FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
                }
                if replay_buffer_active() {
                    let _ = write_frame_to_pipe(
                        &super::REPLAY_FFMPEG_PIPE,
                        &frame.rgba,
                        "replay buffer",
                    );
                }
                if frame.is_screenshot {
                    super::convert::encode_webp(frame.rgba, frame.width, frame.height);
                }
            }
            Err(_) => continue,
        }
    }
}

/// Spawns ffmpeg with piped stdin plus a reaper thread. `what` names the
/// pipe in failure logs. Returns the stdin handle; None keeps the previous
/// state (callers decide whether a missing stdin is fatal).
fn spawn_ffmpeg_with_stdin(args: &[String], what: &str) -> Option<ChildStdin> {
    let mut command = Command::new("ffmpeg");
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(mut child) => match child.stdin.take() {
            Some(stdin) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                Some(stdin)
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("ira-overlay: {what} ffmpeg has no stdin");
                None
            }
        },
        Err(error) => {
            eprintln!("ira-overlay: failed to start {what}: {error}");
            None
        }
    }
}

pub fn toggle_recording() {
    if super::RECORDING.fetch_xor(true, Ordering::Relaxed) {
        super::stop_recording("");
    } else {
        let (extent, has_state) = {
            let s = super::STATE.lock().unwrap();
            (s.as_ref().map(|s| s.extent), s.is_some())
        };
        if !has_state {
            super::RECORDING.store(false, Ordering::Relaxed);
            return;
        }
        let Some(extent) = extent else {
            super::RECORDING.store(false, Ordering::Relaxed);
            return;
        };

        let settings = recording_settings();
        let encoder = resolve_encoder(settings.encoder, settings.format);
        let path = video_path(settings.format);
        if let Some(stdin) = spawn_ffmpeg_with_stdin(
            &recording_args(
                extent.width,
                extent.height,
                encoder,
                settings.quality,
                settings.format,
                &path,
            ),
            "recording",
        ) {
            *super::FFMPEG_PIPE.lock().unwrap() = Some(stdin);
            *super::RECORD_START.lock().unwrap() = Some(std::time::Instant::now());
            super::FRAME_COUNT.store(0, Ordering::Relaxed);
            eprintln!("ira-overlay: recording to {}", path.display());
        } else {
            super::RECORDING.store(false, Ordering::Relaxed);
        }
    }
}

fn write_frame_to_pipe(
    pipe: &std::sync::Mutex<Option<ChildStdin>>,
    rgba: &[u8],
    description: &str,
) -> bool {
    let mut guard = pipe.lock().unwrap();
    let Some(stdin) = guard.as_mut() else {
        return false;
    };
    if let Err(error) = stdin.write_all(rgba) {
        eprintln!("ira-overlay: failed to send frame to {description}: {error}");
        guard.take();
        return false;
    }
    true
}

pub(super) fn replay_buffer_active() -> bool {
    super::REPLAY_FFMPEG_PIPE.lock().unwrap().is_some()
}

pub(super) fn start_replay_buffer(extent: vk::Extent2D) {
    let settings = recording_settings();
    if !settings.replay_buffer_enabled {
        stop_replay_buffer();
        return;
    }
    if replay_buffer_active() {
        return;
    }
    if let Err(error) = crate::capture_ffmpeg::prepare_replay_directory() {
        eprintln!("ira-overlay: failed to prepare replay buffer: {error}");
        return;
    }
    let encoder = resolve_encoder(settings.encoder, ira_overlay_ipc::RecordingFormat::Mp4);
    let output = replay_manifest_path();
    if let Some(stdin) = spawn_ffmpeg_with_stdin(
        &replay_args(
            extent.width,
            extent.height,
            encoder,
            settings.quality,
            settings.replay_buffer_seconds,
            &output,
        ),
        "replay buffer",
    ) {
        *super::REPLAY_FFMPEG_PIPE.lock().unwrap() = Some(stdin);
        eprintln!("ira-overlay: replay buffer writing to {}", output.display());
    }
}

pub(super) fn stop_replay_buffer() {
    super::REPLAY_FFMPEG_PIPE.lock().unwrap().take();
}
