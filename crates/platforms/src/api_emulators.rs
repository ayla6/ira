use ira_models::AppDetails;
pub use crate::api_emulators_gog::{
    find_galaxy_settings, has_original_gog_dlls, install_nge, is_nge_installed, list_gog_versions,
    read_nge_language, uninstall_nge, write_nge_dlc_config, write_nge_language,
};
pub use crate::api_emulators_shared::{api_emulators_dir, ensure_skeleton};
pub use crate::api_emulators_steam::{
    find_steam_settings, has_original_steam_dlls, install_gse, is_gse_installed, list_gse_versions,
    read_gse_language, uninstall_gse, write_gse_dlc_config, write_gse_language,
};

pub fn read_current_language(
    trophy_source: &str,
    game_exe: &str,
    save_dir: &str,
    app_id: &str,
) -> Option<String> {
    match trophy_source {
        ira_models::GSE => {
            find_steam_settings(game_exe, save_dir, app_id)
                .and_then(|dir| read_gse_language(&dir))
        }
        ira_models::NGE => {
            find_galaxy_settings(game_exe)
                .and_then(|dir| read_nge_language(&dir))
        }
        _ => None,
    }
}

pub fn write_language_configs(
    trophy_source: &str,
    game_exe: &str,
    save_dir: &str,
    app_id: &str,
    language: &str,
) {
    if language.is_empty() {
        return;
    }
    match trophy_source {
        ira_models::GSE => {
            if let Some(settings_dir) = find_steam_settings(game_exe, save_dir, app_id) {
                if let Err(e) = write_gse_language(&settings_dir, language) {
                    eprintln!("Language config write failed: {}", e);
                }
            }
        }
        ira_models::NGE => {
            if let Some(settings_dir) = find_galaxy_settings(game_exe) {
                if let Err(e) = write_nge_language(&settings_dir, language) {
                    eprintln!("Language config write failed: {}", e);
                }
            }
        }
        _ => {}
    }
}

pub fn write_dlc_configs(
    trophy_source: &str,
    game_exe: &str,
    save_dir: &str,
    app_id: &str,
    details: &AppDetails,
) {
    if details.dlcs.is_empty() {
        return;
    }
    match trophy_source {
        ira_models::GSE => {
            if let Some(settings_dir) = find_steam_settings(game_exe, save_dir, app_id) {
                if let Err(e) = write_gse_dlc_config(&settings_dir, details) {
                    eprintln!("DLC config write failed: {}", e);
                }
            }
        }
        ira_models::NGE => {
            if let Some(settings_dir) = find_galaxy_settings(game_exe) {
                if let Err(e) = write_nge_dlc_config(&settings_dir, details) {
                    eprintln!("DLC config write failed: {}", e);
                }
            }
        }
        _ => {}
    }
}
