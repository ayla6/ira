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
    #[serde(default)]
    pub ufs_savefiles: Vec<UfsSaveFile>,
    #[serde(default)]
    pub ufs_rootoverrides: Vec<UfsRootOverride>,
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct UfsSaveFile {
    pub path: String,
    pub root: String,
    pub recursive: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct UfsRootOverride {
    pub os: String,
    pub root: String,
    pub useinstead: String,
    pub addpath: String,
    pub pathtransforms: Vec<UfsPathTransform>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct UfsPathTransform {
    pub find: String,
    pub replace: String,
}

fn default_true() -> bool { true }
