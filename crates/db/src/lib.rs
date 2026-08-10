mod crud;
mod discs;
mod game_config;
mod groups;
mod lookup;
mod metadata;
mod profiles;
mod row_mapping;
mod sessions;
mod settings;
mod setup;
mod variants;

pub use crud::*;
pub use discs::*;
pub use game_config::*;
pub use groups::*;
pub use lookup::*;
pub use metadata::*;
pub use profiles::*;
pub(crate) use row_mapping::{game_entry_from_row, lock_db, GAME_COLUMNS};
pub use sessions::*;
pub use settings::*;
pub use setup::{checkpoint, init_db, update_field};
pub use variants::*;

pub type DbConn = r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>;
