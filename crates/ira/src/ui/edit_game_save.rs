use super::add_game_dialog::collect_env_vars;
use super::edit_game_controller::ControllerWidgets;
use super::edit_game_launch::LaunchConfigWidgets;
use super::edit_game_overlay::OverlayWidgets;
use super::edit_game_system::SystemWidgets;
use super::edit_game_variants::VarW;
use super::input_profile_store::add_game_compatibility;
use super::message_helpers::refresh_selected_base_game;
use super::state::{PendingImage, SharedState};
use super::wine_config_env_dll::collect_dll_overrides;
use super::wine_config_widget::WineConfigWidgets;
use adw::prelude::*;
use ira_models::{AssetType, GameLaunchConfig, TrophySource, WineConfig};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;

pub(super) struct SaveGameSettingsParams {
    pub state: SharedState,
    pub win: adw::Window,
    pub db_id: i64,
    pub app_id: String,
    pub trophy_source: TrophySource,
    pub game_kind: ira_models::GameKind,
    pub var_widgets: Rc<RefCell<Vec<VarW>>>,
    pub save_dir: String,
    pub logo_controls: Option<(Rc<RefCell<String>>, gtk4::Adjustment)>,
    pub dlc_switches: Vec<adw::SwitchRow>,
    pub pending_copies: Rc<RefCell<HashMap<String, PendingImage>>>,
    pub old_wine: WineConfig,
    pub app_default_wine: WineConfig,
    pub game_exe: String,
    pub game_folder: String,
    pub language_row: Option<adw::ComboRow>,
    pub languages: Vec<String>,
    pub saved_platform_id: String,
    pub system_widgets: Option<SystemWidgets>,
    pub overlay_widgets: Option<OverlayWidgets>,
    pub controller_widgets: Option<ControllerWidgets>,
    pub title_entry: adw::EntryRow,
    pub sort_entry: adw::EntryRow,
    pub pending_version: Rc<RefCell<Option<String>>>,
    pub app_id_entry: Option<adw::EntryRow>,
    pub pending_ra_core: Rc<RefCell<Option<String>>>,
    pub pending_emulator: Rc<RefCell<Option<String>>>,
    pub launch_config_widgets: Option<LaunchConfigWidgets>,
    pub show_wine_tabs: bool,
    pub wine_widgets: Option<WineConfigWidgets>,
    pub profiles: Vec<ira_models::WineProfile>,
    pub saved_profile_id: Option<i64>,
    pub game_folder_entry: Option<adw::EntryRow>,
    pub runtime_row: Option<adw::ComboRow>,
    pub pending_emu_uninstall: Option<Rc<RefCell<bool>>>,
}

struct AppIdResult {
    changed: bool,
    new_val: String,
}

fn save_title_and_sort(
    db: &ira_db::DbConn,
    db_id: i64,
    title_entry: &adw::EntryRow,
    sort_entry: &adw::EntryRow,
) {
    let title = title_entry.text().to_string();
    let sort_title = sort_entry.text().to_string();
    if let Err(e) = ira_db::update_game_title(db, db_id, &title) {
        eprintln!("Failed to update game: {}", e);
    }
    if let Err(e) = ira_db::update_sort_title(db, db_id, &sort_title) {
        eprintln!("Failed to update sort title: {}", e);
    }
}

fn save_app_id(db: &ira_db::DbConn, params: &SaveGameSettingsParams) -> AppIdResult {
    let mut app_id_changed = false;
    let mut new_app_id_val = String::new();
    if let Some(ref app_id_row) = params.app_id_entry {
        let new_id = app_id_row.text().to_string();
        if new_id != params.app_id {
            app_id_changed = true;
            new_app_id_val = new_id.clone();
            let ts = if new_id.is_empty() {
                TrophySource::Empty
            } else {
                params.trophy_source
            };
            let pid = if params.game_kind == ira_models::GameKind::Ps4
                || params.game_kind == ira_models::GameKind::Ps3
                || params.game_kind == ira_models::GameKind::Retro
            {
                &params.saved_platform_id
            } else if new_id.is_empty() {
                ""
            } else {
                &new_id
            };
            let (steam_id, game_id): (&str, &str) = if params.trophy_source.has_steam_enrichment() {
                (&new_id, "")
            } else {
                ("", &new_id)
            };
            if let Err(e) = ira_db::update_game_ids(db, params.db_id, steam_id, game_id, ts, pid) {
                eprintln!("Failed to update app ID: {}", e);
            }
        }
    }
    AppIdResult {
        changed: app_id_changed,
        new_val: new_app_id_val,
    }
}

