//! The big-picture disc picker: a centered sheet over a dimmed page
//! showing one tile per disc in disc-number order — the ScreenScraper
//! disc art when it exists, a numbered optical-disc icon otherwise.
//! The gamepad and keyboard walk the row; confirming boots the disc,
//! a click boots it straight away. The highlight is the same accent
//! ring (circled, discs are round) and floating name pill the game
//! grid wears, re-derived every frame from the live tile geometry.

use super::super::state::SharedState;
use crate::ui::css::*;
use crate::ui::selection_ring::SelectionRing;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// One tile's widgets, kept so the art can land after the sheet is up.
#[derive(Clone)]
struct DiscTile {
    disc_id: i64,
    disc_number: i32,
    button: gtk4::Button,
    stack: gtk4::Stack,
    picture: gtk4::Picture,
    label: String,
}

pub(super) struct DiscPicker {
    root: gtk4::Overlay,
    title: gtk4::Label,
    row: gtk4::Box,
    ring: SelectionRing,
    pill: super::marquee::Marquee,
    tiles: Rc<RefCell<Vec<DiscTile>>>,
    selection: Rc<Cell<usize>>,
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

        let ring = SelectionRing::new();
        let pill = super::marquee::Marquee::new(0, 480.0);

        let root = gtk4::Overlay::new();
        root.add_css_class(CSS_BP_ROOT);
        root.set_child(Some(&dim));
        root.add_overlay(&sheet);
        root.add_overlay(&ring);
        root.add_overlay(pill.widget());
        root.set_measure_overlay(&ring, false);
        root.set_measure_overlay(pill.widget(), false);
        root.set_clip_overlay(&ring, true);
        root.set_visible(false);
        Self {
            root,
            title,
            row,
            ring,
            pill,
            tiles: Rc::new(RefCell::new(Vec::new())),
            selection: Rc::new(Cell::new(0)),
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
    /// disc-art fetch; tiles swap in their art as it arrives. The ring
    /// and the name pill re-derive from the live tile geometry on every
    /// frame, so no event ordering can park them stale.
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
        if ordered.is_empty() {
            return;
        }
        let scale = crate::ui::css::bp_scale().max(0.5);
        self.row
            .set_spacing((28.0 * scale).round() as i32);
        self.title.set_text(game_name);
        self.db_id.set(db_id);
        self.variant_id.set(variant_id);
        for disc in &ordered {
            self.row.append(&self.build_tile(state, db_id, variant_id, disc));
        }
        self.selection.set(0);
        self.pill.set_text(&self.tile_name(0));
        self.pill
            .set_max_width(480.0 * scale.max(0.5));
        self.track_selection();
        self.root.set_visible(true);
        let tiles = Rc::clone(&self.tiles);
        let place_tiles = Rc::clone(&self.tiles);
        let place_selection = Rc::clone(&self.selection);
        let place_ring = self.ring.downgrade();
        let place_pill = self.pill.clone();
        let place_root = self.root.clone();
        crate::ui::disc_art::fetch_disc_art(state, db_id, move |art| {
            for tile in tiles.borrow().iter() {
                if let Some(texture) = art.get(&tile.disc_number) {
                    tile.picture.set_paintable(Some(texture));
                    tile.stack.set_visible_child_name("art");
                }
            }
            // Tiles grow when the art lands; re-park the ring and the
            // pill on the new geometry instead of waiting for whatever
            // repaints the ring next.
            if let (Some(ring), Some(tile)) = (
                place_ring.upgrade(),
                place_tiles.borrow().get(place_selection.get()).cloned(),
            ) {
                anchor_tile(place_root.upcast_ref(), &ring, &place_pill, &tile);
            }
        });
    }

    pub(super) fn close(&self) {
        self.root.set_visible(false);
    }

