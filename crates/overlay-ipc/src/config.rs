//! Overlay configuration types — shared between the Ira app config and the overlay.
//!
//! These types are serialized into the Ira config file (serde_json).
//! At launch time, the relevant fields are copied into the `ShmHeader`
//! so the overlay can read them without accessing the config file.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const GAMEPAD_A: u32 = 1 << 0;
pub const GAMEPAD_B: u32 = 1 << 1;
pub const GAMEPAD_X: u32 = 1 << 2;
pub const GAMEPAD_Y: u32 = 1 << 3;
pub const GAMEPAD_L1: u32 = 1 << 4;
pub const GAMEPAD_R1: u32 = 1 << 5;
pub const GAMEPAD_L2: u32 = 1 << 6;
pub const GAMEPAD_R2: u32 = 1 << 7;
pub const GAMEPAD_SELECT: u32 = 1 << 8;
pub const GAMEPAD_START: u32 = 1 << 9;
pub const GAMEPAD_GUIDE: u32 = 1 << 10;
pub const GAMEPAD_L3: u32 = 1 << 11;
pub const GAMEPAD_R3: u32 = 1 << 12;
pub const GAMEPAD_DPAD_UP: u32 = 1 << 13;
pub const GAMEPAD_DPAD_DOWN: u32 = 1 << 14;
pub const GAMEPAD_DPAD_LEFT: u32 = 1 << 15;
pub const GAMEPAD_DPAD_RIGHT: u32 = 1 << 16;

pub const DEFAULT_TOGGLE_GAMEPAD_HOTKEY: u32 = GAMEPAD_GUIDE;
pub const DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY: u32 = GAMEPAD_GUIDE | GAMEPAD_DPAD_DOWN;
pub const DEFAULT_RECORD_GAMEPAD_HOTKEY: u32 = GAMEPAD_GUIDE | GAMEPAD_DPAD_UP;

pub fn parse_gamepad_hotkey(hotkey: &str) -> Option<u32> {
    hotkey
        .split('+')
        .map(str::trim)
        .map(gamepad_button_mask_from_name)
        .try_fold(0, |mask, button| button.map(|button| mask | button))
        .filter(|mask| *mask != 0)
}

pub fn gamepad_button_mask_from_evdev(code: u16) -> Option<u32> {
    match code {
        0x130 => Some(GAMEPAD_A),
        0x131 => Some(GAMEPAD_B),
        0x132 => Some(GAMEPAD_X),
        0x133 => Some(GAMEPAD_Y),
        0x136 => Some(GAMEPAD_L1),
        0x137 => Some(GAMEPAD_R1),
        0x138 => Some(GAMEPAD_L2),
        0x139 => Some(GAMEPAD_R2),
        0x13a => Some(GAMEPAD_SELECT),
        0x13b => Some(GAMEPAD_START),
        0x13c => Some(GAMEPAD_GUIDE),
        0x13d => Some(GAMEPAD_L3),
        0x13e => Some(GAMEPAD_R3),
        0x220 => Some(GAMEPAD_DPAD_UP),
        0x221 => Some(GAMEPAD_DPAD_DOWN),
        0x222 => Some(GAMEPAD_DPAD_LEFT),
        0x223 => Some(GAMEPAD_DPAD_RIGHT),
        _ => None,
    }
}

fn gamepad_button_mask_from_name(name: &str) -> Option<u32> {
    match name {
        "A" => Some(GAMEPAD_A),
        "B" => Some(GAMEPAD_B),
        "X" => Some(GAMEPAD_X),
        "Y" => Some(GAMEPAD_Y),
        "L1" => Some(GAMEPAD_L1),
        "R1" => Some(GAMEPAD_R1),
        "L2" => Some(GAMEPAD_L2),
        "R2" => Some(GAMEPAD_R2),
        "Select" => Some(GAMEPAD_SELECT),
        "Start" => Some(GAMEPAD_START),
        "Guide" => Some(GAMEPAD_GUIDE),
        "L3" => Some(GAMEPAD_L3),
        "R3" => Some(GAMEPAD_R3),
        "DpadUp" => Some(GAMEPAD_DPAD_UP),
        "DpadDown" => Some(GAMEPAD_DPAD_DOWN),
        "DpadLeft" => Some(GAMEPAD_DPAD_LEFT),
        "DpadRight" => Some(GAMEPAD_DPAD_RIGHT),
        _ => None,
    }
}

