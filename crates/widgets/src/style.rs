//! Shared Big Picture design system: scaled stylesheet, class names,
//! viewport scale state, and the bundled font installer.
//! Used by the main app and the overlay so both speak the same visual
//! language from one definition.

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
pub const CSS_BP_ALL: &str = "bp-all";
pub const CSS_BP_ALL_TILE: &str = "bp-all-tile";
pub const CSS_BP_ALL_PAGE: &str = "bp-all-page";
pub const CSS_BP_PAGE_TITLE: &str = "bp-page-title";
pub const CSS_BP_PAGE_SUBTITLE: &str = "bp-page-subtitle";
pub const CSS_BP_TABS: &str = "bp-tabs";
pub const CSS_BP_MENU_DIM: &str = "bp-menu-dim";
pub const CSS_BP_MENU_PANEL: &str = "bp-menu-panel";
pub const CSS_BP_EXTENSION: &str = "bp-extension";
pub const CSS_BP_OPTION_PANEL: &str = "bp-option-panel";
pub const CSS_BP_MENU_TITLE: &str = "bp-menu-title";
pub const CSS_BP_MENU_ROW: &str = "bp-menu-row";
pub const CSS_BP_MENU_ROW_SELECTED: &str = "bp-menu-row-selected";
pub const CSS_BP_MENU_ROW_ACTIVE: &str = "bp-menu-row-active";
pub const CSS_BP_CURSOR_HIDDEN: &str = "bp-cursor-hidden";
pub const CSS_BP_GROUP_SLOT: &str = "bp-group-slot";
pub const CSS_BP_RUNNING: &str = "bp-running";
pub const CSS_BP_RUNNING_TITLE: &str = "bp-running-title";
pub const CSS_BP_RUNNING_STATUS: &str = "bp-running-status";
pub const CSS_BP_KEY: &str = "bp-key";
pub const CSS_BP_KEY_SELECTED: &str = "bp-key-selected";
pub const CSS_BP_KEY_ACTIVE: &str = "bp-key-active";
pub const CSS_BP_KEY_DISABLED: &str = "bp-key-disabled";
pub const CSS_BP_KEY_OK: &str = "bp-key-ok";
pub const CSS_BP_KEY_CARET: &str = "bp-key-caret";
pub const CSS_BP_KEY_PREVIEW: &str = "bp-key-preview";
pub const CSS_BP_KEYBOARD: &str = "bp-keyboard";

/// The selection ring reaches this far past a tile's edge (4px gap plus
/// the 4px frame), and the tooltip's tail tip rests 3px beyond the
/// frame — 11px from the tile edge in total, at the 1080p reference;
/// [`bp_ring_outset`] scales it with the viewport.
pub const BP_RING_OUTSET: f64 = 11.0;

thread_local! {
    /// The big-picture viewport scale (1.0 = 1920 logical px wide) most
    /// recently applied through [`install_bp_css`] or [`set_bp_scale`].
    /// Rust-side geometry that mirrors the scaled stylesheet (ring
    /// outsets, pill chrome, key sizes) reads this instead of threading
    /// the scale through every constructor; 1.0 until the first refresh
    /// names a real value.
    static BP_SCALE: std::cell::Cell<f64> = const { std::cell::Cell::new(1.0) };
}

/// The viewport scale the big-picture sheet was last built with. Widgets
/// that draw pixel geometry outside CSS call this at build/draw time so
/// their constants keep proportion with the stylesheet.
pub fn bp_scale() -> f64 {
    BP_SCALE.with(|s| s.get())
}

/// Records the viewport scale (done implicitly by [`install_bp_css`];
/// called explicitly by hosts that manage their own CSS provider).
pub fn set_bp_scale(scale: f64) {
    BP_SCALE.with(|s| s.set(scale));
}

/// [`BP_RING_OUTSET`] at the current viewport scale, kept whole-pixel.
pub fn bp_ring_outset() -> f64 {
    (BP_RING_OUTSET * bp_scale()).round()
}
/// The big-picture stylesheet at a viewport scale (1.0 = the 1080p
/// reference). Includes the bundled-font rule; sizes scale, the desktop
/// section does not (it lives with the app).
pub fn bp_css(s: f64) -> String {
    format!(
        r#"/* Big-picture text: the bundled font (see font::install). Its sizes
   live below, scaled with the viewport. */
.bp-root {{
    font-family: "M PLUS 2", sans-serif;
}}
{}"#,
        big_picture_css(s)
    )
}

/// Loads [`bp_css`] onto the default display at `scale`, recording the
/// scale for Rust-side geometry ([`bp_scale`]). Repeat calls reload sizes
/// in place through a thread-local provider.
pub fn install_bp_css(scale: f64) {
    thread_local! {
        static PROVIDER: gtk4::CssProvider = gtk4::CssProvider::new();
    }
    BP_SCALE.with(|s| s.set(scale));
    PROVIDER.with(|provider| {
        provider.load_from_string(&bp_css(scale));
        let display = gtk4::gdk::Display::default().expect("no default display");
        gtk4::style_context_add_provider_for_display(
            &display,
            provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER,
        );
    });
}

