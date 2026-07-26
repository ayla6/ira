use adw::prelude::*;
use ira_models::{GameLaunchConfig, WineProfile};
use super::helpers;
use super::settings_dialog;
use super::settings_pages::{build_source_overlay_row, OverlayOverrideState};
use super::state::SharedState;

#[derive(Clone)]
pub(super) struct LaunchConfigWidgets {
    pub(super) exe_entry: adw::EntryRow,
    pub(super) args_entry: adw::EntryRow,
    pub(super) wd_entry: adw::EntryRow,
    pub(super) pre_launch_entry: adw::EntryRow,
    pub(super) profile_row: Option<adw::ComboRow>,
    pub(super) overlay_state: OverlayOverrideState,
}

pub(super) struct LaunchConfigParams<'a> {
    pub launch: &'a GameLaunchConfig,
    pub win: &'a adw::Window,
    pub sidebar: &'a gtk4::ListBox,
    pub stack: &'a gtk4::Stack,
    pub has_config: bool,
    pub saved_wine_enabled: bool,
    pub saved_profile_id: Option<i64>,
    pub profiles: &'a [WineProfile],
    pub state: &'a SharedState,
    pub game_slug: &'a str,
    pub overlay_default: bool,
}

pub(super) fn build_launch_config_page(params: LaunchConfigParams) -> Option<LaunchConfigWidgets> {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let profile_row = super::edit_game_pages::build_profile_dropdown(
        super::edit_game_pages::ProfileDropdownParams {
            has_config: params.has_config,
            saved_wine_enabled: params.saved_wine_enabled,
            saved_profile_id: params.saved_profile_id,
            profiles: params.profiles,
            page: &page,
            state: params.state,
            win: params.win,
            game_slug: params.game_slug,
        }
    );

    let lc_group = adw::PreferencesGroup::new();
    lc_group.set_title("Executable");

    let exe_entry = adw::EntryRow::new();
    exe_entry.set_title("Executable path");
    exe_entry.set_text(&params.launch.exe);

    let exe_browse = helpers::make_browse_button(
        Some(params.win),
        "Select executable",
        false,
        Some(("Executable", &["application/x-executable"])),
        {
            let entry = exe_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    exe_entry.add_suffix(&exe_browse);
    lc_group.add(&exe_entry);

    let args_entry = adw::EntryRow::new();
    args_entry.set_title("Arguments");
    args_entry.set_text(&params.launch.args);
    lc_group.add(&args_entry);

    let wd_entry = adw::EntryRow::new();
    wd_entry.set_title("Working directory");
    wd_entry.set_text(&params.launch.working_dir);

    let wd_browse = helpers::make_browse_button(
        Some(params.win),
        "Select working directory",
        true,
        None,
        {
            let entry = wd_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    wd_entry.add_suffix(&wd_browse);
    lc_group.add(&wd_entry);

    let pre_launch_entry = adw::EntryRow::new();
    pre_launch_entry.set_title("Run before game");
    pre_launch_entry.set_text(&params.launch.pre_launch);
    pre_launch_entry.set_tooltip_text(Some("Shell command to run before launching the game. If it fails, the game will not launch."));
    lc_group.add(&pre_launch_entry);

    page.append(&lc_group);

    // Overlay toggle (per-game override of the source setting)
    let (overlay_row, overlay_state) = build_source_overlay_row(
        params.overlay_default,
        params.launch.overlay_enabled,
    );
    let overlay_group = adw::PreferencesGroup::new();
    overlay_group.add(&overlay_row);
    page.append(&overlay_group);

    params.sidebar.append(&settings_dialog::settings_sidebar_row("preferences-other-symbolic", "Launch Config"));
    params.stack.add_named(&page, Some("launch"));

    Some(LaunchConfigWidgets {
        exe_entry,
        args_entry,
        wd_entry,
        pre_launch_entry,
        profile_row,
        overlay_state,
    })
}

/// Minimal page for emulator games — only shows the overlay toggle.
/// The exe/args/etc. fields are not shown because emulator games use
/// `launch_retro`/`launch_ps4`/`launch_ps3` instead of `launch_other`.
pub(super) fn build_emulator_overlay_page(
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    overlay_default: bool,
    overlay_override: Option<bool>,
) -> Option<LaunchConfigWidgets> {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let (overlay_row, overlay_state) = build_source_overlay_row(overlay_default, overlay_override);
    let overlay_group = adw::PreferencesGroup::new();
    overlay_group.add(&overlay_row);
    page.append(&overlay_group);

    sidebar.append(&settings_dialog::settings_sidebar_row("preferences-other-symbolic", "Overlay"));
    stack.add_named(&page, Some("overlay"));

    Some(LaunchConfigWidgets {
        exe_entry: adw::EntryRow::new(),
        args_entry: adw::EntryRow::new(),
        wd_entry: adw::EntryRow::new(),
        pre_launch_entry: adw::EntryRow::new(),
        profile_row: None,
        overlay_state,
    })
}