fn save_version_and_overrides(db: &ira_db::DbConn, params: &SaveGameSettingsParams) {
    if let Some(ver) = params.pending_version.borrow().as_ref() {
        if let Err(e) = ira_db::set_shadps4_version(db, params.db_id, ver) {
            eprintln!("Failed to set shadps4 version: {}", e);
        }
    }
    if let Some(core) = params.pending_ra_core.borrow().as_ref() {
        if let Err(e) = ira_db::set_ra_core(db, params.db_id, core) {
            eprintln!("Failed to set RA core: {}", e);
        }
    }
    if let Some(emu) = params.pending_emulator.borrow().as_ref() {
        if let Err(e) = ira_db::set_emulator_override(db, params.db_id, emu) {
            eprintln!("Failed to set emulator override: {}", e);
        }
    }
}

fn build_launch_config_and_wine(
    params: &SaveGameSettingsParams,
) -> (GameLaunchConfig, WineConfig, Option<i64>) {
    let sw = params.system_widgets.as_ref();
    let ow = params.overlay_widgets.as_ref();
    let cw = params.controller_widgets.as_ref();
    let env_vars = sw.map_or(Vec::new(), |s| collect_env_vars(&s.env_vars_box));
    let ld_preload = sw.map_or(String::new(), |s| s.ld_preload_entry.text().to_string());
    let ld_library_path = sw.map_or(String::new(), |s| {
        s.ld_library_path_entry.text().to_string()
    });
    let overlay_enabled = ow.and_then(|s| *s.overlay_state.borrow());
    let input_mode = cw.and_then(|s| *s.input_mode.borrow());
    let input_profile = cw.and_then(|s| {
        s.input_profile_paths
            .borrow()
            .get(s.input_profile_row.selected() as usize)
            .and_then(Clone::clone)
            .map(|path| path.to_string_lossy().into_owned())
    });
    if let Some(profile) = input_profile.as_deref() {
        if let Err(error) = add_game_compatibility(Path::new(profile), params.db_id) {
            eprintln!("Failed to associate controller profile with game: {error}");
        }
    }
    let gamemode = sw.and_then(|s| *s.gamemode_state.borrow());
    let mangohud = sw.and_then(|s| *s.mangohud_state.borrow());
    let gamescope = sw.and_then(|s| *s.gamescope_state.borrow());
    let gamescope_flags = sw.map_or(String::new(), |s| s.gamescope_flags.text().to_string());
    let gpu = sw
        .and_then(|s| {
            s.gpu_row.as_ref().map(|gr| {
                let idx = gr.selected() as usize;
                if idx == 0 {
                    String::new()
                } else {
                    s.gpu_options.get(idx - 1).cloned().unwrap_or_default()
                }
            })
        })
        .unwrap_or_default();
    let overlay_encoder = ow
        .and_then(|s| s.overlay_encoder_row.as_ref())
        .and_then(|r| {
            let idx = r.selected();
            if idx == 0 {
                None
            } else {
                Some(idx - 1)
            }
        });
    let overlay_recording_quality = ow
        .and_then(|s| s.overlay_quality_row.as_ref())
        .and_then(|r| {
            let idx = r.selected();
            if idx == 0 {
                None
            } else {
                Some(idx - 1)
            }
        });

    let gamescope_w = sw.and_then(|s| *s.gamescope_w_state.borrow());
    let gamescope_h = sw.and_then(|s| *s.gamescope_h_state.borrow());
    let gamescope_fps = sw.and_then(|s| *s.gamescope_fps_state.borrow());
    let gamescope_upscaling = sw.and_then(|s| s.gamescope_upscaling_state.borrow().clone());

    let lc = params.launch_config_widgets.as_ref();
    let launch = GameLaunchConfig {
        exe: lc.map_or(String::new(), |l| l.exe_entry.text().to_string()),
        args: lc.map_or(String::new(), |l| l.args_entry.text().to_string()),
        working_dir: lc.map_or(String::new(), |l| l.wd_entry.text().to_string()),
        env_vars,
        ld_preload,
        ld_library_path,
        pre_launch: lc.map_or(String::new(), |l| l.pre_launch_entry.text().to_string()),
        overlay_enabled,
        input_mode,
        input_profile,
        input_pause_unfocused: cw.and_then(|s| *s.pause_unfocused.borrow()),
        gamemode,
        mangohud,
        gamescope,
        gamescope_flags,
        gamescope_w,
        gamescope_h,
        gamescope_fps,
        gamescope_upscaling,
        gpu,
        overlay_encoder,
        overlay_recording_quality,
        overlay_font_family: None,
    };
    let mut wine = params
        .wine_widgets
        .as_ref()
        .map(|ww| ww.to_wine_config())
        .unwrap_or_else(|| params.old_wine.clone());

    if params.show_wine_tabs {
        if let Some(ref ww) = params.wine_widgets {
            wine.dll_overrides = collect_dll_overrides(&ww.dll_overrides_box);
        }
    }

    if params.game_kind.is_managed_pc() {
        wine.enabled = selected_game_kind(params) == ira_models::GameKind::Wine;
    }

    if wine.dll_overrides != params.app_default_wine.dll_overrides {
        if !wine
            .overridden_fields
            .contains(&"dll_overrides".to_string())
        {
            wine.overridden_fields.push("dll_overrides".to_string());
        }
    } else {
        wine.overridden_fields.retain(|f| f != "dll_overrides");
    }

    let new_profile_id = if let Some(ref lc) = params.launch_config_widgets {
        if let Some(ref profile_row) = lc.profile_row {
            if profile_row.selected() > 0 {
                let profiles = ira_db::get_all_profiles(&params.state.borrow().db)
                    .unwrap_or_else(|_| params.profiles.clone());
                profiles
                    .get((profile_row.selected() - 1) as usize)
                    .map(|p| p.id)
            } else {
                None
            }
        } else {
            params.saved_profile_id
        }
    } else {
        params.saved_profile_id
    };

    if let Some(pid) = new_profile_id {
        if let Ok(Some(profile)) = ira_db::get_profile(&params.state.borrow().db, pid) {
            wine.version = profile.wine_version;
            wine.custom_wine_path = profile.custom_wine_path;
            wine.prefix = profile.prefix;
            wine.arch = profile.arch;
            wine.umu_enabled = profile.umu_enabled;
        }
    }

    (launch, wine, new_profile_id)
}

