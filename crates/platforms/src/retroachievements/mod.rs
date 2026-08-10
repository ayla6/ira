pub mod api;
mod api_types;
pub mod discovery;
mod discovery_helpers;
pub mod paths;

pub use api::{
    build_ra_achievements, enrich_ra_game, load_ra_achievements_from_cache,
    read_console_games_cache, redownload_missing_ra_badges, RaAchievementDef, RaClient, RaGameData,
    RaGameEntry,
};
pub use discovery::build_ra_games;
