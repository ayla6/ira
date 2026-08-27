//! Search Steam's community controller layouts: app-id/text search through
//! `IPublishedFileService` with tag filters, showing results in an
//! `AdwNavigationView` whose preview page handles the actual import.
//! Filter and sort options mirror steaminputdb's search form.

use super::helpers::{
    clamped, clamped_boxed_list, clear_children, esc, poll_channel, status_row, SearchStatus,
};
use super::input_profile_preview::{
    community_stats, layout_display_name, updated_date, PreviewRequest,
};
use super::input_profile_search_filters::{
    build_filter_card, controller_display_label, controller_filter_options, FeatureRow,
};
use super::steam_search_dialog::build_search_row;
use adw::prelude::*;
use ira_api::steam_input::{SteamLayout, SteamLayoutQuery, SteamLayoutSort};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};

/// Game context that pre-fills the query and scopes profile attachment.
#[derive(Clone)]
pub struct SteamLayoutSearchContext {
    pub game_id: i64,
    pub platform_id: Option<String>,
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
    /// Platform scope attached to imported profiles, when opened per-game.
    platform_id: Option<String>,
    nav: adw::NavigationView,
    list: gtk4::ListBox,
    entry: gtk4::SearchEntry,
    sort_row: adw::ComboRow,
    controller_row: adw::ComboRow,
    include_rows: Rc<Vec<FeatureRow>>,
    exclude_rows: Rc<Vec<FeatureRow>>,
    app_only_row: adw::SwitchRow,
    count_label: gtk4::Label,
    status: SearchStatus,
    /// One preview download at a time.
    loading: Rc<Cell<bool>>,
    /// Which results page is displayed (1-based).
    page: Cell<u32>,
    pager: RefCell<Option<adw::ButtonRow>>,
}

/// The search page's interactive widgets, kept out of the widget tree so the
/// dialog context can hold direct handles instead of walking the tree.
struct SearchWidgets {
    page: adw::NavigationPage,
    entry: gtk4::SearchEntry,
    search_btn: gtk4::Button,
    sort_row: adw::ComboRow,
    controller_row: adw::ComboRow,
    include_rows: Vec<FeatureRow>,
    exclude_rows: Vec<FeatureRow>,
    app_only_row: adw::SwitchRow,
    count_label: gtk4::Label,
    list: gtk4::ListBox,
    status: SearchStatus,
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
        platform_id: context.as_ref().and_then(|context| context.platform_id.clone()),
        on_imported,
        steam_app_id: app_id,
        nav,
        list: widgets.list,
        entry: widgets.entry,
        sort_row: widgets.sort_row,
        controller_row: widgets.controller_row,
        include_rows: Rc::new(widgets.include_rows),
        exclude_rows: Rc::new(widgets.exclude_rows),
        app_only_row: widgets.app_only_row,
        count_label: widgets.count_label,
        status: widgets.status,
        loading: Rc::new(Cell::new(false)),
        page: Cell::new(1),
        pager: RefCell::new(None),
    });

    // Like steaminputdb's form: changing any filter re-submits the search.
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
    for changed in [&ctx.sort_row, &ctx.controller_row] {
        let ctx = ctx.clone();
        changed.connect_selected_notify(move |_| run_search(&ctx));
    }
    let scope_changes: Vec<adw::SwitchRow> = std::iter::once(ctx.app_only_row.clone())
        .chain(ctx.include_rows.iter().map(|row| row.switch.clone()))
        .chain(ctx.exclude_rows.iter().map(|row| row.switch.clone()))
        .collect();
    for changed in scope_changes {
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

    let entry_text = context
        .as_ref()
        .map(|context| context.game_name.as_str())
        .unwrap_or_default();
    let (search_row, entry, search_btn) =
        build_search_row(entry_text, Some(&crate::tr!("Search community layouts…")));
    query_column.append(&search_row);

    // A known Steam app id scopes the query to that game's pool; everything
    // else searches all workshop layouts by text only.
    let has_app_context = context
        .as_ref()
        .is_some_and(|context| !context.steam_app_id.is_empty());
    let card = build_filter_card(&SORTS, has_app_context);
    query_column.append(&card.list);
    content.append(&clamped(&query_column, 560, (12, 0, 12, 12)));

    let count_label = gtk4::Label::new(None);
    count_label.set_xalign(0.0);
    count_label.set_visible(false);
    count_label.add_css_class(super::css::CSS_DIM_LABEL);
    content.append(&clamped(&count_label, 560, (8, 0, 12, 12)));

    let (scrolled, list) = clamped_boxed_list(560);
    let status = SearchStatus::for_list(&list);
    content.append(&scrolled);

    toolbar.set_content(Some(&content));
    let page = adw::NavigationPage::new(&toolbar, "search");

    SearchWidgets {
        page,
        entry,
        search_btn,
        sort_row: card.sort_row,
        controller_row: card.controller_row,
        include_rows: card.include_rows,
        exclude_rows: card.exclude_rows,
        app_only_row: card.app_only_row,
        count_label,
        list,
        status,
    }
}

