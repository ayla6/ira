use super::helpers;
use super::settings_dialog;
use super::state::SharedState;
use adw::prelude::*;
use ira_models::{GameLaunchConfig, WineProfile};

#[derive(Clone)]
pub(super) struct LaunchConfigWidgets {
    pub(super) exe_entry: adw::EntryRow,
    pub(super) args_entry: adw::EntryRow,
    pub(super) wd_entry: adw::EntryRow,
    pub(super) pre_launch_entry: adw::EntryRow,
    pub(super) command_prefix_entry: adw::EntryRow,
    pub(super) manual_script_entry: adw::EntryRow,
    pub(super) pre_launch_wait_row: adw::SwitchRow,
    pub(super) post_exit_entry: adw::EntryRow,
    pub(super) profile_row: Option<adw::ComboRow>,
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
        },
    );

    let lc_group = adw::PreferencesGroup::new();
    lc_group.set_title(&crate::tr!("Executable"));

    let exe_entry = adw::EntryRow::new();
    exe_entry.set_title(&crate::tr!("Executable path"));
    exe_entry.set_text(&params.launch.exe);

    let exe_browse = helpers::make_browse_button(
        Some(params.win),
        &crate::tr!("Select executable"),
        false,
        Some((&crate::tr!("Executable"), &["application/x-executable"])),
        helpers::entry_path_closure(&exe_entry),
        {
            let entry = glib::clone::Downgrade::downgrade(&exe_entry);
            move |path| {
                if let Some(entry) = entry.upgrade() {
                    entry.set_text(&path.to_string_lossy());
                }
            }
        },
    );
    exe_entry.add_suffix(&exe_browse);
    lc_group.add(&exe_entry);

    let args_entry = adw::EntryRow::new();
    args_entry.set_title(&crate::tr!("Arguments"));
    args_entry.set_text(&params.launch.args);
    lc_group.add(&args_entry);

    let wd_entry = adw::EntryRow::new();
    wd_entry.set_title(&crate::tr!("Working directory"));
    wd_entry.set_text(&params.launch.working_dir);

    let wd_browse = helpers::make_browse_button(
        Some(params.win),
        &crate::tr!("Select working directory"),
        true,
        None,
        helpers::entry_path_closure(&wd_entry),
        {
            let entry = glib::clone::Downgrade::downgrade(&wd_entry);
            move |path| {
                if let Some(entry) = entry.upgrade() {
                    entry.set_text(&path.to_string_lossy());
                }
            }
        },
    );
    wd_entry.add_suffix(&wd_browse);
    lc_group.add(&wd_entry);

    let pre_launch_entry = adw::EntryRow::new();
    pre_launch_entry.set_title(&crate::tr!("Run before game"));
    pre_launch_entry.set_text(&params.launch.pre_launch);
    pre_launch_entry.set_tooltip_text(Some(&crate::tr!(
        "Shell command to run before launching the game. When waiting is enabled, a failure aborts the launch."
    )));
    lc_group.add(&pre_launch_entry);

    let pre_launch_wait_row = adw::SwitchRow::new();
    pre_launch_wait_row.set_title(&crate::tr!("Wait for pre-launch script completion"));
    pre_launch_wait_row.set_subtitle(&crate::tr!(
        "Run the game only once the pre-launch script has exited"
    ));
    pre_launch_wait_row.set_active(params.launch.pre_launch_wait.unwrap_or(true));
    pre_launch_wait_row.set_sensitive(!params.launch.pre_launch.is_empty());
    lc_group.add(&pre_launch_wait_row);
    {
        let wait_row = pre_launch_wait_row.clone();
        pre_launch_entry.connect_changed(move |entry| {
            wait_row.set_sensitive(!entry.text().is_empty());
        });
    }

    let post_exit_entry = adw::EntryRow::new();
    post_exit_entry.set_title(&crate::tr!("Post-exit script"));
    post_exit_entry.set_text(&params.launch.post_exit);
    post_exit_entry.set_tooltip_text(Some(&crate::tr!(
        "Shell command to run when the game exits."
    )));
    let post_exit_browse = helpers::make_browse_button(
        Some(params.win),
        &crate::tr!("Select post-exit script"),
        false,
        Some((
            &crate::tr!("Scripts"),
            &["application/x-executable", "application/x-shellscript"],
        )),
        helpers::entry_path_closure(&post_exit_entry),
        {
            let entry = glib::clone::Downgrade::downgrade(&post_exit_entry);
            move |path| {
                if let Some(entry) = entry.upgrade() {
                    entry.set_text(&path.to_string_lossy());
                }
            }
        },
    );
    post_exit_entry.add_suffix(&post_exit_browse);
    lc_group.add(&post_exit_entry);

    let command_prefix_entry = adw::EntryRow::new();
    command_prefix_entry.set_title(&crate::tr!("Command prefix"));
    command_prefix_entry.set_text(&params.launch.command_prefix);
    command_prefix_entry.set_tooltip_text(Some(&crate::tr!(
        "Command line instructions to add in front of the game's execution command."
    )));
    lc_group.add(&command_prefix_entry);

    let manual_script_entry = adw::EntryRow::new();
    manual_script_entry.set_title(&crate::tr!("Manual script"));
    manual_script_entry.set_text(&params.launch.manual_script);
    manual_script_entry.set_tooltip_text(Some(&crate::tr!(
        "Script to execute from the game's context menu."
    )));
    let manual_browse = helpers::make_browse_button(
        Some(params.win),
        &crate::tr!("Select manual script"),
        false,
        Some((
            &crate::tr!("Scripts"),
            &["application/x-executable", "application/x-shellscript"],
        )),
        helpers::entry_path_closure(&manual_script_entry),
        {
            let entry = glib::clone::Downgrade::downgrade(&manual_script_entry);
            move |path| {
                if let Some(entry) = entry.upgrade() {
                    entry.set_text(&path.to_string_lossy());
                }
            }
        },
    );
    manual_script_entry.add_suffix(&manual_browse);
    lc_group.add(&manual_script_entry);

    page.append(&lc_group);

    params
        .sidebar
        .append(&settings_dialog::settings_sidebar_row(
            "preferences-other-symbolic",
            &crate::tr!("Launch config"),
            "launch",
        ));
    params
        .stack
        .add_named(&super::helpers::scrolled_page(&page), Some("launch"));

    Some(LaunchConfigWidgets {
        exe_entry,
        args_entry,
        wd_entry,
        pre_launch_entry,
        command_prefix_entry,
        manual_script_entry,
        pre_launch_wait_row,
        post_exit_entry,
        profile_row,
    })
}
