use serde::Deserialize;
use std::collections::HashMap;

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
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub hidden: i64,
    /// Hidden achievements routinely omit the description in Valve's schema.
    #[serde(default)]
    pub description: String,
    pub icon: String,
    #[serde(rename = "icongray", default)]
    pub icon_gray: String,
}

/// The slice of the store `appdetails` answer Ira needs: the store's own
/// image URLs. New releases publish them under hashed directories the
/// legacy fixed paths never cover.
#[derive(Debug, Deserialize)]
pub(crate) struct SteamAppDetailsImages {
    #[serde(rename = "header_image")]
    pub header_image: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SteamAppDetailsEntry {
    pub success: bool,
    #[serde(default)]
    pub data: Option<SteamAppDetailsImages>,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The Steam schema omits `description` on hidden achievements and the
    /// whole stats block on games without any — both used to fail the decode
    /// and abort achievement generation for those games.
    #[test]
    fn test_steam_schema_decodes_achievements_without_description() {
        let fixture = r#"{
            "game": {
                "gameName": "Touhou Koumakyou: New Classic",
                "gameVersion": "7",
                "availableGameStats": {
                    "achievements": [
                        {
                            "name": "Achievement01",
                            "defaultvalue": 0,
                            "displayName": "Welcome to Gensoukyou",
                            "hidden": 0,
                            "description": "Set out to resolve the incident for the first time.",
                            "icon": "https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/4659620/f2ea0a53ef2a93a8377fb3578f0c61e78aa9727b.jpg",
                            "icongray": "https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/4659620/368e6b495be9ddc48995a23d092201a638b739e6.jpg"
                        },
                        {
                            "name": "Achievement05",
                            "defaultvalue": 0,
                            "displayName": "The Very Angry Bookworm",
                            "hidden": 1,
                            "icon": "https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/4659620/195896951f39b2f3697cf84e3c646ed406b927cd.jpg",
                            "icongray": "https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/4659620/8ec0dea112e4f5fae28a07d717027007153f83bb.jpg"
                        }
                    ]
                }
            }
        }"#;

        let schema: SteamSchemaResponse = serde_json::from_str(fixture).unwrap();
        let achs = schema.game.available_game_stats.unwrap().achievements;
        assert_eq!(achs.len(), 2);
        assert_eq!(achs[0].description, "Set out to resolve the incident for the first time.");
        assert_eq!(achs[1].description, "");
        assert_eq!(achs[1].display_name, "The Very Angry Bookworm");
    }

    #[test]
    fn test_steam_schema_decodes_games_without_any_achievements() {
        let fixture = r#"{
            "game": {
                "gameName": "Some Game",
                "gameVersion": "2"
            }
        }"#;

        let schema: SteamSchemaResponse = serde_json::from_str(fixture).unwrap();
        assert!(schema.game.available_game_stats.is_none());
    }

    #[test]
    fn test_deserialize_percent_accepts_string_and_number() {
        #[derive(Deserialize)]
        struct Probe {
            #[serde(deserialize_with = "deserialize_percent")]
            percent: f64,
        }
        let from_string: Probe = serde_json::from_str(r#"{"percent": "87.5"}"#).unwrap();
        assert_eq!(from_string.percent, 87.5);
        let from_number: Probe = serde_json::from_str(r#"{"percent": 12.25}"#).unwrap();
        assert_eq!(from_number.percent, 12.25);
        assert!(serde_json::from_str::<Probe>(r#"{"percent": true}"#).is_err());
    }
}
