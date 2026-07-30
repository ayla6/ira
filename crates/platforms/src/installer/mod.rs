pub mod detect;
pub mod gog_extract;
pub mod inno;

pub use detect::{
    game_platform, installer_type, is_gog_makeself, is_inno_setup, is_linuxrulez, GamePlatform,
    InstallerType,
};
pub use gog_extract::{extract_data_zip, split_gog_installer};
pub use inno::{extract_inno, innoextract_available};
