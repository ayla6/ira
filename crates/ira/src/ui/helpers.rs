use crate::Game;
use adw::prelude::{AdwDialogExt, AlertDialogExt, AdwWindowExt, PreferencesRowExt};
use chrono::TimeZone;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::css::*;
use super::game_item::GameItem;
use super::state::{PendingImage, SgdbAssetsCacheEntry, SharedState};

pub struct DialogLayout {
    pub window: adw::Dialog,
    pub sidebar: gtk4::ListBox,
    pub stack: gtk4::Stack,
    pub header: adw::HeaderBar,
    pub content_area: gtk4::Box,
    pub sidebar_area: gtk4::Box,
}

pub fn dialog_layout() -> DialogLayout {
    let win = adw::Dialog::new();
    win.set_content_width(800);
    win.set_content_height(720);

    let DialogWidgets {
        outer,
        sidebar_area,
        sidebar,
        content_area,
        header,
        stack,
    } = dialog_widgets();
    win.set_child(Some(&outer));

    DialogLayout {
        window: win,
        sidebar,
        stack,
        header,
        content_area,
        sidebar_area,
    }
}

/// The resizable settings window: a normal window (unlike the `adw::Dialog`
/// sheets, which the user cannot resize) sized like the game images picker by
/// default and clamped to the presenting window.
pub struct SettingsWindowLayout {
    pub window: adw::Window,
    pub sidebar: gtk4::ListBox,
    pub stack: gtk4::Stack,
    pub content_area: gtk4::Box,
    pub sidebar_area: gtk4::Box,
}

pub fn settings_window_layout(parent: &adw::ApplicationWindow) -> SettingsWindowLayout {
    settings_window_layout_sized(
        parent,
        &crate::tr!("Settings"),
        (SETTINGS_WINDOW_WIDTH, SETTINGS_WINDOW_HEIGHT),
    )
}

/// [`settings_window_layout`] with a custom title and preferred size: still
/// modal over the presenting window and clamped to its size. The game
/// settings screen uses this to open at its own size under the game's name
/// while staying freely resizable like the app settings window.
pub fn settings_window_layout_sized(
    parent: &adw::ApplicationWindow,
    title: &str,
    preferred: (i32, i32),
) -> SettingsWindowLayout {
    let win = adw::Window::new();
    win.set_modal(true);
    win.set_transient_for(Some(parent));
    win.set_destroy_with_parent(true);
    win.set_title(Some(title));

    let (width, height) = fitted_window_size(preferred, (parent.width(), parent.height()));
    win.set_default_size(width, height);

    let DialogWidgets {
        outer,
        sidebar_area,
        sidebar,
        content_area,
        header,
        stack,
    } = dialog_widgets();
    header.set_title_widget(Some(&gtk4::Label::new(Some(title))));
    // AdwWindow only accepts set_content; gtk_window_set_child aborts.
    win.set_content(Some(&outer));

    SettingsWindowLayout {
        window: win,
        sidebar,
        stack,
        content_area,
        sidebar_area,
    }
}

/// Default settings window size, matching the game images picker.
const SETTINGS_WINDOW_WIDTH: i32 = 900;
const SETTINGS_WINDOW_HEIGHT: i32 = 700;

/// Pure part of [`settings_window_layout`]: keeps the window within the
/// presenting window when that one is mapped; unmapped parents (height 0)
/// leave the preferred size untouched.
fn fitted_window_size(preferred: (i32, i32), parent: (i32, i32)) -> (i32, i32) {
    let fit = |size: i32, available: i32| {
        if available > 300 {
            size.min(available - 40)
        } else {
            size
        }
    };
    (fit(preferred.0, parent.0), fit(preferred.1, parent.1))
}

/// Widget tree shared by the dialog sheet and the settings window layouts.
struct DialogWidgets {
    outer: gtk4::Box,
    sidebar_area: gtk4::Box,
    sidebar: gtk4::ListBox,
    content_area: gtk4::Box,
    header: adw::HeaderBar,
    stack: gtk4::Stack,
}

