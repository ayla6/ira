mod discovery;
mod game;
mod npbind;
mod paths;
mod playtime;
mod versions;
mod watcher;

pub use discovery::*;
pub use game::*;
pub use npbind::*;
pub use paths::*;
pub use playtime::*;
pub use versions::*;
pub use watcher::*;
// Shared Sony parsers live in crate::sony; re-exported here so existing
// ira_platforms::ps4::parse_psf and ps4::psf:: paths keep resolving.
pub use crate::sony::{
    parse_psf, parse_psf_bytes, parse_trop_xml, parse_user_trophies, psf, psf_get_title,
    psf_get_title_id, trophy_xml, PsfValue, TrophyDef,
};
