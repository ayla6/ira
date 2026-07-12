pub const APP_CSS: &str = "
.sidebar-row-title { min-width: 0; }
.global-bar trough { background-color: transparent; border: none; }
.global-bar progress { border: none; border-radius: 0; }
.hidden-game { opacity: 0.5; }
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

.section-title {
    font-weight: 700;
    font-size: 1.25em;
}

.recent-scroll scrollbar {
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

.sidebar-section-header {
    font-size: 0.75em;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    opacity: 0.6;
}

.sidebar-collection-header {
    font-size: 1em;
}

.dim-label {
    opacity: 0.55;
}
";
