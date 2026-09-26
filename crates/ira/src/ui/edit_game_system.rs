use super::settings_pages;
use super::system_settings::{
    build_env_vars_group, build_ld_paths_group, build_override_switch_row, OverrideState,
};
use super::wine_config_helpers::make_revert_btn;
use adw::prelude::*;
use glib::clone::Downgrade;
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
    pub gpu_override: Rc<RefCell<Option<String>>>,
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
        let gse = Downgrade::downgrade(&gamescope_row);
        let rev_c = gs_reverting.clone();
        gs_switch.connect_active_notify(move |sw| {
            if *rev_c.borrow() {
                return;
            }
            *state_c.borrow_mut() = Some(sw.is_active());
            btn_c.set_visible(true);
            if sw.is_active() {
                if let Some(gse) = gse.upgrade() {
                    gse.set_expanded(true);
                }
            }
        });
    }
    {
        let state_c = gamescope_state.clone();
        let btn_c = Downgrade::downgrade(&gs_revert_btn);
        let sw_c = Downgrade::downgrade(&gs_switch);
        let rev_c = gs_reverting.clone();
        gs_revert_btn.connect_clicked(move |_| {
            let Some(sw_c) = sw_c.upgrade() else {
                return;
            };
            let Some(btn_c) = btn_c.upgrade() else {
                return;
            };
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
    // Same override pattern as the upscaling row and the performance
    // switches: the row shows the effective GPU (the override, else the
    // system default), the revert button is visible only while overridden,
    // and reverting restores following the default. "Auto" is its own
    // option (`GPU_AUTO`): force no GPU variables, ignoring the default.
    let gpus = ira_launcher::gpu::detect_gpus();
    let gpu_options: Vec<String> = gpus.iter().map(|g| g.card.clone()).collect();
    let gpu_override: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(validate_gpu_override(
        &params.launch.gpu,
        params.gpu_default,
        &gpu_options,
    )));
    let gpu_effective = gpu_effective_index(params.gpu_default, &gpu_options);
    if gpus.len() > 1 {
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
        row.set_selected(gpu_display_index(
            gpu_override.borrow().as_deref(),
            gpu_effective,
            &gpu_options,
        ));
        let gpu_revert_btn = make_revert_btn();
        gpu_revert_btn.set_visible(gpu_override.borrow().is_some());
        row.add_suffix(&gpu_revert_btn);
        let gpu_reverting = Rc::new(RefCell::new(false));
        {
            let state_c = gpu_override.clone();
            let options_c = gpu_options.clone();
            let btn_c = gpu_revert_btn.clone();
            let rev_c = gpu_reverting.clone();
            row.connect_selected_item_notify(move |row| {
                if *rev_c.borrow() {
                    return;
                }
                let idx = row.selected() as usize;
                // Index 0 ("Auto") is only reachable when it differs from
                // the effective GPU; with no default it is the effective
                // choice and counts as following.
                let card = if idx == 0 && gpu_effective == 0 {
                    None
                } else {
                    gpu_card_for_index(idx, &options_c)
                };
                *state_c.borrow_mut() = card;
                btn_c.set_visible(state_c.borrow().is_some());
            });
        }
        {
            let state_c = gpu_override.clone();
            let btn_c = Downgrade::downgrade(&gpu_revert_btn);
            let row_c = Downgrade::downgrade(&row);
            let rev_c = gpu_reverting.clone();
            gpu_revert_btn.connect_clicked(move |_| {
                let Some(row_c) = row_c.upgrade() else {
                    return;
                };
                let Some(btn_c) = btn_c.upgrade() else {
                    return;
                };
                *rev_c.borrow_mut() = true;
                row_c.set_selected(gpu_effective);
                *rev_c.borrow_mut() = false;
                *state_c.borrow_mut() = None;
                btn_c.set_visible(false);
            });
        }
        group.add(&row);
        page.append(&group);
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
        .append(&settings_pages::settings_sidebar_row(
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
        gpu_override,
        env_vars_box,
        ld_preload_entry,
        ld_library_path_entry,
    }
}

/// Validates a stored per-game GPU override against the detected cards:
/// empty follows the default, `GPU_AUTO` pins Auto (unless the default
/// itself sets nothing, making them equivalent), `cardN` pins one GPU.
/// Unknown/removed cards fall back to following the default.
fn validate_gpu_override(stored: &str, default_gpu: &str, options: &[String]) -> Option<String> {
    if stored.is_empty() {
        return None;
    }
    if stored == ira_models::GPU_AUTO {
        if options.iter().any(|c| c == default_gpu) {
            return Some(ira_models::GPU_AUTO.to_string());
        }
        return None;
    }
    options.iter().find(|c| c == &stored).cloned()
}

/// ComboRow index of the system default: its card at +1, or 0 ("Auto")
/// when no default is set or it is no longer detected.
fn gpu_effective_index(default_gpu: &str, options: &[String]) -> u32 {
    options
        .iter()
        .position(|c| c == default_gpu)
        .map(|i| i + 1)
        .unwrap_or(0) as u32
}

/// ComboRow index to display: the override's entry (`GPU_AUTO` is index 0,
/// cards follow at +1), else the effective index while following.
fn gpu_display_index(override_value: Option<&str>, effective: u32, options: &[String]) -> u32 {
    match override_value {
        None => effective,
        Some(card) if card == ira_models::GPU_AUTO => 0,
        Some(card) => options
            .iter()
            .position(|c| c == card)
            .map(|i| i + 1)
            .unwrap_or(effective as usize) as u32,
    }
}

/// Stored value for a ComboRow index: 0 ("Auto") is `GPU_AUTO`.
fn gpu_card_for_index(idx: usize, options: &[String]) -> Option<String> {
    if idx == 0 {
        return Some(ira_models::GPU_AUTO.to_string());
    }
    options.get(idx - 1).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Vec<String> {
        vec!["card0".to_string(), "card1".to_string()]
    }

    #[test]
    fn test_validate_gpu_override_empty_follows_default() {
        assert_eq!(validate_gpu_override("", "card0", &options()), None);
    }

    #[test]
    fn test_validate_gpu_override_auto_sticks_out_default() {
        assert_eq!(
            validate_gpu_override("auto", "card0", &options()),
            Some("auto".to_string())
        );
    }

    #[test]
    fn test_validate_gpu_override_auto_is_following_without_default() {
        assert_eq!(validate_gpu_override("auto", "", &options()), None);
        assert_eq!(validate_gpu_override("auto", "card9", &options()), None);
    }

    #[test]
    fn test_validate_gpu_override_known_card() {
        assert_eq!(
            validate_gpu_override("card1", "card0", &options()),
            Some("card1".to_string())
        );
    }

    #[test]
    fn test_validate_gpu_override_unknown_card_follows_default() {
        assert_eq!(validate_gpu_override("card9", "card0", &options()), None);
    }

    #[test]
    fn test_gpu_effective_index_resolves_default_card() {
        assert_eq!(gpu_effective_index("card0", &options()), 1);
        assert_eq!(gpu_effective_index("card1", &options()), 2);
    }

    #[test]
    fn test_gpu_effective_index_unset_or_unknown_default_is_auto() {
        assert_eq!(gpu_effective_index("", &options()), 0);
        assert_eq!(gpu_effective_index("card9", &options()), 0);
    }

    #[test]
    fn test_gpu_display_index_shows_override_over_default() {
        // Following shows the effective GPU…
        assert_eq!(gpu_display_index(None, 1, &options()), 1);
        assert_eq!(gpu_display_index(None, 0, &options()), 0);
        // …an explicit Auto stays on the Auto entry…
        assert_eq!(gpu_display_index(Some("auto"), 1, &options()), 0);
        // …and a pinned card shows its own entry.
        assert_eq!(gpu_display_index(Some("card1"), 1, &options()), 2);
    }

    #[test]
    fn test_gpu_card_for_index_round_trip() {
        // Arrange: the index mapping behind the Auto/cards dropdown.
        // Act + assert: saving maps the selection back to the stored value.
        assert_eq!(gpu_card_for_index(0, &options()), Some("auto".to_string()));
        assert_eq!(gpu_card_for_index(1, &options()), Some("card0".to_string()));
        assert_eq!(gpu_card_for_index(2, &options()), Some("card1".to_string()));
        assert_eq!(gpu_card_for_index(99, &options()), None);
    }
}
