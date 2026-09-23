//! The big-picture disc picker: a centered sheet over a dimmed page
//! showing one tile per disc in disc-number order — the ScreenScraper
//! disc art when it exists, a numbered optical-disc icon otherwise.
//! The gamepad and keyboard walk the row; confirming boots the disc,
//! a click boots it straight away.

use super::super::state::SharedState;
use crate::ui::css::*;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// One tile's widgets, kept so the art can land after the sheet is up.
struct DiscTile {
    disc_id: i64,
    disc_number: i32,
    button: gtk4::Button,
    stack: gtk4::Stack,
    picture: gtk4::Picture,
}

pub(super) struct DiscPicker {
    root: gtk4::Overlay,
    title: gtk4::Label,
    row: gtk4::Box,
    tiles: Rc<RefCell<Vec<DiscTile>>>,
    selection: Cell<usize>,
    db_id: Cell<i64>,
    variant_id: Cell<Option<i64>>,
}

impl DiscPicker {
    pub(super) fn build() -> Self {
        let dim = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        dim.add_css_class(CSS_BP_MENU_DIM);

        let sheet = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        sheet.set_halign(gtk4::Align::Center);
        sheet.set_valign(gtk4::Align::Center);

        let title = gtk4::Label::new(None);
        title.set_xalign(0.5);
        title.set_wrap(true);
        title.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        title.add_css_class(CSS_BP_MENU_TITLE);
        crate::ui::helpers::crisp_label(&title);
        sheet.append(&title);

        let subtitle = gtk4::Label::new(Some(&crate::tr!("Select a disc")));
        subtitle.set_xalign(0.5);
        subtitle.add_css_class(CSS_DIM_LABEL);
        crate::ui::helpers::crisp_label(&subtitle);
        sheet.append(&subtitle);

        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        row.set_halign(gtk4::Align::Center);
        sheet.append(&row);

        let root = gtk4::Overlay::new();
        root.add_css_class(CSS_BP_ROOT);
        root.set_child(Some(&dim));
        root.add_overlay(&sheet);
        root.set_visible(false);
        Self {
            root,
            title,
            row,
            tiles: Rc::new(RefCell::new(Vec::new())),
            selection: Cell::new(0),
            db_id: Cell::new(0),
            variant_id: Cell::new(None),
        }
    }

    pub(super) fn root(&self) -> &gtk4::Widget {
        self.root.upcast_ref()
    }

    pub(super) fn is_open(&self) -> bool {
        self.root.is_visible()
    }

    /// Show the sheet for one game's discs, oldest first, and start the
    /// disc-art fetch; tiles swap in their art as it arrives.
    pub(super) fn open(
        &self,
        state: &SharedState,
        db_id: i64,
        variant_id: Option<i64>,
        game_name: &str,
        discs: &[ira_models::GameDisc],
    ) {
        crate::ui::helpers::clear_children(&self.row);
        self.tiles.borrow_mut().clear();
        let mut ordered = discs.to_vec();
        ordered.sort_by_key(|disc| disc.disc_number);
        self.title.set_text(game_name);
        self.db_id.set(db_id);
        self.variant_id.set(variant_id);
        for disc in &ordered {
            self.row.append(&self.build_tile(state, db_id, variant_id, disc));
        }
        self.selection.set(0);
        self.mark_selected();
        self.root.set_visible(true);
        let tiles = Rc::clone(&self.tiles);
        crate::ui::disc_art::fetch_disc_art(state, db_id, move |art| {
            for tile in tiles.borrow().iter() {
                if let Some(texture) = art.get(&tile.disc_number) {
                    tile.picture.set_paintable(Some(texture));
                    tile.stack.set_visible_child_name("art");
                }
            }
        });
    }

    pub(super) fn close(&self) {
        self.root.set_visible(false);
    }

    /// Step the highlight, clamped to the row ends.
    pub(super) fn move_selection(&self, delta: i32) {
        let count = self.tiles.borrow().len();
        if count == 0 {
            return;
        }
        let next = (self.selection.get() as i32 + delta).clamp(0, count as i32 - 1) as usize;
        if next != self.selection.get() {
            self.selection.set(next);
            self.mark_selected();
        }
    }

    /// Boot the highlighted disc through the single launch path.
    pub(super) fn activate(&self, state: &SharedState) {
        let tile = self.tiles.borrow().get(self.selection.get()).map(|tile| {
            (
                tile.disc_id,
                self.db_id.get(),
                self.variant_id.get(),
            )
        });
        let Some((disc_id, db_id, variant_id)) = tile else {
            return;
        };
        self.close();
        super::super::disc_picker::launch_disc(state, db_id, variant_id, disc_id);
    }

    fn mark_selected(&self) {
        for (index, tile) in self.tiles.borrow().iter().enumerate() {
            if index == self.selection.get() {
                tile.button.add_css_class(CSS_BP_MENU_ROW_SELECTED);
            } else {
                tile.button.remove_css_class(CSS_BP_MENU_ROW_SELECTED);
            }
        }
    }

    fn build_tile(
        &self,
        state: &SharedState,
        db_id: i64,
        variant_id: Option<i64>,
        disc: &ira_models::GameDisc,
    ) -> gtk4::Widget {
        let scale = crate::ui::css::bp_scale().max(0.5);
        let btn = gtk4::Button::new();
        btn.add_css_class(CSS_DISC_TILE);

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);

        // The no-art fallback: an optical disc with the number written
        // on it, swapped out when the texture lands.
        let icon = gtk4::Image::from_icon_name("media-optical-symbolic");
        icon.set_pixel_size((96.0 * scale).round() as i32);
        let number = gtk4::Label::new(Some(&disc.disc_number.to_string()));
        number.add_css_class(CSS_DISC_NUMBER);
        let disc_face = gtk4::Overlay::new();
        disc_face.set_child(Some(&icon));
        disc_face.add_overlay(&number);
        let fallback = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        fallback.set_valign(gtk4::Align::Center);
        fallback.append(&disc_face);

        let picture = gtk4::Picture::new();
        let art_px = (192.0 * scale).round() as i32;
        picture.set_size_request(art_px, art_px);
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
        self.tiles.borrow_mut().push(DiscTile {
            disc_id: disc.id,
            disc_number: disc.disc_number,
            button: btn.clone(),
            stack: stack.clone(),
            picture,
        });

        // A click boots straight away, like the desktop picker.
        let click_state = state.clone();
        let click_picker = self.root.clone();
        let click_disc_id = disc.id;
        btn.connect_clicked(move |_| {
            click_picker.set_visible(false);
            super::super::disc_picker::launch_disc(
                &click_state,
                db_id,
                variant_id,
                click_disc_id,
            );
        });
        btn.upcast()
    }
}
