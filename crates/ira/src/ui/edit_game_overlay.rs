use super::settings_dialog;
use super::system_settings::{build_override_switch_row, OverrideState};
use adw::prelude::*;
use ira_models::GameLaunchConfig;

#[derive(Clone)]
pub(super) struct OverlayWidgets {
    pub overlay_state: OverrideState,
    pub overlay_encoder_row: Option<adw::ComboRow>,
    pub overlay_quality_row: Option<adw::ComboRow>,
}

pub(super) struct OverlayPageParams<'a> {
    pub launch: &'a GameLaunchConfig,
    pub overlay_default: bool,
    pub sidebar: &'a gtk4::ListBox,
    pub stack: &'a gtk4::Stack,
}

pub(super) fn build_overlay_page(params: OverlayPageParams) -> OverlayWidgets {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let (overlay_row, overlay_state) = build_override_switch_row(
        "In-game overlay",
        "Achievements, screenshots, and recording",
        params.overlay_default,
        params.launch.overlay_enabled,
    );
    let overlay_group = adw::PreferencesGroup::new();
    overlay_group.add(&overlay_row);
    page.append(&overlay_group);

    let overlay_cfg_group = adw::PreferencesGroup::new();
    overlay_cfg_group.set_title("Overlay");

    let encoder_model = gtk4::StringList::new(&[
        "Default",
        "Auto",
        "VAAPI (AMD/Intel)",
        "NVENC (NVIDIA)",
        "Software (CPU)",
    ]);
    let encoder_row = adw::ComboRow::new();
    encoder_row.set_title("Video encoder");
    encoder_row.set_model(Some(&encoder_model));
    encoder_row.set_selected(params.launch.overlay_encoder.map(|v| v + 1).unwrap_or(0));
    overlay_cfg_group.add(&encoder_row);

    let quality_model = gtk4::StringList::new(&[
        "Default",
        "Low (720p 30fps)",
        "Medium (1080p 30fps)",
        "High (1080p 60fps)",
    ]);
    let quality_row = adw::ComboRow::new();
    quality_row.set_title("Recording quality");
    quality_row.set_model(Some(&quality_model));
    quality_row.set_selected(
        params
            .launch
            .overlay_recording_quality
            .map(|v| v + 1)
            .unwrap_or(0),
    );
    overlay_cfg_group.add(&quality_row);
    page.append(&overlay_cfg_group);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&page));
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    params
        .sidebar
        .append(&settings_dialog::settings_sidebar_row(
            "layers-symbolic",
            "Overlay",
            "overlay",
        ));
    params.stack.add_named(&scroll, Some("overlay"));

    OverlayWidgets {
        overlay_state,
        overlay_encoder_row: Some(encoder_row),
        overlay_quality_row: Some(quality_row),
    }
}
