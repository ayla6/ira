use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use crate::Game;

pub(super) fn build_game_logo_page(game: &Game) -> Option<(gtk4::Box, Rc<RefCell<String>>, gtk4::Adjustment)> {
    if game.logo_path.is_empty() {
        return None;
    }

    let logo_page = gtk4::Box::new(gtk4::Orientation::Vertical, 16);

    let selected_pos: Rc<RefCell<String>> = Rc::new(RefCell::new(game.logo_position.clone()));

    let size_pct = game.logo_size.clamp(5, 100);
    let size_adj = gtk4::Adjustment::new(size_pct as f64, 5.0, 100.0, 1.0, 5.0, 0.0);

    let preview_overlay = gtk4::Overlay::new();
    preview_overlay.set_height_request(220);
    preview_overlay.set_overflow(gtk4::Overflow::Hidden);

    let hero_pic = gtk4::Picture::new();
    if let Some(t) = ira_images::texture_for(&game.hero_image_path) {
        hero_pic.set_paintable(Some(&t));
    }
    hero_pic.set_content_fit(gtk4::ContentFit::Cover);
    hero_pic.set_halign(gtk4::Align::Fill);
    hero_pic.set_valign(gtk4::Align::Fill);
    preview_overlay.set_child(Some(&hero_pic));

    let preview_draw = gtk4::DrawingArea::new();
    preview_draw.set_halign(gtk4::Align::Fill);
    preview_draw.set_valign(gtk4::Align::Fill);
    preview_draw.set_hexpand(true);
    preview_draw.set_vexpand(true);

    if let Ok(ref pixbuf) = gtk4::gdk_pixbuf::Pixbuf::from_file(&game.logo_path) {
        let pb_w = pixbuf.width() as f64;
        let pb_h = pixbuf.height() as f64;
        let pixbuf_clone = pixbuf.clone();
        let pos_for_draw = selected_pos.clone();
        let adj_for_draw = size_adj.clone();

        preview_draw.set_draw_func(move |_area, cr, area_w, area_h| {
            let w = area_w as f64;
            let h = area_h as f64;
            if w <= 0.0 || h <= 0.0 { return; }
            let pct = adj_for_draw.value() as i32;
            let (lw, lh) = super::game_display::logo_scaled_dims(w, h, pb_w, pb_h, pct);
            let pos = pos_for_draw.borrow().clone();
            let (halign, valign) = super::game_display::logo_position_align(&pos);
            let x = match halign {
                gtk4::Align::Start => 12.0,
                gtk4::Align::Center => (w - lw) / 2.0,
                gtk4::Align::End => w - lw - 12.0,
                _ => 12.0,
            };
            let y = match valign {
                gtk4::Align::Start => 12.0,
                gtk4::Align::Center => (h - lh) / 2.0,
                gtk4::Align::End => h - lh - 12.0,
                _ => h - lh - 12.0,
            };
            let _ = cr.save();
            cr.translate(x, y);
            cr.scale(lw / pb_w, lh / pb_h);
            cr.set_source_pixbuf(&pixbuf_clone, 0.0, 0.0);
            let _ = cr.paint();
            let _ = cr.restore();
        });
    }

    preview_overlay.add_overlay(&preview_draw);

    let logo_positions = ["top-left", "top-center", "top-right", "center-left", "center", "center-right", "bottom-left", "bottom-center", "bottom-right"];

    let pos_grid = gtk4::Grid::new();
    pos_grid.set_column_spacing(2);
    pos_grid.set_row_spacing(2);
    pos_grid.set_halign(gtk4::Align::Fill);
    pos_grid.set_valign(gtk4::Align::Fill);
    pos_grid.set_hexpand(true);
    pos_grid.set_vexpand(true);

    let mut all_btns: Vec<gtk4::Button> = Vec::new();
    for (i, &pos) in logo_positions.iter().enumerate() {
        let btn = gtk4::Button::new();
        btn.add_css_class("logo-pos-overlay-btn");
        if pos == game.logo_position {
            btn.add_css_class("selected");
        }
        btn.set_hexpand(true);
        btn.set_vexpand(true);
        let row = i / 3;
        let col = i % 3;
        pos_grid.attach(&btn, col as i32, row as i32, 1, 1);
        all_btns.push(btn);
    }

    let btns: Rc<Vec<gtk4::Button>> = Rc::new(all_btns);
    for (i, &pos) in logo_positions.iter().enumerate() {
        let btns_c = btns.clone();
        let selected_pos_c = selected_pos.clone();
        let pos_owned = pos.to_string();
        let preview_clone = preview_draw.clone();
        btns[i].connect_clicked(move |btn| {
            for b in btns_c.iter() {
                b.remove_css_class("selected");
            }
            btn.add_css_class("selected");
            *selected_pos_c.borrow_mut() = pos_owned.clone();
            preview_clone.queue_draw();
        });
    }

    preview_overlay.add_overlay(&pos_grid);

    let preview_frame = gtk4::Frame::new(None::<&str>);
    preview_frame.set_child(Some(&preview_overlay));
    logo_page.append(&preview_frame);

    let size_label = gtk4::Label::new(Some("Size (% of hero height)"));
    size_label.set_halign(gtk4::Align::Start);
    size_label.add_css_class("heading");
    logo_page.append(&size_label);

    let size_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    size_row.set_hexpand(true);

    let size_scale = gtk4::Scale::new(gtk4::Orientation::Horizontal, Some(&size_adj));
    size_scale.set_draw_value(false);
    size_scale.set_hexpand(true);

    let size_spin = gtk4::SpinButton::new(Some(&size_adj), 1.0, 1);
    size_spin.set_numeric(true);
    size_spin.set_digits(1);

    let preview_draw_for_size = preview_draw.clone();
    size_adj.connect_value_changed(move |_| {
        preview_draw_for_size.queue_draw();
    });

    size_row.append(&size_scale);
    size_row.append(&size_spin);
    logo_page.append(&size_row);

    Some((logo_page, selected_pos, size_adj))
}
