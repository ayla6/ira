use super::css::*;
use crate::Game;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;

pub(super) type LogoControls = (
    gtk4::Box,
    Rc<RefCell<ira_models::LogoPosition>>,
    gtk4::Adjustment,
    Rc<Cell<bool>>,
);

/// Info needed to show a "Reset to Steam" button on the logo page.
pub(super) struct SteamLogoReset {
    pub steam: std::sync::Arc<ira_api::SteamDataClient>,
    pub app_id: String,
    pub db: ira_db::DbConn,
    pub db_id: i64,
}

pub(super) fn build_game_logo_page(
    game: &Game,
    show_reset: bool,
    steam_reset: Option<SteamLogoReset>,
    inherited: bool,
) -> Option<LogoControls> {
    let logo_page = gtk4::Box::new(gtk4::Orientation::Vertical, 16);

    let pos = if inherited {
        ira_models::LogoPosition::DEFAULT
    } else {
        game.logo_position
    };
    let selected_pos: Rc<RefCell<ira_models::LogoPosition>> = Rc::new(RefCell::new(pos));
    let modified: Rc<Cell<bool>> = Rc::new(Cell::new(false));

    let size_pct = if inherited { 50 } else { game.logo_size }.clamp(5, 100);
    let size_adj = gtk4::Adjustment::new(size_pct as f64, 5.0, 100.0, 1.0, 5.0, 0.0);

    let preview_overlay = gtk4::Overlay::new();
    preview_overlay.set_overflow(gtk4::Overflow::Hidden);

    let hero_pic = gtk4::Picture::new();
    let hero_texture = ira_images::texture_for(&game.hero_image_path);
    if let Some(ref t) = hero_texture {
        hero_pic.set_paintable(Some(t));
    }
    hero_pic.set_content_fit(gtk4::ContentFit::Cover);
    hero_pic.set_halign(gtk4::Align::Fill);
    hero_pic.set_valign(gtk4::Align::Fill);
    preview_overlay.set_child(Some(&hero_pic));

    let overlay_h = hero_texture
        .map(|t| {
            let aspect = t.width() as f64 / t.height() as f64;
            (460.0 / aspect).max(100.0) as i32
        })
        .unwrap_or(200);
    preview_overlay.set_height_request(overlay_h);

    let preview_draw = gtk4::DrawingArea::new();
    preview_draw.set_halign(gtk4::Align::Fill);
    preview_draw.set_valign(gtk4::Align::Fill);
    preview_draw.set_hexpand(true);
    preview_draw.set_vexpand(true);

    if !game.logo_path.is_empty() {
        let pixbuf_cell: Rc<RefCell<Option<gtk4::gdk_pixbuf::Pixbuf>>> =
            Rc::new(RefCell::new(None));
        {
            let pixbuf_cell = pixbuf_cell.clone();
            let preview_weak = preview_draw.downgrade();
            ira_images::pixbuf_for_async(game.logo_path.clone(), move |pixbuf| {
                if let Some(pb) = pixbuf {
                    *pixbuf_cell.borrow_mut() = Some(pb);
                    if let Some(area) = preview_weak.upgrade() {
                        area.queue_draw();
                    }
                }
            });
        }
        let pos_for_draw = selected_pos.clone();
        let adj_for_draw = size_adj.clone();

        preview_draw.set_draw_func(move |_area, cr, area_w, area_h| {
            let Some(ref pixbuf) = *pixbuf_cell.borrow() else {
                return;
            };
            let pb_w = pixbuf.width() as f64;
            let pb_h = pixbuf.height() as f64;
            let w = area_w as f64;
            let h = area_h as f64;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let pct = adj_for_draw.value() as i32;
            let pos = *pos_for_draw.borrow();
            let (halign, valign) = super::game_display::logo_position_align(pos);
            let (x, y, lw, lh) =
                super::game_display::logo_rect(w, h, pb_w, pb_h, pct, halign, valign);
            let _ = cr.save();
            cr.translate(x, y);
            cr.scale(lw / pb_w, lh / pb_h);
            cr.set_source_pixbuf(pixbuf, 0.0, 0.0);
            let _ = cr.paint();
            let _ = cr.restore();
        });
    }

    preview_overlay.add_overlay(&preview_draw);

    let logo_positions = ira_models::LogoPosition::all();

    let pos_grid = gtk4::Grid::new();
    pos_grid.set_column_spacing(2);
    pos_grid.set_row_spacing(2);
    pos_grid.set_halign(gtk4::Align::Fill);
    pos_grid.set_valign(gtk4::Align::Fill);
    pos_grid.set_hexpand(true);
    pos_grid.set_vexpand(true);

    let current_pos = *selected_pos.borrow();
    let mut all_btns: Vec<gtk4::Button> = Vec::new();
    for (i, &pos) in logo_positions.iter().enumerate() {
        let btn = gtk4::Button::new();
        btn.add_css_class(CSS_LOGO_POS_OVERLAY_BTN);
        if pos == current_pos {
            btn.add_css_class(CSS_SELECTED);
        }
        btn.set_hexpand(true);
        btn.set_vexpand(true);
        let row = i / 3;
        let col = i % 3;
        pos_grid.attach(&btn, col as i32, row as i32, 1, 1);
        all_btns.push(btn);
    }

    let btns: Rc<Vec<glib::WeakRef<gtk4::Button>>> =
        Rc::new(all_btns.iter().map(|b| b.downgrade()).collect());
    for (i, &pos) in logo_positions.iter().enumerate() {
        let btns_c = btns.clone();
        let selected_pos_c = selected_pos.clone();
        let pos_owned = pos;
        let preview_clone = preview_draw.clone();
        let modified_c = modified.clone();
        let Some(button) = btns[i].upgrade() else {
            continue;
        };
        button.connect_clicked(move |btn| {
            for b in btns_c.iter().filter_map(|b| b.upgrade()) {
                b.remove_css_class(CSS_SELECTED);
            }
            btn.add_css_class(CSS_SELECTED);
            *selected_pos_c.borrow_mut() = pos_owned;
            modified_c.set(true);
            preview_clone.queue_draw();
        });
    }

    preview_overlay.add_overlay(&pos_grid);

    let preview_frame = gtk4::Frame::new(None::<&str>);
    preview_frame.set_child(Some(&preview_overlay));
    logo_page.append(&preview_frame);

    let header_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    header_row.set_halign(gtk4::Align::Fill);
    header_row.set_margin_start(12);
    let pos_label = gtk4::Label::new(Some(&crate::tr!("Logo position")));
    pos_label.set_halign(gtk4::Align::Start);
    pos_label.set_hexpand(true);
    pos_label.add_css_class(CSS_HEADING);
    header_row.append(&pos_label);

    if show_reset && !inherited {
        let reset_btn = gtk4::Button::from_icon_name("edit-undo-symbolic");
        reset_btn.add_css_class(CSS_FLAT);
        reset_btn.set_valign(gtk4::Align::Center);
        reset_btn.set_tooltip_text(Some(&crate::tr!("Reset to base game")));
        let selected_pos_reset = selected_pos.clone();
        let size_adj_reset = size_adj.clone();
        let btns_reset = btns.clone();
        let preview_reset = preview_draw.clone();
        let modified_reset = modified.clone();
        reset_btn.connect_clicked(move |_| {
            *selected_pos_reset.borrow_mut() = ira_models::LogoPosition::DEFAULT;
            size_adj_reset.set_value(50.0);
            modified_reset.set(false);
            for b in btns_reset.iter().filter_map(|b| b.upgrade()) {
                b.remove_css_class(CSS_SELECTED);
            }
            for (i, &pos) in ira_models::LogoPosition::all().iter().enumerate() {
                if pos == ira_models::LogoPosition::DEFAULT {
                    if let Some(b) = btns_reset[i].upgrade() {
                        b.add_css_class(CSS_SELECTED);
                    }
                }
            }
            preview_reset.queue_draw();
        });
        if inherited {
            reset_btn.set_sensitive(false);
        }
        header_row.append(&reset_btn);
    }

    if let Some(info) = steam_reset {
        header_row.append(&build_steam_reset_button(
            info,
            &selected_pos,
            &size_adj,
            &btns,
            &preview_draw,
            &modified,
        ));
    }
    logo_page.append(&header_row);

    logo_page.append(&build_logo_size_row(&size_adj, preview_draw, &modified));

    Some((logo_page, selected_pos, size_adj, modified))
}