fn dialog_widgets() -> DialogWidgets {
    let outer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

    let sidebar_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_area.add_css_class(CSS_SETTINGS_SIDEBAR);
    sidebar_area.set_size_request(200, -1);
    sidebar_area.set_vexpand(true);

    let sidebar = gtk4::ListBox::new();
    sidebar.add_css_class(CSS_NAVIGATION_SIDEBAR);
    sidebar.set_margin_top(6);
    sidebar.set_margin_bottom(6);

    let sidebar_scroll = gtk4::ScrolledWindow::new();
    sidebar_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    sidebar_scroll.set_vexpand(true);
    sidebar_scroll.set_child(Some(&sidebar));
    sidebar_area.append(&sidebar_scroll);

    let sep = gtk4::Separator::new(gtk4::Orientation::Vertical);
    outer.append(&sidebar_area);
    outer.append(&sep);

    let content_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content_area.set_hexpand(true);

    let header = adw::HeaderBar::new();
    header.set_show_title(false);
    header.add_css_class(CSS_SETTINGS_HEADER);
    content_area.append(&header);

    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_margin_start(16);
    stack.set_margin_end(16);
    stack.set_margin_top(16);
    stack.set_margin_bottom(16);

    // Every page wraps itself in a `ScrolledWindow` (see `scrolled_page`),
    // so scrolling happens per page and only by that page's own content
    // overflow: short pages can't be overscrolled and tall pages never grow
    // the window past its configured size.
    content_area.append(&stack);
    outer.append(&content_area);

    DialogWidgets {
        outer,
        sidebar_area,
        sidebar,
        content_area,
        header,
        stack,
    }
}

/// Wrap a page's content in the standard per-page `ScrolledWindow`.
///
/// Settings-style dialog pages must scroll independently of the stack so a
/// page's scroll range is exactly its own content overflow.
pub(crate) fn scrolled_page(content: &impl IsA<gtk4::Widget>) -> gtk4::ScrolledWindow {
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_child(Some(content));
    scroll
}

/// Create a closure that returns the current text of an EntryRow as
/// `Option<String>`, for use as the `initial_path` parameter of
/// `make_browse_button`.
pub fn entry_path_closure(entry: &adw::EntryRow) -> impl Fn() -> Option<String> + 'static {
    let entry = glib::clone::Downgrade::downgrade(entry);
    move || {
        let entry = entry.upgrade()?;
        let t = entry.text().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    }
}

/// Set the initial folder for a `FileDialog` based on a path string.
/// If the path is a file, uses its parent directory. If the path doesn't
/// exist, walks up to find the nearest existing parent, falling back to
/// the home directory.
pub fn set_initial_folder(dialog: &gtk4::FileDialog, path_str: &str) {
    if path_str.is_empty() {
        return;
    }
    let path = std::path::Path::new(path_str);
    let folder = if path.is_file() {
        path.parent()
    } else if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    let final_folder = folder
        .filter(|p| p.exists())
        .or_else(|| {
            let mut p = path.parent();
            while let Some(parent) = p {
                if parent.exists() {
                    return Some(parent);
                }
                p = parent.parent();
            }
            None
        })
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()));
    if final_folder.exists() {
        let file = gio::File::for_path(final_folder);
        dialog.set_initial_folder(Some(&file));
    }
}

/// The hosting `gtk4::Window` for any widget: `AdwDialog` content resolves
/// to the window presenting the dialog. Needed by window-transient APIs
/// (FileDialog, AlertDialog parents...).
pub fn hosting_window(w: &impl IsA<gtk4::Widget>) -> Option<gtk4::Window> {
    w.root().and_then(|root| root.downcast::<gtk4::Window>().ok())
}

/// Standard icon-only button: flat, square, vertically centered so row
/// suffixes and entry rows cannot stretch it into a rectangle.
pub fn icon_button(icon: &str, tooltip: &str) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name(icon);
    button.add_css_class(CSS_FLAT);
    button.add_css_class(CSS_SQUARE_BUTTON);
    button.set_valign(gtk4::Align::Center);
    button.set_tooltip_text(Some(tooltip));
    button
}

