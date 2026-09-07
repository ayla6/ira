pub const CSS_BOXED_LIST: &str = "boxed-list";
pub const CSS_BP_SQ: &str = "bp-sq";
pub const CSS_BP_ROOT: &str = "bp-root";
pub const CSS_BP_TITLE: &str = "bp-title";
pub const CSS_BP_SELECTED: &str = "bp-selected";
pub const CSS_BP_STATUS: &str = "bp-status";
pub const CSS_BP_CLOCK: &str = "bp-clock";
pub const CSS_BP_DATE: &str = "bp-date";
pub const CSS_BP_BATT: &str = "bp-batt";
pub const CSS_BP_BOTTOM: &str = "bp-bottom";
pub const CSS_BP_PAD_DOT: &str = "bp-pad-dot";
pub const CSS_BP_PAD_LIT: &str = "bp-pad-lit";
pub const CSS_BP_PROMPT: &str = "bp-prompt";
pub const CSS_BP_PROMPT_KEY: &str = "bp-prompt-key";
pub const CSS_BP_ALL: &str = "bp-all";
pub const CSS_BP_ALL_TILE: &str = "bp-all-tile";
pub const CSS_BP_ALL_PAGE: &str = "bp-all-page";
pub const CSS_BP_PAGE_TITLE: &str = "bp-page-title";
pub const CSS_BP_PAGE_SUBTITLE: &str = "bp-page-subtitle";
pub const CSS_BP_TABS: &str = "bp-tabs";
pub const CSS_BP_MENU_DIM: &str = "bp-menu-dim";
pub const CSS_BP_MENU_PANEL: &str = "bp-menu-panel";
pub const CSS_BP_OPTION_PANEL: &str = "bp-option-panel";
pub const CSS_BP_MENU_TITLE: &str = "bp-menu-title";
pub const CSS_BP_MENU_ROW: &str = "bp-menu-row";
pub const CSS_BP_MENU_ROW_SELECTED: &str = "bp-menu-row-selected";
pub const CSS_BP_MENU_ROW_ACTIVE: &str = "bp-menu-row-active";
pub const CSS_BP_SHOULDER: &str = "bp-shoulder";
pub const CSS_BP_CURSOR_HIDDEN: &str = "bp-cursor-hidden";
pub const CSS_BP_GROUP_SLOT: &str = "bp-group-slot";
pub const CSS_BP_KEY: &str = "bp-key";
pub const CSS_BP_KEY_SELECTED: &str = "bp-key-selected";
pub const CSS_BP_KEY_ACTIVE: &str = "bp-key-active";
pub const CSS_BP_KEY_PREVIEW: &str = "bp-key-preview";
/// The selection ring reaches this far past a tile's edge (4px gap plus
/// the 4px frame), and the tooltip's tail tip rests 3px beyond the
/// frame — 11px from the tile edge in total, on every page.
pub const BP_RING_OUTSET: f64 = 11.0;
pub const CSS_CAPTION: &str = "caption";
pub const CSS_CIRCULAR: &str = "circular";
pub const CSS_CLICKABLE_STAT: &str = "clickable-stat";
pub const CSS_COVER_BADGE: &str = "cover-badge";
pub const CSS_COVER_ITEM: &str = "cover-item";
pub const CSS_COVER_NAME_FALLBACK: &str = "cover-name-fallback";
pub const CSS_DESTRUCTIVE_ACTION: &str = "destructive-action";
pub const CSS_DIM_LABEL: &str = "dim-label";
pub const CSS_ERROR: &str = "error";
pub const CSS_FLAT: &str = "flat";
pub const CSS_GAME_COVER_PIC: &str = "game-cover-pic";
pub const CSS_GAME_GRID: &str = "game-grid";
pub const CSS_GLOBAL_BAR: &str = "global-bar";
pub const CSS_HEADING: &str = "heading";
pub const CSS_HIDDEN_GAME: &str = "hidden-game";
pub const CSS_LOGO_POS_OVERLAY_BTN: &str = "logo-pos-overlay-btn";
pub const CSS_NAVIGATION_SIDEBAR: &str = "navigation-sidebar";
pub const CSS_PLAY_BTN_LABEL: &str = "play-btn-label";
pub const CSS_PLAYING_GAME: &str = "playing-game";
pub const CSS_POPOVER_MENU_ROW: &str = "popover-menu-row";
pub const CSS_SECTION_TITLE: &str = "section-title";
pub const CSS_RECENT_SCROLL: &str = "recent-scroll";
pub const CSS_SESSION_DELETE: &str = "session-delete";
pub const CSS_SELECTED: &str = "selected";
pub const CSS_SETTINGS_HEADER: &str = "settings-header";
pub const CSS_SETTINGS_SIDEBAR: &str = "settings-sidebar";
pub const CSS_SIDEBAR_ROW_PAD_GAME: &str = "sidebar-row-pad-game";
pub const CSS_SIDEBAR_ROW_PAD_HEADER: &str = "sidebar-row-pad-header";
pub const CSS_SIDEBAR_ROW_TITLE: &str = "sidebar-row-title";
pub const CSS_SIDEBAR_SEPARATOR_ROW: &str = "sidebar-separator-row";
pub const CSS_SUCCESS_LABEL: &str = "success-label";
pub const CSS_SUGGESTED_ACTION: &str = "suggested-action";
pub const CSS_TITLE_1: &str = "title-1";
pub const CSS_LOCKED_TROPHY: &str = "locked-trophy";
pub const CSS_SIDEBAR_SECTION_TITLE: &str = "sidebar-section-title";
pub const CSS_SOURCE_BADGE: &str = "source-badge";
pub const CSS_SQUARE_BUTTON: &str = "square-button";
pub const CSS_STATUS_NO_SCROLL: &str = "status-no-scroll";
pub const CSS_COMMAND_TILE: &str = "command-tile";
pub const CSS_COMMAND_TILE_ACTIVE: &str = "command-tile-active";

