use super::css::*;
use super::edit_game_controller::{build_controller_page, ControllerWidgets};
use super::edit_game_launch::{build_launch_config_page, LaunchConfigWidgets};
use super::edit_game_overlay::{build_overlay_page, OverlayWidgets};
use super::edit_game_pages::{build_api_emulator_page, build_dlc_page};
use super::edit_game_save::{save_game_settings, SaveGameSettingsParams};
use super::edit_game_system::{build_system_page, SystemWidgets};
use super::edit_game_variants::build_variants_page;
use super::state::{PendingImage, SharedState, SgdbAssetsCacheEntry};
use super::wine_config_widget::{build_wine_config_pages, WineConfigWidgets};
use crate::Game;
use adw::prelude::*;
use glib::clone::Downgrade;
use gtk4::prelude::IsA;
use ira_models::{AppDetails, GameLaunchConfig, WineConfig, WineProfile};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

struct LaunchWineAdvancedCtx {
    launch_config_widgets: Option<LaunchConfigWidgets>,
    system_widgets: Option<SystemWidgets>,
    overlay_widgets: Option<OverlayWidgets>,
    controller_widgets: Option<ControllerWidgets>,
    show_wine_tabs: bool,
    wine_widgets_opt: Option<WineConfigWidgets>,
    profiles: Vec<WineProfile>,
}

struct LaunchWineParams<'a> {
    saved_launch: &'a GameLaunchConfig,
    saved_wine: &'a WineConfig,
    has_config: bool,
    saved_profile_id: Option<i64>,
    app_default_wine: &'a WineConfig,
}

fn extract_game_and_config(
    state: &SharedState,
    db_id: i64,
) -> Option<(
    GameLaunchConfig,
    WineConfig,
    Option<i64>,
    WineConfig,
    Game,
    bool,
)> {
    let (game, config, app_default_wine) = {
        let s = state.borrow();
        let game = s.games.iter().find(|g| g.db_id == db_id).cloned();
        let config = ira_db::get_game_config(&s.db, db_id).ok().flatten();
        let app_default_wine = s.cfg.default_wine_config.clone();
        (game, config, app_default_wine)
    };
    let game = game?;
    let has_config = config.is_some();
    let (saved_launch, mut saved_wine, saved_profile_id) = config.unwrap_or_default();
    if !has_config {
        saved_wine = WineConfig::default();
    } else {
        saved_wine = saved_wine.merge_with_default(&app_default_wine);
    }
    Some((
        saved_launch,
        saved_wine,
        saved_profile_id,
        app_default_wine,
        game,
        has_config,
    ))
}

fn create_dialog_window(
    parent: &impl IsA<gtk4::Window>,
    game: &Game,
) -> (adw::Window, gtk4::ListBox, gtk4::Stack, gtk4::Box) {
    let layout = super::helpers::dialog_layout(parent);
    layout.window.set_deletable(false);
    layout.stack.set_hexpand(true);
    layout
        .header
        .set_title_widget(Some(&gtk4::Label::new(Some(&game.name))));
    (
        layout.window,
        layout.sidebar,
        layout.stack,
        layout.content_area,
    )
}

