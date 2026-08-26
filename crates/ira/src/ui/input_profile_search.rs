//! Search Steam's community controller layouts: app-id/text search through
//! `IPublishedFileService` with tag filters, showing results in an
//! `AdwNavigationView` whose preview page handles the actual import.
//! Filter and sort options mirror steaminputdb's search form.

use super::css::{CSS_BOXED_LIST, CSS_SUGGESTED_ACTION};
use super::helpers::{
    clamped, clamped_boxed_list, clear_children, esc, poll_channel, status_row, SearchStatus,
};
use super::input_profile_preview::{
    controller_display_label, controller_filter_options, layout_display_name, PreviewRequest,
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

const SORTS: [SteamLayoutSort; 6] = [
    SteamLayoutSort::Rank,
    SteamLayoutSort::PublicationDate,
    SteamLayoutSort::Trending30Days,
    SteamLayoutSort::TotalSubscriptions,
    SteamLayoutSort::VotesUp,
    SteamLayoutSort::TextSearch,
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
    sort_row: adw::ComboRow,
    controller_row: adw::ComboRow,
    gyro_row: adw::SwitchRow,
    app_only_row: adw::SwitchRow,
    status: SearchStatus,
    /// One preview download at a time.
    loading: Rc<Cell<bool>>,
}

/// The search page's interactive widgets, kept out of the widget tree so the
/// dialog context can hold direct handles instead of walking the tree.
struct SearchWidgets {
    page: adw::NavigationPage,
    entry: gtk4::SearchEntry,
    sort_row: adw::ComboRow,
    controller_row: adw::ComboRow,
    gyro_row: adw::SwitchRow,
    app_only_row: adw::SwitchRow,
    list: gtk4::ListBox,
    status: SearchStatus,
}

fn sort_label(sort: SteamLayoutSort) -> String {
    match sort {
        SteamLayoutSort::Rank => crate::tr!("Rank"),
        SteamLayoutSort::PublicationDate => crate::tr!("Date"),
        SteamLayoutSort::Trending30Days => crate::tr!("Trending (30 days)"),
        SteamLayoutSort::TotalSubscriptions => crate::tr!("Most subscribed"),
        SteamLayoutSort::VotesUp => crate::tr!("Most upvoted"),
        SteamLayoutSort::TextSearch => crate::tr!("Relevance"),
    }
}

pub fn show_steam_layout_search(
    parent: &impl IsA<gtk4::Widget>,
    steam: &Arc<ira_api::SteamDataClient>,
    save_dir: &str,
    context: Option<SteamLayoutSearchContext>,
    on_imported: Rc<dyn Fn(PathBuf)>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Import layout from Steam"));
    dialog.set_content_width(560);
    dialog.set_content_height(560);

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
        sort_row: widgets.sort_row,
        controller_row: widgets.controller_row,
        gyro_row: widgets.gyro_row,
        app_only_row: widgets.app_only_row,
        status: widgets.status,
        loading: Rc::new(Cell::new(false)),
    });

    // Like steaminputdb's form: changing any filter re-submits the search.
    ctx.entry.connect_activate({
        let ctx = ctx.clone();
        move |_| run_search(&ctx)
    });
    for changed in [&ctx.sort_row, &ctx.controller_row] {
        let ctx = ctx.clone();
        changed.connect_selected_notify(move |_| run_search(&ctx));
    }
    for changed in [&ctx.gyro_row, &ctx.app_only_row] {
        let ctx = ctx.clone();
        changed.connect_active_notify(move |_| run_search(&ctx));
    }
    run_search(&ctx);

    dialog.set_child(Some(&ctx.nav.clone()));
    dialog.present(Some(parent));
}

