use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use ira_models::{AppDetails, DlcInfo};

#[derive(Clone)]
pub struct SgdbAsset {
    pub url: String,
    pub thumb: String,
    pub width: i64,
    pub height: i64,
    pub style: String,
    pub author: String,
    pub mime: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SteamReviewSummary {
    pub review_score: i32,
    pub total_positive: i32,
    pub total_negative: i32,
    pub total_reviews: i32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SteamReviewsResponse {
    pub success: i32,
    pub query_summary: SteamReviewSummary,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlobalAchievementsResponse {
    #[serde(default)]
    pub achievementpercentages: Option<GlobalAchievementsInner>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlobalAchievementsInner {
    pub achievements: Vec<GlobalAchievementEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlobalAchievementEntry {
    pub name: String,
    #[serde(deserialize_with = "deserialize_percent")]
    pub percent: f64,
}

pub(crate) fn deserialize_percent<'de, D>(d: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => s.parse::<f64>().map_err(serde::de::Error::custom),
        serde_json::Value::Number(n) => n
            .as_f64()
            .ok_or_else(|| serde::de::Error::custom("invalid number")),
        _ => Err(serde::de::Error::custom("expected string or number")),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct SteamSchemaResponse {
    pub game: SteamSchemaGame,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SteamSchemaGame {
    #[serde(rename = "availableGameStats", default)]
    pub available_game_stats: Option<SteamSchemaStats>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SteamSchemaStats {
    pub achievements: Vec<SteamSchemaAchievement>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SteamSchemaAchievement {
    pub name: String,
    pub _defaultvalue: i64,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub hidden: i64,
    pub description: String,
    pub icon: String,
    #[serde(rename = "icongray")]
    pub icon_gray: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NemirtingasAchievement {
    pub name: String,
    pub hidden: bool,
    pub icon: String,
    #[serde(rename = "icongray")]
    pub icon_gray: String,
    #[serde(rename = "displayName")]
    pub display_name: HashMap<String, String>,
    #[serde(default)]
    pub description: HashMap<String, String>,
}

// ── steamcmd.net API ────────────────────────────────────────────────

/// Top-level response from `api.steamcmd.net/v1/info/{appid}`.
#[derive(Debug, Deserialize)]
pub(crate) struct SteamCmdResponse {
    pub data: HashMap<String, SteamCmdApp>,
    pub status: String,
}

/// Per-app data from steamcmd.net.
#[derive(Debug, Deserialize)]
pub(crate) struct SteamCmdApp {
    #[serde(default)]
    pub common: SteamCmdCommon,
    #[serde(default)]
    pub extended: SteamCmdExtended,
    #[serde(default)]
    pub config: SteamCmdConfig,
    #[serde(default)]
    pub ufs: SteamCmdUfs,
    #[serde(default)]
    pub steam_release_date: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdCommon {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub metacritic_score: String,
    #[serde(default)]
    pub review_percentage: String,
    #[serde(default)]
    pub review_score: String,
    #[serde(default)]
    pub clienticon: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub oslist: String,
    #[serde(default)]
    pub supported_languages: HashMap<String, SteamCmdLanguage>,
    #[serde(default)]
    pub library_assets: SteamCmdLibraryAssets,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLibraryAssets {
    #[serde(default)]
    pub logo_position: SteamCmdLogoPosition,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLogoPosition {
    #[serde(default)]
    pub pinned_position: String,
    #[serde(default)]
    pub width_pct: String,
    #[serde(default)]
    pub _height_pct: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdExtended {
    #[serde(default)]
    pub developer: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub listofdlc: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLanguage {
    #[serde(default)]
    pub _supported: String,
    #[serde(default)]
    pub _full_audio: String,
    #[serde(default)]
    pub _subtitles: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdConfig {
    #[serde(default)]
    pub installdir: String,
    #[serde(default)]
    pub launch: HashMap<String, SteamCmdLaunch>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLaunch {
    #[serde(default)]
    pub executable: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub config: SteamCmdLaunchConfig,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLaunchConfig {
    #[serde(default)]
    pub ownsdlc: String,
    #[serde(default)]
    pub oslist: String,
}

// ── UFS (Unified File System) ───────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdUfs {
    #[serde(default)]
    pub savefiles: HashMap<String, SteamCmdSaveFile>,
    #[serde(default)]
    pub rootoverrides: HashMap<String, SteamCmdRootOverride>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdSaveFile {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub recursive: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdRootOverride {
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub useinstead: String,
    #[serde(default)]
    pub addpath: String,
    #[serde(default)]
    pub pathtransforms: HashMap<String, SteamCmdPathTransform>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdPathTransform {
    #[serde(default)]
    pub find: String,
    #[serde(default)]
    pub replace: String,
}

// ── Parsed app details ──────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamCmdInfo {
    pub name: String,
    pub release_timestamp: i64,
    pub metacritic_score: i64,
    pub review_percentage: i64,
    pub review_score: i64,
    pub developer: String,
    pub publisher: String,
    pub homepage: String,
    pub install_dir: String,
    /// Client icon hash (used for `steam/games/<hash>.ico` and CDN).
    pub clienticon: String,
    /// Community icon hash (used for CDN `apps/<appid>/<hash>.ico`).
    pub icon: String,
    /// Comma-separated OS list from `common.oslist` (e.g. "windows,macos,linux").
    #[serde(default)]
    pub oslist: String,
    /// Sorted launch entries (launch.0 is the default). Empty if none.
    #[serde(default)]
    pub launches: Vec<SteamCmdLaunchInfo>,
    /// Logo position from Steam library_assets (kebab-case, e.g. "bottom-left").
    #[serde(default)]
    pub logo_position: String,
    /// Logo width percentage from Steam (0 if not available).
    #[serde(default)]
    pub logo_size: i32,
}

/// A single launch entry from steamcmd.net `config.launch`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SteamCmdLaunchInfo {
    /// Executable path relative to the install directory.
    pub executable: String,
    /// OS list for this launch (e.g. "windows", "linux").
    pub oslist: String,
    /// Optional human-readable description (e.g. "Start Launcher").
    pub description: String,
}
