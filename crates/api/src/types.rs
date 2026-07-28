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

#[derive(Debug, Deserialize)]
pub(crate) struct AppDetailsResponse {
    #[serde(flatten)]
    pub apps: HashMap<String, AppDetailsEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AppDetailsEntry {
    pub success: bool,
    pub data: SteamGameDetails,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SteamGameDetails {
    pub name: String,
    #[serde(rename = "header_image")]
    pub _header_image: String,
    #[serde(rename = "capsule_imagev5")]
    pub capsule_image: String,
    #[serde(default)]
    pub release_date: Option<SteamReleaseDate>,
    #[serde(default)]
    pub metacritic: Option<SteamMetacritic>,
    #[serde(default)]
    pub recommendations: Option<SteamRecommendations>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SteamReleaseDate {
    pub coming_soon: bool,
    pub date: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SteamMetacritic {
    pub score: i32,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SteamRecommendations {
    pub total: i32,
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
        serde_json::Value::Number(n) => n.as_f64().ok_or_else(|| serde::de::Error::custom("invalid number")),
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
    #[serde(rename = "appid")]
    pub _appid: String,
    #[serde(default)]
    pub common: SteamCmdCommon,
    #[serde(default)]
    pub extended: SteamCmdExtended,
    #[serde(default)]
    pub config: SteamCmdConfig,
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
    pub _associations: HashMap<String, SteamCmdAssociation>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdAssociation {
    #[serde(rename = "name")]
    pub _name: String,
    #[serde(rename = "type")]
    pub _kind: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdExtended {
    #[serde(default)]
    pub developer: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub homepage: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdConfig {
    #[serde(default)]
    pub installdir: String,
}

/// Parsed fields extracted from a steamcmd.net response.
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
}