    /// Step the highlight, clamped to the row ends; the pill text follows
    /// and the rect lands synchronously, with the per-frame repositioner
    /// covering resizes and art swaps underneath.
    pub(super) fn move_selection(&self, delta: i32) {
        let count = self.tiles.borrow().len();
        if count == 0 {
            return;
        }
        let next = (self.selection.get() as i32 + delta).clamp(0, count as i32 - 1) as usize;
        if next != self.selection.get() {
            self.selection.set(next);
            self.pill.set_text(&self.tile_name(next));
            self.place_selection();
        }
    }

    /// Park the ring and the pill anchor on the selected tile right now,
    /// from live geometry. Returns false when there is nothing real to
    /// draw yet (unmapped tiles on the opening frame).
    fn place_selection(&self) -> bool {
        let tiles = self.tiles.borrow();
        let Some(tile) = tiles.get(self.selection.get()) else {
            return false;
        };
        anchor_tile(self.root.upcast_ref(), &self.ring, &self.pill, tile)
    }

    /// Boot the highlighted disc through the single launch path.
    pub(super) fn activate(&self, state: &SharedState) {
        if super::launch_locked(state) {
            return;
        }
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

    fn tile_name(&self, index: usize) -> String {
        self.tiles.borrow().get(index).map_or_else(String::new, |tile| {
            tile.label.clone()
        })
    }

    /// Re-derive the ring rect and the pill anchor from the selected
    /// tile's live position every frame, like the group tiles do.
    fn track_selection(&self) {
        let tiles = Rc::clone(&self.tiles);
        let selection = Rc::clone(&self.selection);
        let ring = self.ring.downgrade();
        let pill = self.pill.clone();
        let root = self.root.clone();
        self.ring.set_repositioner(Some(Box::new(move || {
            let (Some(ring), Some(tile)) =
                (ring.upgrade(), tiles.borrow().get(selection.get()).cloned())
            else {
                return false;
            };
            anchor_tile(root.upcast_ref(), &ring, &pill, &tile)
        })));
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
        btn.add_css_class(CSS_FLAT);

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
        btn.set_child(Some(&stack));

        let label = if disc.label.is_empty() {
            crate::tr!("Disc {}").replacen("{}", &disc.disc_number.to_string(), 1)
        } else {
            disc.label.clone()
        };
        self.tiles.borrow_mut().push(DiscTile {
            disc_id: disc.id,
            disc_number: disc.disc_number,
            button: btn.clone(),
            stack: stack.clone(),
            picture,
            label,
        });

        // A click boots straight away, like the desktop picker.
        let click_state = state.clone();
        let click_picker = self.root.clone();
        let click_disc_id = disc.id;
        btn.connect_clicked(move |_| {
            if super::launch_locked(&click_state) {
                return;
            }
            click_picker.set_visible(false);
            super::super::disc_picker::launch_disc(&click_state, db_id, variant_id, click_disc_id);
        });
        btn.upcast()
    }
}

/// Park `ring` circled round the tile and anchor `pill` above it, both
/// from live geometry in `root` coordinates. False when there is
/// nothing real to draw yet.
fn anchor_tile(
    root: &gtk4::Widget,
    ring: &crate::ui::selection_ring::SelectionRing,
    pill: &super::marquee::Marquee,
    tile: &DiscTile,
) -> bool {
    if !tile.button.is_mapped() {
        return false;
    }
    let Some(point) = tile
        .button
        .compute_point(root, &gtk4::graphene::Point::zero())
    else {
        return false;
    };
    let (w, h) = (tile.button.width() as f64, tile.button.height() as f64);
    if w < 8.0 || h < 8.0 {
        return false;
    }
    let scale = root.width() as f64 / 1920.0;
    ring.set_rect(point.x() as f64, point.y() as f64, w, h, scale, true);
    pill.set_position(
        point.x() as f64 + w / 2.0,
        root.width() as f64,
        point.y() as f64 - crate::ui::css::bp_ring_outset(),
        false,
    );
    true
}
