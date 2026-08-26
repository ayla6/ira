//! Search Steam's community controller layouts and import one as an Ira
//! profile: app-id/text search through `IPublishedFileService`, CDN download,
//! VDF→`InputProfile` conversion, then a save into `controller_profiles`.

use super::css::{CSS_BOXED_LIST, CSS_DIM_LABEL, CSS_SUGGESTED_ACTION};
use super::helpers::{clear_children, esc};
use super::input_profile_store::{new_managed_profile_path, write_profile};
use adw::prelude::*;
use ira_api::steam_input::{SteamLayout, SteamLayoutQuery, SteamLayoutSort};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};

/// Game context that pre-fills the query and scopes profile attachment.
#[derive(Clone)]
pub struct SteamLayoutSearchContext {
    pub game_id: i64,
    pub game_name: String,
    /// Non-empty for games with a known Steam app id.
    pub steam_app_id: String,
}

const SORTS: [SteamLayoutSort; 4] = [
    SteamLayoutSort::BestMatch,
    SteamLayoutSort::Trending30Days,
    SteamLayoutSort::MostSubscribed,
    SteamLayoutSort::Newest,
];

const RESULTS_PER_PAGE: u32 = 50;

/// Everything the dialog's async pieces need; kept off worker threads
/// (GTK widgets are main-thread only — workers get plain owned data).
struct DialogContext {
    steam: Arc<ira_api::SteamDataClient>,
    save_dir: String,
    /// Internal game id new profiles are attached to, when opened per-game.
    attach_game_id: Option<i64>,
    on_imported: Rc<dyn Fn(PathBuf)>,
    /// Steam app id scoping ("This game only"), when opened per-game.
    steam_app_id: Option<String>,
    list: gtk4::ListBox,
    entry: gtk4::Entry,
    sort_dropdown: gtk4::DropDown,
    app_only: gtk4::CheckButton,
    status: gtk4::Label,
}

fn sort_label(sort: SteamLayoutSort) -> String {
    match sort {
        SteamLayoutSort::BestMatch => crate::tr!("Best match"),
        SteamLayoutSort::Trending30Days => crate::tr!("Trending (30 days)"),
        SteamLayoutSort::MostSubscribed => crate::tr!("Most subscribed"),
        SteamLayoutSort::Newest => crate::tr!("Newest"),
    }
}

