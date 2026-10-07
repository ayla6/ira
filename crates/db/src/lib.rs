mod crud;
mod crud_merge;
mod entities;
mod entities_merge;
mod discs;
mod game_config;
mod groups;
mod links;
mod lookup;
mod metadata;
mod metadata_local;
mod metadata_lookup;
mod metadata_marks;
mod profiles;
mod query;
mod rom_serials;
mod row_mapping;
mod sessions;
mod settings;
mod setup;
mod variants;
mod auto_groups;

pub use crud::*;
pub use crud_merge::*;
pub use entities::*;
pub use entities_merge::*;
pub use discs::*;
pub use game_config::*;
pub use groups::*;
pub use auto_groups::*;
pub use links::*;
pub use lookup::*;
pub use metadata::*;
pub use metadata_local::*;
pub use metadata_lookup::*;
pub use metadata_marks::*;
pub use profiles::*;
pub use rom_serials::*;
pub(crate) use query::{
    execute, positional_placeholders, query_all, query_optional, query_optional_scalar,
    query_scalar_all, IN_QUERY_CHUNK_SIZE,
};
pub(crate) use row_mapping::{err, game_entry_from_row, lock_db, GAME_COLUMNS};
pub use sessions::*;
pub use settings::*;
pub use setup::{checkpoint, init_db, update_field};
pub use variants::*;

pub type DbConn = r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>;