fn selected_game_kind(params: &SaveGameSettingsParams) -> ira_models::GameKind {
    match params.runtime_row.as_ref().map(|row| row.selected()) {
        Some(1) => ira_models::GameKind::Linux,
        Some(0) => ira_models::GameKind::Wine,
        _ => params.game_kind,
    }
}

fn apply_wine_registry(old_wine: &WineConfig, wine: &WineConfig) {
    if !wine.enabled {
        return;
    }
    let reg_changed = wine.mouse_warp_override != old_wine.mouse_warp_override
        || wine.dpi_enabled != old_wine.dpi_enabled
        || wine.dpi != old_wine.dpi
        || wine.show_crash_dialogs != old_wine.show_crash_dialogs
        || wine.audio != old_wine.audio;

    if !reg_changed {
        return;
    }

    let pfx = ira_launcher::wine_launch::wine_prefix(wine);
    let prefix_ready = std::path::Path::new(&pfx).join("system.reg").is_file();
    if !prefix_ready {
        return;
    }

    let wine_exe =
        ira_launcher::wine_launch::find_wine_binary(&wine.version, &wine.custom_wine_path);
    let Ok(wine_exe) = wine_exe else { return };

    let reg_cmds = ira_launcher::wine_launch::build_wine_reg_commands(wine, &wine_exe);
    let env = ira_launcher::wine_launch::build_wine_env(wine, &wine_exe);
    std::thread::spawn(move || {
        for reg_cmd in reg_cmds {
            let mut child = std::process::Command::new(&reg_cmd[0]);
            for arg in &reg_cmd[1..] {
                child.arg(arg);
            }
            child.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            match child.status() {
                Ok(s) if !s.success() && s.code() != Some(1) => {
                    eprintln!(
                        "Wine reg command failed (exit {:?}): {:?}",
                        s.code(),
                        reg_cmd
                    );
                }
                Err(e) => eprintln!("Failed to run wine reg command: {}", e),
                _ => {}
            }
        }
    });
}

