use crate::load::xdg_dir;

pub(crate) const DEFAULT_GRID_COVER_WIDTH: i32 = 200;

pub(crate) fn default_grid_cover_width() -> i32 {
    DEFAULT_GRID_COVER_WIDTH
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_console_enabled() -> bool {
    true
}

pub(crate) fn default_language_preferences() -> Vec<String> {
    vec!["english".to_string()]
}

pub(crate) fn default_save_dir() -> String {
    xdg_dir(xdg::BaseDirectories::new().get_data_home())
        .join("ira")
        .to_string_lossy()
        .to_string()
}
