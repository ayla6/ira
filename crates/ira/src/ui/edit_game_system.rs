use std::cell::RefCell;
use std::rc::Rc;
use gtk4::prelude::*;
use adw::prelude::*;
use ira_models::GameLaunchConfig;
use super::add_game_dialog::build_env_var_row;
use super::settings_dialog;
use super::settings_pages::{build_source_overlay_row, OverlayOverrideState};
use super::wine_config_helpers::make_revert_btn;
use super::css::*;

#[derive(Clone)]
pub(super) struct SystemWidgets {
    pub overlay_state: OverlayOverrideState,
    pub gamemode_state: Rc<RefCell<Option<bool>>>,
    pub mangohud_state: Rc<RefCell<Option<bool>>>,
    pub gamescope_state: Rc<RefCell<Option<bool>>>,
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
    pub overlay_encoder_row: Option<adw::ComboRow>,
    pub overlay_quality_row: Option<adw::ComboRow>,
}

fn build_override_switch_row(
    title: &str,
    subtitle: &str,
    global_default: bool,
    override_value: Option<bool>,
) -> (adw::SwitchRow, Rc<RefCell<Option<bool>>>) {
    let row = adw::SwitchRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);

    let value = override_value.unwrap_or(global_default);
    row.set_active(value);

    let revert_btn = make_revert_btn();
    revert_btn.set_visible(override_value.is_some());
    row.add_suffix(&revert_btn);

    let state: Rc<RefCell<Option<bool>>> = Rc::new(RefCell::new(override_value));
    let reverting = Rc::new(RefCell::new(false));

    let state_c = state.clone();
    let btn_c = revert_btn.clone();
    let row_c = row.clone();
    let rev_c = reverting.clone();
    row.connect_active_notify(move |_| {
        if *rev_c.borrow() { return; }
        *state_c.borrow_mut() = Some(row_c.is_active());
        btn_c.set_visible(true);
        let _ = rev_c;
    });

    let state_c2 = state.clone();
    let btn_c2 = revert_btn.clone();
    let row_c2 = row.clone();
    let rev_c2 = reverting.clone();
    revert_btn.connect_clicked(move |_| {
        *rev_c2.borrow_mut() = true;
        row_c2.set_active(global_default);
        *rev_c2.borrow_mut() = false;
        *state_c2.borrow_mut() = None;
        btn_c2.set_visible(false);
    });

    (row, state)
}

