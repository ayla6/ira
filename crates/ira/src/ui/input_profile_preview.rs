//! Layout preview for the Steam community-layout search: downloads and
//! converts a workshop layout without saving, renders the resulting action
//! sets and bindings, and performs the actual import from the preview page.

use super::css::{CSS_DIM_LABEL, CSS_SUGGESTED_ACTION};
use super::helpers::{esc, poll_channel, SearchStatus};
use super::input_profile_editor_regions::source_label;
use super::input_profile_options::output_display_label;
use super::input_profile_search_filters::controller_display_label;
use super::input_profile_store::{new_managed_profile_path, write_profile};
use adw::prelude::*;
use ira_api::steam_input::SteamLayout;
use ira_input::{ActivatorKind, InputMapping, InputProfile, SourceMode};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};

pub(super) fn layout_display_name(layout: &SteamLayout) -> String {
    if layout.title.trim().is_empty() {
        format!("Steam layout {}", layout.published_file_id)
    } else {
        layout.title.clone()
    }
}

/// Everything needed to open a layout's preview page; fully owned so it can
/// be built on row activation and consumed by the worker hand-off.
pub(super) struct PreviewRequest {
    pub nav: adw::NavigationView,
    pub steam: Arc<ira_api::SteamDataClient>,
    pub save_dir: String,
    pub attach_game_id: Option<i64>,
    pub layout: SteamLayout,
    pub on_imported: Rc<dyn Fn(PathBuf)>,
    pub search_status: SearchStatus,
    pub loading: Rc<Cell<bool>>,
}

impl PreviewRequest {
    /// Download and convert a layout, then push a preview page onto `nav`.
    /// Nothing is written until the preview's import button is used.
    pub(super) fn open(self) {
        let Self {
            nav,
            steam,
            save_dir,
            attach_game_id,
            layout,
            on_imported,
            search_status,
            loading,
        } = self;
        if loading.replace(true) {
            return;
        }
        search_status.show(&crate::tr!("Downloading {}…").replacen(
            "{}",
            &layout_display_name(&layout),
            1,
        ));

        let (tx, rx) = mpsc::channel::<Result<(InputProfile, Vec<String>), String>>();
        let file_id = layout.published_file_id.clone();
        std::thread::spawn(move || {
            let _ = tx.send(convert_layout(&steam, &file_id));
        });

        let status = search_status.clone();
        let loading = loading.clone();
        let save_dir = save_dir.clone();
        poll_channel(rx, move |outcome| {
            loading.set(false);
            match outcome {
                Ok((profile, warnings)) => {
                    status.clear();
                    nav.push(&build_preview_page(
                        &layout,
                        profile,
                        warnings,
                        &save_dir,
                        attach_game_id,
                        on_imported,
                    ));
                }
                Err(error) => {
                    status.show(&crate::tr!("Preview failed: {}").replacen("{}", &error, 1));
                }
            }
        });
    }
}

/// Fetch the workshop VDF and map it onto Ira's profile model, without
/// touching the disk. The import button stores the result it produced.
fn convert_layout(
    steam: &Arc<ira_api::SteamDataClient>,
    published_file_id: &str,
) -> Result<(InputProfile, Vec<String>), String> {
    let vdf = steam.fetch_steam_layout_vdf(published_file_id)?;
    let (profile, report) = ira_input::import_vdf(&vdf)?;
    Ok((profile, report.warnings))
}

fn build_preview_page(
    layout: &SteamLayout,
    profile: InputProfile,
    warnings: Vec<String>,
    save_dir: &str,
    attach_game_id: Option<i64>,
    on_imported: Rc<dyn Fn(PathBuf)>,
) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk4::Label::new(Some(&esc(&layout_display_name(
        layout,
    ))))));
    toolbar.add_top_bar(&header);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    scrolled.set_child(Some(&content));
    toolbar.set_content(Some(&scrolled));

    let import_status = gtk4::Label::new(None);
    import_status.set_hexpand(true);
    import_status.set_xalign(0.0);
    import_status.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    import_status.add_css_class(CSS_DIM_LABEL);
    let import_btn = gtk4::Button::with_label(&crate::tr!("Import layout"));
    import_btn.add_css_class(CSS_SUGGESTED_ACTION);
    let import_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    import_row.set_margin_top(8);
    import_row.set_margin_bottom(8);
    import_row.set_margin_start(12);
    import_row.set_margin_end(12);
    import_row.append(&import_status);
    import_row.append(&import_btn);
    toolbar.add_bottom_bar(&import_row);

    content.append(&summary_group(layout, &profile));
    append_description(&content, layout);
    for set in &profile.action_sets {
        content.append(&bindings_group(set));
    }
    append_warnings(&content, &warnings);

    let job = StoreJob {
        profile,
        title: layout.title.clone(),
        published_file_id: layout.published_file_id.clone(),
        save_dir: save_dir.to_string(),
        attach_game_id,
    };
    {
        let status = import_status.clone();
        let btn = import_btn.clone();
        import_btn.connect_clicked(move |_| {
            start_store(job.clone(), &status, &btn, on_imported.clone());
        });
    }

    let page = adw::NavigationPage::new(&toolbar, "preview");
    page.set_title(&layout_display_name(layout));
    page
}

