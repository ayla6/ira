use std::os::fd::OwnedFd;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ashpd::desktop::{
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    PersistMode,
};
use ira_overlay_ipc::{
    clamp_replay_buffer_seconds, MappedShm, RecordingFormat, RecordingQuality, VideoEncoder,
};
use pipewire as pw;
use pw::{properties::properties, spa};

use super::capture_frame;
use super::capture_types::WriterCommand;
use super::capture_writer;

pub(crate) use super::capture_types::RecordingSettings;

impl RecordingSettings {
    pub(crate) fn from_shm(shm: &MappedShm) -> Self {
        let header = shm.header();
        Self {
            encoder: VideoEncoder::from_u32(header.video_encoder),
            quality: RecordingQuality::from_u32(header.recording_quality),
            format: RecordingFormat::from_u32(header.recording_format),
            replay_buffer_enabled: header.replay_buffer_enabled != 0,
            replay_buffer_seconds: clamp_replay_buffer_seconds(header.replay_buffer_seconds),
        }
    }
}

pub(crate) struct CaptureController {
    sender: mpsc::SyncSender<CaptureCommand>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureController {
    pub(crate) fn new(settings: RecordingSettings) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(16);
        let thread = thread::Builder::new()
            .name("ira-capture-portal".to_string())
            .spawn(move || coordinator(receiver, settings))
            .map_err(|error| format!("failed to start capture worker: {error}"))?;
        Ok(Self {
            sender,
            thread: Some(thread),
        })
    }

    pub(crate) fn request_screenshot(&self) {
        self.send(CaptureCommand::Screenshot);
    }

    pub(crate) fn toggle_recording(&self, settings: RecordingSettings) {
        self.send(CaptureCommand::ToggleRecording(settings));
    }

    pub(crate) fn set_direct_capture_ready(&self, ready: bool) {
        self.send(CaptureCommand::SetDirectCaptureReady(ready));
    }

    fn send(&self, command: CaptureCommand) {
        match self.sender.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                eprintln!("ira-overlay-standalone: capture request queue is full")
            }
            Err(TrySendError::Disconnected(_)) => {
                eprintln!("ira-overlay-standalone: capture worker is unavailable")
            }
        }
    }
}