pub(super) struct SystemPageParams<'a> {
    pub launch: &'a GameLaunchConfig,
    pub overlay_default: bool,
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

    let (gamemode_row, gamemode_state) = build_override_switch_row(
        "Gamemode", "Feral Interactive GameMode",
        params.gamemode_default, params.launch.gamemode,
    );
    perf_group.add(&gamemode_row);

    let (mangohud_row, mangohud_state) = build_override_switch_row(
        "MangoHud", "Performance overlay",
        params.mangohud_default, params.launch.mangohud,
    );
    perf_group.add(&mangohud_row);

    let gs_resolved = params.launch.gamescope.unwrap_or(params.gamescope_default);
    let gamescope_row = adw::ExpanderRow::new();
    gamescope_row.set_title("Gamescope");
    gamescope_row.set_subtitle("Valve Gamescope compositor");
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
            if *rev_c.borrow() { return; }
            *state_c.borrow_mut() = Some(sw.is_active());
            btn_c.set_visible(true);
            if sw.is_active() { gse.set_expanded(true); }
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

    let gamescope_flags = adw::EntryRow::new();
    gamescope_flags.set_title("Gamescope flags");
    gamescope_flags.set_text(&params.launch.gamescope_flags);
    gamescope_row.add_row(&gamescope_flags);
    page.append(&perf_group);

    fn make_spin_override_row(
        title: &str, subtitle: &str,
        default_val: u32, override_val: Option<u32>,
        min: f64, max: f64,
    ) -> (adw::ActionRow, gtk4::SpinButton, Rc<RefCell<Option<u32>>>) {
        let row = adw::ActionRow::new();
        row.set_title(title);
        row.set_subtitle(subtitle);
        let val = override_val.unwrap_or(default_val);
        let adj = gtk4::Adjustment::new(val as f64, min, max, 1.0, 10.0, 0.0);
        let spin = gtk4::SpinButton::new(Some(&adj), 1.0, 0);
        spin.set_valign(gtk4::Align::Center);
        let state: Rc<RefCell<Option<u32>>> = Rc::new(RefCell::new(override_val));
        let revert_btn = make_revert_btn();
        revert_btn.set_visible(override_val.is_some());
        let rev = Rc::new(RefCell::new(false));
        {
            let state_c = state.clone();
            let btn_c = revert_btn.clone();
            let rev_c = rev.clone();
            spin.connect_value_changed(move |s| {
                if *rev_c.borrow() { return; }
                *state_c.borrow_mut() = Some(s.value() as u32);
                btn_c.set_visible(true);
            });
        }
        {
            let state_c = state.clone();
            let btn_c = revert_btn.clone();
            let spin_c = spin.clone();
            let rev_c = rev.clone();
            let d = default_val as f64;
            revert_btn.connect_clicked(move |_| {
                *rev_c.borrow_mut() = true;
                spin_c.set_value(d);
                *rev_c.borrow_mut() = false;
                *state_c.borrow_mut() = None;
                btn_c.set_visible(false);
            });
        }
        row.add_suffix(&revert_btn);
        row.add_suffix(&spin);
        (row, spin, state)
    }

    let w_dataset = params.launch.gamescope_w;
    let h_dataset = params.launch.gamescope_h;
    let fps_dataset = params.launch.gamescope_fps;
    let upscale_dataset = params.launch.gamescope_upscaling.clone();

    let (w_row, _w_spin, gamescope_w_state) = make_spin_override_row(
        "Resolution width", "0 = auto", params.gamescope_w_default, w_dataset, 0.0, 16384.0,
    );
    gamescope_row.add_row(&w_row);

    let (h_row, _h_spin, gamescope_h_state) = make_spin_override_row(
        "Resolution height", "0 = auto", params.gamescope_h_default, h_dataset, 0.0, 16384.0,
    );
    gamescope_row.add_row(&h_row);

    let (fps_row, _fps_spin, gamescope_fps_state) = make_spin_override_row(
        "FPS limit", "0 = no limit", params.gamescope_fps_default, fps_dataset, 0.0, 360.0,
    );
    gamescope_row.add_row(&fps_row);

    let upscaling_model = gtk4::StringList::new(&["Linear", "FSR", "NIS", "Integer", "Nearest"]);
    let upscaling_row = adw::ComboRow::new();
    upscaling_row.set_title("Upscaling method");
    upscaling_row.set_model(Some(&upscaling_model));
    let upscaling_values = ["linear", "fsr", "nis", "integer", "nearest"];
    let upscale_current = params.launch.gamescope_upscaling.as_deref().unwrap_or("linear").to_string();
    let upscale_default = params.gamescope_upscaling_default;
    let upscale_selected = if upscale_dataset.is_some() {
        upscaling_values.iter().position(|&v| v == upscale_current).unwrap_or(0)
    } else {
        upscaling_values.iter().position(|&v| v == upscale_default.as_str()).unwrap_or(0)
    } as u32;
    upscaling_row.set_selected(upscale_selected);

    let gamescope_upscaling_state: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(upscale_dataset));
    let upscale_revert_btn = make_revert_btn();
    upscale_revert_btn.set_visible(params.launch.gamescope_upscaling.is_some());
    upscaling_row.add_suffix(&upscale_revert_btn);

    let upscale_rev = Rc::new(RefCell::new(false));
    {
        let state_c = gamescope_upscaling_state.clone();
        let btn_c = upscale_revert_btn.clone();
        let rev_c = upscale_rev.clone();
        let values = upscaling_values;
        upscaling_row.connect_selected_item_notify(move |r| {
            if *rev_c.borrow() { return; }
            let idx = r.selected() as usize;
            let v = values.get(idx).copied().unwrap_or("linear");
            *state_c.borrow_mut() = Some(v.to_string());
            btn_c.set_visible(true);
        });
    }
    {
        let state_c = gamescope_upscaling_state.clone();
        let btn_c = upscale_revert_btn.clone();
        let rev_c = upscale_rev.clone();
        let values = upscaling_values;
        let default_idx = values.iter().position(|&v| v == upscale_default.as_str()).unwrap_or(0) as u32;
        let row_c = upscaling_row.clone();
        upscale_revert_btn.connect_clicked(move |_| {
            *rev_c.borrow_mut() = true;
            row_c.set_selected(default_idx);
            *rev_c.borrow_mut() = false;
            *state_c.borrow_mut() = None;
            btn_c.set_visible(false);
        });
        gamescope_row.add_row(&upscaling_row);
    }

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
        overlay_encoder_row: Some(encoder_row),
        overlay_quality_row: Some(quality_row),
    }
}
