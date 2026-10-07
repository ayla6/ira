use serde::{Deserialize, Serialize};

use super::asset_type::LogoPosition;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GameVariant {
    pub id: i64,
    pub game_id: i64,
    pub name: String,
    pub exe: String,
    pub working_dir: String,
    pub args: String,
    #[serde(default)]
    pub env_vars: Vec<(String, String)>,
    #[serde(default)]
    pub sort_order: i32,
    #[serde(default)]
    pub pre_launch: String,
    #[serde(default)]
    pub custom_images: bool,
    #[serde(default)]
    pub show_as_entry: bool,
    #[serde(default)]
    pub playtime: f64,
    #[serde(default)]
    pub last_played: i64,
    #[serde(default = "crate::helpers::default_true")]
    pub count_playtime: bool,
    /// Logo overlay override. None inherits the base game's position
    /// (the database stores this as an empty string); Some pins one.
    #[serde(default)]
    pub logo_position: Option<LogoPosition>,
    #[serde(default)]
    pub logo_size: i32,
}