pub fn make_browse_button(
    parent: Option<&impl IsA<gtk4::Widget>>,
    title: &str,
    select_folder: bool,
    filter: Option<(&str, &[&str])>,
    initial_path: impl Fn() -> Option<String> + 'static,
    on_select: impl Fn(&std::path::Path) + 'static,
) -> gtk4::Button {
    let browse = icon_button("folder-open-symbolic", title);
    let parent = parent.map(|w| w.downgrade());
    let title = title.to_string();
    let filter = filter.map(|(name, mimes)| {
        (
            name.to_string(),
            mimes.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
    });
    let on_select = std::rc::Rc::new(on_select);
    let initial_path = std::rc::Rc::new(initial_path);
    browse.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title(&title);
        if let Some((name, mimes)) = &filter {
            let f = gtk4::FileFilter::new();
            f.set_name(Some(name));
            for mime in mimes {
                f.add_mime_type(mime);
            }
            f.add_pattern("*");
            dialog.set_default_filter(Some(&f));
        }
        if let Some(path_str) = initial_path() {
            set_initial_folder(&dialog, &path_str);
        }
        let on_select = on_select.clone();
        let cb = move |result: Result<gio::File, glib::Error>| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    on_select(&path);
                }
            }
        };
        // File dialogs must be transient over a real window; any widget
        // (dialog content included) resolves to its hosting window.
        let parent = parent
            .as_ref()
            .and_then(|w| w.upgrade())
            .and_then(|w| hosting_window(&w));
        if select_folder {
            dialog.select_folder(parent.as_ref(), None::<&gio::Cancellable>, cb);
        } else {
            dialog.open(parent.as_ref(), None::<&gio::Cancellable>, cb);
        }
    });
    browse
}

/// Merge enrichment results into the existing in-memory game.
///
/// Starts from `existing` (which has the user's current edits) and applies only
/// the fields enrichment is responsible for (achievements, images, metadata).
/// All other fields are preserved from `existing` by default, so stale
/// enrichment data (read from DB before a save) can never revert user edits.
pub fn merge_game_enrichment(existing: &Game, enriched: &Game) -> Game {
    let mut result = existing.clone();

    // Achievements — enrichment loads these from disk.
    result.achievements = enriched.achievements.clone();
    if !existing.achievements.is_empty() {
        let existing_pcts: HashMap<String, f64> = existing
            .achievements
            .iter()
            .map(|a| (a.name.clone(), a.global_percent))
            .collect();
        for a in &mut result.achievements {
            if a.global_percent == 0.0 {
                if let Some(&pct) = existing_pcts.get(&a.name) {
                    a.global_percent = pct;
                }
            }
        }
    }
    result.earned_count = enriched.earned_count;
    result.total_count = enriched.total_count;

    // Images — apply only if enrichment found something.
    // Skip image paths from variant pseudo-games — their images are variant-specific
    // and must not overwrite the base game's images.
    if enriched.variant_id.is_none() {
        if !enriched.icon_path.is_empty() {
            result.icon_path = enriched.icon_path.clone();
        }
        if !enriched.hero_image_path.is_empty() {
            result.hero_image_path = enriched.hero_image_path.clone();
        }
        if !enriched.grid_path.is_empty() {
            result.grid_path = enriched.grid_path.clone();
        }
        if !enriched.header_path.is_empty() {
            result.header_path = enriched.header_path.clone();
        }
        if !enriched.logo_path.is_empty() {
            result.logo_path = enriched.logo_path.clone();
        }
        if !enriched.square_path.is_empty() {
            result.square_path = enriched.square_path.clone();
        }
    }

    // Name — apply only if existing is a placeholder.
    if existing.name.is_empty() || existing.name.starts_with("App ID:") {
        result.set_name(&enriched.name);
    }

    // Metadata — apply only if enrichment fetched something.
    if !enriched.release_date.is_empty() {
        result.release_date = enriched.release_date.clone();
    }
    if enriched.release_timestamp != 0 {
        result.release_timestamp = enriched.release_timestamp;
    }
    if enriched.metacritic_score >= 0 {
        result.metacritic_score = enriched.metacritic_score;
    }
    if enriched.steam_review_score >= 0 {
        result.steam_review_score = enriched.steam_review_score;
    }
    if enriched.steam_review_count != 0 {
        result.steam_review_count = enriched.steam_review_count;
    }

    result
}

pub fn open_folder(path: &str) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

pub fn open_file_location(file_path: &str) {
    let path = std::path::Path::new(file_path);
    let dir = path.parent().map(|p| p.to_string_lossy().to_string());
    let uri = format!("file://{}", file_path);
    let dbus_result = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.FileManager1",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
            &format!("array:string:{}", uri),
            "string:",
        ])
        .output();
    match dbus_result {
        Ok(o) if o.status.success() => return,
        _ => {}
    }
    if let Some(dir) = dir {
        open_folder(&dir);
    }
}

