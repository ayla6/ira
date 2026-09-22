//! The launch-time disc picker (desktop): a sheet listing one tile per
//! disc — the ScreenScraper disc art when it exists, a numbered
//! optical-disc icon otherwise. Picking a tile boots that disc through
//! the shared launch path.

use super::css::*;
use super::state::SharedState;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Boot one disc through the single launch path, surfacing failures the
/// same way every other launch entry point does.
pub(super) fn launch_disc(state: &SharedState, db_id: i64, variant_id: Option<i64>, disc_id: i64) {
    if let Err(e) = super::play_button::launch_game_disc(state, db_id, variant_id, disc_id) {
        eprintln!("Failed to launch game: {e}");
        let _ = state.borrow().sender.send(crate::AppMessage::AddGameError(e));
    }
}

/// One tile's widgets, kept so the art can land after the dialog is up.
struct DiscTile {
    disc: i32,
    stack: gtk4::Stack,
    picture: gtk4::Picture,
}

/// Present the disc picker for one game's `discs`. Picking a tile closes
/// the sheet and boots that disc.
pub(super) fn present(
    state: &SharedState,
    db_id: i64,
    variant_id: Option<i64>,
    game_name: &str,
    discs: &[ira_models::GameDisc],
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Select a disc"));
    dialog.set_content_width(520);
    dialog.set_content_height(360);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.add_css_class(CSS_FLAT);
    header.set_title_widget(Some(&gtk4::Label::new(Some(game_name))));
    toolbar.add_top_bar(&header);

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.set_margin_start(24);
    body.set_margin_end(24);
    body.set_margin_top(6);
    body.set_margin_bottom(24);

    let subtitle = gtk4::Label::new(Some(&crate::tr!("Choose which disc to boot")));
    subtitle.add_css_class(CSS_DIM_LABEL);
    body.append(&subtitle);

    let tiles: Rc<RefCell<Vec<DiscTile>>> = Rc::new(RefCell::new(Vec::new()));
    let grid = gtk4::FlowBox::new();
    grid.set_selection_mode(gtk4::SelectionMode::None);
    grid.set_homogeneous(true);
    grid.set_max_children_per_line(4);
    grid.set_min_children_per_line(discs.len().clamp(1, 2) as u32);
    grid.set_column_spacing(12);
    grid.set_row_spacing(12);
    grid.set_halign(gtk4::Align::Center);
    grid.set_valign(gtk4::Align::Start);

    for disc in discs {
        grid.append(&build_tile(state, &dialog, disc, db_id, variant_id, &tiles));
    }
    body.append(&grid);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_child(Some(&body));
    toolbar.set_content(Some(&scroll));
    dialog.set_child(Some(&toolbar));

    // The art lands after the dialog is up; swap each tile's stack page
    // as its texture arrives. The idle closure holds the tiles (and, via
    // their click handlers, the dialog) only until the art is delivered.
    let tiles_for_art = tiles;
    super::disc_art::fetch_disc_art(state, db_id, move |art| {
        for tile in tiles_for_art.borrow().iter() {
            if let Some(texture) = art.get(&tile.disc) {
                tile.picture.set_paintable(Some(texture));
                tile.stack.set_visible_child_name("art");
            }
        }
    });

    dialog.present(Some(&state.borrow().window));
}

fn build_tile(
    state: &SharedState,
    dialog: &adw::Dialog,
    disc: &ira_models::GameDisc,
    db_id: i64,
    variant_id: Option<i64>,
    tiles: &Rc<RefCell<Vec<DiscTile>>>,
) -> gtk4::Widget {
    let btn = gtk4::Button::new();
    btn.add_css_class(CSS_DISC_TILE);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);

    // The no-art fallback: an optical disc with the number written on it.
    let icon = gtk4::Image::from_icon_name("media-optical-symbolic");
    icon.set_pixel_size(72);
    let number = gtk4::Label::new(Some(&disc.disc_number.to_string()));
    number.add_css_class(CSS_DISC_NUMBER);
    let disc_face = gtk4::Overlay::new();
    disc_face.set_child(Some(&icon));
    disc_face.add_overlay(&number);
    let fallback = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    fallback.set_valign(gtk4::Align::Center);
    fallback.append(&disc_face);

    let picture = gtk4::Picture::new();
    picture.set_size_request(96, 96);
    picture.set_content_fit(gtk4::ContentFit::Contain);

    let stack = gtk4::Stack::new();
    stack.add_named(&fallback, Some("icon"));
    stack.add_named(&picture, Some("art"));
    stack.set_vhomogeneous(false);
    vbox.append(&stack);

    let caption = if disc.label.is_empty() {
        crate::tr!("Disc {}").replacen("{}", &disc.disc_number.to_string(), 1)
    } else {
        disc.label.clone()
    };
    let caption_label = gtk4::Label::new(Some(&caption));
    caption_label.add_css_class(CSS_DISC_TILE_CAPTION);
    vbox.append(&caption_label);

    btn.set_child(Some(&vbox));
    tiles.borrow_mut().push(DiscTile {
        disc: disc.disc_number,
        stack: stack.clone(),
        picture,
    });

    let click_state = state.clone();
    let click_dialog = dialog.clone();
    let disc_id = disc.id;
    btn.connect_clicked(move |_| {
        click_dialog.force_close();
        launch_disc(&click_state, db_id, variant_id, disc_id);
    });

    btn.upcast()
}
