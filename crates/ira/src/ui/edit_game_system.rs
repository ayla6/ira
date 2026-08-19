use super::settings_dialog;
use super::system_settings::{
    build_env_vars_group, build_ld_paths_group, build_override_switch_row, OverrideState,
};
use super::wine_config_helpers::make_revert_btn;
use adw::prelude::*;
use ira_models::GameLaunchConfig;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub(super) struct SystemWidgets {
    pub gamemode_state: OverrideState,
    pub mangohud_state: OverrideState,
    pub gamescope_state: OverrideState,
    pub gamescope_flags: adw::EntryRow,
    pub gamescope_w_state: Rc<RefCell<Option<u32>>>,
    pub gamescope_h_state: Rc<RefCell<Option<u32>>>,
    pub gamescope_fps_state: Rc<RefCell<Option<u32>>>,
    pub gamescope_upscaling_state: Rc<RefCell<Option<String>>>,
    pub gpu_row: Option<adw::ComboRow>,
    pub gpu_options: Vec<String>,
    pub env_vars_box: gtk4::ListBox,
    pub ld_preload_entry: adw::EntryRow,
    pub ld_library_path_entry: adw::EntryRow,
}

pub(super) struct SystemPageParams<'a> {
    pub launch: &'a GameLaunchConfig,
    pub gpu_default: &'a str,
    pub gamemode_default: bool,
    pub mangohud_default: bool,
    pub gamescope_default: bool,
    pub gamescope_w_default: u32,
    pub gamescope_h_default: u32,
    pub gamescope_fps_default: u32,
    pub gamescope_upscaling_default: String,
    pub sidebar: &'a gtk4::ListBox,
    pub stack: &'a gtk4::Stack,
}