pub fn confirm_dialog(
    parent: &adw::ApplicationWindow,
    title: &str,
    body: &str,
    confirm_label: &str,
    appearance: adw::ResponseAppearance,
    on_confirm: impl Fn() + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(body));
    dialog.add_response("cancel", &crate::tr!("Cancel"));
    dialog.add_response("confirm", confirm_label);
    dialog.set_response_appearance("confirm", appearance);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, resp| {
        if resp == "confirm" {
            on_confirm();
        }
    });
    dialog.present(Some(parent));
}

pub trait Clearable {
    fn clear_all_children(&self);
}

impl Clearable for gtk4::Box {
    fn clear_all_children(&self) {
        while let Some(child) = self.first_child() {
            self.remove(&child);
        }
    }
}

impl Clearable for gtk4::ListBox {
    fn clear_all_children(&self) {
        while let Some(child) = self.first_child() {
            self.remove(&child);
        }
    }
}

impl Clearable for gtk4::FlowBox {
    fn clear_all_children(&self) {
        while let Some(child) = self.first_child() {
            self.remove(&child);
        }
    }
}

pub fn clear_children(w: &impl Clearable) {
    w.clear_all_children();
}

/// An insensitive `ActionRow` carrying list status text ("Searching…",
/// "No results found") inside boxed lists of the match/search dialogs.
pub(crate) fn status_row(text: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(text);
    row.set_sensitive(false);
    row
}

/// A status row parked at the top of a boxed result list: prepended while a
/// worker runs ("Searching…", "Downloading…"), removable without touching
/// the results beneath it.
#[derive(Clone, Default)]
pub(crate) struct SearchStatus {
    list: gtk4::ListBox,
    current: Rc<RefCell<Option<adw::ActionRow>>>,
}

impl SearchStatus {
    pub(crate) fn for_list(list: &gtk4::ListBox) -> Self {
        Self {
            list: list.clone(),
            current: Rc::new(RefCell::new(None)),
        }
    }

    /// Replace any current status row with `text` (prepended, insensitive).
    pub(crate) fn show(&self, text: &str) {
        self.clear();
        let row = status_row(text);
        self.list.insert(&row, 0);
        *self.current.borrow_mut() = Some(row);
    }

    /// Remove the status row, leaving the results untouched.
    pub(crate) fn clear(&self) {
        if let Some(row) = self.current.borrow_mut().take() {
            self.list.remove(&row);
        }
    }
}

/// Wrap `child` in an `adw::Clamp` so wide dialogs don't stretch it.
/// Margins are `(top, bottom, start, end)`.
pub(crate) fn clamped(
    child: &impl IsA<gtk4::Widget>,
    max_width: i32,
    margins: (i32, i32, i32, i32),
) -> adw::Clamp {
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(max_width);
    clamp.set_margin_top(margins.0);
    clamp.set_margin_bottom(margins.1);
    clamp.set_margin_start(margins.2);
    clamp.set_margin_end(margins.3);
    clamp.set_child(Some(child));
    clamp
}

/// A vertically scrolling, width-clamped boxed `ListBox` for dialog result
/// areas. Returns the scrolled container to pack and the list to fill.
pub(crate) fn clamped_boxed_list(max_width: i32) -> (gtk4::ScrolledWindow, gtk4::ListBox) {
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.set_valign(gtk4::Align::Start);
    list.add_css_class(CSS_BOXED_LIST);
    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    column.set_margin_top(8);
    column.set_margin_bottom(12);
    column.append(&list);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(max_width);
    clamp.set_child(Some(&column));
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&clamp));
    (scrolled, list)
}

