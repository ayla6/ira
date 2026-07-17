use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GameDisc {
    pub id: i64,
    pub game_id: i64,
    pub disc_number: i32,
    pub rom_path: String,
    pub label: String,
}
