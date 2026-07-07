use crate::parser::Game;
use crate::state::SharedState;
use crate::AppMessage;
use adw::prelude::*;
use gtk4::glib;

/// Build the game header widget: hero image + logo overlay + stats bar.
pub fn build_game_header(
    game: &Game,
    fraction: f64,
    state: &SharedState,
    content_width: i32,
) -> gtk4::Widget {
    let title_row = {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
        let title_label = gtk4::Label::new(Some(&game.name));
        title_label.set_xalign(0.0);
        title_label.add_css_class("title-1");
        row.append(&title_label);
        row
    };

    let stats_row = {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
        row.set_valign(gtk4::Align::Center);
        row.append(&play_button(state, game.lutris_id));
        row.append(&stat_label("Last played", &format_lastplayed(game.lastplayed)));
        row.append(&stat_label("Play time", &format_playtime(game.playtime)));
        if game.total_count > 0 {
            let tbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            tbox.set_valign(gtk4::Align::Center);
            let cap = gtk4::Label::new(Some("Trophies"));
            cap.set_xalign(0.0);
            cap.add_css_class("dim-label");
            cap.add_css_class("caption");
            tbox.append(&cap);
            let trow = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            trow.set_valign(gtk4::Align::Center);
            let val = gtk4::Label::new(Some(&format!(
                "{}/{}",
                game.earned_count, game.total_count
            )));
            val.set_xalign(0.0);
            val.set_valign(gtk4::Align::Center);
            val.add_css_class("heading");
            trow.append(&val);
            let prog = gtk4::ProgressBar::new();
            prog.set_fraction(fraction);
            prog.set_valign(gtk4::Align::Center);
            prog.set_size_request(120, -1);
            trow.append(&prog);
            tbox.append(&trow);
            row.append(&tbox);
        }
        row
    };

    if game.hero_image_path.is_empty() {
        let header = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        header.set_margin_top(24);
        header.set_margin_bottom(8);
        header.set_margin_start(24);
        header.set_margin_end(24);
        header.append(&title_row);
        header.append(&stats_row);
        return header.upcast();
    }

    // Hero overlay with logo painted via DrawingArea (exact pixel positioning)
    let overlay = gtk4::Overlay::new();
    overlay.set_vexpand(false);
    overlay.set_hexpand(true);
    overlay.set_height_request(((content_width as f64) / 3.1).max(150.0) as i32);

    let hero = gtk4::Picture::for_filename(&game.hero_image_path);
    hero.set_halign(gtk4::Align::Fill);
    hero.set_valign(gtk4::Align::Fill);
    hero.set_hexpand(true);
    hero.set_content_fit(gtk4::ContentFit::Cover);
    overlay.set_child(Some(&hero));

    if !game.logo_path.is_empty() {
        let logo_pct = game.logo_size.clamp(5, 100);
        let logo_pos = game.logo_position.clone();

        if let Ok(pixbuf) = gtk4::gdk_pixbuf::Pixbuf::from_file(&game.logo_path) {
            let pb_w = pixbuf.width() as f64;
            let pb_h = pixbuf.height() as f64;

            let logo_area = gtk4::DrawingArea::new();
            logo_area.set_halign(gtk4::Align::Fill);
            logo_area.set_valign(gtk4::Align::Fill);
            logo_area.set_hexpand(true);
            logo_area.set_vexpand(true);

            logo_area.set_draw_func(move |_, cr, area_w, area_h| {
                let w = area_w as f64;
                let h = area_h as f64;
                if w <= 0.0 || h <= 0.0 {
                    return;
                }

                let (sw, sh) = logo_scaled_dims(w, h, pb_w, pb_h, logo_pct);
                let lw = sw as f64;
                let lh = sh as f64;

                let (halign, valign) = logo_position_align(&logo_pos);
                let x = margin_coord(w, lw, halign, 24.0);
                let y = margin_coord(h, lh, valign, 24.0);

                let _ = cr.save();
                cr.translate(x, y);
                cr.scale(lw / pb_w, lh / pb_h);
                cr.set_source_pixbuf(&pixbuf, 0.0, 0.0);
                let _ = cr.paint();
                let _ = cr.restore();
            });

            overlay.add_overlay(&logo_area);
        }
    }

    // Resize tick: keep hero height proportional to width
    overlay.add_tick_callback(move |o, _fc| {
        let w = o.allocated_width();
        if w > 0 {
            let target = ((w as f64) / 3.1).max(150.0) as i32;
            if o.height_request() != target {
                o.set_height_request(target);
            }
        }
        glib::ControlFlow::Continue
    });

    let stats_container = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
    stats_container.set_margin_start(24);
    stats_container.set_margin_end(24);
    stats_container.set_margin_top(12);
    stats_container.set_margin_bottom(12);
    stats_container.append(&stats_row);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.append(&overlay);
    outer.append(&stats_container);
    outer.upcast()
}

// ---------------------------------------------------------------------------
// Logo helpers
// ---------------------------------------------------------------------------

fn margin_coord(outer: f64, inner: f64, align: gtk4::Align, margin: f64) -> f64 {
    match align {
        gtk4::Align::Start => margin,
        gtk4::Align::Center => (outer - inner) / 2.0,
        _ => outer - inner - margin,
    }
}

