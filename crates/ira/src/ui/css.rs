pub const CSS_BOXED_LIST: &str = "boxed-list";
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
    min-width: 32px;
    min-height: 32px;
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
";
