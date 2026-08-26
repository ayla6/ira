//! Search Steam's community controller layouts: app-id/text search through
//! `IPublishedFileService` with tag filters, showing results in an
//! `AdwNavigationView` whose preview page handles the actual import.

use super::css::{CSS_BOXED_LIST, CSS_DIM_LABEL, CSS_SUGGESTED_ACTION};
use super::helpers::{clear_children, esc, poll_channel};
use super::input_profile_preview::{
    controller_display_label, controller_filter_options, layout_display_name,
};
use adw::prelude::*;
use ira_api::steam_input::{SteamLayout, SteamLayoutQuery, SteamLayoutSort};
use std::cell::Cell;
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
const GYRO_TAG: &str = "feature_gyro";

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
    nav: adw::NavigationView,
    list: gtk4::ListBox,
    entry: gtk4::SearchEntry,
    sort_dropdown: gtk4::DropDown,
    controller_dropdown: gtk4::DropDown,
    gyro_check: gtk4::CheckButton,
    app_only: gtk4::CheckButton,
    status: gtk4::Label,
    /// One preview download at a time.
    loading: Rc<Cell<bool>>,
}

/// The search page's interactive widgets, kept out of the widget tree so the
/// dialog context can hold direct handles instead of walking the tree.
struct SearchWidgets {
    page: adw::NavigationPage,
    entry: gtk4::SearchEntry,
    search_btn: gtk4::Button,
    sort_dropdown: gtk4::DropDown,
    controller_dropdown: gtk4::DropDown,
    gyro_check: gtk4::CheckButton,
    app_only: gtk4::CheckButton,
    list: gtk4::ListBox,
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

pub fn show_steam_layout_search(
    parent: &adw::Window,
    steam: &Arc<ira_api::SteamDataClient>,
    save_dir: &str,
    context: Option<SteamLayoutSearchContext>,
    on_imported: Rc<dyn Fn(PathBuf)>,
) {
    let dialog = adw::Window::new();
    dialog.set_default_size(560, 540);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(parent));

    let nav = adw::NavigationView::new();
    let widgets = build_search_ui(&context);
    nav.add(&widgets.page);

    // A known Steam app id scopes the query to that game's pool; everything
    // else searches all workshop layouts by text only.
    let app_id = context
        .as_ref()
        .map(|context| context.steam_app_id.trim())
        .filter(|app_id| !app_id.is_empty())
        .map(str::to_string);
    let ctx = Rc::new(DialogContext {
        steam: steam.clone(),
        save_dir: save_dir.to_string(),
        attach_game_id: context.as_ref().map(|context| context.game_id),
        on_imported,
        steam_app_id: app_id,
        nav,
        list: widgets.list,
        entry: widgets.entry,
        sort_dropdown: widgets.sort_dropdown,
        controller_dropdown: widgets.controller_dropdown,
        gyro_check: widgets.gyro_check,
        app_only: widgets.app_only,
        status: widgets.status,
        loading: Rc::new(Cell::new(false)),
    });

    ctx.entry.connect_activate({
        let ctx = ctx.clone();
        move |_| run_search(&ctx)
    });
    {
        let ctx = ctx.clone();
        widgets
            .search_btn
            .connect_clicked(move |_| run_search(&ctx));
    }
    run_search(&ctx);

    dialog.set_content(Some(&ctx.nav.clone()));
    dialog.present();
}