fn run_search(ctx: &Rc<DialogContext>) {
    ctx.page.set(1);
    // Detach the pager before clearing: removing a row the clear already
    // took makes GTK warn "Tried to remove non-child" and trips a
    // g_sequence_iter critical.
    remove_pager(ctx);
    clear_children(&ctx.list);
    ctx.status.show(&crate::tr!("Searching…"));
    fetch_page(ctx, 1);
}

fn load_more(ctx: &Rc<DialogContext>) {
    remove_pager(ctx);
    let next = ctx.page.get() + 1;
    ctx.page.set(next);
    fetch_page(ctx, next);
}

fn fetch_page(ctx: &Rc<DialogContext>, page: u32) {
    let query = current_query(ctx, page);
    let (tx, rx) = mpsc::channel::<Result<ira_api::steam_input::SteamLayoutPage, String>>();
    let steam = ctx.steam.clone();
    std::thread::spawn(move || {
        let _ = tx.send(steam.query_steam_layouts(&query));
    });

    let poll_ctx = ctx.clone();
    poll_channel(rx, move |outcome| {
        populate_results(&poll_ctx, page, outcome);
    });
}

fn current_query(ctx: &Rc<DialogContext>, page: u32) -> SteamLayoutQuery {
    let term = ctx.entry.text().trim().to_string();
    let scoped_to_game = ctx.app_only_row.is_visible() && ctx.app_only_row.is_active();
    let app_id = if scoped_to_game {
        ctx.steam_app_id.clone()
    } else {
        None
    };
    let mut required_tags = Vec::new();
    if ctx.controller_row.selected() > 0 {
        if let Some((_, tag)) =
            controller_filter_options().get(ctx.controller_row.selected() as usize - 1)
        {
            required_tags.push(tag.clone());
        }
    }
    for row in ctx.include_rows.iter().filter(|row| row.switch.is_active()) {
        required_tags.push(row.tag.clone());
    }
    let excluded_tags = ctx
        .exclude_rows
        .iter()
        .filter(|row| row.switch.is_active())
        .map(|row| row.tag.clone())
        .collect();
    SteamLayoutQuery {
        search_text: term,
        app_id,
        required_tags,
        excluded_tags,
        page,
        page_size: RESULTS_PER_PAGE,
        sort: SORTS[ctx.sort_row.selected() as usize],
    }
}

fn populate_results(
    ctx: &Rc<DialogContext>,
    page: u32,
    outcome: Result<ira_api::steam_input::SteamLayoutPage, String>,
) {
    if page == 1 {
        ctx.status.clear();
        clear_children(&ctx.list);
    }
    match outcome {
        Err(error) if error.contains("no Steam Web API key") => {
            fail_results(
                ctx,
                page,
                &crate::tr!(
                "A Steam Web API key is needed to browse community layouts. Add one under Settings."
            ),
            );
        }
        Err(error) => {
            fail_results(
                ctx,
                page,
                &crate::tr!("Search failed: {}").replacen("{}", &error, 1),
            );
        }
        Ok(page_result) => {
            update_count_label(ctx, page_result.total);
            for layout in &page_result.items {
                ctx.list.append(&layout_row(ctx, layout));
            }
            let shown = page as i64 * RESULTS_PER_PAGE as i64;
            if shown < page_result.total {
                append_pager(ctx);
            }
        }
    }
}

/// First-page failures replace the list; load-more failures surface as a
/// status row and restore the pager so the fetch can be retried.
fn fail_results(ctx: &Rc<DialogContext>, page: u32, text: &str) {
    if page == 1 {
        ctx.count_label.set_visible(false);
        ctx.list.append(&status_row(text));
    } else {
        ctx.status.show(text);
        append_pager(ctx);
    }
}

fn update_count_label(ctx: &Rc<DialogContext>, total: i64) {
    if total > 0 {
        ctx.count_label
            .set_text(&crate::tr!("{} layouts").replacen("{}", &total.to_string(), 1));
        ctx.count_label.set_visible(true);
    } else {
        ctx.count_label.set_visible(false);
    }
}

fn append_pager(ctx: &Rc<DialogContext>) {
    let pager = adw::ButtonRow::new();
    pager.set_title(&crate::tr!("Load more"));
    pager.set_start_icon_name(Some("view-more-symbolic"));
    {
        let ctx = ctx.clone();
        pager.connect_activated(move |_| load_more(&ctx));
    }
    ctx.list.append(&pager);
    *ctx.pager.borrow_mut() = Some(pager);
}

fn remove_pager(ctx: &Rc<DialogContext>) {
    if let Some(row) = ctx.pager.borrow_mut().take() {
        // A list clear elsewhere may already have removed it.
        if row.parent().is_some() {
            ctx.list.remove(&row);
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
            platform_id: request_ctx.platform_id.clone(),
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
    parts.extend(community_stats(layout));
    if let Some(date) = updated_date(layout.time_updated) {
        parts.push(date);
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
