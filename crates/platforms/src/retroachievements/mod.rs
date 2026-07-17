pub mod api;
pub mod discovery;
pub mod paths;

pub use api::{RaClient, RaGameEntry, RaGameData, RaAchievementDef, build_ra_achievements, enrich_ra_game, load_ra_achievements_from_cache, read_console_games_cache};
pub use discovery::build_ra_games;