fn handle_unmatch(db: &ira_db::DbConn, params: &SaveGameSettingsParams) {
    if params.pending_copies.borrow().contains_key("__unmatch__") {
        if let Err(e) = ira_db::set_sgdb_id(db, params.db_id, "") {
            eprintln!("Failed to clear SGDB ID: {}", e);
        }
        if let Err(e) = ira_db::set_manual_unmatch(db, params.db_id, true) {
            eprintln!("Failed to set manual unmatch: {}", e);
        }
        if let Some(g) = params
            .state
            .borrow_mut()
            .games
            .iter_mut()
            .find(|g| g.db_id == params.db_id)
        {
            g.sgdb_id.clear();
            g.manual_unmatch = true;
        }
        params.pending_copies.borrow_mut().remove("__unmatch__");
    }

    let ra_unmatch_key = format!("__ra_unmatch_{}", params.db_id);
    if params.pending_copies.borrow().contains_key(&ra_unmatch_key) {
        if let Err(e) = ira_db::update_game_ids(
            db,
            params.db_id,
            "",
            "",
            TrophySource::Empty,
            &params.saved_platform_id,
        ) {
            eprintln!("Failed to unmatch RA game: {}", e);
        }
        if let Err(e) = ira_db::set_manual_unmatch(db, params.db_id, true) {
            eprintln!("Failed to set manual unmatch: {}", e);
        }
        {
            let mut s = params.state.borrow_mut();
            if let Some(g) = s.games.iter_mut().find(|g| g.db_id == params.db_id) {
                g.app_id.clear();
                g.trophy_source = TrophySource::Empty;
                g.achievements.clear();
                g.earned_count = 0;
                g.total_count = 0;
                g.manual_unmatch = true;
            }
        }
        params.pending_copies.borrow_mut().remove(&ra_unmatch_key);
    }
}

fn save_logo_settings(db: &ira_db::DbConn, params: &SaveGameSettingsParams) {
    if let Some((ref selected_pos, ref size_adj)) = params.logo_controls {
        let pos = selected_pos.borrow().clone();
        let size = size_adj.value() as i32;
        if params.db_id != 0 {
            if let Err(e) = ira_db::set_logo_settings(db, params.db_id, &pos, size) {
                eprintln!("Failed to update logo settings: {}", e);
            }
        }
        if let Some(g) = params
            .state
            .borrow_mut()
            .games
            .iter_mut()
            .find(|g| g.db_id == params.db_id)
        {
            g.logo_position = pos;
            g.logo_size = size;
        }
    }
}

fn save_dlc_config(params: &SaveGameSettingsParams) {
    let details = crate::game_loader::read_app_details(&params.save_dir, &params.app_id);
    let Some(ref details) = details else { return };
    if params.dlc_switches.is_empty() {
        return;
    }
    let mut details = details.clone();
    let dlcs_vec: Vec<_> = details.dlcs.iter_mut().collect();
    for (i, (_, dlc)) in dlcs_vec.into_iter().enumerate() {
        if i < params.dlc_switches.len() {
            dlc.enabled = params.dlc_switches[i].is_active();
        }
    }
    let path = ira_parser::data_dir(&params.save_dir, &params.app_id).join("dlc_config.json");
    if let Ok(b) = serde_json::to_vec(&details) {
        let _ = std::fs::write(&path, b);
    }
    ira_platforms::api_emulators::write_dlc_configs(
        params.trophy_source,
        &params.game_exe,
        &params.save_dir,
        &params.app_id,
        &details,
    );
}

fn save_language_config(params: &SaveGameSettingsParams) {
    if let Some(ref lang_row) = params.language_row {
        let idx = lang_row.selected() as usize;
        if let Some(lang) = params.languages.get(idx) {
            ira_platforms::api_emulators::write_language_configs(
                params.trophy_source,
                &params.game_exe,
                &params.save_dir,
                &params.app_id,
                lang,
            );
        }
    }
}

