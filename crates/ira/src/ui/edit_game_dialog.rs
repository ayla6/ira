use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use adw::prelude::*;
use gtk4::prelude::IsA;
use ira_models::{AppDetails, GameLaunchConfig, WineConfig, WineProfile};
use super::state::{PendingImage, SharedState};
use super::edit_game_launch::{build_launch_config_page, LaunchConfigWidgets};
use super::edit_game_system::{build_system_page, SystemWidgets};
use super::edit_game_pages::{build_api_emulator_page, build_dlc_page};
use super::edit_game_variants::build_variants_page;
use super::edit_game_save::{save_game_settings, SaveGameSettingsParams};
use super::wine_config_widget::{build_wine_config_pages, WineConfigWidgets};
use crate::Game;
use super::css::*;

struct LaunchWineAdvancedCtx {
    launch_config_widgets: Option<LaunchConfigWidgets>,
    system_widgets: Option<SystemWidgets>,
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

fn extract_game_and_config(state: &SharedState, db_id: i64) -> Option<(GameLaunchConfig, WineConfig, Option<i64>, WineConfig, Game, bool)> {
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
    Some((saved_launch, saved_wine, saved_profile_id, app_default_wine, game, has_config))
}

fn create_dialog_window(parent: &impl IsA<gtk4::Window>, game: &Game) -> (adw::Window, gtk4::ListBox, gtk4::Stack, gtk4::Box) {
    let layout = super::helpers::dialog_layout(parent);
    layout.window.set_deletable(false);
    layout.stack.set_hexpand(true);
    layout.header.set_title_widget(Some(&gtk4::Label::new(Some(&format!("{} [{}]", game.name, game.db_id)))));
    (layout.window, layout.sidebar, layout.stack, layout.content_area)
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
    let show_launch_config = game.kind != ira_models::GameKind::Steam && game.kind != ira_models::GameKind::Ps4 && game.kind != ira_models::GameKind::Ps3 && game.kind != ira_models::GameKind::Retro;
    let profiles = ira_db::get_all_profiles(&state.borrow().db).unwrap_or_default();

    let overlay_source_id = match game.kind {
        ira_models::GameKind::Steam => Some("steam"),
        ira_models::GameKind::Retro => Some(game.platform_id.as_str()),
        ira_models::GameKind::Ps4 => Some("ps4"),
        ira_models::GameKind::Ps3 => Some("ps3"),
        _ => None,
    };
    let overlay_default = overlay_source_id.map_or(state.borrow().cfg.overlay.enabled, |id| {
        state.borrow().cfg.overlay.source_enabled(id)
    });
    let gamemode_default = state.borrow().cfg.default_system.gamemode;
    let mangohud_default = state.borrow().cfg.default_system.mangohud;
    let default_gamescope = state.borrow().cfg.default_system.gamescope;
    let gamescope_default = overlay_source_id
        .and_then(|id| state.borrow().cfg.overlay.source_gamescope.get(id).copied())
        .unwrap_or(default_gamescope);
    let gs = state.borrow().cfg.default_system.clone();
    let gamescope_w_default = gs.gamescope_w;
    let gamescope_h_default = gs.gamescope_h;
    let gamescope_fps_default = gs.gamescope_fps;
    let gamescope_upscaling_default = gs.gamescope_upscaling;

    // Launch Config page — only for non-emulator games (Wine, Linux native, etc.)
    let launch_config_widgets = if show_launch_config {
        build_launch_config_page(super::edit_game_launch::LaunchConfigParams {
            launch: saved_launch,
            win,
            sidebar,
            stack,
            has_config,
            saved_wine_enabled: saved_wine.enabled && game.kind == ira_models::GameKind::Wine,
            saved_profile_id,
            profiles: &profiles,
            state,
            game_slug: &game.slug,
        })
    } else {
        None
    };

    // System page — shown for ALL games (Wine, emulator, native, etc.)
    let system_widgets = Some(build_system_page(super::edit_game_system::SystemPageParams {
        launch: saved_launch,
        overlay_default,
        gamemode_default,
        mangohud_default,
        gamescope_default,
        gamescope_w_default,
        gamescope_h_default,
        gamescope_fps_default,
        gamescope_upscaling_default,
        sidebar,
        stack,
    }));

    // Wine pages — only for Wine games with wine enabled
    let show_wine_tabs = game.kind == ira_models::GameKind::Wine && saved_wine.enabled;
    let wine_widgets_opt = if show_wine_tabs {
        let (wine_pages, ww) = build_wine_config_pages(saved_wine, Some(app_default_wine));
        for wp in &wine_pages {
            sidebar.append(&super::settings_dialog::settings_sidebar_row(wp.icon, wp.label, wp.label));
            stack.add_named(&wp.page, Some(wp.label));
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
        show_wine_tabs,
        wine_widgets_opt,
        profiles,
    }
}

fn setup_sidebar_navigation(sidebar: &gtk4::ListBox, stack: &gtk4::Stack) {
    let stack_clone = stack.clone();
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            let page_id = row.widget_name().to_string().to_string();
            stack_clone.set_visible_child_name(&page_id);
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
    let DialogContent { game, win, sidebar, stack, content_area, app_details, save_dir, has_config } = content;
    let DialogConfig { saved_launch, saved_wine, saved_profile_id, app_default_wine } = config;
    let languages = app_details.as_ref().map(|d| d.languages.clone()).unwrap_or_default();
    let pending_copies: Rc<RefCell<HashMap<String, PendingImage>>> = Default::default();
    let (general_page, title_entry, sort_entry, pending_version, app_id_entry, language_row, pending_ra_core, pending_emulator, ra_container) =
        super::game_settings::build_game_general_page(&state, &game, &win, &languages, &pending_copies);
    sidebar.append(&super::settings_dialog::settings_sidebar_row("preferences-system-symbolic", "General", "general"));
    stack.add_named(&general_page, Some("general"));

    let lwa = build_launch_wine_advanced_pages(
        &state, &game, &LaunchWineParams {
            saved_launch: &saved_launch,
            saved_wine: &saved_wine,
            has_config,
            saved_profile_id,
            app_default_wine: &app_default_wine,
        }, &win, &sidebar, &stack,
    );

    if !game.app_id.is_empty() {
        let images_page = super::image_manager::build_image_manager_content_with_drafts(
            &state, &game, &win, Some(pending_copies.clone()),
        );
        sidebar.append(&super::settings_dialog::settings_sidebar_row("image-x-generic-symbolic", "Images", "images"));
        stack.add_named(&images_page, Some("images"));
    }

    let logo_controls: Option<(Rc<RefCell<String>>, gtk4::Adjustment)> =
        if let Some((logo_page, selected_pos, size_adj, _modified)) = super::game_logo::build_game_logo_page(&game, false) {
            sidebar.append(&super::settings_dialog::settings_sidebar_row("preferences-desktop-wallpaper-symbolic", "Logo", "logo"));
            stack.add_named(&logo_page, Some("logo"));
            Some((selected_pos, size_adj))
        } else {
            None
        };

    let dlc_switches = if game.kind != ira_models::GameKind::Steam {
        build_dlc_page(&app_details, &sidebar, &stack)
    } else {
        Vec::new()
    };

    let emu_save_dir = state.borrow().save_dir.clone();
    build_api_emulator_page(
        super::edit_game_pages::ApiEmuPageParams {
            emu_exe: &saved_launch.exe,
            emu_trophy_source: game.trophy_source,
            emu_app_id: &game.app_id,
            save_dir: &emu_save_dir,
        },
        &state,
        &languages,
        &sidebar,
        &stack,
    );

    let var_widgets = build_variants_page(&state, db_id, game.kind, has_config, &sidebar, &stack, &win);

    setup_sidebar_navigation(&sidebar, &stack);

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
    save_btn.add_css_class(CSS_SUGGESTED_ACTION);

    let save_btn_c = save_btn.clone();
    let state_s = state.clone();
    let win_s = win.clone();
    let app_id = game.app_id.clone();
    let trophy_source = game.trophy_source;
    let game_kind = game.kind;
    let var_widgets_s = var_widgets.clone();
    let save_dir_s = save_dir.clone();
    let logo_controls_s = logo_controls.clone();
    let dlc_switches_s = dlc_switches.clone();
    let pending_copies_s = pending_copies.clone();
    let old_wine_s = saved_wine.clone();
    let app_default_wine_s = app_default_wine.clone();
    let game_exe_s = saved_launch.exe.clone();
    let language_row_s = language_row.clone();
    let languages_s = languages.clone();
    let saved_platform_id_s = game.platform_id.clone();
    let system_widgets_s = lwa.system_widgets.clone();
    let title_entry_s = title_entry.clone();
    let sort_entry_s = sort_entry.clone();
    let pending_version_s = pending_version.clone();
    let app_id_entry_s = app_id_entry.clone();
    let pending_ra_core_s = pending_ra_core.clone();
    let pending_emulator_s = pending_emulator.clone();
    let profiles_s = lwa.profiles.clone();

    save_btn.connect_clicked(move |_| {
        save_btn_c.set_sensitive(false);
        save_game_settings(SaveGameSettingsParams {
            state: state_s.clone(),
            win: win_s.clone(),
            db_id,
            app_id: app_id.clone(),
            trophy_source,
            game_kind,
            var_widgets: var_widgets_s.clone(),
            save_dir: save_dir_s.clone(),
            logo_controls: logo_controls_s.clone(),
            dlc_switches: dlc_switches_s.clone(),
            pending_copies: pending_copies_s.clone(),
            old_wine: old_wine_s.clone(),
            app_default_wine: app_default_wine_s.clone(),
            game_exe: game_exe_s.clone(),
            language_row: language_row_s.clone(),
            languages: languages_s.clone(),
            saved_platform_id: saved_platform_id_s.clone(),
            system_widgets: system_widgets_s.clone(),
            title_entry: title_entry_s.clone(),
            sort_entry: sort_entry_s.clone(),
            pending_version: pending_version_s.clone(),
            app_id_entry: app_id_entry_s.clone(),
            pending_ra_core: pending_ra_core_s.clone(),
            pending_emulator: pending_emulator_s.clone(),
            launch_config_widgets: lwa.launch_config_widgets.clone(),
            show_wine_tabs: lwa.show_wine_tabs,
            wine_widgets: lwa.wine_widgets_opt.clone(),
            profiles: profiles_s.clone(),
            saved_profile_id,
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
            ra_container,
        });
    }
    let state_close = state.clone();
    win.connect_close_request(move |_| {
        state_close.borrow_mut().settings_data = None;
        glib::Propagation::Proceed
    });
}

pub fn show_edit_game_dialog(state: &SharedState, db_id: i64) {
    let Some((saved_launch, saved_wine, saved_profile_id, app_default_wine, game, has_config)) =
        extract_game_and_config(state, db_id)
    else { return };

    let parent = state.borrow().window.clone();
    let save_dir = state.borrow().save_dir.clone();
    let (win, sidebar, stack, content_area) = create_dialog_window(&parent, &game);

    let app_details = crate::game_loader::read_app_details(&save_dir, &game.app_id);
    let win_clone = win.clone();
    build_dialog_contents(
        state.clone(),
        DialogContent {
            game, win, sidebar, stack, content_area,
            app_details, save_dir, has_config,
        },
        DialogConfig {
            saved_launch, saved_wine, saved_profile_id, app_default_wine,
        },
        db_id,
    );
    win_clone.present();
}
