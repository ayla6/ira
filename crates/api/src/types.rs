use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use ira_models::{AppDetails, DlcInfo};

#[derive(Clone)]
pub struct SgdbAsset {
    pub url: String,
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
