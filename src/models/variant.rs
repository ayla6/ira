use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameVariant {
    pub id: i64,
    pub game_id: i64,
    pub name: String,
    pub exe: String,
    pub working_dir: String,
    pub args: String,
    #[serde(default)]
    pub env_vars: Vec<(String, String)>,
    pub emu_version: String,
    pub emu_installed: bool,
}

impl Default for GameVariant {
    fn default() -> Self {
        Self {
            id: 0,
            game_id: 0,
            name: String::new(),
            exe: String::new(),
            working_dir: String::new(),
            args: String::new(),
            env_vars: Vec::new(),
            emu_version: String::new(),
            emu_installed: false,
        }
    }
}
