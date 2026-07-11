//! Shared domain types — dependency leaf (no crate-internal imports).
//! Used by parser, db, api, platforms, and ui modules.

mod game;
mod game_entry;
pub mod achievement;
pub mod launch_config;
pub mod session;
mod message;
mod kind;

pub use game::{Game, unmatched_game};
pub use game_entry::GameEntry;
pub use launch_config::{GameLaunchConfig, WineConfig, LaunchConfig};
pub use session::PlaySession;
pub use achievement::{AchievementStatus, MergedAchievement, StringOrMap, GogAchievementStatus};
pub use message::{AppMessage, AppSender};
pub use kind::*;
