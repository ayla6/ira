use adw::prelude::*;
use ira_models::GameLaunchConfig;
use super::add_game_dialog::build_env_var_row;
use super::settings_dialog;
use super::settings_pages::{build_source_overlay_row, OverlayOverrideState};
use super::css::*;

#[derive(Clone)]
pub(super) struct SystemWidgets {
    pub overlay_state: OverlayOverrideState,
    pub gamemode: adw::SwitchRow,
    pub mangohud: adw::SwitchRow,
    pub gamescope: adw::SwitchRow,
    pub gamescope_flags: adw::EntryRow,
    pub gpu_row: Option<adw::ComboRow>,
    pub gpu_options: Vec<String>,
    pub env_vars_box: gtk4::ListBox,
    pub ld_preload_entry: adw::EntryRow,
    pub ld_library_path_entry: adw::EntryRow,
    pub overlay_encoder_row: Option<adw::ComboRow>,
    pub overlay_quality_row: Option<adw::ComboRow>,
}

pub(super) struct SystemPageParams<'a> {
    pub launch: &'a GameLaunchConfig,
    pub overlay_default: bool,
    pub sidebar: &'a gtk4::ListBox,
    pub stack: &'a gtk4::Stack,
}

pub(super) fn build_system_page(params: SystemPageParams) -> SystemWidgets {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    // ─── Overlay ───
    let (overlay_row, overlay_state) = build_source_overlay_row(
        params.overlay_default,
        params.launch.overlay_enabled,
    );
    let overlay_group = adw::PreferencesGroup::new();
    overlay_group.add(&overlay_row);
    page.append(&overlay_group);

    // ─── Overlay settings (per-game overrides) ───
    let overlay_cfg_group = adw::PreferencesGroup::new();
    overlay_cfg_group.set_title("Overlay");

    let encoder_model = gtk4::StringList::new(&["Default", "Auto", "VAAPI (AMD/Intel)", "NVENC (NVIDIA)", "Software (CPU)"]);
    let encoder_row = adw::ComboRow::new();
    encoder_row.set_title("Video encoder");
    encoder_row.set_model(Some(&encoder_model));
    encoder_row.set_selected(params.launch.overlay_encoder.map(|v| v + 1).unwrap_or(0));
    overlay_cfg_group.add(&encoder_row);

    let quality_model = gtk4::StringList::new(&["Default", "Low (720p 30fps)", "Medium (1080p 30fps)", "High (1080p 60fps)"]);
    let quality_row = adw::ComboRow::new();
    quality_row.set_title("Recording quality");
    quality_row.set_model(Some(&quality_model));
    quality_row.set_selected(params.launch.overlay_recording_quality.map(|v| v + 1).unwrap_or(0));
    overlay_cfg_group.add(&quality_row);
    page.append(&overlay_cfg_group);

    // ─── Performance ───
    let perf_group = adw::PreferencesGroup::new();
    perf_group.set_title("Performance");

    let gamemode = adw::SwitchRow::new();
    gamemode.set_title("Gamemode");
    gamemode.set_subtitle("Feral Interactive GameMode");
    gamemode.set_active(params.launch.gamemode);
    perf_group.add(&gamemode);

    let mangohud = adw::SwitchRow::new();
    mangohud.set_title("MangoHud");
    mangohud.set_subtitle("Performance overlay");
    mangohud.set_active(params.launch.mangohud);
    perf_group.add(&mangohud);

    let gamescope = adw::SwitchRow::new();
    gamescope.set_title("Gamescope");
    gamescope.set_subtitle("Valve Gamescope compositor");
    gamescope.set_active(params.launch.gamescope.unwrap_or(false));
    perf_group.add(&gamescope);

    let gamescope_flags = adw::EntryRow::new();
    gamescope_flags.set_title("Gamescope flags");
    gamescope_flags.set_text(&params.launch.gamescope_flags);
    gamescope_flags.set_visible(params.launch.gamescope.unwrap_or(false));
    perf_group.add(&gamescope_flags);
    {
        let gf = gamescope_flags.clone();
        gamescope.connect_active_notify(move |sw| { gf.set_visible(sw.is_active()); });
    }
    page.append(&perf_group);

    // ─── GPU (only when multiple GPUs detected) ───
    let gpus = ira_launcher::gpu::detect_gpus();
    let gpu_options: Vec<String> = gpus.iter().map(|g| g.card.clone()).collect();
    let gpu_row = if gpus.len() > 1 {
        let group = adw::PreferencesGroup::new();
        group.set_title("Graphics");

        let model = gtk4::StringList::new(&[]);
        model.append("Default");
        for g in &gpus {
            model.append(&g.short_name());
        }
        let row = adw::ComboRow::new();
        row.set_title("GPU");
        row.set_subtitle("Graphics card to use for rendering");
        row.set_model(Some(&model));
        let idx = if params.launch.gpu.is_empty() {
            0
        } else {
            gpu_options.iter().position(|c| c == &params.launch.gpu)
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

    // ─── Environment Variables ───
    let env_group = adw::PreferencesGroup::new();
    env_group.set_title("Environment Variables");
    let add_env_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_env_btn.set_tooltip_text(Some("Add variable"));
    add_env_btn.set_valign(gtk4::Align::Center);
    add_env_btn.add_css_class(CSS_FLAT);
    env_group.set_header_suffix(Some(&add_env_btn));

    let env_vars_box = gtk4::ListBox::new();
    env_vars_box.add_css_class(CSS_BOXED_LIST);
    for (name, value) in &params.launch.env_vars {
        env_vars_box.append(&build_env_var_row(name, value));
    }
    let env_box_clone = env_vars_box.clone();
    add_env_btn.connect_clicked(move |_| {
        env_box_clone.append(&build_env_var_row("", ""));
    });
    env_group.add(&env_vars_box);
    page.append(&env_group);

    // ─── Dynamic Libraries ───
    let ld_group = adw::PreferencesGroup::new();
    let ld_preload_entry = adw::EntryRow::new();
    ld_preload_entry.set_title("LD_PRELOAD");
    ld_preload_entry.set_text(&params.launch.ld_preload);
    ld_group.add(&ld_preload_entry);
    let ld_library_path_entry = adw::EntryRow::new();
    ld_library_path_entry.set_title("LD_LIBRARY_PATH");
    ld_library_path_entry.set_text(&params.launch.ld_library_path);
    ld_group.add(&ld_library_path_entry);
    page.append(&ld_group);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&page));
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    params.sidebar.append(&settings_dialog::settings_sidebar_row(
        "applications-science-symbolic", "System", "system",
    ));
    params.stack.add_named(&scroll, Some("system"));

    SystemWidgets {
        overlay_state,
        gamemode,
        mangohud,
        gamescope,
        gamescope_flags,
        gpu_row,
        gpu_options,
        env_vars_box,
        ld_preload_entry,
        ld_library_path_entry,
        overlay_encoder_row: Some(encoder_row),
        overlay_quality_row: Some(quality_row),
    }
}