impl Drop for CaptureController {
    fn drop(&mut self) {
        let _ = self.sender.send(CaptureCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum CaptureCommand {
    Screenshot,
    ToggleRecording(RecordingSettings),
    SetDirectCaptureReady(bool),
    Shutdown,
}

enum PipeCommand {
    Screenshot,
    Recording(bool),
    Replay(bool),
    Quit,
}

struct PortalCapture {
    session: ashpd::desktop::Session<Screencast>,
    pipewire: PipeWireCapture,
}

struct PipeWireCapture {
    sender: pw::channel::Sender<PipeCommand>,
    thread: Option<JoinHandle<()>>,
}

// Vulkan swapchains are often created after a launcher or shader warm-up.
const DIRECT_CAPTURE_GRACE: Duration = Duration::from_secs(10);

fn coordinator(receiver: Receiver<CaptureCommand>, settings: RecordingSettings) {
    let (writer_sender, writer_receiver) = mpsc::sync_channel(4);
    let writer = thread::Builder::new()
        .name("ira-capture-ffmpeg".to_string())
        .spawn(move || capture_writer::writer_loop(writer_receiver));
    let Ok(writer) = writer else {
        eprintln!("ira-overlay-standalone: failed to start capture writer");
        return;
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("ira-overlay-standalone: failed to start portal runtime: {error}");
            let _ = writer_sender.send(WriterCommand::Quit);
            let _ = writer.join();
            return;
        }
    };

    let mut direct_capture_ready = false;
    let mut replay_deadline = settings
        .replay_buffer_enabled
        .then(|| Instant::now() + DIRECT_CAPTURE_GRACE);
    let mut replaying = false;
    let mut portal_capture = None;
    let mut recording = false;
    loop {
        let timeout = replay_deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(60));
        let command = match receiver.recv_timeout(timeout) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                replay_deadline = None;
                if !direct_capture_ready && portal_capture.is_none() {
                    portal_capture = open_portal_capture(&runtime, &writer_sender);
                    if let Some(capture) = portal_capture.as_ref() {
                        start_replay(&writer_sender, capture, settings, &mut replaying);
                    }
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        match command {
            CaptureCommand::Shutdown => break,
            CaptureCommand::SetDirectCaptureReady(ready) => {
                if ready == direct_capture_ready {
                    continue;
                }
                direct_capture_ready = ready;
                if ready {
                    close_portal_capture(&runtime, &mut portal_capture);
                    send_writer_command(&writer_sender, WriterCommand::ConfigureRecording(None));
                    send_writer_command(&writer_sender, WriterCommand::ConfigureReplay(None));
                    recording = false;
                    replaying = false;
                    replay_deadline = None;
                } else if settings.replay_buffer_enabled {
                    replay_deadline = Some(Instant::now() + DIRECT_CAPTURE_GRACE);
                }
            }
            CaptureCommand::Screenshot | CaptureCommand::ToggleRecording(_)
                if direct_capture_ready => {}
            CaptureCommand::Screenshot | CaptureCommand::ToggleRecording(_) => {
                if portal_capture.is_none() {
                    portal_capture = open_portal_capture(&runtime, &writer_sender);
                }
                let Some(capture) = portal_capture.as_ref() else {
                    recording = false;
                    continue;
                };
                replay_deadline = None;
                start_replay(&writer_sender, capture, settings, &mut replaying);
                match command {
                    CaptureCommand::Screenshot => {
                        send_pipe_command(&capture.pipewire, PipeCommand::Screenshot)
                    }
                    CaptureCommand::ToggleRecording(settings) => {
                        recording = !recording;
                        let config = recording.then_some(settings);
                        send_writer_command(
                            &writer_sender,
                            WriterCommand::ConfigureRecording(config),
                        );
                        send_pipe_command(&capture.pipewire, PipeCommand::Recording(recording));
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    close_portal_capture(&runtime, &mut portal_capture);
    send_writer_command(&writer_sender, WriterCommand::Quit);
    let _ = writer.join();
}

fn open_portal_capture(
    runtime: &tokio::runtime::Runtime,
    writer_sender: &SyncSender<WriterCommand>,
) -> Option<PortalCapture> {
    match open_capture(runtime, writer_sender.clone()) {
        Ok(capture) => Some(capture),
        Err(error) => {
            eprintln!("ira-overlay-standalone: screen capture unavailable: {error}");
            None
        }
    }
}

fn close_portal_capture(
    runtime: &tokio::runtime::Runtime,
    portal_capture: &mut Option<PortalCapture>,
) {
    let Some(capture) = portal_capture.take() else {
        return;
    };
    capture.pipewire.shutdown();
    if let Err(error) = runtime.block_on(capture.session.close()) {
        eprintln!("ira-overlay-standalone: failed to close portal session: {error}");
    }
}

fn start_replay(
    writer_sender: &SyncSender<WriterCommand>,
    capture: &PortalCapture,
    settings: RecordingSettings,
    replaying: &mut bool,
) {
    if !settings.replay_buffer_enabled || *replaying {
        return;
    }
    send_writer_command(
        writer_sender,
        WriterCommand::ConfigureReplay(Some(settings)),
    );
    send_pipe_command(&capture.pipewire, PipeCommand::Replay(true));
    *replaying = true;
}

fn send_pipe_command(capture: &PipeWireCapture, command: PipeCommand) {
    if capture.sender.send(command).is_err() {
        eprintln!("ira-overlay-standalone: PipeWire capture loop is unavailable");
    }
}

fn send_writer_command(sender: &SyncSender<WriterCommand>, command: WriterCommand) {
    if sender.send(command).is_err() {
        eprintln!("ira-overlay-standalone: FFmpeg writer is unavailable");
    }
}

fn open_capture(
    runtime: &tokio::runtime::Runtime,
    writer: SyncSender<WriterCommand>,
) -> Result<PortalCapture, String> {
    let (session, node, fd) = runtime.block_on(open_portal())?;
    let pipewire = match start_pipewire(node, fd, writer) {
        Ok(pipewire) => pipewire,
        Err(error) => {
            let _ = runtime.block_on(session.close());
            return Err(error);
        }
    };
    Ok(PortalCapture { session, pipewire })
}

async fn open_portal() -> Result<(ashpd::desktop::Session<Screencast>, u32, OwnedFd), String> {
    let proxy = Screencast::new()
        .await
        .map_err(|error| format!("failed to create ScreenCast portal: {error}"))?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|error| format!("failed to create portal session: {error}"))?;
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Hidden)
                .set_sources(SourceType::Monitor | SourceType::Window)
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(|error| format!("failed to select portal capture source: {error}"))?
        .response()
        .map_err(|error| format!("portal source selection was not approved: {error}"))?;
    let streams = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|error| format!("failed to start portal capture: {error}"))?
        .response()
        .map_err(|error| format!("portal capture was not approved: {error}"))?;
    let stream = streams
        .streams()
        .first()
        .ok_or_else(|| "portal returned no capture stream".to_string())?;
    let node = stream.pipe_wire_node_id();
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(|error| format!("failed to open PipeWire remote: {error}"))?;
    Ok((session, node, fd))
}

fn start_pipewire(
    node: u32,
    fd: OwnedFd,
    writer: SyncSender<WriterCommand>,
) -> Result<PipeWireCapture, String> {
    let (sender, receiver) = pw::channel::channel();
    let (ready_sender, ready_receiver) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("ira-capture-pipewire".to_string())
        .spawn(move || run_pipewire(node, fd, writer, receiver, ready_sender))
        .map_err(|error| format!("failed to start PipeWire thread: {error}"))?;
    match ready_receiver.recv() {
        Ok(Ok(())) => Ok(PipeWireCapture {
            sender,
            thread: Some(thread),
        }),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(error) => {
            let _ = thread.join();
            Err(format!("PipeWire thread stopped during startup: {error}"))
        }
    }
}

impl PipeWireCapture {
    fn shutdown(mut self) {
        let _ = self.sender.send(PipeCommand::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_pipewire(
    node: u32,
    fd: OwnedFd,
    writer: SyncSender<WriterCommand>,
    receiver: pw::channel::Receiver<PipeCommand>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    let result = run_pipewire_loop(node, fd, writer, receiver, &ready);
    if let Err(error) = result {
        let _ = ready.send(Err(error));
    }
}

fn run_pipewire_loop(
    node: u32,
    fd: OwnedFd,
    writer: SyncSender<WriterCommand>,
    receiver: pw::channel::Receiver<PipeCommand>,
    ready: &mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|error| format!("failed to create PipeWire main loop: {error}"))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|error| format!("failed to create PipeWire context: {error}"))?;
    let core = context
        .connect_fd_rc(fd, None)
        .map_err(|error| format!("failed to connect PipeWire remote: {error}"))?;
    let stream = pw::stream::StreamRc::new(
        core,
        "ira-screen-capture",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|error| format!("failed to create PipeWire stream: {error}"))?;
    let state = std::rc::Rc::new(std::cell::RefCell::new(capture_frame::PipeState {
        format: Default::default(),
        screenshot: false,
        recording: false,
        replay: false,
        writer,
    }));
    let listener = stream
        .add_local_listener_with_user_data(state.clone())
        .state_changed(|_, _, old, new| {
            eprintln!("ira-overlay-standalone: PipeWire state {old:?} -> {new:?}");
        })
        .param_changed(|_, state, id, param| capture_frame::update_format(state, id, param))
        .process(|stream, state| capture_frame::process_frame(stream, state))
        .register()
        .map_err(|error| format!("failed to register PipeWire listener: {error}"))?;
    let mainloop_for_commands = mainloop.clone();
    let state_for_commands = state.clone();
    let _commands = receiver.attach(mainloop.loop_(), move |command| {
        let mut state = state_for_commands.borrow_mut();
        match command {
            PipeCommand::Screenshot => state.screenshot = true,
            PipeCommand::Recording(recording) => state.recording = recording,
            PipeCommand::Replay(replay) => state.replay = replay,
            PipeCommand::Quit => mainloop_for_commands.quit(),
        }
    });
    let bytes = build_video_param()?;
    let pod = spa::pod::Pod::from_bytes(&bytes)
        .ok_or_else(|| "failed to create PipeWire format pod".to_string())?;
    let mut params = [pod];
    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|error| format!("failed to connect PipeWire stream: {error}"))?;
    ready
        .send(Ok(()))
        .map_err(|error| format!("failed to report PipeWire startup: {error}"))?;
    mainloop.run();
    drop(listener);
    Ok(())
}

fn build_video_param() -> Result<Vec<u8>, String> {
    let object = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            spa::param::format::MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            spa::param::format::MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::RGBx,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::BGRA
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction { num: 60, denom: 1 },
            spa::utils::Fraction { num: 1, denom: 1 },
            spa::utils::Fraction { num: 120, denom: 1 }
        ),
    );
    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .map_err(|error| format!("failed to serialize PipeWire format: {error}"))?
    .0
    .into_inner();
    Ok(bytes)
}