pub const APP_CSS: &str = "
.sidebar-row-title { min-width: 0; }
.global-bar trough { background-color: transparent; border: none; }
.global-bar progress { border: none; border-radius: 0; }
 .hidden-game { opacity: 0.5; }
 listview.navigation-sidebar > row { padding: 0; }
 listview.navigation-sidebar row:selected { background-color: transparent; }
 listview.navigation-sidebar row:selected > box { background-color: alpha(@theme_fg_color, 0.07); border-radius: 9px; }
 listview.navigation-sidebar row:selected > box.playing-game { background-color: alpha(@accent_color, 0.22); }
 .playing-game { color: @accent_color; background-color: alpha(@accent_color, 0.08); border-radius: 9px; }
 .sidebar-row-pad-game { padding: 4px 10px 4px 24px; }
 .sidebar-row-pad-header { padding: 4px 10px 4px 4px; }
.play-btn-label { font-size: 1.15em; }

.popover-menu-row {
    padding-left: 10px;
    padding-right: 10px;
    font-weight: normal;
}
.popover-menu-row button.flat {
    padding: 0;
}
.popover-menu-row label {
    font-weight: normal;
}

.success-label { color: @accent_color; font-weight: bold; }

.app-content-header {
    background: @window_bg_color;
}
.app-content-header > box.title {
    margin: 0;
    padding: 0;
    min-width: 0;
}

.settings-sidebar { background-color: @headerbar_bg_color; }
.settings-header { background-color: transparent; box-shadow: none; }


.logo-pos-overlay-btn {
    background: transparent;
    border: 1px solid rgba(255,255,255,0.15);
    border-radius: 4px;
    transition: 100ms ease;
}
.logo-pos-overlay-btn:hover {
    background: rgba(255,255,255,0.2);
    border-color: rgba(255,255,255,0.4);
}
.logo-pos-overlay-btn.selected {
    background: rgba(255,255,255,0.15);
    border-color: @accent_color;
    border-width: 2px;
}

