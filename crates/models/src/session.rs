#[derive(Debug, Clone, Default)]
pub struct PlaySession {
    pub id: i64,
    pub game_id: i64,
    pub variant_id: Option<i64>,
    pub started_at: i64,
    pub ended_at: i64,
    pub duration_seconds: i64,
}
