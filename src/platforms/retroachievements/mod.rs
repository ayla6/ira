pub mod api;
pub mod discovery;
pub mod paths;

pub use api::{RaClient, RaGameEntry, RaGameData, RaAchievementDef, build_ra_achievements, enrich_ra_game};
pub use discovery::build_ra_games;
