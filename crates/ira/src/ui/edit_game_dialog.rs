use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::rc::Rc;
use adw::prelude::*;
use ira_models::{GameLaunchConfig, WineConfig};
use super::state::SharedState;
use super::add_game_dialog::collect_env_vars;
use super::edit_game_launch::build_launch_config_page;
use super::edit_game_pages::*;

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

pub fn show_edit_game_dialog(state: &SharedState, db_id: i64) {
    let (game, config, app_default_wine) = {
        let s = state.borrow();
        let game = s.games.iter().find(|g| g.db_id == db_id).cloned();
        let config = ira_db::get_game_config(&s.db, db_id).ok().flatten();
        let app_default_wine = s.cfg.default_wine_config.clone();
        (game, config, app_default_wine)
    };
    let Some(game) = game else { return };
    let has_config = config.is_some();
    let (saved_launch, mut saved_wine, saved_profile_id) = config.clone().unwrap_or_default();

    if !has_config {
        saved_wine = WineConfig::default();
    } else {
        saved_wine = saved_wine.merge_with_default(&app_default_wine);
    }

    let parent = state.borrow().window.clone();
    let save_dir = state.borrow().save_dir.clone();
    let app_details = crate::game_loader::read_app_details(&save_dir, &game.app_id);

    let layout = super::helpers::dialog_layout(&parent);
    layout.window.set_deletable(false);
    layout.stack.set_hexpand(true);
    layout.header.set_title_widget(Some(&gtk4::Label::new(Some(&format!("{} [{}]", game.name, game.db_id)))));

    let win = layout.window;
    let sidebar = layout.sidebar;
    let stack = layout.stack;
    let content_area = layout.content_area;

    // --- General page ---
    let languages = app_details.as_ref().map(|d| d.languages.clone()).unwrap_or_default();
    let pending_copies: Rc<RefCell<HashMap<String, String>>> = Default::default();
    let (general_page, title_entry, sort_entry, pending_version, app_id_entry, language_row, pending_ra_core, pending_emulator, ra_container) =
        super::game_settings::build_game_general_page(state, &game, &win, &languages, &pending_copies);
    sidebar.append(&super::settings_dialog::settings_sidebar_row("preferences-system-symbolic", "General"));
    stack.add_named(&general_page, Some("general"));

    // --- Profile dropdown (only when wine config exists and wine is enabled) ---
    let profiles = ira_db::get_all_profiles(&state.borrow().db).unwrap_or_default();
    let profile_row = build_profile_dropdown(has_config, saved_wine.enabled, saved_profile_id, &profiles, &general_page, state, &win);

    // --- Launch Config + Wine Config (not for steam/ps4/retro) ---
    let show_launch_config = game.kind != ira_models::GameKind::Steam && game.kind != ira_models::GameKind::Ps4 && game.kind != ira_models::GameKind::Retro;
    let launch_config_widgets = if show_launch_config {
        build_launch_config_page(&saved_launch, &win, &sidebar, &stack, true)
    } else {
        None
    };

    let show_wine_tabs = game.kind == ira_models::GameKind::Wine && saved_wine.enabled;
    let wine_widgets_opt = if show_wine_tabs {
        let (wine_pages, ww) = crate::ui::wine_config_widget::build_wine_config_pages(&saved_wine, Some(&app_default_wine));
        for wp in &wine_pages {
            sidebar.append(&super::settings_dialog::settings_sidebar_row(wp.icon, wp.label));
            stack.add_named(&wp.page, Some(wp.label));
        }
        Some(ww)
    } else {
        None
    };

    if show_launch_config || show_wine_tabs {
        sidebar.append(&super::settings_dialog::sidebar_separator());
    }

    // --- Images page ---
    if !game.app_id.is_empty() {
        let images_page = super::image_manager::build_image_manager_content_with_drafts(
            state, &game, &win, Some(pending_copies.clone()),
        );
        sidebar.append(&super::settings_dialog::settings_sidebar_row("image-x-generic-symbolic", "Images"));
        stack.add_named(&images_page, Some("images"));
    }

    // --- Logo page ---
    let logo_controls: Option<(Rc<RefCell<String>>, gtk4::Adjustment)> =
        if let Some((logo_page, selected_pos, size_adj)) = super::game_logo::build_game_logo_page(&game) {
            sidebar.append(&super::settings_dialog::settings_sidebar_row("preferences-desktop-wallpaper-symbolic", "Logo"));
            stack.add_named(&logo_page, Some("logo"));
            Some((selected_pos, size_adj))
        } else {
            None
        };

    // --- DLC page (not for Steam — can't know which DLCs the user owns) ---
    let dlc_switches = if game.kind != ira_models::GameKind::Steam {
        build_dlc_page(&app_details, &sidebar, &stack)
    } else {
        Vec::new()
    };

    // --- API Emulator page ---
    let emu_save_dir = state.borrow().save_dir.clone();
    build_api_emulator_page(
        super::edit_game_pages::ApiEmuPageParams {
            emu_exe: &saved_launch.exe,
            emu_trophy_source: game.trophy_source,
            emu_app_id: &game.app_id,
            save_dir: &emu_save_dir,
        },
        state,
        &languages,
        &sidebar,
        &stack,
    );

    // --- Variants page ---
    let var_widgets = build_variants_page(state, db_id, game.kind, has_config, &sidebar, &stack);

    // --- Sidebar navigation ---
    let stack_clone = stack.clone();
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            if let Some(child) = row.child() {
                if let Some(hbox) = child.downcast_ref::<gtk4::Box>() {
                    if let Some(sibling) = hbox.last_child() {
                        if let Some(label) = sibling.downcast_ref::<gtk4::Label>() {
                            let page_id = match label.text().as_str() {
                                "General" => "general",
                                "Launch Config" => "launch",
                                "Advanced" => "advanced",
                                "Performance" => "Performance",
                                "Graphics" => "Graphics",
                                "Wine Advanced" => "Wine Advanced",
                                "Images" => "images",
                                "Logo" => "logo",
                                "DLC" => "dlc",
                                "API Emulator" => "api_emulator",
                                "Variants" => "variants",
                                _ => "general",
                            };
                            stack_clone.set_visible_child_name(page_id);
                        }
                    }
                }
            }
        }
    });

    if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }

    // --- Save / Cancel ---
    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(16);
    btn_row.set_margin_end(16);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(12);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());

    let save_btn = gtk4::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");

    // --- Save handler ---
    let state_clone = state.clone();
    let win_s = win.clone();
    let db_id_s = db_id;
    let app_id = game.app_id.clone();
    let trophy_source = game.trophy_source;
    let game_kind = game.kind;
    let var_widgets_save = var_widgets.clone();
    let save_dir_c = save_dir.clone();
    let logo_controls_c = logo_controls.clone();
    let dlc_switches_c = dlc_switches.clone();
    let pending_copies_c = pending_copies.clone();
    let old_wine = saved_wine.clone();
    let app_default_wine_c = app_default_wine.clone();
    let game_exe = saved_launch.exe.clone();
    let language_row_c = language_row.clone();
    let languages_c = languages.clone();
    let saved_platform_id = game.platform_id.clone();

    save_btn.connect_clicked(move |_| {
        let title = title_entry.text().to_string();
        let sort_title = sort_entry.text().to_string();

        let db = state_clone.borrow().db.clone();

        if let Err(e) = ira_db::update_game_title(&db, db_id_s, &title) {
            eprintln!("Failed to update game: {}", e);
        }
        if let Err(e) = ira_db::update_sort_title(&db, db_id_s, &sort_title) {
            eprintln!("Failed to update sort title: {}", e);
        }

        let mut app_id_changed = false;
        let mut new_app_id_val = String::new();
        if let Some(ref app_id_row) = app_id_entry {
            let new_id = app_id_row.text().to_string();
            if new_id != app_id {
                app_id_changed = true;
                new_app_id_val = new_id.clone();
                let ts = if new_id.is_empty() { ira_models::TrophySource::Empty } else { trophy_source };
                let pid = if game_kind == ira_models::GameKind::Ps4 || game_kind == ira_models::GameKind::Retro {
                    &saved_platform_id
                } else if new_id.is_empty() { "" } else { &new_id };
                let (steam_id, game_id): (&str, &str) = if game_kind == ira_models::GameKind::Ps4 || game_kind == ira_models::GameKind::Retro { ("", &new_id) } else { (&new_id, "") };
                if let Err(e) = ira_db::update_game_ids(&db, db_id_s, steam_id, game_id, ts, pid) {
                    eprintln!("Failed to update app ID: {}", e);
                }
            }
        }

        if let Some(ver) = pending_version.borrow().as_ref() {
            let _ = ira_db::set_shadps4_version(&db, db_id_s, ver);
        }

        if let Some(core) = pending_ra_core.borrow().as_ref() {
            let _ = ira_db::set_ra_core(&db, db_id_s, core);
        }
        if let Some(emu) = pending_emulator.borrow().as_ref() {
            let _ = ira_db::set_emulator_override(&db, db_id_s, emu);
        }

        // Save launch config + wine config
        if let Some(ref lc) = launch_config_widgets {
            let launch = GameLaunchConfig {
                exe: lc.exe_entry.text().to_string(),
                args: lc.args_entry.text().to_string(),
                working_dir: lc.wd_entry.text().to_string(),
                env_vars: collect_env_vars(&lc.env_vars_box),
                ld_preload: lc.ld_preload_entry.text().to_string(),
                ld_library_path: lc.ld_path_entry.text().to_string(),
            };
            let mut wine = wine_widgets_opt.as_ref().map_or(WineConfig::default(), |ww| ww.to_wine_config());

            if wine.dll_overrides != app_default_wine_c.dll_overrides {
                if !wine.overridden_fields.contains(&"dll_overrides".to_string()) {
                    wine.overridden_fields.push("dll_overrides".to_string());
                }
            } else {
                wine.overridden_fields.retain(|f| f != "dll_overrides");
            }
            if wine.wine_env_vars != app_default_wine_c.wine_env_vars {
                if !wine.overridden_fields.contains(&"wine_env_vars".to_string()) {
                    wine.overridden_fields.push("wine_env_vars".to_string());
                }
            } else {
                wine.overridden_fields.retain(|f| f != "wine_env_vars");
            }
            let new_profile_id = if let Some(ref profile_row) = profile_row {
                if profile_row.selected() > 0 {
                    profiles.get((profile_row.selected() - 1) as usize).map(|p| p.id)
                } else {
                    None
                }
            } else {
                saved_profile_id
            };
            let _ = ira_db::save_game_config(&db, db_id_s, &launch, &wine, new_profile_id);

            if wine.enabled {
                let reg_changed = wine.mouse_warp_override != old_wine.mouse_warp_override
                    || wine.virtual_desktop != old_wine.virtual_desktop
                    || wine.virtual_desktop_res != old_wine.virtual_desktop_res
                    || wine.dpi_enabled != old_wine.dpi_enabled
                    || wine.dpi != old_wine.dpi
                    || wine.show_crash_dialogs != old_wine.show_crash_dialogs
                    || wine.audio != old_wine.audio;

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

        // Pending image copies — copy files on main thread (fast), convert in background
        {
            let pc = pending_copies_c.borrow();
            let _s = tracing::info_span!("pending_image_copies", db_id = db_id_s, count = pc.len()).entered();

            // Invalidate old textures before replacing files so grid items reload
            if !pc.is_empty() {
                if let Some(g) = state_clone.borrow().games.iter().find(|g| g.db_id == db_id_s).cloned() {
                    for path in [&g.icon_path, &g.hero_image_path, &g.grid_path, &g.header_path, &g.logo_path] {
                        if !path.is_empty() {
                            ira_images::invalidate_texture(path);
                        }
                    }
                }
            }

            let is_steam = game.trophy_source.has_steam_enrichment();
            let cloud_dir = if game.kind == ira_models::GameKind::Retro {
                ira_parser::retro_data_dir(&save_dir_c, db_id_s)
            } else if is_steam {
                ira_parser::data_dir(&save_dir_c, &app_id)
            } else if !game.sgdb_id.is_empty() {
                ira_parser::sgdb_data_dir(&save_dir_c, &game.sgdb_id)
            } else if game.kind == ira_models::GameKind::Ps4 {
                ira_parser::ps4_data_dir(&save_dir_c, &app_id)
            } else {
                ira_parser::data_dir(&save_dir_c, &app_id)
            };
            let _ = std::fs::create_dir_all(&cloud_dir);
            let mut pending_images: Vec<(String, std::path::PathBuf, u32, u32)> = Vec::new();
            for (asset, src_path) in pc.iter() {
                if asset == "__unmatch__" { continue; }
                if asset.starts_with("__ra_unmatch_") { continue; }
                let base_name = match asset.as_str() {
                    "icon" => "icon",
                    "hero" => "hero",
                    "grid" => "vertical",
                    "header" => "header",
                    "logo" => "logo",
                    _ => continue,
                };
                let (max_w, max_h) = match asset.as_str() {
                    "icon" => (32u32, 32u32),
                    "hero" => (1920, 620),
                    "grid" => (300, 450),
                    "header" => (460, 215),
                    "logo" => (620, 620),
                    _ => continue,
                };
                ira_parser::remove_image_variants(&cloud_dir, base_name);
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
            // Convert and generate small images in background
            if !pending_images.is_empty() {
                let cloud_dir_bg = cloud_dir.clone();
                let db_id_bg = db_id_s;
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

        // SGDB unmatch
        if pending_copies_c.borrow().contains_key("__unmatch__") {
            let _ = ira_db::set_sgdb_id(&db, db_id_s, "");
            if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id_s) {
                g.sgdb_id.clear();
            }
            pending_copies_c.borrow_mut().remove("__unmatch__");
        }

        // RA unmatch
        let ra_unmatch_key = format!("__ra_unmatch_{}", db_id_s);
        if pending_copies_c.borrow().contains_key(&ra_unmatch_key) {
            let _ = ira_db::update_game_ids(&db, db_id_s, "", "", ira_models::TrophySource::Empty, &saved_platform_id);
            let _ = ira_db::set_manual_unmatch(&db, db_id_s, true);
            {
                let mut s = state_clone.borrow_mut();
                if let Some(g) = s.games.iter_mut().find(|g| g.db_id == db_id_s) {
                    g.app_id.clear();
                    g.trophy_source = ira_models::TrophySource::Empty;
                    g.achievements.clear();
                    g.earned_count = 0;
                    g.total_count = 0;
                    g.manual_unmatch = true;
                }
            }
            pending_copies_c.borrow_mut().remove(&ra_unmatch_key);
        }

        if let Some((ref selected_pos, ref size_adj)) = logo_controls_c {
            let pos = selected_pos.borrow().clone();
            let size = size_adj.value() as i32;
            if db_id_s != 0 {
                if let Err(e) = ira_db::set_logo_settings(&db, db_id_s, &pos, size) {
                    eprintln!("Failed to update logo settings: {}", e);
                }
            }
            if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id_s) {
                g.logo_position = pos;
                g.logo_size = size;
            }
        }

        // DLC state
        {
            let details = crate::game_loader::read_app_details(&save_dir_c, &app_id);
            if let Some(ref details) = details {
                if !dlc_switches_c.is_empty() {
                    let mut details = details.clone();
                    let dlcs_vec: Vec<_> = details.dlcs.iter_mut().collect();
                    for (i, (_, dlc)) in dlcs_vec.into_iter().enumerate() {
                        if i < dlc_switches_c.len() {
                            dlc.enabled = dlc_switches_c[i].is_active();
                        }
                    }
                    let path = ira_parser::data_dir(&save_dir_c, &app_id).join("appdetails.json");
                    if let Ok(b) = serde_json::to_vec(&details) {
                        let _ = std::fs::write(&path, b);
                    }
                    ira_platforms::api_emulators::write_dlc_configs(
                        trophy_source, &game_exe, &save_dir_c, &app_id, &details,
                    );
                }
            }
        }

        // Game language
        if let Some(ref lang_row) = language_row_c {
            let idx = lang_row.selected() as usize;
            if let Some(lang) = languages_c.get(idx) {
                ira_platforms::api_emulators::write_language_configs(
                    trophy_source, &game_exe, &save_dir_c, &app_id, lang,
                );
            }
        }

        // Update in-memory state
        if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id_s) {
            g.name = title.clone();
            g.sort_title = sort_title.clone();
            if let Some(ver) = pending_version.borrow().as_ref() {
                g.shadps4_version = ver.clone();
            }
            if let Some(core) = pending_ra_core.borrow().as_ref() {
                g.ra_core = core.clone();
            }
            if let Some(emu) = pending_emulator.borrow().as_ref() {
                g.emulator_override = emu.clone();
            }
            if app_id_changed {
                if new_app_id_val.is_empty() {
                    g.app_id.clear();
                    g.trophy_source = ira_models::TrophySource::Empty;
                    g.platform_id.clear();
                    g.achievements.clear();
                    g.earned_count = 0;
                    g.total_count = 0;
                    g.manual_unmatch = true;
                } else {
                    state_clone.borrow().game_names.lock().unwrap().remove(&app_id);
                    g.app_id = new_app_id_val.clone();
                    g.platform_id = new_app_id_val.clone();
                    g.manual_unmatch = false;
                }
            }
        }
        if !new_app_id_val.is_empty() {
            state_clone.borrow().game_names.lock().unwrap().insert(new_app_id_val.clone(), title);
        } else if app_id_changed {
            state_clone.borrow().game_names.lock().unwrap().remove(&app_id);
        }

        // Reload image paths from disk after copying pending files.
        if !pending_copies_c.borrow().is_empty() {
            if let Ok(Some(entry)) = ira_db::find_by_db_id(&db, db_id_s) {
                if let Ok(reloaded) = crate::game_loader::load_game(&entry, &save_dir_c) {
                    if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id_s) {
                        g.icon_path = reloaded.icon_path;
                        g.hero_image_path = reloaded.hero_image_path;
                        g.grid_path = reloaded.grid_path;
                        g.header_path = reloaded.header_path;
                        g.logo_path = reloaded.logo_path;
                    }
                }
            }
        }

        super::sidebar::rebuild_sidebar(&state_clone);

        // Update grid store so covers reflect new images immediately.
        {
            let store = state_clone.borrow().grid_store.clone();
            let games = state_clone.borrow().games.clone();
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

        let selected = state_clone.borrow().selected_id.clone();
        let game_after_save = if selected == db_id_s.to_string() {
            state_clone.borrow().games.iter().find(|g| g.db_id == db_id_s).cloned()
        } else {
            None
        };
        if let Some(g) = game_after_save {
            crate::ui::game_display::display_game(&g, &state_clone);
        }

        // Save variants
        {
            let _ = ira_db::delete_all_variants(&db, db_id_s);
            for vw in var_widgets_save.borrow().iter() {
                if vw._group.parent().is_none() { continue; }
                let name = vw._name.text().to_string();
                if name.is_empty() { continue; }
                let variant = ira_models::GameVariant {
                    id: 0,
                    game_id: db_id_s,
                    name,
                    exe: vw._exe.text().to_string(),
                    working_dir: vw._wd.text().to_string(),
                    args: vw._args.text().to_string(),
                    env_vars: Vec::new(),
                };
                let _ = ira_db::add_variant(&db, &variant);
            }
        }

        win_s.close();
    });

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    content_area.append(&btn_row);

    {
        let mut s = state.borrow_mut();
        s.settings_data = Some(super::state::SettingsData {
            window: win.clone(),
            stack: stack.clone(),
            db_id,
            pending_copies: pending_copies.clone(),
            ra_container,
        });
    }
    let state_close = state.clone();
    win.connect_close_request(move |_| {
        state_close.borrow_mut().settings_data = None;
        glib::Propagation::Proceed
    });

    win.present();
}