fn build_logo_size_row(
    size_adj: &gtk4::Adjustment,
    preview_draw: gtk4::DrawingArea,
    modified: &Rc<Cell<bool>>,
) -> gtk4::Box {
    let size_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    size_row.set_hexpand(true);

    let size_scale = gtk4::Scale::new(gtk4::Orientation::Horizontal, Some(size_adj));
    size_scale.set_draw_value(false);
    size_scale.set_hexpand(true);

    let size_spin = gtk4::SpinButton::new(Some(size_adj), 1.0, 1);
    size_spin.set_numeric(true);
    size_spin.set_digits(1);

    let modified_for_size = modified.clone();
    size_adj.connect_value_changed(move |_| {
        preview_draw.queue_draw();
        modified_for_size.set(true);
    });

    size_row.append(&size_scale);
    size_row.append(&size_spin);
    size_row
}

fn build_steam_reset_button(
    info: SteamLogoReset,
    selected_pos: &Rc<RefCell<ira_models::LogoPosition>>,
    size_adj: &gtk4::Adjustment,
    btns: &Rc<Vec<glib::WeakRef<gtk4::Button>>>,
    preview_draw: &gtk4::DrawingArea,
    modified: &Rc<Cell<bool>>,
) -> gtk4::Button {
    let steam_reset_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
    steam_reset_btn.add_css_class(CSS_FLAT);
    steam_reset_btn.set_valign(gtk4::Align::Center);
    steam_reset_btn.set_tooltip_text(Some(&crate::tr!("Reset")));
    let selected_pos_r = selected_pos.clone();
    let size_adj_r = size_adj.clone();
    let btns_r = btns.clone();
    let preview_r = preview_draw.clone();
    let modified_r = modified.clone();
    let app_id_clone = info.app_id.clone();
    let steam_clone = info.steam.clone();
    let db = info.db.clone();
    let db_id = info.db_id;
    let btn_weak = steam_reset_btn.downgrade();
    steam_reset_btn.connect_clicked(move |_| {
        if let Some(btn) = btn_weak.upgrade() {
            btn.set_sensitive(false);
        }
        let (tx, rx) = mpsc::channel::<Option<(Option<ira_models::LogoPosition>, i32)>>();
        let app_id = app_id_clone.clone();
        let steam = steam_clone.clone();
        std::thread::spawn(move || {
            let info = steam.fetch_steamcmd_info(&app_id);
            let _ = tx.send(info.map(|i| (i.logo_position, i.logo_size)));
        });
        let rx = Rc::new(RefCell::new(rx));
        let db = db.clone();
        let btns_weak = Rc::downgrade(&btns_r);
        let selected_weak = Rc::downgrade(&selected_pos_r);
        let size_weak = size_adj_r.downgrade();
        let preview_weak = preview_r.downgrade();
        let modified_weak = Rc::downgrade(&modified_r);
        let btn_weak = btn_weak.clone();
        glib::source::idle_add_local_full(glib::Priority::LOW, move || {
            apply_steam_logo_result(&SteamLogoApply {
                btn_weak: btn_weak.clone(),
                selected_weak: selected_weak.clone(),
                size_weak: size_weak.clone(),
                modified_weak: modified_weak.clone(),
                btns_weak: btns_weak.clone(),
                preview_weak: preview_weak.clone(),
                rx: rx.clone(),
                db: db.clone(),
                db_id,
            })
        });
    });
    steam_reset_btn
}