fn update_game_state_in_memory(
    params: &SaveGameSettingsParams,
    title: &str,
    sort_title: &str,
    app_id_result: &AppIdResult,
) {
    let mut state_borrow = params.state.borrow_mut();
    let Some(i) = state_borrow
        .games
        .iter()
        .position(|g| g.db_id == params.db_id && g.variant_id.is_none())
    else {
        return;
    };
    let old_name = state_borrow.games[i].name.clone();
    let title_changed = old_name != title;

    if title_changed {
        for vg in state_borrow.games.iter_mut() {
            if vg.db_id == params.db_id && vg.variant_id.is_some() {
                if let Some(suffix) = vg.name.strip_prefix(&old_name) {
                    vg.set_name(format!("{}{}", title, suffix));
                }
            }
        }
    }

    if params.game_kind.is_managed_pc() {
        let target_kind = selected_game_kind(params);
        for game in state_borrow
            .games
            .iter_mut()
            .filter(|game| game.db_id == params.db_id)
        {
            game.kind = target_kind;
        }
    }

    let state = &mut *state_borrow;
    let g = &mut state.games[i];
    g.set_name(title.to_string());
    g.sort_title = sort_title.to_string();

    if let Some(ver) = params.pending_version.borrow().as_ref() {
        g.shadps4_version = ver.clone();
    }
    if let Some(core) = params.pending_ra_core.borrow().as_ref() {
        g.ra_core = core.clone();
    }
    if let Some(emu) = params.pending_emulator.borrow().as_ref() {
        g.emulator_override = emu.clone();
    }
    if app_id_result.changed {
        if app_id_result.new_val.is_empty() {
            g.app_id.clear();
            g.trophy_source = TrophySource::Empty;
            g.platform_id.clear();
            g.achievements.clear();
            g.earned_count = 0;
            g.total_count = 0;
            g.manual_unmatch = true;
        } else {
            state.game_names.lock().unwrap().remove(&params.app_id);
            g.app_id = app_id_result.new_val.clone();
            g.platform_id = app_id_result.new_val.clone();
            g.manual_unmatch = false;
            g.achievements.clear();
            g.earned_count = 0;
            g.total_count = 0;
        }
    }
}

fn update_game_names(
    state: &SharedState,
    app_id_result: &AppIdResult,
    old_app_id: &str,
    title: &str,
) {
    if !app_id_result.new_val.is_empty() {
        state
            .borrow()
            .game_names
            .lock()
            .unwrap()
            .insert(app_id_result.new_val.clone(), title.to_string());
    } else if app_id_result.changed {
        state.borrow().game_names.lock().unwrap().remove(old_app_id);
    }
}

fn spawn_image_copy_thread(
    images: Vec<(String, PendingImage)>,
    cloud_dir: std::path::PathBuf,
    db_id: i64,
    tx: mpsc::Sender<Vec<String>>,
) {
    std::thread::spawn(move || {
        let _s = tracing::info_span!(
            "pending_image_copy_convert",
            db_id = db_id,
            count = images.len()
        )
        .entered();
        let mut pending_images = Vec::new();
        for (asset, img) in &images {
            let Some(at) = AssetType::from_string(asset) else {
                continue;
            };
            let base_name = at.file_base();
            let (max_w, max_h) = at.thumb_dims();
            ira_parser::remove_image_variants(&cloud_dir, base_name);
            ira_parser::remove_image_variants(&cloud_dir, &format!("{}_small", base_name));

            let dest = match img {
                PendingImage::Path(src_path) => {
                    if ira_parser::is_ico_data(&std::fs::read(src_path).unwrap_or_default()) {
                        let ico_path = cloud_dir.join(format!("{}.ico", base_name));
                        let _ = std::fs::copy(src_path, &ico_path);
                        ico_path
                    } else {
                        let ext = std::path::Path::new(src_path)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("png");
                        let dest = cloud_dir.join(format!("{}.{}", base_name, ext));
                        if let Err(e) = std::fs::copy(src_path, &dest) {
                            eprintln!("Failed to copy {}: {}", asset, e);
                        }
                        dest
                    }
                }
                PendingImage::Bytes(data) => {
                    let data_ref: &[u8] = data;
                    let ext = if ira_parser::is_ico_data(data_ref) {
                        "ico"
                    } else if data_ref.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
                        "png"
                    } else if data_ref.starts_with(&[0xFF, 0xD8, 0xFF]) {
                        "jpg"
                    } else if data_ref.starts_with(b"RIFF")
                        && data_ref.len() > 11
                        && &data_ref[8..12] == b"WEBP"
                    {
                        "webp"
                    } else {
                        "png"
                    };
                    let dest = cloud_dir.join(format!("{}.{}", base_name, ext));
                    if let Err(e) = std::fs::write(&dest, data_ref) {
                        eprintln!("Failed to write {}: {}", asset, e);
                    }
                    dest
                }
            };
            if dest.is_file() {
                pending_images.push((base_name.to_string(), dest, max_w, max_h));
            }
        }

        for (base_name, dest, max_w, max_h) in &pending_images {
            let ext = dest
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "webp" && ext != "jpg" {
                ira_parser::convert_to_lossless_webp(dest);
            }
            let small_base = format!("{}_small", base_name);
            ira_parser::remove_image_variants(&cloud_dir, &small_base);
            ira_parser::ensure_small_image(&cloud_dir, base_name, *max_w, *max_h);
        }

        let base_names: Vec<String> = pending_images.into_iter().map(|(b, _, _, _)| b).collect();
        let _ = tx.send(base_names);
    });
}

