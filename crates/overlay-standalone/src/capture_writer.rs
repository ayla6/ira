use std::io::Write;
use std::sync::mpsc::Receiver;

use crate::capture_types::{CapturedFrame, RecordingSettings, WriterCommand};
use crate::ffmpeg;

pub(crate) fn writer_loop(receiver: Receiver<WriterCommand>) {
    let mut recording_settings = None;
    let mut replay_settings = None;
    let mut recording = None;
    let mut replay = None;
    while let Ok(command) = receiver.recv() {
        match command {
            WriterCommand::ConfigureRecording(new_settings) => {
                recording = None;
                recording_settings = new_settings;
            }
            WriterCommand::ConfigureReplay(new_settings) => {
                replay = None;
                replay_settings = new_settings;
            }
            WriterCommand::Frame(frame) => {
                write_frame(
                    &frame,
                    recording_settings,
                    replay_settings,
                    &mut recording,
                    &mut replay,
                );
            }
            WriterCommand::Quit => break,
        }
    }
}

fn write_frame(
    frame: &CapturedFrame,
    recording_settings: Option<RecordingSettings>,
    replay_settings: Option<RecordingSettings>,
    recording: &mut Option<std::process::ChildStdin>,
    replay: &mut Option<std::process::ChildStdin>,
) {
    if frame.screenshot {
        if let Err(error) = ffmpeg::save_screenshot(&frame.rgba, frame.width, frame.height) {
            eprintln!("ira-overlay-standalone: screenshot failed: {error}");
        }
    }
    if !frame.recording {
        if frame.replay {
            write_replay_frame(frame, replay_settings, replay);
        }
        return;
    }
    write_recording_frame(frame, recording_settings, recording);
    if frame.replay {
        write_replay_frame(frame, replay_settings, replay);
    }
}

fn write_recording_frame(
    frame: &CapturedFrame,
    settings: Option<RecordingSettings>,
    recording: &mut Option<std::process::ChildStdin>,
) {
    let Some(settings) = settings else {
        eprintln!("ira-overlay-standalone: recording frame arrived without settings");
        return;
    };
    if recording.is_none() {
        *recording = match ffmpeg::start_recording(
            frame.width,
            frame.height,
            settings.encoder,
            settings.quality,
            settings.format,
        ) {
            Ok(stdin) => Some(stdin),
            Err(error) => {
                eprintln!("ira-overlay-standalone: recording failed: {error}");
                None
            }
        };
    }
    write_pipe(recording, &frame.rgba, "recording");
}

fn write_replay_frame(
    frame: &CapturedFrame,
    settings: Option<RecordingSettings>,
    replay: &mut Option<std::process::ChildStdin>,
) {
    let Some(settings) = settings else {
        eprintln!("ira-overlay-standalone: replay frame arrived without settings");
        return;
    };
    if replay.is_none() {
        *replay = match ffmpeg::start_replay_buffer(
            frame.width,
            frame.height,
            settings.encoder,
            settings.quality,
            settings.replay_buffer_seconds,
        ) {
            Ok(stdin) => Some(stdin),
            Err(error) => {
                eprintln!("ira-overlay-standalone: replay buffer failed: {error}");
                None
            }
        };
    }
    write_pipe(replay, &frame.rgba, "replay buffer");
}

fn write_pipe(pipe: &mut Option<std::process::ChildStdin>, rgba: &[u8], description: &str) {
    if let Some(stdin) = pipe.as_mut() {
        if let Err(error) = stdin.write_all(rgba) {
            eprintln!("ira-overlay-standalone: failed to send {description} frame: {error}");
            *pipe = None;
        }
    }
}