fn summary_group(layout: &SteamLayout, profile: &InputProfile) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Layout"));

    if !layout.controller_type.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&crate::tr!("Controller"));
        row.add_css_class(CSS_DIM_LABEL);
        row.set_subtitle(&controller_display_label(&layout.controller_type));
        group.add(&row);
    }
    let mut community = Vec::new();
    if layout.lifetime_subscriptions > 0 {
        community.push(crate::tr!("{} subscribers").replacen(
            "{}",
            &layout.lifetime_subscriptions.to_string(),
            1,
        ));
    }
    if layout.votes_up > 0 {
        community.push(crate::tr!("{} upvotes").replacen("{}", &layout.votes_up.to_string(), 1));
    }
    if !community.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&crate::tr!("Community"));
        row.add_css_class(CSS_DIM_LABEL);
        row.set_subtitle(&community.join(" · "));
        group.add(&row);
    }
    if layout.time_updated > 0 {
        let row = adw::ActionRow::new();
        row.set_title(&crate::tr!("Updated"));
        row.add_css_class(CSS_DIM_LABEL);
        let date = chrono::DateTime::from_timestamp(layout.time_updated, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        row.set_subtitle(&date);
        group.add(&row);
    }
    let sets = profile.action_sets.len();
    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Action sets"));
    row.add_css_class(CSS_DIM_LABEL);
    row.set_subtitle(&sets.to_string());
    group.add(&row);
    group
}

fn append_description(content: &gtk4::Box, layout: &SteamLayout) {
    if layout.description.trim().is_empty() {
        return;
    }
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Description"));
    let label = gtk4::Label::new(Some(&layout.description));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_selectable(true);
    group.add(&label);
    content.append(&group);
}

fn bindings_group(set: &ira_input::ActionSet) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let name = if set.name.trim().is_empty() {
        crate::tr!("Action set")
    } else {
        set.name.clone()
    };
    group.set_title(&esc(&name));
    for input in &set.inputs {
        let Some(outputs) = binding_subtitle(input) else {
            continue;
        };
        let row = adw::ActionRow::new();
        row.set_title(&esc(&source_label(input.source)));
        row.add_css_class(CSS_DIM_LABEL);
        row.set_subtitle(&outputs);
        group.add(&row);
    }
    group
}

/// `None` for inputs the importer kept but that fire nothing.
fn binding_subtitle(input: &InputMapping) -> Option<String> {
    let parts: Vec<String> = input
        .activators
        .iter()
        .filter(|activator| !activator.outputs.is_empty())
        .map(|activator| {
            let outputs = activator
                .outputs
                .iter()
                .map(output_display_label)
                .collect::<Vec<_>>()
                .join(", ");
            match activator_prefix(&activator.kind) {
                Some(prefix) => format!("{prefix}: {outputs}"),
                None => outputs,
            }
        })
        .collect();
    if parts.is_empty() {
        return None;
    }
    let note = mode_note(&input.mode);
    if note.is_empty() {
        Some(parts.join(" · "))
    } else {
        Some(format!("{} — {}", note, parts.join(" · ")))
    }
}

fn activator_prefix(kind: &ActivatorKind) -> Option<String> {
    match kind {
        ActivatorKind::FullPress | ActivatorKind::SoftPress { .. } => None,
        ActivatorKind::DoublePress { .. } => Some(crate::tr!("double-press")),
        ActivatorKind::LongPress { .. } => Some(crate::tr!("long-press")),
        ActivatorKind::StartPress => Some(crate::tr!("on start")),
        ActivatorKind::Release => Some(crate::tr!("on release")),
    }
}

/// Short note for analog sources that do something other than act as a
/// plain stick/trigger.
fn mode_note(mode: &Option<SourceMode>) -> String {
    match mode {
        Some(SourceMode::Mouse { .. }) => crate::tr!("mouse"),
        Some(SourceMode::Flickstick { .. }) => crate::tr!("flick stick"),
        Some(SourceMode::Dpad { .. }) => crate::tr!("d-pad"),
        _ => String::new(),
    }
}

fn append_warnings(content: &gtk4::Box, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Import notes"));
    let label = gtk4::Label::new(Some(&warnings.join("\n")));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.add_css_class(CSS_DIM_LABEL);
    group.add(&label);
    content.append(&group);
}

/// Owned, `Send` payload for the store worker thread.
#[derive(Clone)]
struct StoreJob {
    profile: InputProfile,
    title: String,
    published_file_id: String,
    save_dir: String,
    attach_game_id: Option<i64>,
}

fn start_store(
    job: StoreJob,
    status: &gtk4::Label,
    button: &gtk4::Button,
    on_imported: Rc<dyn Fn(PathBuf)>,
) {
    button.set_sensitive(false);
    status.set_text(&crate::tr!("Saving…"));

    let (tx, rx) = mpsc::channel::<Result<PathBuf, String>>();
    std::thread::spawn(move || {
        let _ = tx.send(store_profile(job));
    });

    let status = status.clone();
    let button = button.clone();
    poll_channel(rx, move |result| match result {
        Ok(path) => {
            status.set_text(&crate::tr!("Layout imported"));
            on_imported(path);
        }
        Err(error) => {
            status.set_text(&crate::tr!("Import failed: {}").replacen("{}", &error, 1));
            button.set_sensitive(true);
        }
    });
}

/// Write the converted profile into the managed profile pool, attaching the
/// game when the dialog was opened with one.
fn store_profile(job: StoreJob) -> Result<PathBuf, String> {
    let mut profile = job.profile;
    if profile.name.trim().is_empty() {
        profile.name = if job.title.trim().is_empty() {
            format!("Steam layout {}", job.published_file_id)
        } else {
            job.title.clone()
        };
    }
    if let Some(game_id) = job.attach_game_id {
        if !profile.compatible_game_ids.contains(&game_id) {
            profile.compatible_game_ids.push(game_id);
        }
    }
    let path = new_managed_profile_path(&job.save_dir, &profile.name);
    write_profile(&path, &profile)?;
    Ok(path)
}
