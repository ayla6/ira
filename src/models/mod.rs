//! Shared domain types — dependency leaf (no crate-internal imports).
//! Used by parser, db, api, platforms, and ui modules.

mod game;
mod game_entry;
pub mod achievement;
mod message;
mod kind;

pub use game::{Game, unmatched_game};
pub use game_entry::GameEntry;
pub use achievement::{AchievementStatus, MergedAchievement, StringOrMap, GogAchievementStatus};
pub use message::{AppMessage, AppSender};
pub use kind::*;