struct SteamLogoApply {
    btn_weak: glib::WeakRef<gtk4::Button>,
    selected_weak: std::rc::Weak<RefCell<ira_models::LogoPosition>>,
    size_weak: glib::WeakRef<gtk4::Adjustment>,
    modified_weak: std::rc::Weak<Cell<bool>>,
    btns_weak: std::rc::Weak<Vec<glib::WeakRef<gtk4::Button>>>,
    preview_weak: glib::WeakRef<gtk4::DrawingArea>,
    rx: LogoFetchRx,
    db: ira_db::DbConn,
    db_id: i64,
}

type LogoFetchRx = Rc<RefCell<mpsc::Receiver<Option<(Option<ira_models::LogoPosition>, i32)>>>>;

fn apply_steam_logo_result(ctx: &SteamLogoApply) -> glib::ControlFlow {
    let (
        Some(btn),
        Some(selected),
        Some(size_adj),
        Some(modified),
        Some(btns),
        Some(preview),
    ) = (
        ctx.btn_weak.upgrade(),
        ctx.selected_weak.upgrade(),
        ctx.size_weak.upgrade(),
        ctx.modified_weak.upgrade(),
        ctx.btns_weak.upgrade(),
        ctx.preview_weak.upgrade(),
    )
    else {
        return glib::ControlFlow::Break;
    };
    match ctx.rx.borrow_mut().try_recv() {
        Ok(Some((pos, size))) => {
            // Steam naming nothing resets to the default, same as before.
            let pos = pos.unwrap_or(ira_models::LogoPosition::DEFAULT);
            *selected.borrow_mut() = pos;
            size_adj.set_value(size.clamp(5, 100) as f64);
            modified.set(true);
            for b in btns.iter().filter_map(|b| b.upgrade()) {
                b.remove_css_class(CSS_SELECTED);
            }
            for (i, &p) in ira_models::LogoPosition::all().iter().enumerate() {
                if p == pos {
                    if let Some(b) = btns[i].upgrade() {
                        b.add_css_class(CSS_SELECTED);
                    }
                }
            }
            preview.queue_draw();
            let _ = ira_db::set_logo_settings(&ctx.db, ctx.db_id, pos.as_str(), size.clamp(5, 100));
            btn.set_sensitive(true);
            glib::ControlFlow::Break
        }
        Ok(None) => {
            btn.set_sensitive(true);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => {
            btn.set_sensitive(true);
            glib::ControlFlow::Break
        }
    }
}
