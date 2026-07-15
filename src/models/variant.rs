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
}