gridview.game-grid {
    background: transparent;
    border-spacing: 0;
}
gridview.game-grid child {
    background: transparent;
    box-shadow: none;
    border: none;
    padding: 0;
    margin: 0;
    outline: none;
}
gridview.game-grid child:hover,
gridview.game-grid child:selected,
gridview.game-grid child:focus,
gridview.game-grid child:focus-visible,
gridview.game-grid child:focus-within {
    background: transparent;
}

.cover-item .game-cover-pic {
    transition: 100ms ease;
    box-shadow: 0 2px 14px 3px rgba(0,0,0,0.4);
}
.cover-item:hover .game-cover-pic {
    transform: scale(1.06);
    box-shadow: 0 6px 24px 6px rgba(0,0,0,0.5);
}

.cover-badge {
    background-color: rgba(0, 0, 0, 0.75);
    color: white;
    border-radius: 4px;
    padding: 2px 8px;
    font-weight: 600;
}

.cover-name-fallback {
    color: @theme_fg_color;
    opacity: 0.7;
    font-weight: 600;
}

.section-title {
    font-weight: 700;
    font-size: 1.25em;
}

.recent-scroll scrollbar,
.status-no-scroll scrollbar {
    min-width: 0;
    min-height: 0;
    opacity: 0;
    background: transparent;
    border: none;
}

.sidebar-separator-row {
    min-height: 0;
    padding: 0;
    margin: 0;
    background: transparent;
    border: none;
    box-shadow: none;
}
.sidebar-separator-row:hover,
.sidebar-separator-row:selected {
    background: transparent;
}
.sidebar-separator-row > separator {
    margin-top: 4px;
    margin-bottom: 4px;
    min-height: 1px;
}

.sidebar-section-title {
    min-height: 0;
    padding: 0;
    margin: 0;
    background: transparent;
    border: none;
    box-shadow: none;
}
.sidebar-section-title:hover,
.sidebar-section-title:selected {
    background: transparent;
}
.sidebar-section-title > box > separator {
    min-height: 1px;
    margin-top: 1px;
}
.sidebar-section-title > box > label {
    opacity: 0.65;
    font-weight: 700;
    font-size: 0.85em;
    margin-bottom: 2px;
}

.dim-label {
    opacity: 0.55;
}

.square-button {
    /* GTK CSS min-* sets the content minimum; with the 4px padding this
       yields a 32x32 button in containers that honor minimums, and the
       symmetric padding keeps the natural size square everywhere else. */
    min-width: 24px;
    min-height: 24px;
    padding: 4px;
}
.source-badge {
    padding: 2px 8px;
    border-radius: 8px;
    background-color: alpha(@theme_fg_color, 0.08);
    font-weight: 600;
}

.command-tile {
    min-width: 92px;
    min-height: 52px;
    padding: 8px 12px;
    margin: 2px;
    border-radius: 12px;
}
.command-tile label {
    font-weight: 500;
}
.command-tile.command-tile-active {
    box-shadow: inset 0 0 0 2px @accent_color;
}

.unmapped-row {
    opacity: 0.55;
}
.unmapped-row:hover,
.unmapped-row:selected {
    opacity: 1;
}

.variant-card {
    padding: 12px;
    border-radius: 10px;
    background-color: alpha(@theme_fg_color, 0.05);
}
.variant-card.dragging {
    opacity: 0.5;
}
.variant-drag-handle {
    opacity: 0.5;
}
.variant-drag-handle:active {
    opacity: 0.8;
}

.clickable-stat { transition: 100ms ease; border-radius: 6px; padding: 6px 10px; margin: -6px -10px; }
.clickable-stat:hover { background-color: alpha(@theme_fg_color, 0.07); }
.locked-trophy { filter: grayscale(100%); }
.hero-fallback-bg { background: shade(@theme_bg_color, 0.5); }
.hero-title-overlay { color: white; font-size: 1.4em; font-weight: 700; text-shadow: 0 2px 8px rgba(0,0,0,0.8); }

