mod crud;
mod lookup;
mod settings;
mod game_config;
mod sessions;
mod profiles;
mod variants;
mod groups;
mod metadata;
mod migration;
mod discs;
mod setup;
mod row_mapping;

pub use crud::*;
pub use lookup::*;
pub use settings::*;
pub use game_config::*;
pub use sessions::*;
pub use profiles::*;
pub use variants::*;
pub use groups::*;
pub use metadata::*;
pub use discs::*;
pub use setup::{checkpoint, init_db, update_field};
pub(crate) use row_mapping::{game_entry_from_row, lock_db, GAME_COLUMNS};

pub type DbConn = r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>;