/// Poll a worker-thread channel on the GTK main loop without blocking it;
/// one value is delivered to `on_value`. A dropped sender ends polling
/// silently — a panicking worker has nothing useful to report anyway.
pub(crate) fn poll_channel<T: Send + 'static>(
    rx: std::sync::mpsc::Receiver<T>,
    on_value: impl FnOnce(T) + 'static,
) {
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
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// A button with an icon next to its label. Composed from plain gtk4
/// widgets on purpose: `adw::ButtonContent` used as a suffix fires GTK
/// criticals (`gtk_widget_get_parent`/`add_css_class` on a finalized
/// widget) during construction inside preferences groups.
pub(crate) fn icon_label_button(icon: &str, label: &str) -> gtk4::Button {
    let button = gtk4::Button::new();
    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    content.append(&gtk4::Image::from_icon_name(icon));
    content.append(&gtk4::Label::new(Some(label)));
    button.set_child(Some(&content));
    button.set_valign(gtk4::Align::Center);
    button
}

pub fn replace_grid_game(state: &SharedState, game: &Game) {
    let store = state.borrow().grid_store.clone();
    let grid_id = game.grid_id();
    for i in 0..store.n_items() {
        if let Some(item) = store.item(i).and_then(|o| o.downcast::<GameItem>().ok()) {
            if item
                .game()
                .is_some_and(|current| current.grid_id() == grid_id)
            {
                store.splice(i, 1, &[GameItem::new(game)]);
                break;
            }
        }
    }
}

/// The most recently played visible games, newest first. Shared by the
/// desktop grid header and the big-picture carousel so both agree on what
/// "recent" means.
pub fn recently_played(state: &SharedState, limit: usize) -> Vec<Game> {
    let s = state.borrow();
    let show_hidden = s.cfg.show_hidden_games;
    let mut recent: Vec<Game> = s
        .games
        .iter()
        .filter(|g| g.last_played > 0 && (!g.hidden || show_hidden))
        .cloned()
        .collect();
    recent.sort_by_key(|a| std::cmp::Reverse(a.last_played));
    recent.truncate(limit);
    recent
}

pub fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

pub fn refresh_settings_images_page(
    state: &SharedState,
    db_id: i64,
    build_page: impl Fn(
        &SharedState,
        &Game,
        &adw::Window,
        Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
        Option<Rc<RefCell<HashMap<String, SgdbAssetsCacheEntry>>>>,
    ) -> gtk4::Widget,
) {
    let sd = match state.borrow().settings_data.clone() {
        Some(d) => d,
        None => return,
    };
    if sd.db_id == db_id && sd.window.is_visible() {
        let images_was_visible = sd
            .stack
            .visible_child_name()
            .is_some_and(|name| name == "images");
        if let Some(old) = sd.stack.child_by_name("images") {
            sd.stack.remove(&old);
        }
        if let Some(game) = state
            .borrow()
            .games
            .iter()
            .find(|g| g.db_id == db_id)
            .cloned()
        {
            let new_page = build_page(
                state,
                &game,
                &sd.window,
                Some(sd.pending_copies.clone()),
                Some(sd.sgdb_cache.clone()),
            );
            // Same wrapper as the initial build: the raw page's natural
            // height would become the window minimum and force the settings
            // window to grow past its size.
            sd.stack.add_named(&scrolled_page(&new_page), Some("images"));
            if images_was_visible {
                sd.stack.set_visible_child_name("images");
            }
        }
    }
}

pub fn string_list_from(strings: &[String]) -> gtk4::StringList {
    let str_refs: Vec<&str> = strings.iter().map(|s| s.as_str()).collect();
    gtk4::StringList::new(&str_refs)
}

pub fn format_duration(seconds: i64) -> String {
    let total_mins = ((seconds.max(0) as f64) / 60.0).round() as u64;
    let h = total_mins / 60;
    let m = total_mins % 60;
    match (h, m) {
        (0, 0) => "0min".to_string(),
        (0, m) => format!("{}min", m),
        (h, 0) => format!("{}h", h),
        (h, m) => format!("{}h{:02}min", h, m),
    }
}

/// Unix timestamp (seconds) as the user's local wall-clock time; `None` when
/// the value is outside the representable range. All UI display of stored
/// timestamps must go through this so times follow the user's timezone
/// instead of UTC.
pub fn local_datetime(secs: i64) -> Option<chrono::DateTime<chrono::Local>> {
    chrono::Local.timestamp_opt(secs, 0).single()
}

/// Spawn a terminal running bash, with the given env vars set.
/// Tries common terminal emulators in order since `-e` isn't universal.
pub fn spawn_terminal(env: &[(String, String)]) {
    let terminals: &[(&str, &[&str])] = &[
        ("gnome-terminal", &["--", "bash"]),
        ("konsole", &["-e", "bash"]),
        ("xfce4-terminal", &["-e", "bash"]),
        ("mate-terminal", &["--", "bash"]),
        ("alacritty", &["-e", "bash"]),
        ("kitty", &["-e", "bash"]),
        ("foot", &["-e", "bash"]),
        ("wezterm", &["start", "--", "bash"]),
        ("tilix", &["-e", "bash"]),
        ("qterminal", &["-e", "bash"]),
        ("lxterminal", &["-e", "bash"]),
        ("terminator", &["-e", "bash"]),
        ("xterm", &["-e", "bash"]),
    ];

    if let Ok(term) = std::env::var("TERMINAL") {
        let mut cmd = std::process::Command::new(&term);
        cmd.arg("-e").arg("bash");
        for (k, v) in env {
            cmd.env(k, v);
        }
        if cmd.spawn().is_ok() {
            return;
        }
    }

    for (term, args) in terminals {
        let mut cmd = std::process::Command::new(term);
        cmd.args(*args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        if cmd.spawn().is_ok() {
            return;
        }
    }
    eprintln!("No terminal emulator found. Set $TERMINAL or install gnome-terminal/konsole/xterm.");
}


/// A small confirm/cancel prompt with one extra widget (usually an entry
/// row). `on_confirm` fires only for the confirm response; the dialog
/// closes either way.
pub(crate) fn confirm_dialog_with_extra(
    parent: &adw::Dialog,
    title: &str,
    body: &str,
    extra_child: &impl IsA<gtk4::Widget>,
    on_confirm: impl Fn() + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(body));
    dialog.set_extra_child(Some(extra_child));
    dialog.add_response("cancel", &crate::tr!("Cancel"));
    dialog.add_response("confirm", &crate::tr!("Confirm"));
    dialog.set_response_appearance("confirm", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("confirm"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| {
        if response == "confirm" {
            on_confirm();
        }
    });
    dialog.present(Some(parent));
}


/// Caps a floating dialog's preferred height to the window it is presented
/// over. A floating sheet adds its own chrome around the content, so a
/// dialog whose preferred height merely meets the parent's measures larger
/// than what the parent offers — libadwaita then warns (and clips) on every
/// relayout. Callers pass a `preferred` size that already fits the common
/// presenting windows; the parent clamp is a further shrink for smaller
/// ones. Unmapped parents report height 0, in which case `preferred`
/// stands.
pub(crate) fn fit_dialog_height(
    dialog: &adw::Dialog,
    parent: &impl IsA<gtk4::Widget>,
    preferred: i32,
) {
    dialog.set_content_height(fitted_height(preferred, parent.height()));
}

/// Pure part of [`fit_dialog_height`]: 72 px covers the sheet chrome plus
/// breathing room; below that threshold a parent height of 0 (unmapped)
/// or a tiny stale allocation is ignored.
fn fitted_height(preferred: i32, available: i32) -> i32 {
    if available > 300 {
        preferred.min(available - 72)
    } else {
        preferred
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ira_models::LogoPosition;

    #[test]
    fn test_format_duration_zero() {
        assert_eq!(format_duration(0), "0min");
    }

    #[test]
    fn test_format_duration_negative() {
        assert_eq!(format_duration(-10), "0min");
    }

    #[test]
    fn test_format_duration_sub_minute_rounds_up() {
        assert_eq!(format_duration(30), "1min");
        assert_eq!(format_duration(45), "1min");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(300), "5min");
        assert_eq!(format_duration(600), "10min");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(7200), "2h");
    }

    #[test]
    fn test_format_duration_hours_minutes() {
        assert_eq!(format_duration(7500), "2h05min");
        assert_eq!(format_duration(9000), "2h30min");
    }

    #[test]
    fn test_format_duration_rounds_near_hour() {
        assert_eq!(format_duration(3570), "1h");
        assert_eq!(format_duration(3630), "1h01min");
    }

    #[test]
    fn test_local_datetime_noon_is_local_wall_clock() {
        use chrono::TimeZone;
        let noon = chrono::Local
            .with_ymd_and_hms(2026, 3, 10, 12, 0, 0)
            .single()
            .unwrap();
        let dt = local_datetime(noon.timestamp()).unwrap();
        assert_eq!(dt.format("%H:%M").to_string(), "12:00");
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2026-03-10");
    }

    #[test]
    fn test_local_datetime_out_of_range_is_none() {
        assert!(local_datetime(i64::MAX).is_none());
    }

    #[test]
    fn test_fitted_window_size_clamps_to_parent() {
        assert_eq!(fitted_window_size((900, 700), (1000, 800)), (900, 700));
        assert_eq!(fitted_window_size((900, 700), (800, 600)), (760, 560));
    }

    #[test]
    fn test_fitted_window_size_ignores_unmapped_parent() {
        assert_eq!(fitted_window_size((900, 700), (0, 0)), (900, 700));
        assert_eq!(fitted_window_size((900, 700), (1200, 250)), (900, 700));
    }

    #[test]
    fn test_merge_game_enrichment_preserves_user_edits() {
        let existing = Game {
            shadps4_version: "/new/shadps4".to_string(),
            ra_core: "snes9x".to_string(),
            emulator_override: "/new/emu".to_string(),
            platform_id: "NEWPID".to_string(),
            trophy_source: ira_models::TrophySource::Ra,
            sgdb_id: "12345".to_string(),
            rom_path: "/new/rom.sfc".to_string(),
            sort_title: "My Sort".to_string(),
            logo_position: "center".to_string(),
            logo_size: 75,
            ..Default::default()
        };

        let enriched = Game {
            platform_id: "OLDPID".to_string(),
            trophy_source: ira_models::TrophySource::SteamNative,
            logo_position: LogoPosition::BottomLeft.to_string(),
            logo_size: 50,
            ..Default::default()
        };

        let result = merge_game_enrichment(&existing, &enriched);

        assert_eq!(result.shadps4_version, "/new/shadps4");
        assert_eq!(result.ra_core, "snes9x");
        assert_eq!(result.emulator_override, "/new/emu");
        assert_eq!(result.platform_id, "NEWPID");
        assert_eq!(result.trophy_source, ira_models::TrophySource::Ra);
        assert_eq!(result.sgdb_id, "12345");
        assert_eq!(result.rom_path, "/new/rom.sfc");
        assert_eq!(result.sort_title, "My Sort");
        assert_eq!(result.logo_position, "center");
        assert_eq!(result.logo_size, 75);
    }

    #[test]
    fn test_merge_game_enrichment_preserves_name_and_sort_title() {
        let existing = Game {
            name: "My Game".to_string(),
            sort_title: "Sort Key".to_string(),
            ..Default::default()
        };

        let enriched = Game {
            name: "App ID: 123".to_string(),
            ..Default::default()
        };

        let result = merge_game_enrichment(&existing, &enriched);

        assert_eq!(result.name, "My Game");
        assert_eq!(result.sort_title, "Sort Key");
    }

    #[test]
    fn test_merge_game_enrichment_applies_enrichment_achievements() {
        let existing = Game::default();

        let enriched = Game {
            earned_count: 5,
            total_count: 10,
            ..Default::default()
        };

        let result = merge_game_enrichment(&existing, &enriched);

        assert_eq!(result.earned_count, 5);
        assert_eq!(result.total_count, 10);
    }

    #[test]
    fn test_merge_game_enrichment_applies_enrichment_images() {
        let existing = Game::default();

        let enriched = Game {
            icon_path: "/new/icon.webp".to_string(),
            hero_image_path: "/new/hero.webp".to_string(),
            ..Default::default()
        };

        let result = merge_game_enrichment(&existing, &enriched);

        assert_eq!(result.icon_path, "/new/icon.webp");
        assert_eq!(result.hero_image_path, "/new/hero.webp");
    }

    #[test]
    fn test_merge_game_enrichment_applies_name_for_placeholder() {
        let existing = Game {
            name: "App ID: 123".to_string(),
            ..Default::default()
        };

        let enriched = Game {
            name: "Real Game Name".to_string(),
            ..Default::default()
        };

        let result = merge_game_enrichment(&existing, &enriched);

        assert_eq!(result.name, "Real Game Name");
    }

    #[test]
    fn test_fitted_height_caps_to_parent_and_ignores_unmapped() {
        // Mapped parents get the preferred size only when it fits.
        assert_eq!(super::fitted_height(740, 1080), 740);
        assert_eq!(super::fitted_height(740, 720), 648);
        assert_eq!(super::fitted_height(500, 720), 500);
        // Unmapped (0) and absurdly small allocations keep the preference;
        // libadwaita falls back to covering the window then.
        assert_eq!(super::fitted_height(740, 0), 740);
        assert_eq!(super::fitted_height(740, 100), 740);
    }
}