fn build_launch_wine_advanced_pages(
    state: &SharedState,
    game: &Game,
    params: &LaunchWineParams,
    win: &adw::Window,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
) -> LaunchWineAdvancedCtx {
    let saved_launch = params.saved_launch;
    let saved_wine = params.saved_wine;
    let has_config = params.has_config;
    let saved_profile_id = params.saved_profile_id;
    let app_default_wine = params.app_default_wine;
    let show_launch_config = game.kind.is_managed_pc() || game.kind == ira_models::GameKind::Other;
    let profiles = ira_db::get_all_profiles(&state.borrow().db).unwrap_or_default();
    let save_dir = state.borrow().save_dir.clone();
    let registry = state.borrow().controller_registry.clone();

    let overlay_source_id = match game.kind {
        ira_models::GameKind::Steam => Some("steam"),
        ira_models::GameKind::Retro => Some(game.platform_id.as_str()),
        ira_models::GameKind::Ps4 => Some("ps4"),
        ira_models::GameKind::Ps3 => Some("ps3"),
        ira_models::GameKind::PsVita => Some("psvita"),
        ira_models::GameKind::WiiU => Some("wiiu"),
        _ => None,
    };
    let (
        overlay_default,
        gamemode_default,
        mangohud_default,
        gamescope_default,
        gamescope_w_default,
        gamescope_h_default,
        gamescope_fps_default,
        gamescope_upscaling_default,
        gpu_default,
    ) = {
        let s = state.borrow();
        let gs_default = s.cfg.default_system.gamescope;
        let gs = overlay_source_id
            .and_then(|id| s.cfg.overlay.source_gamescope.get(id).copied())
            .unwrap_or(gs_default);
        let overlay_def =
            overlay_source_id.map_or(s.cfg.overlay.enabled, |id| s.cfg.overlay.source_enabled(id));
        (
            overlay_def,
            s.cfg.default_system.gamemode,
            s.cfg.default_system.mangohud,
            gs,
            s.cfg.default_system.gamescope_w,
            s.cfg.default_system.gamescope_h,
            s.cfg.default_system.gamescope_fps,
            s.cfg.default_system.gamescope_upscaling.clone(),
            s.cfg.default_system.gpu.clone(),
        )
    };

    // Launch Config page — only for non-emulator games (Wine, Linux native, etc.)
    let launch_config_widgets = if show_launch_config {
        build_launch_config_page(super::edit_game_launch::LaunchConfigParams {
            launch: saved_launch,
            win,
            sidebar,
            stack,
            has_config,
            saved_wine_enabled: saved_wine.enabled,
            saved_profile_id,
            profiles: &profiles,
            state,
            game_slug: &game.slug,
        })
    } else {
        None
    };

    // System page — shown for ALL games (Wine, emulator, native, etc.)
    let system_widgets = Some(build_system_page(
        super::edit_game_system::SystemPageParams {
            launch: saved_launch,
            gpu_default: &gpu_default,
            gamemode_default,
            mangohud_default,
            gamescope_default,
            gamescope_w_default,
            gamescope_h_default,
            gamescope_fps_default,
            gamescope_upscaling_default,
            sidebar,
            stack,
        },
    ));

    // Overlay page — shown for ALL games
    let overlay_widgets = Some(build_overlay_page(
        super::edit_game_overlay::OverlayPageParams {
            launch: saved_launch,
            overlay_default,
            sidebar,
            stack,
        },
    ));
    let controller_widgets = Some(build_controller_page(
        super::edit_game_controller::ControllerPageParams {
            launch: saved_launch,
            game,
            save_dir: &save_dir,
            sidebar,
            stack,
            registry,
        },
    ));

    // Wine pages — only for Wine games with wine enabled
    let show_wine_tabs = game.kind.is_managed_pc();
    let wine_widgets_opt = if show_wine_tabs {
        let (wine_pages, ww) = build_wine_config_pages(saved_wine, Some(app_default_wine), &save_dir);
        for wp in &wine_pages {
            sidebar.append(&super::settings_dialog::settings_sidebar_row(
                wp.icon, &wp.label, wp.page_id,
            ));
            stack.add_named(&wp.page, Some(wp.page_id));
        }
        Some(ww)
    } else {
        None
    };

    if show_launch_config || show_wine_tabs {
        sidebar.append(&super::settings_dialog::sidebar_separator());
    }

    LaunchWineAdvancedCtx {
        launch_config_widgets,
        system_widgets,
        overlay_widgets,
        controller_widgets,
        show_wine_tabs,
        wine_widgets_opt,
        profiles,
    }
}

fn setup_sidebar_navigation(sidebar: &gtk4::ListBox, stack: &gtk4::Stack) {
    let stack_weak = Downgrade::downgrade(stack);
    sidebar.connect_row_selected(move |_, row| {
        let Some(stack) = stack_weak.upgrade() else {
            return;
        };
        if let Some(row) = row {
            let page_id = row.widget_name().to_string();
            stack.set_visible_child_name(&page_id);
        }
    });
    if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }
}

struct DialogContent {
    game: Game,
    win: adw::Window,
    sidebar: gtk4::ListBox,
    stack: gtk4::Stack,
    content_area: gtk4::Box,
    app_details: Option<AppDetails>,
    save_dir: String,
    has_config: bool,
}

struct DialogConfig {
    saved_launch: GameLaunchConfig,
    saved_wine: WineConfig,
    saved_profile_id: Option<i64>,
    app_default_wine: WineConfig,
}