/// Video encoder backend selection.
/// Auto probes VAAPI → NVENC → Software at recording start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoEncoder {
    #[default]
    Auto,
    Vaapi,
    Nvenc,
    Software,
}

impl VideoEncoder {
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Auto => 0,
            Self::Vaapi => 1,
            Self::Nvenc => 2,
            Self::Software => 3,
        }
    }

    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Vaapi,
            2 => Self::Nvenc,
            3 => Self::Software,
            _ => Self::Auto,
        }
    }

    /// ffmpeg codec name for this encoder.
    pub fn ffmpeg_codec(self) -> &'static str {
        match self {
            Self::Vaapi => "h264_vaapi",
            Self::Nvenc => "h264_nvenc",
            Self::Software => "libx264",
            Self::Auto => "libx264",
        }
    }
}

/// Where the overlay panel appears on screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

impl OverlayPosition {
    pub fn as_u32(self) -> u32 {
        match self {
            Self::TopLeft => 0,
            Self::TopRight => 1,
            Self::BottomLeft => 2,
            Self::BottomRight => 3,
            Self::Center => 4,
        }
    }

    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::TopRight,
            2 => Self::BottomLeft,
            3 => Self::BottomRight,
            4 => Self::Center,
            _ => Self::TopLeft,
        }
    }
}

/// Recording quality presets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingQuality {
    Low,
    #[default]
    Medium,
    High,
    Custom,
}

impl RecordingQuality {
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
            Self::Custom => 3,
        }
    }

    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Low,
            2 => Self::High,
            3 => Self::Custom,
            _ => Self::Medium,
        }
    }

    /// (resolution_w, resolution_h, fps, bitrate_mbps)
    pub fn params(self) -> (u32, u32, u32, u32) {
        match self {
            Self::Low => (1280, 720, 30, 2),
            Self::Medium => (1920, 1080, 30, 5),
            Self::High => (1920, 1080, 60, 8),
            Self::Custom => (1920, 1080, 60, 8),
        }
    }
}

/// Output container format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingFormat {
    #[default]
    Mp4,
    Mkv,
    Webm,
}

impl RecordingFormat {
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Mp4 => 0,
            Self::Mkv => 1,
            Self::Webm => 2,
        }
    }

    pub fn from_u32(value: u32) -> Self {
        match value {
            1 => Self::Mkv,
            2 => Self::Webm,
            _ => Self::Mp4,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
        }
    }
}

/// Global overlay settings, stored in the Ira config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlaySettings {
    /// Global kill switch. When false, the launcher never injects the overlay.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub position: OverlayPosition,
    #[serde(default)]
    pub encoder: VideoEncoder,
    #[serde(default)]
    pub recording_quality: RecordingQuality,
    #[serde(default)]
    pub recording_format: RecordingFormat,
    /// Human-readable hotkey strings (e.g. "Shift+Tab", "F12", "F11").
    /// Converted to X11 keysyms when writing the ShmHeader.
    #[serde(default = "default_toggle_hotkey")]
    pub toggle_hotkey: String,
    #[serde(default = "default_screenshot_hotkey")]
    pub screenshot_hotkey: String,
    #[serde(default = "default_record_hotkey")]
    pub record_hotkey: String,
    #[serde(default)]
    pub toggle_hotkey_gamepad: String,
    #[serde(default)]
    pub screenshot_hotkey_gamepad: String,
    #[serde(default)]
    pub record_hotkey_gamepad: String,
    /// Per-source overlay overrides. Key = source ID ("steam", "ra", "ps3",
    /// "ps4", or a console ID like "nes"). Value = forced enable/disable.
    /// Absent key = follow global `enabled` setting.
    #[serde(default)]
    pub source_overrides: HashMap<String, bool>,
    #[serde(default)]
    pub source_gamescope: HashMap<String, bool>,
    #[serde(default)]
    pub font_family: Option<String>,
}

fn default_toggle_hotkey() -> String {
    "Shift+Tab".to_string()
}
fn default_screenshot_hotkey() -> String {
    "F12".to_string()
}
fn default_record_hotkey() -> String {
    "F11".to_string()
}

