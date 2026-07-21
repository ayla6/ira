use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
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
    #[serde(default = "default_true")]
    pub count_playtime: bool,
    #[serde(default)]
    pub logo_position: String,
    #[serde(default)]
    pub logo_size: i32,
}

fn default_true() -> bool { true }

