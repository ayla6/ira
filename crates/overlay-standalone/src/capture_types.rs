use ira_overlay_ipc::{RecordingFormat, RecordingQuality, VideoEncoder};

pub(crate) enum WriterCommand {
    ConfigureRecording(Option<RecordingSettings>),
    ConfigureReplay(Option<RecordingSettings>),
    Frame(CapturedFrame),
    Quit,
}

pub(crate) struct CapturedFrame {
    pub(crate) rgba: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) screenshot: bool,
    pub(crate) recording: bool,
    pub(crate) replay: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct RecordingSettings {
    pub(crate) encoder: VideoEncoder,
    pub(crate) quality: RecordingQuality,
    pub(crate) format: RecordingFormat,
    pub(crate) replay_buffer_enabled: bool,
    pub(crate) replay_buffer_seconds: u32,
}