/// Poll a worker-thread channel on the GTK main loop without blocking it;
/// one value is delivered to `on_value`. A dropped sender ends polling
/// silently — a panicking worker has nothing useful to report anyway.
fn poll_channel<T: Send + 'static>(rx: mpsc::Receiver<T>, on_value: impl FnOnce(T) + 'static) {
    let rx = Rc::new(RefCell::new(Some(rx)));
    let on_value = Rc::new(RefCell::new(Some(on_value)));
    glib::source::idle_add_local_full(glib::Priority::DEFAULT, move || {
        let polled = {
            let rx = rx.borrow();
            rx.as_ref().map(|receiver| receiver.try_recv())
        };
        let Some(polled) = polled else {
            return glib::ControlFlow::Break;
        };
        match polled {
            Ok(value) => {
                if let Some(on_value) = on_value.borrow_mut().take() {
                    on_value(value);
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

pub fn show_steam_layout_search(
    parent: &adw::Window,
    steam: &Arc<ira_api::SteamDataClient>,
    save_dir: &str,
    context: Option<SteamLayoutSearchContext>,
    on_imported: Rc<dyn Fn(PathBuf)>,
) {
    let (steam, save_dir) = (steam.clone(), save_dir.to_string());

    let dialog = adw::Window::new();
    dialog.set_default_size(560, 520);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(parent));

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk4::Label::new(Some(&crate::tr!(
        "Import layout from Steam"
    )))));
    outer.append(&header);

    // Query row: free-text search plus, when a specific game is known, the
    // choice between its own layout pool and the whole workshop.
    let filter_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    filter_row.set_margin_start(12);
    filter_row.set_margin_end(12);
    filter_row.set_margin_top(8);

    let entry = gtk4::Entry::new();
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some(&crate::tr!("Search community layouts…")));
    if let Some(context) = &context {
        entry.set_text(&context.game_name);
    }
    filter_row.append(&entry);

    let sort_labels: Vec<String> = SORTS.iter().map(|sort| sort_label(*sort)).collect();
    let sort_dropdown =
        gtk4::DropDown::from_strings(&sort_labels.iter().map(String::as_str).collect::<Vec<_>>());
    filter_row.append(&sort_dropdown);

    let search_btn = gtk4::Button::with_label(&crate::tr!("Search"));
    search_btn.add_css_class(CSS_SUGGESTED_ACTION);
    filter_row.append(&search_btn);

    // A known Steam app id scopes the initial query to that game's pool;
    // everything else searches all workshop layouts by text only.
    let app_id = context
        .as_ref()
        .map(|context| context.steam_app_id.trim())
        .filter(|app_id| !app_id.is_empty())
        .map(str::to_string);
    let app_only = gtk4::CheckButton::with_label(&crate::tr!("This game only"));
    app_only.set_active(app_id.is_some());
    app_only.set_visible(app_id.is_some());
    filter_row.append(&app_only);
    outer.append(&filter_row);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_margin_top(8);
    let list = gtk4::ListBox::new();
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_valign(gtk4::Align::Start);
    list.add_css_class(CSS_BOXED_LIST);
    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let status = gtk4::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class(CSS_DIM_LABEL);
    status.set_margin_start(12);
    status.set_margin_end(12);
    status.set_margin_top(6);
    status.set_margin_bottom(10);
    outer.append(&status);

    let ctx = Rc::new(DialogContext {
        steam,
        save_dir,
        attach_game_id: context.as_ref().map(|context| context.game_id),
        on_imported,
        steam_app_id: app_id.clone(),
        list,
        entry: entry.clone(),
        sort_dropdown,
        app_only,
        status: status.clone(),
    });

    {
        let ctx = ctx.clone();
        search_btn.connect_clicked(move |_| run_search(&ctx));
    }
    {
        let ctx = ctx.clone();
        entry.connect_activate(move |_| run_search(&ctx));
    }

    dialog.set_content(Some(&outer));
    dialog.present();
    run_search(&ctx);
}

fn run_search(ctx: &Rc<DialogContext>) {
    let term = ctx.entry.text().trim().to_string();
    let scoped_to_game = ctx.app_only.is_visible() && ctx.app_only.is_active();
    let app_id = if scoped_to_game {
        ctx.steam_app_id.clone()
    } else {
        None
    };
    let query = SteamLayoutQuery {
        search_text: term,
        app_id,
        page: 1,
        page_size: RESULTS_PER_PAGE,
        sort: SORTS[ctx.sort_dropdown.selected() as usize],
    };
    clear_children(&ctx.list);
    ctx.status.set_text(&crate::tr!("Searching…"));

    let (tx, rx) = mpsc::channel::<Result<Vec<SteamLayout>, String>>();
    let steam = ctx.steam.clone();
    std::thread::spawn(move || {
        let _ = tx.send(steam.query_steam_layouts(&query));
    });

    let search_ctx = ctx.clone();
    poll_channel(rx, move |outcome| {
        populate_results(&search_ctx, outcome);
    });
}

fn populate_results(ctx: &Rc<DialogContext>, outcome: Result<Vec<SteamLayout>, String>) {
    clear_children(&ctx.list);
    match &outcome {
        Err(error) if error.contains("no Steam Web API key") => {
            ctx.status.set_text(&crate::tr!(
                "A Steam Web API key is needed to browse community layouts. Add one under Settings."
            ));
        }
        Err(error) => {
            ctx.status
                .set_text(&crate::tr!("Search failed: {}").replacen("{}", error, 1));
        }
        Ok(layouts) if layouts.is_empty() => {
            ctx.status.set_text(&crate::tr!("No layouts found"));
        }
        Ok(_) => {}
    }
    let Ok(layouts) = outcome else {
        return;
    };
    ctx.status.set_text("");
    for layout in layouts {
        ctx.list.append(&layout_row(ctx, &layout));
    }
}

/// An owned, `Send` unit of work for the download/import worker thread.
struct ImportJob {
    published_file_id: String,
    /// Workshop title; used when the VDF itself carries no usable name.
    title: String,
    save_dir: String,
    attach_game_id: Option<i64>,
}

fn layout_row(ctx: &Rc<DialogContext>, layout: &SteamLayout) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(&layout_display_name(layout)));
    row.set_subtitle(&layout_subtitle(layout));

    let import_btn = gtk4::Button::with_label(&crate::tr!("Import"));
    import_btn.add_css_class(CSS_SUGGESTED_ACTION);
    import_btn.set_valign(gtk4::Align::Center);
    {
        let ctx = ctx.clone();
        let layout = layout.clone();
        import_btn.connect_clicked(move |button| start_import(&ctx, &layout, button));
    }
    row.add_suffix(&import_btn);
    row
}

