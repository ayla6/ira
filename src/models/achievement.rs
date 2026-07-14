use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AchievementStatus {
    pub earned: bool,
    pub earned_time: i64,
}

/// GOG emulator status entry: { "unlock_time": 1234567890 }
/// Only earned achievements appear; absent = not earned.
#[derive(Debug, Clone, Deserialize)]
pub struct GogAchievementStatus {
    #[serde(default)]
    pub unlock_time: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StringOrMap {
    pub val: String,
}

impl<'de> Deserialize<'de> for StringOrMap {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Str(String),
            Map(HashMap<String, String>),
        }
        match Raw::deserialize(de)? {
            Raw::Str(s) => Ok(StringOrMap { val: s }),
            Raw::Map(m) => {
                let val = m
                    .get("english")
                    .cloned()
                    .or_else(|| m.into_iter().next().map(|(_, v)| v))
                    .unwrap_or_default();
                Ok(StringOrMap { val })
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AchievementMeta {
    #[serde(default)]
    pub description: StringOrMap,
    #[serde(default, rename = "displayName")]
    pub display_name: StringOrMap,
    #[serde(default)]
    pub hidden: serde_json::Value,
    #[serde(default)]
    pub icon: String,
    #[serde(default, rename = "icongray")]
    pub icon_gray: String,
    #[serde(default, rename = "icon_gray")]
    pub icon_gray_alt: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct MergedAchievement {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub hidden: bool,
    pub earned: bool,
    pub earned_time: i64,
    pub icon_path: String,
    pub icon_gray_path: String,
    pub global_percent: f64,
    pub trophy_type: char,
}

pub(crate) fn parse_hidden(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        serde_json::Value::String(s) => s == "1" || s == "true",
        _ => false,
    }
}
