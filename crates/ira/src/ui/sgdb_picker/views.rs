use super::super::css::*;
use adw::prelude::*;

/// Widgets of a fresh picker window, built once by [`build_picker_views`].
pub(super) struct PickerViews {
    pub(super) scrolled: gtk4::ScrolledWindow,
    pub(super) flow: gtk4::FlowBox,
    pub(super) list_view: gtk4::ListBox,
    pub(super) stack: gtk4::Stack,
    pub(super) toggle_btn: gtk4::ToggleButton,
    pub(super) zoom_scale: gtk4::Scale,
    pub(super) loading_grid: gtk4::Label,
    pub(super) loading_list: gtk4::Label,
}

/// Fresh picker shell: header with view toggle and zoom, grid/list stack
/// in a scrolled window. Returns the root widget and the live views.
pub(super) fn build_picker_views(asset: &str, zoom_initial: i32) -> (gtk4::Box, PickerViews) {
    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header_bar = adw::HeaderBar::new();
    let title = crate::tr!("Pick {}").replacen("{}", asset, 1);
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(&title))));

    let toggle_btn = gtk4::ToggleButton::new();
    toggle_btn.set_icon_name("view-list-symbolic");
    toggle_btn.set_tooltip_text(Some(&crate::tr!("Switch to list view")));
    toggle_btn.add_css_class(CSS_FLAT);

    let zoom_adj = gtk4::Adjustment::new(
        zoom_initial as f64,
        100.0,
        500.0,
        50.0,
        100.0,
        0.0,
    );
    let zoom_scale = gtk4::Scale::new(gtk4::Orientation::Horizontal, Some(&zoom_adj));
    zoom_scale.set_tooltip_text(Some(&crate::tr!("Zoom")));
    zoom_scale.set_width_request(120);
    zoom_scale.set_draw_value(false);
    zoom_scale.add_css_class(CSS_FLAT);

    header_bar.pack_end(&zoom_scale);
    header_bar.pack_end(&toggle_btn);
    outer.append(&header_bar);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);

    let stack = gtk4::Stack::new();

    let flow = gtk4::FlowBox::new();
    flow.set_selection_mode(gtk4::SelectionMode::None);
    // Children size themselves to their tiles; the box wraps by width so
    // columns follow the window and zoom like the game grid does.
    flow.set_homogeneous(false);
    flow.set_min_children_per_line(1);
    flow.set_row_spacing(12);
    flow.set_column_spacing(12);
    // Top-aligned: a filling flow stretches its rows to the viewport
    // height (measured 800 vs a 516 card), which is the tall-tile dead
    // space. Natural height keeps rows content-sized like the game grid.
    flow.set_valign(gtk4::Align::Start);
    flow.set_margin_start(12);
    flow.set_margin_end(12);
    flow.set_margin_top(8);
    flow.set_margin_bottom(8);
    flow.set_halign(gtk4::Align::Fill);

    let list_view = gtk4::ListBox::new();
    list_view.set_selection_mode(gtk4::SelectionMode::None);
    list_view.set_margin_start(12);
    list_view.set_margin_end(12);
    list_view.set_margin_top(8);
    list_view.set_margin_bottom(8);
    // Same row-stretch guard as the grid: short lists must not grow
    // tall rows to fill the viewport.
    list_view.set_valign(gtk4::Align::Start);

    stack.add_named(&flow, Some("grid"));
    stack.add_named(&list_view, Some("list"));
    stack.set_visible_child_name("grid");

    let loading_grid = gtk4::Label::new(Some(&crate::tr!("Loading\u{2026}")));
    loading_grid.add_css_class(CSS_DIM_LABEL);
    flow.append(&loading_grid);
    let loading_list = gtk4::Label::new(Some(&crate::tr!("Loading\u{2026}")));
    loading_list.add_css_class(CSS_DIM_LABEL);
    list_view.append(&loading_list);

    scrolled.set_child(Some(&stack));
    outer.append(&scrolled);

    (
        outer,
        PickerViews {
            scrolled,
            flow,
            list_view,
            stack,
            toggle_btn,
            zoom_scale,
            loading_grid,
            loading_list,
        },
    )
}