pub(super) fn build_system_page(params: SystemPageParams) -> SystemWidgets {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    // ─── Performance ───
    let perf_group = adw::PreferencesGroup::new();
    perf_group.set_title(&crate::tr!("Performance"));

    let (gamemode_row, gamemode_state) = build_override_switch_row(
        &crate::tr!("Gamemode"),
        &crate::tr!("Feral Interactive GameMode"),
        params.gamemode_default,
        params.launch.gamemode,
    );
    perf_group.add(&gamemode_row);

    let (mangohud_row, mangohud_state) = build_override_switch_row(
        &crate::tr!("MangoHud"),
        &crate::tr!("Performance overlay"),
        params.mangohud_default,
        params.launch.mangohud,
    );
    perf_group.add(&mangohud_row);

    let gs_resolved = params.launch.gamescope.unwrap_or(params.gamescope_default);
    let gamescope_row = adw::ExpanderRow::new();
    gamescope_row.set_title(&crate::tr!("Gamescope"));
    gamescope_row.set_subtitle(&crate::tr!("Valve Gamescope compositor"));
    gamescope_row.set_expanded(gs_resolved);

    let gs_switch = gtk4::Switch::new();
    gs_switch.set_active(gs_resolved);
    gs_switch.set_valign(gtk4::Align::Center);
    let gs_revert_btn = make_revert_btn();
    gs_revert_btn.set_visible(params.launch.gamescope.is_some());
    gamescope_row.add_suffix(&gs_revert_btn);
    gamescope_row.add_suffix(&gs_switch);
    perf_group.add(&gamescope_row);

    let gamescope_state: Rc<RefCell<Option<bool>>> = Rc::new(RefCell::new(params.launch.gamescope));
    let gs_reverting = Rc::new(RefCell::new(false));
    {
        let state_c = gamescope_state.clone();
        let btn_c = gs_revert_btn.clone();
        let gse = gamescope_row.clone();
        let rev_c = gs_reverting.clone();
        gs_switch.connect_active_notify(move |sw| {
            if *rev_c.borrow() {
                return;
            }
            *state_c.borrow_mut() = Some(sw.is_active());
            btn_c.set_visible(true);
            if sw.is_active() {
                gse.set_expanded(true);
            }
        });
    }
    {
        let state_c = gamescope_state.clone();
        let btn_c = gs_revert_btn.clone();
        let sw_c = gs_switch.clone();
        let rev_c = gs_reverting.clone();
        gs_revert_btn.connect_clicked(move |_| {
            *rev_c.borrow_mut() = true;
            sw_c.set_active(params.gamescope_default);
            *rev_c.borrow_mut() = false;
            *state_c.borrow_mut() = None;
            btn_c.set_visible(false);
        });
    }

    let gs_widgets = super::system_settings::add_gamescope_rows(
        &gamescope_row,
        &super::system_settings::GamescopeDefaults {
            flags: String::new(),
            w: params.gamescope_w_default,
            h: params.gamescope_h_default,
            fps: params.gamescope_fps_default,
            upscaling: params.gamescope_upscaling_default.clone(),
        },
        Some(&super::system_settings::GamescopeOverride {
            flags: params.launch.gamescope_flags.clone(),
            w: params.launch.gamescope_w,
            h: params.launch.gamescope_h,
            fps: params.launch.gamescope_fps,
            upscaling: params.launch.gamescope_upscaling.clone(),
        }),
    );
    let gamescope_flags = gs_widgets.flags;
    let gamescope_w_state = gs_widgets.w_state;
    let gamescope_h_state = gs_widgets.h_state;
    let gamescope_fps_state = gs_widgets.fps_state;
    let gamescope_upscaling_state = gs_widgets.upscaling_state;
    page.append(&perf_group);

    // ─── GPU (only when multiple GPUs detected) ───
    let gpus = ira_launcher::gpu::detect_gpus();
    let gpu_options: Vec<String> = gpus.iter().map(|g| g.card.clone()).collect();
    let gpu_row = if gpus.len() > 1 {
        let group = adw::PreferencesGroup::new();
        group.set_title(&crate::tr!("Graphics"));

        let model = gtk4::StringList::new(&[]);
        model.append(&crate::tr!("Auto"));
        for g in &gpus {
            model.append(&g.short_name());
        }
        let row = adw::ComboRow::new();
        row.set_title(&crate::tr!("GPU"));
        row.set_subtitle(&crate::tr!("Graphics card to use for rendering"));
        row.set_model(Some(&model));
        let idx = if params.launch.gpu.is_empty() {
            // If app-level default is set, select it; otherwise "Auto"
            if !params.gpu_default.is_empty() {
                gpu_options
                    .iter()
                    .position(|c| c == params.gpu_default)
                    .map(|i| i + 1)
                    .unwrap_or(0)
            } else {
                0
            }
        } else {
            gpu_options
                .iter()
                .position(|c| c == &params.launch.gpu)
                .map(|i| i + 1)
                .unwrap_or(0)
        };
        row.set_selected(idx as u32);
        group.add(&row);
        page.append(&group);
        Some(row)
    } else {
        None
    };

    let (env_group, env_vars_box) = build_env_vars_group(&params.launch.env_vars);
    page.append(&env_group);

    let (ld_group, ld_preload_entry, ld_library_path_entry) =
        build_ld_paths_group(&params.launch.ld_preload, &params.launch.ld_library_path);
    page.append(&ld_group);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&page));
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    params
        .sidebar
        .append(&settings_dialog::settings_sidebar_row(
            "emblem-system-symbolic",
            "System",
            "system",
        ));
    params.stack.add_named(&scroll, Some("system"));

    SystemWidgets {
        gamemode_state,
        mangohud_state,
        gamescope_state,
        gamescope_flags,
        gamescope_w_state,
        gamescope_h_state,
        gamescope_fps_state,
        gamescope_upscaling_state,
        gpu_row,
        gpu_options,
        env_vars_box,
        ld_preload_entry,
        ld_library_path_entry,
    }
}
