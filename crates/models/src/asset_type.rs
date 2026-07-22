use serde::{Deserialize, Serialize};

/// The five image asset types used throughout the app.
/// Replaces raw string literals ("icon", "hero", "grid", "header", "logo")
/// that were scattered across 9+ files.
///
/// `as_str()` returns the asset type identifier used in match arms and API calls.
/// `file_base()` returns the on-disk filename stem (note: `Grid` → `"vertical"`).
/// `display_name()` returns the human-readable label for UI.
/// `thumb_dims()` returns `(max_w, max_h)` for thumbnail generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetType {
    Icon,
    Hero,
    Grid,
    Header,
    Logo,
}

impl AssetType {
    pub fn as_str(self) -> &'static str {
        match self {
            AssetType::Icon => "icon",
            AssetType::Hero => "hero",
            AssetType::Grid => "grid",
            AssetType::Header => "header",
            AssetType::Logo => "logo",
        }
    }

    /// The on-disk filename stem. `Grid` assets are stored as `vertical.webp`.
    pub fn file_base(self) -> &'static str {
        match self {
            AssetType::Grid => "vertical",
            _ => self.as_str(),
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            AssetType::Icon => "Icon",
            AssetType::Hero => "Hero",
            AssetType::Grid => "Capsule",
            AssetType::Header => "Header",
            AssetType::Logo => "Logo",
        }
    }

    /// `(max_width, max_height)` for thumbnail generation.
    pub fn thumb_dims(self) -> (u32, u32) {
        match self {
            AssetType::Icon => (32, 32),
            AssetType::Hero => (1920, 620),
            AssetType::Grid => (300, 450),
            AssetType::Header => (460, 215),
            AssetType::Logo => (620, 620),
        }
    }

    pub fn from_string(s: &str) -> Option<Self> {
        match s {
            "icon" => Some(AssetType::Icon),
            "hero" => Some(AssetType::Hero),
            "grid" => Some(AssetType::Grid),
            "header" => Some(AssetType::Header),
            "logo" => Some(AssetType::Logo),
            _ => None,
        }
    }

    pub fn sgdb_dimensions(self) -> &'static [&'static str] {
        match self {
            AssetType::Grid => &["600x900"],
            AssetType::Header => &["460x215", "920x430"],
            _ => &[],
        }
    }

    pub fn all() -> &'static [AssetType] {
        &[
            AssetType::Icon,
            AssetType::Hero,
            AssetType::Grid,
            AssetType::Header,
            AssetType::Logo,
        ]
    }
}

impl std::fmt::Display for AssetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for AssetType {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AssetType {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        AssetType::from_string(&s).ok_or_else(|| serde::de::Error::custom(format!("unknown asset type: {s}")))
    }
}

/// Logo overlay position on the hero image.
/// Replaces raw string literals ("top-left", "bottom-center", etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoPosition {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl LogoPosition {
    pub fn as_str(self) -> &'static str {
        match self {
            LogoPosition::TopLeft => "top-left",
            LogoPosition::TopCenter => "top-center",
            LogoPosition::TopRight => "top-right",
            LogoPosition::CenterLeft => "center-left",
            LogoPosition::Center => "center",
            LogoPosition::CenterRight => "center-right",
            LogoPosition::BottomLeft => "bottom-left",
            LogoPosition::BottomCenter => "bottom-center",
            LogoPosition::BottomRight => "bottom-right",
        }
    }

    pub fn from_string(s: &str) -> Self {
        match s {
            "top-left" => LogoPosition::TopLeft,
            "top-center" => LogoPosition::TopCenter,
            "top-right" => LogoPosition::TopRight,
            "center-left" => LogoPosition::CenterLeft,
            "center" => LogoPosition::Center,
            "center-right" => LogoPosition::CenterRight,
            "bottom-center" => LogoPosition::BottomCenter,
            "bottom-right" => LogoPosition::BottomRight,
            _ => LogoPosition::BottomLeft,
        }
    }

    pub fn all() -> &'static [LogoPosition] {
        &[
            LogoPosition::TopLeft,
            LogoPosition::TopCenter,
            LogoPosition::TopRight,
            LogoPosition::CenterLeft,
            LogoPosition::Center,
            LogoPosition::CenterRight,
            LogoPosition::BottomLeft,
            LogoPosition::BottomCenter,
            LogoPosition::BottomRight,
        ]
    }

    pub const DEFAULT: LogoPosition = LogoPosition::BottomLeft;
}

impl std::fmt::Display for LogoPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Default for LogoPosition {
    fn default() -> Self {
        LogoPosition::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_type_roundtrip() {
        for &at in AssetType::all() {
            let s = at.as_str();
            let back = AssetType::from_string(s).unwrap();
            assert_eq!(at, back);
        }
    }

    #[test]
    fn test_asset_type_file_base() {
        assert_eq!(AssetType::Icon.file_base(), "icon");
        assert_eq!(AssetType::Hero.file_base(), "hero");
        assert_eq!(AssetType::Grid.file_base(), "vertical");
        assert_eq!(AssetType::Header.file_base(), "header");
        assert_eq!(AssetType::Logo.file_base(), "logo");
    }

    #[test]
    fn test_asset_type_thumb_dims() {
        assert_eq!(AssetType::Icon.thumb_dims(), (32, 32));
        assert_eq!(AssetType::Hero.thumb_dims(), (1920, 620));
        assert_eq!(AssetType::Grid.thumb_dims(), (300, 450));
        assert_eq!(AssetType::Header.thumb_dims(), (460, 215));
        assert_eq!(AssetType::Logo.thumb_dims(), (620, 620));
    }

    #[test]
    fn test_asset_type_display_name() {
        assert_eq!(AssetType::Icon.display_name(), "Icon");
        assert_eq!(AssetType::Grid.display_name(), "Capsule");
        assert_eq!(AssetType::Logo.display_name(), "Logo");
    }

    #[test]
    fn test_logo_position_roundtrip() {
        for &pos in LogoPosition::all() {
            let s = pos.as_str();
            let back = LogoPosition::from_string(s);
            assert_eq!(pos, back);
        }
    }

    #[test]
    fn test_logo_position_default() {
        assert_eq!(LogoPosition::default(), LogoPosition::BottomLeft);
    }

    #[test]
    fn test_logo_position_from_string_fallback() {
        assert_eq!(LogoPosition::from_string("garbage"), LogoPosition::BottomLeft);
    }
}