fn build_dialog_contents(
    state: SharedState,
    content: DialogContent,
    config: DialogConfig,
    db_id: i64,
) {
    let DialogContent {
        game,
        win,
        sidebar,
        stack,
        content_area,
        app_details,
        save_dir,
        has_config,
    } = content;
    let DialogConfig {
        saved_launch,
        saved_wine,
        saved_profile_id,
        app_default_wine,
    } = config;
    let mut languages = app_details
        .as_ref()
        .map(|d| d.languages.clone())
        .unwrap_or_default();
    languages
        .sort_by(|a, b| ira_models::steam_language_name(a).cmp(ira_models::steam_language_name(b)));
    let pending_copies: Rc<RefCell<HashMap<String, PendingImage>>> = Default::default();
    let sgdb_cache: Rc<RefCell<HashMap<String, SgdbAssetsCacheEntry>>> = Default::default();
    let (
        general_page,
        title_entry,
        sort_entry,
        pending_version,
        app_id_entry,
        language_row,
        pending_ra_core,
        pending_emulator,
        ra_container,
        game_folder_entry,
        migrate_btn,
        runtime_row,
    ) = super::game_settings::build_game_general_page(
        &state,
        &game,
        &win,
        &languages,
        &pending_copies,
    );
    sidebar.append(&super::settings_dialog::settings_sidebar_row(
        "emblem-system-symbolic",
        "General",
        "general",
    ));
    stack.add_named(&general_page, Some("general"));

    let lwa = build_launch_wine_advanced_pages(
        &state,
        &game,
        &LaunchWineParams {
            saved_launch: &saved_launch,
            saved_wine: &saved_wine,
            has_config,
            saved_profile_id,
            app_default_wine: &app_default_wine,
        },
        &win,
        &sidebar,
        &stack,
    );

    {
        let images_page = super::image_manager::build_image_manager_content_with_drafts(
            &state,
            &game,
            &win,
            Some(pending_copies.clone()),
            Some(sgdb_cache.clone()),
        );
        sidebar.append(&super::settings_dialog::settings_sidebar_row(
            "image-x-generic-symbolic",
            &crate::tr!("Images"),
            "images",
        ));
        stack.add_named(&images_page, Some("images"));
    }

    let logo_controls: Option<(Rc<RefCell<String>>, gtk4::Adjustment)> = {
        let steam_reset = if game.trophy_source.has_steam_enrichment() && !game.app_id.is_empty() {
            let s = state.borrow();
            Some(super::game_logo::SteamLogoReset {
                steam: s.steam.clone(),
                app_id: game.app_id.clone(),
                db: s.db.clone(),
                db_id: game.db_id,
            })
        } else {
            None
        };
        if let Some((logo_page, selected_pos, size_adj, _modified)) =
            super::game_logo::build_game_logo_page(&game, false, steam_reset)
        {
            sidebar.append(&super::settings_dialog::settings_sidebar_row(
                "preferences-desktop-wallpaper-symbolic",
                "Logo",
                "logo",
            ));
            stack.add_named(&logo_page, Some("logo"));
            Some((selected_pos, size_adj))
        } else {
            None
        }
    };

    let dlc_switches = if game.kind != ira_models::GameKind::Steam {
        build_dlc_page(&app_details, &sidebar, &stack)
    } else {
        Vec::new()
    };

    let emu_save_dir = state.borrow().save_dir.clone();
    let pending_emu_uninstall = build_api_emulator_page(
        super::edit_game_pages::ApiEmuPageParams {
            emu_exe: &saved_launch.exe,
            emu_game_folder: &game.game_folder,
            emu_db_id: game.db_id,
            emu_trophy_source: game.trophy_source,
            emu_app_id: &game.app_id,
            save_dir: &emu_save_dir,
            win: &win,
            emu_pending_uninstall: None,
        },
        &state,
        &languages,
        &sidebar,
        &stack,
    );

    let var_widgets =
        build_variants_page(&state, db_id, game.kind, has_config, &sidebar, &stack, &win);

    setup_sidebar_navigation(&sidebar, &stack);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(16);
    btn_row.set_margin_end(16);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(12);

    let cancel_btn = gtk4::Button::with_label(&crate::tr!("Cancel"));
    let win_c = Downgrade::downgrade(&win);
    cancel_btn.connect_clicked(move |_| {
        if let Some(win) = win_c.upgrade() {
            win.close();
        }
    });

    if let Some(btn) = &migrate_btn {
        let state_w = Rc::downgrade(&state);
        let game_app_id = game.app_id.clone();
        let game_db_id = game.db_id;
        let game_kind = game.kind;
        let save_dir_m = save_dir.clone();
        let btn_w = Downgrade::downgrade(btn);
        btn.connect_clicked(move |_| {
            let Some(state) = state_w.upgrade() else {
                return;
            };
            let Some(btn_m) = btn_w.upgrade() else {
                return;
            };
            let Some(details) = crate::game_loader::read_app_details(&save_dir_m, &game_app_id)
            else {
                btn_m.set_label(&crate::tr!("No save paths known"));
                return;
            };
            if details.ufs_savefiles.is_empty() {
                btn_m.set_label(&crate::tr!("No save paths known"));
                return;
            }
            let (wine_prefix, is_wine) = {
                let s = state.borrow();
                let cfg = ira_db::get_game_config(&s.db, game_db_id)
                    .ok()
                    .flatten()
                    .map(|(_, w, _)| w)
                    .unwrap_or_default();
                (
                    ira_launcher::wine_launch::wine_prefix(&cfg),
                    game_kind == ira_models::GameKind::Wine && cfg.enabled,
                )
            };
            let pfx = if is_wine {
                Some(wine_prefix.as_str())
            } else {
                None
            };
            let count = match ira_launcher::game_saves::setup_game_saves_checked(
                &details.ufs_savefiles,
                &details.ufs_rootoverrides,
                &game_app_id,
                &save_dir_m,
                pfx,
            ) {
                Ok(count) => count,
                Err(error) => {
                    btn_m.set_label(&crate::tr!("Save migration failed"));
                    eprintln!("Failed to centralize saves: {error}");
                    return;
                }
            };
            if count > 0 {
                btn_m.set_label(&crate::tr!("Migrated {} save folder(s)").replacen(
                    "{}",
                    &count.to_string(),
                    1,
                ));
            } else {
                btn_m.set_label(&crate::tr!("Already centralized"));
            }
            if let Err(e) = ira_db::set_saves_centralized(&state.borrow().db, game_db_id, true) {
                eprintln!("Failed to cache saves centralized: {}", e);
            }
            btn_m.set_sensitive(false);
        });
    }

    let save_btn = gtk4::Button::with_label(&crate::tr!("Save"));
    save_btn.add_css_class(CSS_SUGGESTED_ACTION);

    let save_btn_w = Downgrade::downgrade(&save_btn);
    let state_w = Rc::downgrade(&state);
    let win_w = Downgrade::downgrade(&win);
    let game_app_id = game.app_id.clone();
    let game_folder = game.game_folder.clone();
    let trophy_source = game.trophy_source;
    let game_kind = game.kind;
    let saved_platform_id_s = game.platform_id.clone();
    let var_widgets_w = Rc::downgrade(&var_widgets);
    let save_dir_s = save_dir.clone();
    let logo_controls_s = logo_controls.clone();
    let dlc_switches_s = dlc_switches.clone();
    let pending_copies_w = Rc::downgrade(&pending_copies);
    let old_wine_s = saved_wine.clone();
    let app_default_wine_s = app_default_wine.clone();
    let game_exe_s = saved_launch.exe.clone();
    let language_row_w = language_row.as_ref().map(Downgrade::downgrade);
    let languages_s = languages.clone();
    let lwa_rc = Rc::new(lwa);
    let lwa_w = Rc::downgrade(&lwa_rc);
    let title_entry_w = Downgrade::downgrade(&title_entry);
    let sort_entry_w = Downgrade::downgrade(&sort_entry);
    let pending_version_w = Rc::downgrade(&pending_version);
    let app_id_entry_w = app_id_entry.as_ref().map(Downgrade::downgrade);
    let pending_ra_core_w = Rc::downgrade(&pending_ra_core);
    let pending_emulator_w = Rc::downgrade(&pending_emulator);
    let game_folder_entry_w = game_folder_entry.as_ref().map(Downgrade::downgrade);
    let runtime_row_w = runtime_row.as_ref().map(Downgrade::downgrade);
    let pending_emu_uninstall_w = pending_emu_uninstall.as_ref().map(Rc::downgrade);

    save_btn.connect_clicked(move |_| {
        let Some(win) = win_w.upgrade() else {
            return;
        };
        let Some(state) = state_w.upgrade() else {
            return;
        };
        let Some(pending_copies) = pending_copies_w.upgrade() else {
            return;
        };
        let Some(var_widgets) = var_widgets_w.upgrade() else {
            return;
        };
        let Some(title_entry) = title_entry_w.upgrade() else {
            return;
        };
        let Some(sort_entry) = sort_entry_w.upgrade() else {
            return;
        };
        let Some(pending_version) = pending_version_w.upgrade() else {
            return;
        };
        let Some(pending_ra_core) = pending_ra_core_w.upgrade() else {
            return;
        };
        let Some(pending_emulator) = pending_emulator_w.upgrade() else {
            return;
        };
        let Some(lwa) = lwa_w.upgrade() else {
            return;
        };
        let Some(save_btn) = save_btn_w.upgrade() else {
            return;
        };
        let language_row = language_row_w.as_ref().and_then(|w| w.upgrade());
        let app_id_entry = app_id_entry_w.as_ref().and_then(|w| w.upgrade());
        let game_folder_entry = game_folder_entry_w.as_ref().and_then(|w| w.upgrade());
        let runtime_row = runtime_row_w.as_ref().and_then(|w| w.upgrade());
        let pending_emu_uninstall = pending_emu_uninstall_w
            .as_ref()
            .and_then(|w| w.upgrade());
        save_btn.set_sensitive(false);
        save_game_settings(SaveGameSettingsParams {
            state,
            win,
            db_id,
            app_id: game_app_id.clone(),
            trophy_source,
            game_kind,
            var_widgets,
            save_dir: save_dir_s.clone(),
            logo_controls: logo_controls_s.clone(),
            dlc_switches: dlc_switches_s.clone(),
            pending_copies,
            old_wine: old_wine_s.clone(),
            app_default_wine: app_default_wine_s.clone(),
            game_exe: game_exe_s.clone(),
            game_folder: game_folder.clone(),
            language_row,
            languages: languages_s.clone(),
            saved_platform_id: saved_platform_id_s.clone(),
            system_widgets: lwa.system_widgets.clone(),
            overlay_widgets: lwa.overlay_widgets.clone(),
            controller_widgets: lwa.controller_widgets.clone(),
            title_entry,
            sort_entry,
            pending_version,
            app_id_entry,
            pending_ra_core,
            pending_emulator,
            launch_config_widgets: lwa.launch_config_widgets.clone(),
            show_wine_tabs: lwa.show_wine_tabs,
            wine_widgets: lwa.wine_widgets_opt.clone(),
            profiles: lwa.profiles.clone(),
            saved_profile_id,
            game_folder_entry,
            runtime_row,
            pending_emu_uninstall,
        });
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
            sgdb_cache: sgdb_cache.clone(),
            ra_container,
        });
    }
    let state_close_w = Rc::downgrade(&state);
    win.connect_close_request(move |_| {
        if let Some(state) = state_close_w.upgrade() {
            state.borrow_mut().settings_data = None;
        }
        glib::Propagation::Proceed
    });
    // Anchor the page-widget structs to the dialog window: the save button
    // reaches them through weak refs, so something must own them strongly
    // for exactly as long as the dialog exists. The destroy handler dies
    // with the window, releasing them at teardown.
    let lwa_lifetime = lwa_rc;
    win.connect_destroy(move |_| {
        let _ = lwa_lifetime;
    });
}

pub fn show_edit_game_dialog(state: &SharedState, db_id: i64) {
    let Some((saved_launch, saved_wine, saved_profile_id, app_default_wine, game, has_config)) =
        extract_game_and_config(state, db_id)
    else {
        return;
    };

    let parent = state.borrow().window.clone();
    let save_dir = state.borrow().save_dir.clone();
    let (win, sidebar, stack, content_area) = create_dialog_window(&parent, &game);

    let app_details = crate::game_loader::read_app_details(&save_dir, &game.app_id);
    let win_clone = win.clone();
    build_dialog_contents(
        state.clone(),
        DialogContent {
            game,
            win,
            sidebar,
            stack,
            content_area,
            app_details,
            save_dir,
            has_config,
        },
        DialogConfig {
            saved_launch,
            saved_wine,
            saved_profile_id,
            app_default_wine,
        },
        db_id,
    );
    win_clone.present();
}