impl OverlaySettings {
    /// Resolves the overlay enabled state for a given source.
    /// Returns the per-source override if present, otherwise the global setting.
    pub fn source_enabled(&self, source_id: &str) -> bool {
        self.source_overrides
            .get(source_id)
            .copied()
            .unwrap_or(self.enabled)
    }

    pub fn source_gamescope(&self, source_id: &str) -> bool {
        self.source_gamescope
            .get(source_id)
            .copied()
            .unwrap_or(false)
    }
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            position: OverlayPosition::default(),
            encoder: VideoEncoder::default(),
            recording_quality: RecordingQuality::default(),
            recording_format: RecordingFormat::default(),
            toggle_hotkey: default_toggle_hotkey(),
            screenshot_hotkey: default_screenshot_hotkey(),
            record_hotkey: default_record_hotkey(),
            toggle_hotkey_gamepad: String::new(),
            screenshot_hotkey_gamepad: String::new(),
            record_hotkey_gamepad: String::new(),
            source_overrides: HashMap::new(),
            source_gamescope: HashMap::new(),
            font_family: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_settings_default() {
        let s = OverlaySettings::default();
        assert!(!s.enabled);
        assert_eq!(s.position, OverlayPosition::TopLeft);
        assert_eq!(s.encoder, VideoEncoder::Auto);
        assert_eq!(s.recording_quality, RecordingQuality::Medium);
        assert_eq!(s.recording_format, RecordingFormat::Mp4);
        assert_eq!(s.toggle_hotkey, "Shift+Tab");
    }

    #[test]
    fn test_video_encoder_roundtrip() {
        for v in [
            VideoEncoder::Auto,
            VideoEncoder::Vaapi,
            VideoEncoder::Nvenc,
            VideoEncoder::Software,
        ] {
            assert_eq!(VideoEncoder::from_u32(v.as_u32()), v);
        }
    }

    #[test]
    fn test_video_encoder_ffmpeg_codec() {
        assert_eq!(VideoEncoder::Vaapi.ffmpeg_codec(), "h264_vaapi");
        assert_eq!(VideoEncoder::Nvenc.ffmpeg_codec(), "h264_nvenc");
        assert_eq!(VideoEncoder::Software.ffmpeg_codec(), "libx264");
    }

    #[test]
    fn test_parse_gamepad_hotkey_encodes_chords() {
        assert_eq!(parse_gamepad_hotkey("Guide"), Some(GAMEPAD_GUIDE));
        assert_eq!(
            parse_gamepad_hotkey("Guide+DpadDown"),
            Some(GAMEPAD_GUIDE | GAMEPAD_DPAD_DOWN)
        );
        assert_eq!(parse_gamepad_hotkey("Guide+Unknown"), None);
    }

    #[test]
    fn test_recording_quality_params() {
        let (w, h, fps, bitrate) = RecordingQuality::High.params();
        assert_eq!((w, h, fps, bitrate), (1920, 1080, 60, 8));

        let (w, h, fps, bitrate) = RecordingQuality::Low.params();
        assert_eq!((w, h, fps, bitrate), (1280, 720, 30, 2));
    }

    #[test]
    fn test_overlay_settings_serde_roundtrip() {
        let s = OverlaySettings {
            enabled: true,
            encoder: VideoEncoder::Vaapi,
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: OverlaySettings = serde_json::from_str(&json).unwrap();
        assert!(back.enabled);
        assert_eq!(back.encoder, VideoEncoder::Vaapi);
    }

    #[test]
    fn test_overlay_settings_serde_missing_fields() {
        let json = r#"{}"#;
        let s: OverlaySettings = serde_json::from_str(json).unwrap();
        assert!(!s.enabled);
        assert_eq!(s.toggle_hotkey, "Shift+Tab");
        assert_eq!(s.screenshot_hotkey, "F12");
        assert_eq!(s.record_hotkey, "F11");
    }

    #[test]
    fn test_source_enabled_follows_global_by_default() {
        let s = OverlaySettings {
            enabled: true,
            ..Default::default()
        };
        assert!(s.source_enabled("steam"));
        assert!(s.source_enabled("ps4"));
    }

    #[test]
    fn test_source_enabled_override() {
        let mut s = OverlaySettings {
            enabled: true,
            ..Default::default()
        };
        s.source_overrides.insert("ps3".to_string(), false);
        assert!(s.source_enabled("steam"));
        assert!(!s.source_enabled("ps3"));
        assert!(s.source_enabled("ps4"));
    }
}