.session-delete {
    margin-left: 6px;
    padding: 2px;
    color: alpha(@theme_fg_color, 0.75);
}
.session-delete:hover {
    color: @error_color;
    background-color: alpha(@error_color, 0.12);
}
.session-delete:active {
    color: @error_color;
    background-color: alpha(@error_color, 0.22);
}

.bp-root .cover-item:hover .game-cover-pic {
    transform: none;
    box-shadow: 0 2px 14px 3px rgba(0,0,0,0.4);
}
.fetch-strip label {
    font-weight: normal;
}
.cover-item.bp-sq.bp-selected .game-cover-pic {
    transform: none;
    box-shadow: none;
}
.cover-item.bp-sq:hover .game-cover-pic {
    transform: none;
    box-shadow: 0 2px 14px 3px rgba(0,0,0,0.4);
}
/* Big-picture text: the experimental bundled font. Its sizes live in
   big_picture_css(), which scales them with the viewport. */
.bp-root {
    font-family: \"M PLUS 2\", sans-serif;
}
.bp-ring { color: @accent_color; }
/* A connected pad lights its dot green, like a player LED. Dimming for
   empty slots is done per-widget in code; a CSS opacity here would
   multiply into the lit dots and wash the color out. */
.bp-pad-dot.bp-pad-lit { color: @green_2; }
.bp-all-tile {
    border-radius: 9999px;
    background: alpha(@theme_fg_color, 0.08);
}
.cover-item.bp-all:hover .bp-all-tile {
    background: alpha(@theme_fg_color, 0.13);
}
/* SGDB picker: the per-item filter button floats on the art with a shadow
   (no pill background) and fades in while the pointer is anywhere over the
   card or row; hovering the button itself shows a disc so it reads as
   clickable. Reveal is driven by a motion controller (sgdb-reveal). */
button.sgdb-filter {
    background: none;
    box-shadow: none;
    min-width: 0;
    min-height: 0;
    padding: 4px;
    opacity: 0;
    transition: opacity 150ms ease;
}
button.sgdb-filter image {
    color: currentcolor;
    -gtk-icon-shadow: 0 1px 3px rgba(0, 0, 0, 0.8);
}
button.sgdb-filter.sgdb-reveal,
button.sgdb-filter:hover {
    opacity: 1;
    background: rgba(0, 0, 0, 0.55);
}
";