fn layout_display_name(layout: &SteamLayout) -> String {
    if layout.title.trim().is_empty() {
        format!("Steam layout {}", layout.published_file_id)
    } else {
        layout.title.clone()
    }
}

fn start_import(ctx: &Rc<DialogContext>, layout: &SteamLayout, button: &gtk4::Button) {
    button.set_sensitive(false);
    let display = layout_display_name(layout);
    ctx.status
        .set_text(&crate::tr!("Downloading {}…").replacen("{}", &display, 1));

    let job = ImportJob {
        published_file_id: layout.published_file_id.clone(),
        title: layout.title.clone(),
        save_dir: ctx.save_dir.clone(),
        attach_game_id: ctx.attach_game_id,
    };
    let steam = ctx.steam.clone();
    let (tx, rx) = mpsc::channel::<Result<(PathBuf, Vec<String>), String>>();
    std::thread::spawn(move || {
        let _ = tx.send(import_layout(steam, job));
    });

    let ui_ctx = ctx.clone();
    poll_channel(rx, move |result| finish_import(&ui_ctx, result));
}

/// Download the workshop VDF, convert it and store it as a managed profile.
fn import_layout(
    steam: Arc<ira_api::SteamDataClient>,
    job: ImportJob,
) -> Result<(PathBuf, Vec<String>), String> {
    let vdf = steam.fetch_steam_layout_vdf(&job.published_file_id)?;
    let (mut profile, report) = ira_input::import_vdf(&vdf)?;
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
    Ok((path, report.warnings))
}

fn finish_import(ctx: &Rc<DialogContext>, result: Result<(PathBuf, Vec<String>), String>) {
    match result {
        Ok((path, warnings)) => {
            let detail = if warnings.is_empty() {
                crate::tr!("Layout imported")
            } else {
                crate::tr!("Layout imported with {} warnings").replacen(
                    "{}",
                    &warnings.len().to_string(),
                    1,
                )
            };
            let saved_as = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            ctx.status.set_text(&format!("{detail} · {saved_as}"));
            (ctx.on_imported)(path);
        }
        Err(error) => {
            ctx.status
                .set_text(&crate::tr!("Import failed: {}").replacen("{}", &error, 1));
        }
    }
}

fn layout_subtitle(layout: &SteamLayout) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !layout.controller_type.is_empty() {
        parts.push(controller_label(&layout.controller_type));
    }
    if layout.lifetime_subscriptions > 0 {
        parts.push(crate::tr!("{} subscribers").replacen(
            "{}",
            &layout.lifetime_subscriptions.to_string(),
            1,
        ));
    }
    if layout.votes_up > 0 {
        parts.push(crate::tr!("{} upvotes").replacen("{}", &layout.votes_up.to_string(), 1));
    }
    if let Some(date) = chrono::DateTime::from_timestamp(layout.time_updated, 0) {
        parts.push(date.format("%Y-%m-%d").to_string());
    }
    if layout.description.trim().is_empty() {
        parts.join(" · ")
    } else {
        // Keep the blurb short: the row title already carries the name.
        let mut description = layout
            .description
            .trim()
            .chars()
            .take(80)
            .collect::<String>();
        if layout.description.trim().chars().count() > 80 {
            description.push('…');
        }
        if parts.is_empty() {
            description
        } else {
            format!("{} — {}", parts.join(" · "), description)
        }
    }
}

/// "controller_ps5" → "DualSense", "controller_switch" → "Switch Pro", etc.
fn controller_label(tag: &str) -> String {
    let kind = tag.trim_start_matches("controller_");
    let label = match kind {
        "ps5" | "dualsense" => Some("DualSense"),
        "ps4" | "dualshock4" => Some("DualShock 4"),
        "switch" | "switch_pro" => Some("Switch Pro"),
        "xboxone" | "xbox360" | "xboxelite" | "xbox" => Some("Xbox"),
        "neptune" => Some("Steam Deck"),
        _ => None,
    };
    match label {
        Some(label) => label.to_string(),
        None => capitalize(kind),
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