/// Compute logo scaled dimensions:
/// max height = logo_pct% of hero height, max width = logo_pct/2 % of hero width.
fn logo_scaled_dims(
    hero_w: f64,
    hero_h: f64,
    src_w: f64,
    src_h: f64,
    logo_pct: i32,
) -> (i32, i32) {
    let max_h = hero_h * (logo_pct as f64 / 100.0);
    let max_w = hero_w * (logo_pct as f64 / 200.0);
    let scale = (max_w / src_w).min(max_h / src_h);
    let w = (src_w * scale).max(32.0) as i32;
    let h = (src_h * scale).max(32.0) as i32;
    (w, h)
}

fn logo_position_align(pos: &str) -> (gtk4::Align, gtk4::Align) {
    match pos {
        "bottom-center" => (gtk4::Align::Center, gtk4::Align::End),
        "bottom-right" => (gtk4::Align::End, gtk4::Align::End),
        "center-left" => (gtk4::Align::Start, gtk4::Align::Center),
        "center" => (gtk4::Align::Center, gtk4::Align::Center),
        "center-right" => (gtk4::Align::End, gtk4::Align::Center),
        "top-left" => (gtk4::Align::Start, gtk4::Align::Start),
        "top-center" => (gtk4::Align::Center, gtk4::Align::Start),
        "top-right" => (gtk4::Align::End, gtk4::Align::Start),
        _ => (gtk4::Align::Start, gtk4::Align::End),
    }
}

// ---------------------------------------------------------------------------
// Stats helpers
// ---------------------------------------------------------------------------

fn format_playtime(hours: f64) -> String {
    let total = (hours * 60.0).round() as u64;
    let h = total / 60;
    let m = total % 60;
    match (h, m) {
        (0, 0) => "0min".to_string(),
        (0, m) => format!("{}min", m),
        (h, 0) => format!("{}h", h),
        (h, m) => format!("{}h{:02}min", h, m),
    }
}

fn format_lastplayed(ts: i64) -> String {
    if ts == 0 {
        return "Never".to_string();
    }
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%b %-d").to_string())
        .unwrap_or_else(|| "Never".to_string())
}

fn stat_label(caption: &str, value: &str) -> gtk4::Box {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_valign(gtk4::Align::Center);
    vbox.set_size_request(110, -1);
    let cap = gtk4::Label::new(Some(caption));
    cap.set_xalign(0.0);
    cap.add_css_class("dim-label");
    cap.add_css_class("caption");
    vbox.append(&cap);
    let val = gtk4::Label::new(Some(value));
    val.set_xalign(0.0);
    val.add_css_class("heading");
    vbox.append(&val);
    vbox
}

// ---------------------------------------------------------------------------
// Play / Stop button
// ---------------------------------------------------------------------------

fn play_button(state: &SharedState, lutris_id: i64) -> gtk4::Button {
    let running_games = state.borrow().running_games.clone();
    let sender = state.borrow().sender.clone();

    let btn = gtk4::Button::new();
    btn.set_valign(gtk4::Align::Center);
    btn.set_size_request(130, 48);

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_valign(gtk4::Align::Center);
    hbox.set_halign(gtk4::Align::Center);

    let icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
    icon.set_pixel_size(20);
    hbox.append(&icon);

    let label = gtk4::Label::new(Some("Play"));
    label.add_css_class("play-btn-label");
    hbox.append(&label);

    btn.set_child(Some(&hbox));

    let is_running = running_games.lock().unwrap().contains_key(&lutris_id);
    if is_running {
        icon.set_icon_name(Some("window-close-symbolic"));
        label.set_text("Stop");
    } else {
        btn.add_css_class("suggested-action");
    }

    let icon_click = icon.clone();
    let label_click = label.clone();
    let rg = running_games.clone();
    let s = sender.clone();
    btn.connect_clicked(move |btn| {
        let uri = format!("lutris:rungameid/{}", lutris_id);
        let mut map = rg.lock().unwrap();
        if let Some(mut child) = map.remove(&lutris_id) {
            drop(map);
            let _ = std::process::Command::new("kill")
                .arg(child.id().to_string())
                .spawn();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            icon_click.set_icon_name(Some("media-playback-start-symbolic"));
            label_click.set_text("Play");
            btn.add_css_class("suggested-action");
            s.send(AppMessage::GameStopped(lutris_id)).ok();
        } else {
            drop(map);
            match std::process::Command::new("lutris").arg(&uri).spawn() {
                Ok(child) => {
                    rg.lock().unwrap().insert(lutris_id, child);
                    icon_click.set_icon_name(Some("window-close-symbolic"));
                    label_click.set_text("Stop");
                    btn.remove_css_class("suggested-action");

                    let rg_mon = rg.clone();
                    let s_mon = s.clone();
                    let id = lutris_id;
                    std::thread::spawn(move || {
                        loop {
                            std::thread::sleep(std::time::Duration::from_secs(2));
                            let mut map = rg_mon.lock().unwrap();
                            if let Some(child) = map.get_mut(&id) {
                                match child.try_wait() {
                                    Ok(Some(_)) => {
                                        map.remove(&id);
                                        drop(map);
                                        s_mon.send(AppMessage::GameStopped(id)).ok();
                                        return;
                                    }
                                    Ok(None) => {}
                                    Err(_) => {
                                        map.remove(&id);
                                        drop(map);
                                        s_mon.send(AppMessage::GameStopped(id)).ok();
                                        return;
                                    }
                                }
                            } else {
                                return;
                            }
                        }
                    });
                }
                Err(e) => {
                    eprintln!("Failed to launch {}: {}", uri, e);
                }
            }
        }
    });

    btn
}
