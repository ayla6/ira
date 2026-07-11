use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone)]
pub struct SgdbAsset {
    pub url: String,
    pub width: i64,
    pub height: i64,
    pub style: String,
    pub author: String,
    pub mime: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppDetails {
    #[serde(default, alias = "Name")]
    pub name: String,
    #[serde(default, alias = "Languages")]
    pub languages: Vec<String>,
    #[serde(default, alias = "Dlcs")]
    pub dlcs: HashMap<String, DlcInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DlcInfo {
    #[serde(default, alias = "Name")]
    pub name: String,
    #[serde(default, alias = "AppId")]
    pub app_id: i64,
    #[serde(default, alias = "ImageUrl")]
    pub image_url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

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
    #[serde(rename = "availableGameStats")]
    pub available_game_stats: SteamSchemaStats,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SteamSchemaStats {
    pub achievements: Vec<SteamSchemaAchievement>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SteamSchemaAchievement {
    pub name: String,
    #[allow(dead_code)]
    pub defaultvalue: i64,
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
