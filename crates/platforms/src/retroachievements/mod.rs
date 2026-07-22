pub mod api;
pub mod discovery;
pub mod paths;
mod api_types;
mod discovery_helpers;

pub use api::{RaClient, RaGameEntry, RaGameData, RaAchievementDef, build_ra_achievements, enrich_ra_game, load_ra_achievements_from_cache, read_console_games_cache, redownload_missing_ra_badges};
pub use discovery::build_ra_games;