fn big_picture_css(s: f64) -> String {
    [bp_css_header(s), bp_css_tabs_menu(s), bp_css_keyboard(s)].concat()
}

/// Shared pixel helpers for the big-picture sections, scaled from the
/// 1080p reference by `s`. Everything is whole pixels: fractional font
/// sizes render choppy.
struct BpPx {
    s: f64,
}

impl BpPx {
    fn new(s: f64) -> Self {
        Self { s }
    }

    /// Rounded and clamped to at least 1px (borders never vanish).
    fn px(&self, v: i32) -> String {
        format!("{}px", (f64::from(v) * self.s).round().max(1.0))
    }

    /// Rounded, can shrink to 0.
    fn pxs(&self, v: i32) -> String {
        format!("{}px", (f64::from(v) * self.s).round())
    }
}

/// libadwaita's keyboard focus ring, ported from its stylesheet's
/// focus-ring mixin: a 2px outline in the accent at half strength,
/// inset 2px, resting as an invisible zero-width outline one step
/// further out, and animated between the two with ease-out-quad.
fn bp_focus_ring(p: &BpPx) -> (String, String) {
    let ease = "cubic-bezier(0.25, 0.46, 0.45, 0.94)";
    let ring_rest = format!(
        "outline: 0 solid transparent;\n    outline-offset: {rest};\n    transition: outline-color 200ms {ease}, outline-width 200ms {ease}, outline-offset 200ms {ease};",
        rest = p.pxs(4),
    );
    let ring_on = format!(
        "outline-color: alpha(@accent_color, 0.5);\n    outline-width: {width};\n    outline-offset: {offset};",
        width = p.pxs(2),
        offset = p.pxs(-2),
    );
    (ring_rest, ring_on)
}

/// Home screen: status bar, clock, bottom bar, prompt, titles.
fn bp_css_header(s: f64) -> String {
    let p = BpPx::new(s);
    format!(
        r#".bp-status {{
    background-color: @window_bg_color;
    padding: {status_pad_v} {status_pad_x};
}}
.bp-clock {{ font-size: {small}; padding: {clock_pad} 0; }}
.bp-date, .bp-batt, .bp-clock, .bp-prompt {{ font-size: {small}; padding: {clock_pad} 0; }}
.bp-bottom {{
    background-color: @window_bg_color;
    padding: {pad_v} {pad_h};
    min-height: {min_h};
}}
.bp-title {{
    font-size: {title};
    font-weight: 400;
    color: @accent_color;
    padding: {clock_pad} 0;
}}
.bp-page-title {{ font-size: {page_title}; font-weight: 400; }}
.bp-page-subtitle {{
    font-size: {subtitle};
    color: alpha(@theme_fg_color, 0.55);
}}
.bp-running-title {{ font-size: {page_title}; font-weight: 700; }}
.bp-running-status {{ font-size: {subtitle}; }}
"#,
        status_pad_v = p.px(12),
        status_pad_x = p.px(24),
        clock_pad = p.pxs(2),
        small = p.px(24),
        pad_v = p.px(18),
        pad_h = p.px(36),
        min_h = p.px(72),
        title = p.px(28),
        page_title = p.px(30),
        subtitle = p.px(22),
    )
}

