use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