/// The big-picture UI's sizes, scaled from the 1080p reference by `s`. Everything
/// is integer pixels: fractional font sizes render choppy.
fn big_picture_css(s: f64) -> String {
    let px = |v: i32| format!("{}px", (v as f64 * s).round().max(1.0));
    let status_pad_x = px(24);
    format!(
        r#".bp-status {{ padding: {status_pad_top} {status_pad_x} {status_pad_bottom} {status_pad_x}; min-height: {status_min_h}; }}
.bp-clock {{ font-size: {small}; padding: 2px 0; }}
.bp-date, .bp-batt, .bp-clock, .bp-prompt {{ font-size: {small}; padding: 2px 0; }}
.bp-bottom {{
    padding: {pad_v} {pad_h};
    min-height: {min_h};
}}
.bp-prompt-key {{
    min-width: {key};
    min-height: {key};
    padding: 0;
    border-radius: 9999px;
    border: 2px solid alpha(@theme_fg_color, 0.9);
    font-weight: 800;
    font-size: {key_font};
}}
.bp-title {{
    font-size: {title};
    font-weight: 400;
    color: @accent_color;
    padding: 2px 0;
}}
.bp-page-title {{ font-size: {page_title}; font-weight: 400; }}
.bp-page-subtitle {{
    font-size: {subtitle};
    color: alpha(@theme_fg_color, 0.55);
}}
/* The tab picker: libadwaita's toggle group, minus its bold labels. */
.bp-tabs toggle {{
    font-weight: normal;
}}
.bp-tabs toggle label {{
    font-size: {subtitle};
}}
.bp-menu-dim {{
    background: alpha(black, 0.6);
}}
.bp-menu-panel {{
    background: alpha(@theme_bg_color, 0.98);
    border-radius: 16px;
    padding: {menu_pad};
}}
/* The options menu: a full-height panel docked on the right, Switch-style. */
.bp-option-panel {{
    border-radius: 0;
    border-left: 1px solid alpha(white, 0.08);
}}
.bp-menu-title {{
    font-size: {subtitle};
    color: alpha(@theme_fg_color, 0.55);
    padding: {menu_pad} {menu_pad} 4px;
}}
.bp-menu-row {{
    padding: {group_row_pad};
    padding-left: {menu_pad};
    padding-right: {menu_pad};
    border-radius: 10px;
}}
.bp-menu-row label {{
    font-size: {subtitle};
}}
.bp-menu-row-selected {{ background: alpha(@accent_color, 0.45); }}
.bp-menu-row-active label {{
    color: @accent_color;
}}
/* The pointer is parked (a gamepad or the keyboard is driving): hover must
   not keep dressing whatever the parked pointer rests on — covers, group
   tiles, header buttons alike. */
.bp-cursor-hidden .cover-item:hover .game-cover-pic {{
    box-shadow: none;
    transform: none;
}}
.bp-cursor-hidden .cover-item.bp-all:hover .bp-all-tile {{
    background: alpha(@theme_fg_color, 0.08);
}}
.bp-cursor-hidden flowbox > child:hover {{ background: none; }}
.bp-cursor-hidden button:hover {{
    background: none;
    box-shadow: none;
}}
.bp-shoulder {{
    padding: 2px 10px;
    border: 2px solid alpha(@theme_fg_color, 0.55);
    border-radius: 8px;
    font-size: {small};
}}
.bp-group-slot {{
    background: alpha(white, 0.06);
}}
.bp-key {{
    background: alpha(white, 0.08);
    border-radius: 8px;
}}
.bp-key label {{
    font-size: {subtitle};
}}
.bp-key-selected {{
    background: alpha(@accent_color, 0.45);
}}
.bp-key-active label {{
    color: @accent_color;
}}
.bp-key-preview {{
    font-size: {page_title};
    padding: {group_row_pad};
    padding-left: 22px;
    background: alpha(white, 0.05);
    border-radius: 10px;
}}
"#,
        status_pad_top = px(16),
        status_pad_bottom = px(4),
        // The clock line plus both paddings: without a floor of its own,
        // the window's first transitional allocation starves the rail
        // below its content and GTK warns about measuring it for ~13px.
        status_min_h = px(56),
        small = px(24),
        pad_v = px(18),
        pad_h = px(36),
        min_h = px(72),
        key = px(32),
        key_font = px(16),
        title = px(28),
        page_title = px(30),
        subtitle = px(22),
        group_row_pad = px(14),
        menu_pad = px(22),
    )
}

/// The full stylesheet at a given viewport scale (1.0 = the 1080p
/// reference). The big-picture section scales; the desktop section does not.
pub fn app_css(ui_scale: f64) -> String {
    format!("{APP_CSS}
{}", big_picture_css(ui_scale))
}

/// Install the global stylesheet and icon theme additions on the default
/// display. Called once per window build (desktop or big picture) and on
/// viewport resizes; repeat calls reload the big picture sizes in place.
pub fn init_styles(ui_scale: f64) {
    thread_local! {
        static PROVIDER: gtk4::CssProvider = gtk4::CssProvider::new();
    }
    PROVIDER.with(|provider| {
        provider.load_from_string(&app_css(ui_scale));
        let display = gtk4::gdk::Display::default().expect("no default display");
        gtk4::style_context_add_provider_for_display(
            &display,
            provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER,
        );
        gtk4::IconTheme::for_display(&display).add_resource_path("/com/github/ira/icons");
    });
}
