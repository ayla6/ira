#[derive(Debug, Clone)]
pub struct PlaySession {
    pub id: i64,
    pub game_id: i64,
    pub started_at: i64,
    pub ended_at: i64,
    pub duration_seconds: i64,
}

impl Default for PlaySession {
    fn default() -> Self {
        Self {
            id: 0,
            game_id: 0,
            started_at: 0,
            ended_at: 0,
            duration_seconds: 0,
        }
    }
}
