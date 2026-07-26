//! Overlay configuration types — shared between the Ira app config and the overlay.
//!
//! These types are serialized into the Ira config file (serde_json).
//! At launch time, the relevant fields are copied into the `ShmHeader`
//! so the overlay can read them without accessing the config file.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Per-source overlay overrides. Key = source ID ("steam", "ra", "ps3",
    /// "ps4", or a console ID like "nes"). Value = forced enable/disable.
    /// Absent key = follow global `enabled` setting.
    #[serde(default)]
    pub source_overrides: HashMap<String, bool>,
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
            source_overrides: HashMap::new(),
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
        let s = OverlaySettings { enabled: true, ..Default::default() };
        assert!(s.source_enabled("steam"));
        assert!(s.source_enabled("ps4"));
    }

    #[test]
    fn test_source_enabled_override() {
        let mut s = OverlaySettings { enabled: true, ..Default::default() };
        s.source_overrides.insert("ps3".to_string(), false);
        assert!(s.source_enabled("steam"));
        assert!(!s.source_enabled("ps3"));
        assert!(s.source_enabled("ps4"));
    }
}