fn build_search_ui(context: &Option<SteamLayoutSearchContext>) -> SearchWidgets {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk4::Label::new(Some(&crate::tr!(
        "Import layout from Steam"
    )))));
    toolbar.add_top_bar(&header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // Query + filters, clamped so wide windows don't stretch them.
    let header_area = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    header_area.set_margin_top(12);
    header_area.set_margin_bottom(4);
    header_area.set_margin_start(12);
    header_area.set_margin_end(12);

    let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let entry = gtk4::SearchEntry::new();
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some(&crate::tr!("Search community layouts…")));
    if let Some(context) = context {
        entry.set_text(&context.game_name);
    }
    search_row.append(&entry);
    let sort_labels: Vec<String> = SORTS.iter().map(|sort| sort_label(*sort)).collect();
    let sort_dropdown =
        gtk4::DropDown::from_strings(&sort_labels.iter().map(String::as_str).collect::<Vec<_>>());
    search_row.append(&sort_dropdown);
    let search_btn = gtk4::Button::with_label(&crate::tr!("Search"));
    search_btn.add_css_class(CSS_SUGGESTED_ACTION);
    search_row.append(&search_btn);
    header_area.append(&search_row);

    let filter_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    let mut controller_labels = vec![crate::tr!("Any controller")];
    controller_labels.extend(
        controller_filter_options()
            .into_iter()
            .map(|(label, _)| label),
    );
    let controller_dropdown = gtk4::DropDown::from_strings(
        &controller_labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    filter_row.append(&controller_dropdown);
    let gyro_check = gtk4::CheckButton::with_label(&crate::tr!("Gyro"));
    gyro_check.set_tooltip_text(Some(&crate::tr!("Only layouts that use gyro")));
    filter_row.append(&gyro_check);
    // A known Steam app id scopes the query to that game's pool; everything
    // else searches all workshop layouts by text only.
    let has_app_context = context
        .as_ref()
        .is_some_and(|context| !context.steam_app_id.is_empty());
    let app_only = gtk4::CheckButton::with_label(&crate::tr!("This game only"));
    app_only.set_active(has_app_context);
    app_only.set_visible(has_app_context);
    filter_row.append(&app_only);
    header_area.append(&filter_row);

    let header_clamp = adw::Clamp::new();
    header_clamp.set_maximum_size(560);
    header_clamp.set_child(Some(&header_area));
    content.append(&header_clamp);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let list_column = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    list_column.set_margin_top(8);
    list_column.set_margin_bottom(10);
    let list = gtk4::ListBox::new();
    list.set_valign(gtk4::Align::Start);
    list.add_css_class(CSS_BOXED_LIST);
    list_column.append(&list);
    let status = gtk4::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class(CSS_DIM_LABEL);
    status.set_margin_start(12);
    status.set_margin_end(12);
    list_column.append(&status);
    let list_clamp = adw::Clamp::new();
    list_clamp.set_maximum_size(560);
    list_clamp.set_child(Some(&list_column));
    scrolled.set_child(Some(&list_clamp));
    content.append(&scrolled);

    toolbar.set_content(Some(&content));
    let page = adw::NavigationPage::new(&toolbar, "search");

    SearchWidgets {
        page,
        entry,
        search_btn,
        sort_dropdown,
        controller_dropdown,
        gyro_check,
        app_only,
        list,
        status,
    }
}

fn run_search(ctx: &Rc<DialogContext>) {
    let term = ctx.entry.text().trim().to_string();
    let scoped_to_game = ctx.app_only.is_visible() && ctx.app_only.is_active();
    let app_id = if scoped_to_game {
        ctx.steam_app_id.clone()
    } else {
        None
    };
    let mut required_tags = Vec::new();
    if ctx.gyro_check.is_active() {
        required_tags.push(GYRO_TAG.to_string());
    }
    if ctx.controller_dropdown.selected() > 0 {
        if let Some((_, tag)) =
            controller_filter_options().get(ctx.controller_dropdown.selected() as usize - 1)
        {
            required_tags.push(tag.clone());
        }
    }
    let query = SteamLayoutQuery {
        search_text: term,
        app_id,
        required_tags,
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

/// Rows open the preview page when activated; import happens from there.
fn layout_row(ctx: &Rc<DialogContext>, layout: &SteamLayout) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(&layout_display_name(layout)));
    row.set_subtitle(&layout_subtitle(layout));
    row.set_activatable(true);

    let request_ctx = ctx.clone();
    let row_layout = layout.clone();
    row.connect_activated(move |_| {
        super::input_profile_preview::PreviewRequest {
            nav: request_ctx.nav.clone(),
            steam: request_ctx.steam.clone(),
            save_dir: request_ctx.save_dir.clone(),
            attach_game_id: request_ctx.attach_game_id,
            layout: row_layout.clone(),
            on_imported: request_ctx.on_imported.clone(),
            search_status: request_ctx.status.clone(),
            loading: request_ctx.loading.clone(),
        }
        .open();
    });
    row
}

fn layout_subtitle(layout: &SteamLayout) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !layout.controller_type.is_empty() {
        parts.push(controller_display_label(&layout.controller_type));
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
        return parts.join(" · ");
    }
    // Keep the blurb short: the preview page shows the full text.
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
