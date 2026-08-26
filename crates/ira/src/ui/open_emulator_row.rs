use super::css::{CSS_FLAT, CSS_SQUARE_BUTTON};
use super::input_profile_settings::ConsoleProfileWidgets;
use super::play_button::active_controller_input;
use super::play_button_helpers::{console_input_mode, resolve_input_profile};
use super::state::SharedState;
use crate::AppMessage;
use adw::prelude::*;
use ira_config::Config;
use ira_models::ControllerInputMode;

/// Appends an "Open emulator" row to a console settings page. It spawns the
/// emulator currently entered on the page with no game loaded, wrapped in the
/// controller layout shown on the page (falling back to the saved console
/// override, then to any connected-controller default).
pub(super) fn add_open_emulator_row(
    page: &gtk4::Box,
    state: &SharedState,
    display_label: &str,
    console_id: String,
    exe_source: impl Fn() -> String + 'static,
    profile: &ConsoleProfileWidgets,
) {
    let group = adw::PreferencesGroup::new();
    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Open emulator"));
    row.set_subtitle(
        &crate::tr!(
            "Start {name} without loading a game, using the current controller mapping"
        )
        .replace("{name}", display_label),
    );

    let open = gtk4::Button::from_icon_name("media-playback-start-symbolic");
    open.add_css_class(CSS_FLAT);
    open.add_css_class(CSS_SQUARE_BUTTON);
    open.set_tooltip_text(Some(&crate::tr!("Open emulator")));
    open.set_valign(gtk4::Align::Center);

    let mode_widget = profile.mode.clone();
    let profile_path_widget = profile.profile_path.clone();
    let state_c = state.clone();
    let console_id_c = console_id.clone();
    open.connect_clicked(move |_| {
        // Read widget state on the main thread; heavy work happens off-thread.
        let exe = exe_source();
        let live = (*mode_widget.borrow(), profile_path_widget.borrow().clone());
        let (cfg, save_dir, sender, registry) = {
            let s = state_c.borrow();
            (
                s.cfg.clone(),
                s.save_dir.clone(),
                s.sender.clone(),
                s.controller_registry.clone(),
            )
        };
        let label = console_id_c.clone();
        std::thread::spawn(move || {
            if let Err(e) = spawn_emulator_open(&cfg, &save_dir, &registry, &label, &exe, live) {
                eprintln!("Failed to open emulator: {}", e);
                let _ = sender.send(AppMessage::AddGameError(e));
            }
        });
    });

    row.add_suffix(&open);
    group.add(&row);
    page.append(&group);
}

/// Merges the page's live controller-layout values over the saved console
/// overrides; the live values win when present.
fn merge_console_overrides(
    live: (Option<ControllerInputMode>, Option<std::path::PathBuf>),
    saved_mode: Option<ControllerInputMode>,
    saved_profile: &str,
) -> (Option<ControllerInputMode>, Option<String>) {
    let selected = live
        .1
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path.to_string_lossy().into_owned())
        .or_else(|| (!saved_profile.is_empty()).then(|| saved_profile.to_string()));
    (live.0.or(saved_mode), selected)
}

/// Resolves the console's effective input mapping, builds the bare emulator
/// command, wraps it with the input broker when enabled, and detaches it.
fn spawn_emulator_open(
    cfg: &Config,
    save_dir: &str,
    registry: &std::sync::Arc<ira_input::ControllerRegistry>,
    console_id: &str,
    exe: &str,
    live: (Option<ControllerInputMode>, Option<std::path::PathBuf>),
) -> Result<(), String> {
    if exe.trim().is_empty() {
        return Err(crate::tr!("No emulator configured").to_string());
    }

    let cc = cfg.console(console_id);
    let (override_mode, selected) =
        merge_console_overrides(live, cc.controller_mode, &cc.controller_profile);
    let device_default = active_controller_input(cfg, save_dir, registry);
    let mode = console_input_mode(None, override_mode, device_default.0, selected.as_deref());
    let layout = resolve_input_profile(mode, selected.as_deref(), device_default.1.as_deref())?;

    let mut cmd = ira_platforms::emulator_detect::build_command_with_filesystem(exe, &[], None);
    if mode != ControllerInputMode::Disabled {
        let calibration = ira_input::calibration_store_path(save_dir);
        ira_launcher::env_builder::wrap_with_input(
            &mut cmd,
            layout.as_deref(),
            Some(calibration.to_str().unwrap_or_default()),
            true,
        )?;
    }
    let env = ira_launcher::env_builder::clean_parent_env();
    ira_launcher::wrapper::spawn_detached(
        &cmd,
        &env,
        0,
        format!("Started {} (no game)", console_id),
    )
}

#[cfg(test)]
mod tests {
    use super::merge_console_overrides;
    use ira_models::ControllerInputMode;

    #[test]
    fn test_merge_console_overrides_prefers_live_values() {
        let live = (
            Some(ControllerInputMode::Enabled),
            Some(std::path::PathBuf::from("/live/layout.json")),
        );
        assert_eq!(
            merge_console_overrides(live, Some(ControllerInputMode::Disabled), "/saved"),
            (
                Some(ControllerInputMode::Enabled),
                Some("/live/layout.json".to_string())
            )
        );
    }

    #[test]
    fn test_merge_console_overrides_falls_back_to_saved() {
        assert_eq!(
            merge_console_overrides((None, None), None, "/saved.json"),
            (None, Some("/saved.json".to_string()))
        );
        assert_eq!(merge_console_overrides((None, None), None, ""), (None, None));
    }
}