fn process_pending_images_background(params: &SaveGameSettingsParams, db: &ira_db::DbConn) -> bool {
    let pc = params.pending_copies.borrow();
    if pc.is_empty() {
        return false;
    }

    if let Some(g) = params
        .state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == params.db_id)
        .cloned()
    {
        for path in [
            &g.icon_path,
            &g.hero_image_path,
            &g.grid_path,
            &g.header_path,
            &g.logo_path,
        ] {
            if !path.is_empty() {
                ira_images::invalidate_texture(path);
            }
        }
    }
    let game = params
        .state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == params.db_id)
        .cloned();
    let cloud_dir = game
        .as_ref()
        .map(|g| ira_parser::game_data_dir(&params.save_dir, g))
        .unwrap_or_default();
    let _ = std::fs::create_dir_all(&cloud_dir);

    let image_list: Vec<(String, PendingImage)> = pc
        .iter()
        .filter(|(k, _)| !k.starts_with("__"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let has_image_assets = !image_list.is_empty();
    drop(pc);
    if !has_image_assets {
        return false;
    }

    let (tx, rx) = mpsc::channel::<Vec<String>>();
    let state_cb = params.state.clone();
    let win_cb = params.win.downgrade();
    let var_widgets_cb = params.var_widgets.clone();
    let db_cb = db.clone();
    let db_id_cb = params.db_id;
    let save_dir_cb = params.save_dir.clone();
    let cloud_dir_cb = cloud_dir.clone();

    glib::idle_add_local(move || {
        let base_names = match rx.try_recv() {
            Ok(names) => names,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(win) = win_cb.upgrade() {
                    win.close();
                }
                return glib::ControlFlow::Break;
            }
        };
        for base_name in &base_names {
            let webp = cloud_dir_cb.join(format!("{}.webp", base_name));
            if webp.is_file() {
                ira_images::invalidate_texture(&webp.to_string_lossy());
            }
            let small = cloud_dir_cb.join(format!("{}_small.webp", base_name));
            if small.is_file() {
                ira_images::invalidate_texture(&small.to_string_lossy());
            }
        }
        if let Ok(Some(entry)) = ira_db::find_by_db_id(&db_cb, db_id_cb) {
            if let Ok(reloaded) = crate::game_loader::load_game(&entry, &save_dir_cb) {
                if let Some(g) = state_cb
                    .borrow_mut()
                    .games
                    .iter_mut()
                    .find(|g| g.db_id == db_id_cb)
                {
                    g.icon_path = reloaded.icon_path;
                    g.hero_image_path = reloaded.hero_image_path;
                    g.grid_path = reloaded.grid_path;
                    g.header_path = reloaded.header_path;
                    g.logo_path = reloaded.logo_path;
                }
            }
        }

        super::sidebar::rebuild_sidebar(&state_cb);
        let games = state_cb.borrow().games.clone();
        for game in games.iter().filter(|g| g.db_id == db_id_cb) {
            super::helpers::replace_grid_game(&state_cb, game);
        }
        refresh_selected_base_game(&state_cb, db_id_cb);

        super::edit_game_variants::save_variants(&db_cb, db_id_cb, &var_widgets_cb);
        let _ = state_cb
            .borrow()
            .sender
            .send(crate::AppMessage::VariantsChanged(db_id_cb));
        if let Some(win) = win_cb.upgrade() {
            win.close();
        }
        glib::ControlFlow::Break
    });

    spawn_image_copy_thread(image_list, cloud_dir, params.db_id, tx);
    true
}