/// The tab picker and the menu/option panels, including the ported
/// focus-ring states.
fn bp_css_tabs_menu(s: f64) -> String {
    let p = BpPx::new(s);
    let (ring_rest, ring_on) = bp_focus_ring(&p);
    /* The tab picker: libadwaita's toggle-group metrics, ported from its
    stylesheet (3px group padding, 9px group radius, 34x28 toggles with
    8px side padding, 6px icon-label spacing) and scaled to the
    big-picture type size — minus the bold labels. */
    format!(
        r#".bp-tabs toggle,
.bp-tabs toggle label {{
    font-weight: normal;
}}
.bp-tabs {{
    padding: {tabs_group_pad};
    border-radius: {tabs_radius};
}}
.bp-tabs > toggle {{
    min-width: {tabs_min_w};
    min-height: {tabs_min_h};
    border-radius: {tabs_toggle_radius};
    padding-left: {tabs_pad_x};
    padding-right: {tabs_pad_x};
    /* Measured on a real screenshot: the toggle's content sits ~2.5px
       below the pill's center (the bundled font's asymmetric metrics put
       the glyph mass low in its line box). Bottom padding lifts the
       content to the measured optical center. */
    padding-bottom: {tabs_pad_bottom};
}}
.bp-tabs buttoncontent > box {{
    border-spacing: {tabs_spacing};
}}
.bp-tabs toggle label {{
    font-size: {subtitle};
}}
.bp-menu-dim {{
    background: alpha(black, 0.6);
}}
.bp-menu-panel {{
    background: alpha(@theme_bg_color, 0.98);
    border-radius: {menu_radius};
    padding: {menu_pad};
}}
/* The options menu: a full-height panel docked on the left. */
.bp-option-panel {{
    border-radius: 0;
    border-right: {hairline} solid alpha(white, 0.08);
}}
/* The content area as an extension of the sidebar: same surface,
   square corners, flush against it — never a floating card. */
.bp-extension {{
    background: alpha(@theme_bg_color, 0.98);
    border-radius: 0;
    padding: {menu_pad};
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
    border-radius: {row_radius};
    {ring_rest}
}}
.bp-menu-row label {{
    font-size: {subtitle};
    /* Same optical center as the tab pills: the bundled font's glyph mass
       sits low in its line box, and bottom padding lifts the text to the
       measured middle. */
    padding-bottom: {menu_optical};
}}
/* The cursor's row wears libadwaita's focus ring, not a fill: gamepad
   navigation reads exactly like GTK keyboard navigation. */
.bp-menu-row-selected {{ {ring_on} }}
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
"#,
        tabs_group_pad = p.px(3),
        tabs_radius = p.px(9),
        tabs_min_w = p.px(34),
        tabs_min_h = p.px(28),
        tabs_toggle_radius = p.px(6),
        tabs_pad_x = p.px(8),
        tabs_pad_bottom = p.px(5),
        tabs_spacing = p.px(6),
        subtitle = p.px(22),
        menu_radius = p.px(16),
        menu_pad = p.px(22),
        menu_optical = p.px(5),
        hairline = p.px(1),
        group_row_pad = p.px(14),
        row_radius = p.px(10),
        ring_rest = ring_rest,
        ring_on = ring_on,
    )
}

/// The on-screen keyboard: keys, caret and preview.
fn bp_css_keyboard(s: f64) -> String {
    let p = BpPx::new(s);
    let (ring_rest, ring_on) = bp_focus_ring(&p);
    format!(
        r#".bp-group-slot {{
    background: alpha(white, 0.06);
}}
.bp-key {{
    background: alpha(white, 0.08);
    border-radius: {key_radius};
    {ring_rest}
}}
.bp-key label {{
    font-size: {subtitle};
}}
/* The cursor rests on a key the way GTK focus rests on a button: the
   ported focus ring, never a fill. */
.bp-key-selected {{ {ring_on} }}
/* OK is the suggested action: libadwaita's opaque accent button. */
.bp-key-ok {{
    background: @accent_bg_color;
}}
.bp-key-ok label {{
    color: @accent_fg_color;
}}
/* The text caret: a thin accent bar that blinks (opacity from code). */
.bp-key-caret {{
    min-width: {caret_width};
    border-radius: 9999px;
    background: @accent_color;
}}
.bp-key-active {{
    background: alpha(@accent_color, 0.45);
}}
.bp-key-active label {{
    color: @theme_fg_color;
}}
.bp-key-disabled label {{
    color: alpha(@theme_fg_color, 0.35);
}}
.bp-key-preview, .bp-keyboard label {{
    font-family: "M PLUS 2", sans-serif;
}}
.bp-keyboard {{
    border-radius: {kbd_radius} {kbd_radius} 0 0;
    padding-top: {kbd_pad_top};
}}
.bp-key-preview {{
    padding: {group_row_pad};
    padding-left: {preview_inset_x};
    background: alpha(white, 0.05);
    border-radius: {row_radius};
}}
.bp-key-preview label {{
    font-size: {page_title};
}}
"#,
        key_radius = p.px(8),
        ring_rest = ring_rest,
        ring_on = ring_on,
        subtitle = p.px(22),
        caret_width = p.px(2),
        kbd_radius = p.px(16),
        kbd_pad_top = p.px(10),
        group_row_pad = p.px(14),
        preview_inset_x = p.px(22),
        row_radius = p.px(10),
        page_title = p.px(30),
    )
}

#[cfg(test)]
mod tests {
    use super::{bp_css, bp_ring_outset, set_bp_scale};

    #[test]
    fn test_bp_css_scales_with_viewport() {
        let base = bp_css(1.0);
        assert!(base.contains("M PLUS 2"));
        assert!(base.contains("font-size: 28px"));
        let doubled = bp_css(2.0);
        assert!(doubled.contains("font-size: 56px"));
    }

    #[test]
    fn test_bp_ring_outset_follows_scale() {
        let prev = super::bp_scale();
        set_bp_scale(1.0);
        assert_eq!(bp_ring_outset(), 11.0);
        set_bp_scale(2.0);
        assert_eq!(bp_ring_outset(), 22.0);
        set_bp_scale(prev);
    }
}