fn build_search_ui(context: &Option<SteamLayoutSearchContext>) -> SearchWidgets {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar.add_top_bar(&header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // Search row plus filter card, clamped together like the other dialogs.
    let query_column = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let entry = gtk4::SearchEntry::new();
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some(&crate::tr!("Search community layouts…")));
    if let Some(context) = context {
        entry.set_text(&context.game_name);
    }
    search_row.append(&entry);
    let search_btn = gtk4::Button::with_label(&crate::tr!("Search"));
    search_btn.add_css_class(CSS_SUGGESTED_ACTION);
    search_row.append(&search_btn);
    query_column.append(&search_row);

    // Filters in one boxed list, like steaminputdb's card: sort, controller
    // kind, feature toggles — each an ordinary Adwaita row.
    let filters = gtk4::ListBox::new();
    filters.add_css_class(CSS_BOXED_LIST);
    filters.set_selection_mode(gtk4::SelectionMode::None);

    let sort_labels: Vec<String> = SORTS.iter().map(|sort| sort_label(*sort)).collect();
    let sort_row = adw::ComboRow::new();
    sort_row.set_title(&crate::tr!("Sort by"));
    sort_row.set_model(Some(&gtk4::StringList::new(
        &sort_labels.iter().map(String::as_str).collect::<Vec<_>>(),
    )));
    filters.append(&sort_row);

    let mut controller_labels = vec![crate::tr!("Any controller")];
    controller_labels.extend(
        controller_filter_options()
            .into_iter()
            .map(|(label, _)| label),
    );
    let controller_row = adw::ComboRow::new();
    controller_row.set_title(&crate::tr!("Controller"));
    controller_row.set_model(Some(&gtk4::StringList::new(
        &controller_labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    )));
    filters.append(&controller_row);

    let gyro_row = adw::SwitchRow::new();
    gyro_row.set_title(&crate::tr!("Uses gyro"));
    gyro_row.set_subtitle(&crate::tr!("Only layouts that use gyro aiming or motion"));
    filters.append(&gyro_row);

    // A known Steam app id scopes the query to that game's pool; everything
    // else searches all workshop layouts by text only.
    let has_app_context = context
        .as_ref()
        .is_some_and(|context| !context.steam_app_id.is_empty());
    let app_only_row = adw::SwitchRow::new();
    app_only_row.set_title(&crate::tr!("This game only"));
    app_only_row.set_subtitle(&crate::tr!("Only layouts published for this game"));
    app_only_row.set_active(has_app_context);
    app_only_row.set_visible(has_app_context);
    filters.append(&app_only_row);
    query_column.append(&filters);

    content.append(&clamped(&query_column, 560, (12, 0, 12, 12)));

    let (scrolled, list) = clamped_boxed_list(560);
    let status = SearchStatus::for_list(&list);
    content.append(&scrolled);

    toolbar.set_content(Some(&content));
    let page = adw::NavigationPage::new(&toolbar, "search");

    SearchWidgets {
        page,
        entry,
        sort_row,
        controller_row,
        gyro_row,
        app_only_row,
        list,
        status,
    }
}

fn run_search(ctx: &Rc<DialogContext>) {
    let term = ctx.entry.text().trim().to_string();
    let scoped_to_game = ctx.app_only_row.is_visible() && ctx.app_only_row.is_active();
    let app_id = if scoped_to_game {
        ctx.steam_app_id.clone()
    } else {
        None
    };
    let mut required_tags = Vec::new();
    if ctx.gyro_row.is_active() {
        required_tags.push(GYRO_TAG.to_string());
    }
    if ctx.controller_row.selected() > 0 {
        if let Some((_, tag)) =
            controller_filter_options().get(ctx.controller_row.selected() as usize - 1)
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
        sort: SORTS[ctx.sort_row.selected() as usize],
    };
    clear_children(&ctx.list);
    ctx.status.show(&crate::tr!("Searching…"));

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
    ctx.status.clear();
    clear_children(&ctx.list);
    match &outcome {
        Err(error) if error.contains("no Steam Web API key") => {
            ctx.list.append(&status_row(&crate::tr!(
                "A Steam Web API key is needed to browse community layouts. Add one under Settings."
            )));
        }
        Err(error) => {
            ctx.list.append(&status_row(
                &crate::tr!("Search failed: {}").replacen("{}", error, 1),
            ));
        }
        Ok(layouts) if layouts.is_empty() => {
            ctx.list
                .append(&status_row(&crate::tr!("No layouts found")));
        }
        Ok(layouts) => {
            for layout in layouts {
                ctx.list.append(&layout_row(ctx, layout));
            }
        }
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
        PreviewRequest {
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
    // Keep the blurb short: the preview page shows the full text. The row
    // subtitle parses Pango markup, so escape the workshop text.
    let mut description = layout
        .description
        .trim()
        .chars()
        .take(80)
        .collect::<String>();
    if layout.description.trim().chars().count() > 80 {
        description.push('…');
    }
    let description = esc(&description);
    if parts.is_empty() {
        description
    } else {
        format!("{} — {}", parts.join(" · "), description)
    }
}
