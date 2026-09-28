//! Overlay IPC protocol — shared memory layout, config types, and encoder selection.
//! Level 0 crate: no deps on other ira crates. Both the Ira app and overlay crates depend on this.

mod canvas;
mod commands;
mod config;
mod geometry;
mod hotkey;
mod map;
mod protocol;
mod replay;
mod shm;

pub use canvas::{
    canvas_shm_file_path, canvas_shm_path, CanvasHeader, CanvasShm, CANVAS_BPP, CANVAS_MAGIC,
    CANVAS_MAX_BYTES, CANVAS_MAX_H, CANVAS_MAX_W, CANVAS_SHM_SIZE, CANVAS_VERSION, CURSOR_MAX,
    CURSOR_MAX_BYTES,
};
pub use geometry::{canvas_panel_rect, fit_panel_size};
pub use commands::{
    command_from_raw, ACT_EXIT_GAME, ACT_HIDE_OVERLAY, ACT_SCREENSHOT, ACT_TOGGLE_RECORD,
    CMD_ACTIVATE, CMD_BACK, CMD_EXIT_GAME, CMD_HIDE, CMD_MOUSE_DOWN, CMD_MOUSE_MOVE, CMD_MOUSE_UP,
    CMD_NAV_DOWN, CMD_NAV_LEFT, CMD_NAV_RIGHT, CMD_NAV_UP, CMD_SCROLL, CMD_SHOW, CanvasAction,
    CanvasCommand, MAX_CANVAS_ACTIONS, MAX_CANVAS_COMMANDS,
};
pub use config::{
    gamepad_button_mask_from_evdev, parse_gamepad_hotkey, OverlayPosition, OverlaySettings,
    RecordingFormat, RecordingQuality, VideoEncoder, DEFAULT_RECORD_GAMEPAD_HOTKEY,
    DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY, DEFAULT_TOGGLE_GAMEPAD_HOTKEY, GAMEPAD_A, GAMEPAD_B,
    GAMEPAD_DPAD_DOWN, GAMEPAD_DPAD_LEFT, GAMEPAD_DPAD_RIGHT, GAMEPAD_DPAD_UP, GAMEPAD_GUIDE,
    GAMEPAD_L1, GAMEPAD_L2, GAMEPAD_L3, GAMEPAD_R1, GAMEPAD_R2, GAMEPAD_R3, GAMEPAD_SELECT,
    GAMEPAD_START, GAMEPAD_X, GAMEPAD_Y,
};
pub use hotkey::{
    parse_hotkey, resolve_defaults, KbdChord, DEFAULT_RECORD_KEYCODE, DEFAULT_RECORD_MODS,
    DEFAULT_SCREENSHOT_KEYCODE, DEFAULT_SCREENSHOT_MODS, DEFAULT_TOGGLE_KEYCODE,
    DEFAULT_TOGGLE_MODS, EVDEV_LEFTALT, EVDEV_LEFTCTRL, EVDEV_LEFTMETA, EVDEV_LEFTSHIFT,
    EVDEV_RIGHTALT, EVDEV_RIGHTCTRL, EVDEV_RIGHTSHIFT, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER,
    X11_KEYCODE_OFFSET,
};
pub use protocol::{
    AchievementEntry, InputEventRaw, NotificationEntry, NotificationType, ShmHeader,
    DAEMON_TOGGLE_ENV, MAX_ACHIEVEMENTS, MAX_NOTIFICATIONS, OVERLAY_VISIBLE_OFFSET, SHM_MAGIC,
    SHM_VERSION,
};
pub use replay::{
    clamp_replay_buffer_seconds, replay_buffer_directory, replay_manifest_path, replay_window_size,
    DEFAULT_REPLAY_BUFFER_SECONDS, MAX_REPLAY_BUFFER_SECONDS, MIN_REPLAY_BUFFER_SECONDS,
    REPLAY_SEGMENT_SECONDS,
};
pub use shm::{shm_file_path, shm_path, MappedShm};
