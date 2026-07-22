use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::rc::Rc;
use adw::prelude::*;
use ira_models::{AssetType, GameLaunchConfig, TrophySource, WineConfig};
use super::add_game_dialog::collect_env_vars;
use super::edit_game_advanced::AdvancedWidgets;
use super::edit_game_launch::LaunchConfigWidgets;
use super::edit_game_variants::VarW;
use super::state::SharedState;
use super::wine_config_env_dll::collect_dll_overrides;
use super::wine_config_widget::WineConfigWidgets;

fn is_ico_bytes(path: &str) -> bool {
    if path.ends_with(".ico") {
        return true;
    }
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 4];
    f.read_exact(&mut buf).is_ok() && buf == [0x00, 0x00, 0x01, 0x00]
}

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
    pub pending_copies: Rc<RefCell<HashMap<String, String>>>,
    pub old_wine: WineConfig,
    pub app_default_wine: WineConfig,
    pub game_exe: String,
    pub language_row: Option<adw::ComboRow>,
    pub languages: Vec<String>,
    pub saved_platform_id: String,
    pub advanced_widgets: Option<AdvancedWidgets>,
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
}

pub(super) fn save_game_settings(params: SaveGameSettingsParams) {
    let title = params.title_entry.text().to_string();
    let sort_title = params.sort_entry.text().to_string();

    let db = params.state.borrow().db.clone();

    if let Err(e) = ira_db::update_game_title(&db, params.db_id, &title) {
        eprintln!("Failed to update game: {}", e);
    }
    if let Err(e) = ira_db::update_sort_title(&db, params.db_id, &sort_title) {
        eprintln!("Failed to update sort title: {}", e);
    }

    let mut app_id_changed = false;
    let mut new_app_id_val = String::new();
    if let Some(ref app_id_row) = params.app_id_entry {
        let new_id = app_id_row.text().to_string();
        if new_id != params.app_id {
            app_id_changed = true;
            new_app_id_val = new_id.clone();
            let ts = if new_id.is_empty() { TrophySource::Empty } else { params.trophy_source };
            let pid = if params.game_kind == ira_models::GameKind::Ps4 || params.game_kind == ira_models::GameKind::Ps3 || params.game_kind == ira_models::GameKind::Retro {
                &params.saved_platform_id
            } else if new_id.is_empty() { "" } else { &new_id };
            let (steam_id, game_id): (&str, &str) = if params.game_kind == ira_models::GameKind::Ps4 || params.game_kind == ira_models::GameKind::Ps3 || params.game_kind == ira_models::GameKind::Retro { ("", &new_id) } else { (&new_id, "") };
            if let Err(e) = ira_db::update_game_ids(&db, params.db_id, steam_id, game_id, ts, pid) {
                eprintln!("Failed to update app ID: {}", e);
            }
        }
    }

    if let Some(ver) = params.pending_version.borrow().as_ref() {
        if let Err(e) = ira_db::set_shadps4_version(&db, params.db_id, ver) {
            eprintln!("Failed to set shadps4 version: {}", e);
        }
    }

    if let Some(core) = params.pending_ra_core.borrow().as_ref() {
        if let Err(e) = ira_db::set_ra_core(&db, params.db_id, core) {
            eprintln!("Failed to set RA core: {}", e);
        }
    }
    if let Some(emu) = params.pending_emulator.borrow().as_ref() {
        if let Err(e) = ira_db::set_emulator_override(&db, params.db_id, emu) {
            eprintln!("Failed to set emulator override: {}", e);
        }
    }

    if let Some(ref lc) = params.launch_config_widgets {
        let env_vars = if let Some(ref aw) = params.advanced_widgets {
            collect_env_vars(&aw.env_vars_box)
        } else {
            Vec::new()
        };
        let ld_preload = if let Some(ref aw) = params.advanced_widgets {
            aw.ld_preload_entry.text().to_string()
        } else {
            String::new()
        };
        let ld_library_path = if let Some(ref aw) = params.advanced_widgets {
            aw.ld_library_path_entry.text().to_string()
        } else {
            String::new()
        };
        let launch = GameLaunchConfig {
            exe: lc.exe_entry.text().to_string(),
            args: lc.args_entry.text().to_string(),
            working_dir: lc.wd_entry.text().to_string(),
            env_vars: if params.show_wine_tabs { Vec::new() } else { env_vars.clone() },
            ld_preload,
            ld_library_path,
            pre_launch: lc.pre_launch_entry.text().to_string(),
        };
        let mut wine = params.wine_widgets.as_ref().map_or(WineConfig::default(), |ww| ww.to_wine_config());

        if params.show_wine_tabs {
            wine.wine_env_vars = env_vars;
            if let Some(ref aw) = params.advanced_widgets {
                if let Some(ref dob) = aw.dll_overrides_box {
                    wine.dll_overrides = collect_dll_overrides(dob);
                }
            }
        }

        if wine.dll_overrides != params.app_default_wine.dll_overrides {
            if !wine.overridden_fields.contains(&"dll_overrides".to_string()) {
                wine.overridden_fields.push("dll_overrides".to_string());
            }
        } else {
            wine.overridden_fields.retain(|f| f != "dll_overrides");
        }
        if wine.wine_env_vars != params.app_default_wine.wine_env_vars {
            if !wine.overridden_fields.contains(&"wine_env_vars".to_string()) {
                wine.overridden_fields.push("wine_env_vars".to_string());
            }
        } else {
            wine.overridden_fields.retain(|f| f != "wine_env_vars");
        }
        let new_profile_id = if let Some(ref lc) = params.launch_config_widgets {
            if let Some(ref profile_row) = lc.profile_row {
                if profile_row.selected() > 0 {
                    params.profiles.get((profile_row.selected() - 1) as usize).map(|p| p.id)
                } else {
                    None
                }
            } else {
                params.saved_profile_id
            }
        } else {
            params.saved_profile_id
        };
        if let Err(e) = ira_db::save_game_config(&db, params.db_id, &launch, &wine, new_profile_id) {
            eprintln!("Failed to save game config: {}", e);
        }

        if wine.enabled {
            let reg_changed = wine.mouse_warp_override != params.old_wine.mouse_warp_override
                || wine.virtual_desktop != params.old_wine.virtual_desktop
                || wine.virtual_desktop_res != params.old_wine.virtual_desktop_res
                || wine.dpi_enabled != params.old_wine.dpi_enabled
                || wine.dpi != params.old_wine.dpi
                || wine.show_crash_dialogs != params.old_wine.show_crash_dialogs
                || wine.audio != params.old_wine.audio;

            if reg_changed {
                let pfx = ira_launcher::wine_launch::wine_prefix(&wine);
                let prefix_ready = std::path::Path::new(&pfx).join("system.reg").is_file();
                if prefix_ready {
                    let wine_exe = ira_launcher::wine_launch::find_wine_binary(&wine.version, &wine.custom_wine_path);
                    if let Ok(wine_exe) = wine_exe {
                        let reg_cmds = ira_launcher::wine_launch::build_wine_reg_commands(&wine, &wine_exe);
                        let env = ira_launcher::wine_launch::build_wine_env(&wine, &wine_exe);
                        std::thread::spawn(move || {
                            for reg_cmd in reg_cmds {
                                let mut child = std::process::Command::new(&reg_cmd[0]);
                                for arg in &reg_cmd[1..] {
                                    child.arg(arg);
                                }
                                child.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
                                match child.status() {
                                    Ok(s) if !s.success() && s.code() != Some(1) => {
                                        eprintln!("Wine reg command failed (exit {:?}): {:?}", s.code(), reg_cmd);
                                    }
                                    Err(e) => eprintln!("Failed to run wine reg command: {}", e),
                                    _ => {}
                                }
                            }
                        });
                    }
                }
            }
        }
    }

    {
        let pc = params.pending_copies.borrow();
        let _s = tracing::info_span!("pending_image_copies", db_id = params.db_id, count = pc.len()).entered();

        if !pc.is_empty() {
            if let Some(g) = params.state.borrow().games.iter().find(|g| g.db_id == params.db_id).cloned() {
                for path in [&g.icon_path, &g.hero_image_path, &g.grid_path, &g.header_path, &g.logo_path] {
                    if !path.is_empty() {
                        ira_images::invalidate_texture(path);
                    }
                }
            }
        }

        let game = params.state.borrow().games.iter().find(|g| g.db_id == params.db_id).cloned();
        let cloud_dir = game.as_ref().map(|g| ira_parser::game_data_dir(&params.save_dir, g)).unwrap_or_default();
        let _ = std::fs::create_dir_all(&cloud_dir);
        let mut pending_images: Vec<(String, std::path::PathBuf, u32, u32)> = Vec::new();
        for (asset, src_path) in pc.iter() {
            if asset == "__unmatch__" { continue; }
            if asset.starts_with("__ra_unmatch_") { continue; }
            let Some(at) = AssetType::from_string(asset) else { continue; };
            let base_name = at.file_base();
            let (max_w, max_h) = at.thumb_dims();
            ira_parser::remove_image_variants(&cloud_dir, base_name);
            ira_parser::remove_image_variants(&cloud_dir, &format!("{}_small", base_name));
            let is_ico = is_ico_bytes(src_path);
            let dest = if is_ico {
                let ico_path = cloud_dir.join(format!("{}.ico", base_name));
                std::fs::copy(src_path, &ico_path).ok();
                ico_path
            } else {
                let ext = std::path::Path::new(src_path).extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("png");
                let dest = cloud_dir.join(format!("{}.{}", base_name, ext));
                if let Err(e) = std::fs::copy(src_path, &dest) {
                    eprintln!("Failed to copy {}: {}", asset, e);
                }
                dest
            };
            if dest.is_file() {
                pending_images.push((base_name.to_string(), dest, max_w, max_h));
            }
        }
        if !pending_images.is_empty() {
            let cloud_dir_bg = cloud_dir.clone();
            let db_id_bg = params.db_id;
            std::thread::spawn(move || {
                let _s = tracing::info_span!("pending_image_conversion", db_id = db_id_bg, count = pending_images.len()).entered();
                for (base_name, dest, max_w, max_h) in &pending_images {
                    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                    if ext != "webp" && ext != "jpg" {
                        ira_parser::convert_to_lossless_webp(dest);
                    }
                    let small_base = format!("{}_small", base_name);
                    ira_parser::remove_image_variants(&cloud_dir_bg, &small_base);
                    ira_parser::ensure_small_image(&cloud_dir_bg, base_name, *max_w, *max_h);
                }
                let base_names: Vec<String> = pending_images.into_iter().map(|(b, _, _, _)| b).collect();
                glib::idle_add_once(move || {
                    for base_name in &base_names {
                        let webp = cloud_dir_bg.join(format!("{}.webp", base_name));
                        if webp.is_file() {
                            ira_images::invalidate_texture(&webp.to_string_lossy());
                        }
                        let small = cloud_dir_bg.join(format!("{}_small.webp", base_name));
                        if small.is_file() {
                            ira_images::invalidate_texture(&small.to_string_lossy());
                        }
                    }
                });
            });
        }
    }

    if params.pending_copies.borrow().contains_key("__unmatch__") {
        if let Err(e) = ira_db::set_sgdb_id(&db, params.db_id, "") {
            eprintln!("Failed to clear SGDB ID: {}", e);
        }
        if let Err(e) = ira_db::set_manual_unmatch(&db, params.db_id, true) {
            eprintln!("Failed to set manual unmatch: {}", e);
        }
        if let Some(g) = params.state.borrow_mut().games.iter_mut().find(|g| g.db_id == params.db_id) {
            g.sgdb_id.clear();
            g.manual_unmatch = true;
        }
        params.pending_copies.borrow_mut().remove("__unmatch__");
    }

    let ra_unmatch_key = format!("__ra_unmatch_{}", params.db_id);
    if params.pending_copies.borrow().contains_key(&ra_unmatch_key) {
        if let Err(e) = ira_db::update_game_ids(&db, params.db_id, "", "", TrophySource::Empty, &params.saved_platform_id) {
            eprintln!("Failed to unmatch RA game: {}", e);
        }
        if let Err(e) = ira_db::set_manual_unmatch(&db, params.db_id, true) {
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

    if let Some((ref selected_pos, ref size_adj)) = params.logo_controls {
        let pos = selected_pos.borrow().clone();
        let size = size_adj.value() as i32;
        if params.db_id != 0 {
            if let Err(e) = ira_db::set_logo_settings(&db, params.db_id, &pos, size) {
                eprintln!("Failed to update logo settings: {}", e);
            }
        }
        if let Some(g) = params.state.borrow_mut().games.iter_mut().find(|g| g.db_id == params.db_id) {
            g.logo_position = pos;
            g.logo_size = size;
        }
    }

    {
        let details = crate::game_loader::read_app_details(&params.save_dir, &params.app_id);
        if let Some(ref details) = details {
            if !params.dlc_switches.is_empty() {
                let mut details = details.clone();
                let dlcs_vec: Vec<_> = details.dlcs.iter_mut().collect();
                for (i, (_, dlc)) in dlcs_vec.into_iter().enumerate() {
                    if i < params.dlc_switches.len() {
                        dlc.enabled = params.dlc_switches[i].is_active();
                    }
                }
                let path = ira_parser::data_dir(&params.save_dir, &params.app_id).join("appdetails.json");
                if let Ok(b) = serde_json::to_vec(&details) {
                    let _ = std::fs::write(&path, b);
                }
                ira_platforms::api_emulators::write_dlc_configs(
                    params.trophy_source, &params.game_exe, &params.save_dir, &params.app_id, &details,
                );
            }
        }
    }

    if let Some(ref lang_row) = params.language_row {
        let idx = lang_row.selected() as usize;
        if let Some(lang) = params.languages.get(idx) {
            ira_platforms::api_emulators::write_language_configs(
                params.trophy_source, &params.game_exe, &params.save_dir, &params.app_id, lang,
            );
        }
    }

    if let Some(g) = params.state.borrow_mut().games.iter_mut().find(|g| g.db_id == params.db_id) {
        g.name = title.clone();
        g.sort_title = sort_title.clone();
        if let Some(ver) = params.pending_version.borrow().as_ref() {
            g.shadps4_version = ver.clone();
        }
        if let Some(core) = params.pending_ra_core.borrow().as_ref() {
            g.ra_core = core.clone();
        }
        if let Some(emu) = params.pending_emulator.borrow().as_ref() {
            g.emulator_override = emu.clone();
        }
        if app_id_changed {
            if new_app_id_val.is_empty() {
                g.app_id.clear();
                g.trophy_source = TrophySource::Empty;
                g.platform_id.clear();
                g.achievements.clear();
                g.earned_count = 0;
                g.total_count = 0;
                g.manual_unmatch = true;
            } else {
                params.state.borrow().game_names.lock().unwrap().remove(&params.app_id);
                g.app_id = new_app_id_val.clone();
                g.platform_id = new_app_id_val.clone();
                g.manual_unmatch = false;
            }
        }
    }
    if !new_app_id_val.is_empty() {
        params.state.borrow().game_names.lock().unwrap().insert(new_app_id_val.clone(), title);
    } else if app_id_changed {
        params.state.borrow().game_names.lock().unwrap().remove(&params.app_id);
    }

    if !params.pending_copies.borrow().is_empty() {
        if let Ok(Some(entry)) = ira_db::find_by_db_id(&db, params.db_id) {
            if let Ok(reloaded) = crate::game_loader::load_game(&entry, &params.save_dir) {
                if let Some(g) = params.state.borrow_mut().games.iter_mut().find(|g| g.db_id == params.db_id) {
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

    {
        let store = params.state.borrow().grid_store.clone();
        let games = params.state.borrow().games.clone();
        for i in 0..store.n_items() {
            if let Some(item) = store.item(i).and_then(|o| o.downcast::<super::game_item::GameItem>().ok()) {
                if let Some(gi) = item.game() {
                    if let Some(g) = games.iter().find(|g| g.db_id == gi.db_id) {
                        store.splice(i, 1, &[super::game_item::GameItem::new(g)]);
                    }
                }
            }
        }
    }

    let selected = params.state.borrow().selected_id.clone();
    let game_after_save = if selected == params.db_id.to_string() {
        params.state.borrow().games.iter().find(|g| g.db_id == params.db_id).cloned()
    } else {
        None
    };
    if let Some(g) = game_after_save {
        super::game_display::display_game(&g, &params.state);
    }

    super::edit_game_variants::save_variants(&db, params.db_id, &params.var_widgets);

    let _ = params.state.borrow().sender.send(crate::AppMessage::VariantsChanged(params.db_id));
    params.win.close();
}