fn finish_save(params: &SaveGameSettingsParams, db: &ira_db::DbConn) {
    if !params.pending_copies.borrow().is_empty() {
        if let Ok(Some(entry)) = ira_db::find_by_db_id(db, params.db_id) {
            if let Ok(reloaded) = crate::game_loader::load_game(&entry, &params.save_dir) {
                if let Some(g) = params
                    .state
                    .borrow_mut()
                    .games
                    .iter_mut()
                    .find(|g| g.db_id == params.db_id)
                {
                    g.icon_path = reloaded.icon_path;
                    g.hero_image_path = reloaded.hero_image_path;
                    g.grid_path = reloaded.grid_path;
                    g.header_path = reloaded.header_path;
                    g.logo_path = reloaded.logo_path;
                }
            }
        }
    }

    super::sidebar::rebuild_sidebar(&params.state);
    super::grid_view::refresh_grid_store(&params.state);

    refresh_selected_base_game(&params.state, params.db_id);

    super::edit_game_variants::save_variants(db, params.db_id, &params.var_widgets);
    let _ = params
        .state
        .borrow()
        .sender
        .send(crate::AppMessage::VariantsChanged(params.db_id));
    params.win.close();
}

pub(super) fn save_game_settings(params: SaveGameSettingsParams) {
    let title = params.title_entry.text().to_string();
    let sort_title = params.sort_entry.text().to_string();
    let db = params.state.borrow().db.clone();
    let target_kind = selected_game_kind(&params);

    if params.game_kind.is_managed_pc() && target_kind != params.game_kind {
        if let Err(e) = ira_db::update_game_kind(&db, params.db_id, target_kind) {
            eprintln!("Failed to update game runtime: {}", e);
        }
    }

    save_title_and_sort(&db, params.db_id, &params.title_entry, &params.sort_entry);

    if let Some(ref folder_row) = params.game_folder_entry {
        let folder = folder_row.text().to_string();
        if let Err(e) = ira_db::update_game_folder(&db, params.db_id, &folder) {
            eprintln!("Failed to update game folder: {}", e);
        }
    } else {
        let entry = ira_db::find_by_db_id(&db, params.db_id).ok().flatten();
        let existing_folder = entry
            .as_ref()
            .map(|e| e.game_folder.clone())
            .unwrap_or_default();
        if existing_folder.is_empty() {
            let (launch, _wine, _) = build_launch_config_and_wine(&params);
            let cfg = params.state.borrow().cfg.clone();
            let title = params.title_entry.text().to_string();
            let install_dir = String::new();
            let game_folders = cfg.all_game_folders();
            if let Some(detected) = ira_platforms::game_folder::detect_game_folder(
                &launch.exe,
                &game_folders,
                &install_dir,
                &title,
            ) {
                if let Err(e) =
                    ira_db::update_game_folder(&db, params.db_id, &detected.to_string_lossy())
                {
                    eprintln!("Failed to auto-detect game folder: {}", e);
                }
            }
        }
    }

    let app_id_result = save_app_id(&db, &params);

    save_version_and_overrides(&db, &params);

    let (launch, wine, new_profile_id) = build_launch_config_and_wine(&params);
    if let Err(e) = ira_db::save_game_config(&db, params.db_id, &launch, &wine, new_profile_id) {
        eprintln!("Failed to save game config: {}", e);
    }
    if params.launch_config_widgets.is_some() {
        apply_wine_registry(&params.old_wine, &wine);
    }

    let has_images = process_pending_images_background(&params, &db);

    handle_unmatch(&db, &params);
    save_logo_settings(&db, &params);
    save_dlc_config(&params);
    save_language_config(&params);

    // Apply pending API emulator uninstall
    if let Some(ref pu) = params.pending_emu_uninstall {
        if *pu.borrow() {
            let exe = params.game_exe.clone();
            let game_folder = params.game_folder.clone();
            let ts = params.trophy_source;
            let result = if ts == TrophySource::Gse {
                ira_platforms::api_emulators::uninstall_gse(&exe, &game_folder)
            } else {
                ira_platforms::api_emulators::uninstall_nge(&exe, &game_folder)
            };
            match result {
                Ok(()) => {
                    if let Err(e) = ira_db::set_api_dll_folder(&db, params.db_id, "") {
                        eprintln!("Failed to clear API DLL folder cache: {}", e);
                    }
                }
                Err(e) => eprintln!("Failed to uninstall API emulator: {}", e),
            }
        }
    }

    update_game_state_in_memory(&params, &title, &sort_title, &app_id_result);
    update_game_names(&params.state, &app_id_result, &params.app_id, &title);

    if has_images {
        return;
    }
    finish_save(&params, &db);
}
