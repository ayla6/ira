use super::context_menu::show_game_context_menu;
use super::css::*;
use super::grid_view::queue_cover_load_priority;
use super::message_helpers::switch_to_game;
use super::recent_carousel::RecentRow;
use super::state::SharedState;
use super::virtual_grid::VirtualGrid;
use crate::Game;
use gtk4::prelude::*;
use std::rc::Rc;

pub(super) fn build_recent_row(
    state: &SharedState,
    recent: &[Game],
    cover_height: i32,
) -> gtk4::Widget {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);

    let title_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    title_row.set_hexpand(true);

    let title = gtk4::Label::new(Some(&crate::tr!("Recently played")));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_margin_start(16);
    title.add_css_class(CSS_SECTION_TITLE);
    title_row.append(&title);

    let left_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    left_btn.add_css_class(CSS_FLAT);
    left_btn.set_valign(gtk4::Align::Center);
    left_btn.set_tooltip_text(Some(&crate::tr!("Previous games")));
    left_btn.set_sensitive(false);

    let right_btn = gtk4::Button::from_icon_name("go-next-symbolic");
    right_btn.add_css_class(CSS_FLAT);
    right_btn.set_valign(gtk4::Align::Center);
    right_btn.set_tooltip_text(Some(&crate::tr!("Next games")));

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    btn_box.set_margin_end(16);
    btn_box.append(&left_btn);
    btn_box.append(&right_btn);
    title_row.append(&btn_box);

    vbox.append(&title_row);

    let spacing = VirtualGrid::grid_spacing_for_item_w((cover_height as f64 * 2.0 / 3.0) as i32);
    let row = RecentRow::new(spacing, cover_height);

    let mut game_widths: Vec<i32> = Vec::with_capacity(recent.len());
    for (i, game) in recent.iter().enumerate() {
        let (w, h, use_header) = if i == 0 {
            let w = ((cover_height as f64) * 460.0 / 215.0) as i32;
            (w, cover_height, true)
        } else {
            let w = ((cover_height as f64) * 2.0 / 3.0) as i32;
            (w, cover_height, false)
        };
        game_widths.push(w);
        let path = if use_header {
            ira_parser::full_image_path(&game.header_path)
        } else {
            game.grid_path.clone()
        };
        let db_id = game.db_id;
        let variant_id = game.variant_id;
        let item = build_cover(
            state,
            game,
            &path,
            w,
            h,
            false,
            move |state| {
                switch_to_game(state, db_id, variant_id);
                super::sidebar::scroll_to_row(state, db_id, variant_id);
            },
        );
        row.append_cover(&item);
    }

    // The row is a GtkScrollable, so the scrolled window uses it directly
    // instead of wrapping it in a clipping viewport: hover shadows and the
    // hover scale escape the covers like they do in the grid, while wheel,
    // touchpad and scrollbar interactions stay fully native.
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);
    scrolled.set_hexpand(true);
    scrolled.set_vexpand(false);
    scrolled.set_valign(gtk4::Align::Start);
    scrolled.set_kinetic_scrolling(true);
    scrolled.set_overlay_scrolling(true);
    scrolled.add_css_class(CSS_RECENT_SCROLL);
    scrolled.set_child(Some(&row));

    vbox.append(&scrolled);

    let mut step_sizes = vec![spacing];
    step_sizes.extend(game_widths.iter().map(|w| w + spacing));
    let steps = Rc::new(step_sizes);

    let adj = scrolled.hadjustment();

    let left_btn2 = left_btn.clone();
    let right_btn2 = right_btn.clone();
    let adj_vc = adj.clone();
    adj.connect_changed(move |_| update_scroll_buttons(&adj_vc, &left_btn2, &right_btn2));
    let left_btn2 = left_btn.clone();
    let right_btn2 = right_btn.clone();
    let adj_vc = adj.clone();
    adj.connect_value_changed(move |_| update_scroll_buttons(&adj_vc, &left_btn2, &right_btn2));

    let adj_click = adj.clone();
    let steps2 = steps.clone();
    right_btn.connect_clicked(move |_| {
        let max = adj_click.upper() - adj_click.page_size();
        adj_click.set_value(step_target_right(adj_click.value(), &steps2, max));
    });

    let adj_click = adj;
    let steps2 = steps;
    left_btn.connect_clicked(move |_| {
        adj_click.set_value(step_target_left(adj_click.value(), &steps2));
    });

    vbox.upcast()
}

/// One clickable cover: the artwork (or the game name as fallback) with a
/// hover scale. `on_click` decides what clicking means — the desktop row
/// navigates to the game, the big-picture carousel selects and launches.
/// `square` marks a couch capsule: square frame, cover-fit (centered, the
/// overflow cropped), selection ring around the capsule instead of the art.
pub(super) fn build_cover(
    state: &SharedState,
    game: &Game,
    image_path: &str,
    w: i32,
    h: i32,
    square: bool,
    on_click: impl Fn(&SharedState) + 'static,
) -> gtk4::Widget {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_valign(gtk4::Align::Start);
    vbox.set_halign(gtk4::Align::Center);
    vbox.add_css_class(CSS_COVER_ITEM);
    vbox.set_size_request(w, h);
    vbox.set_overflow(gtk4::Overflow::Visible);
    if square {
        vbox.add_css_class(CSS_BP_SQ);
    }

    let overlay = gtk4::Overlay::new();
    overlay.set_overflow(gtk4::Overflow::Visible);

    let pic = gtk4::Picture::new();
    pic.set_content_fit(gtk4::ContentFit::Cover);
    pic.set_size_request(w, h);
    pic.add_css_class(CSS_GAME_COVER_PIC);
    if !image_path.is_empty() {
        if square {
            // Cover-fit: hand the Picture the raw texture so its
            // ContentFit::Cover scales it, centered, cropping the
            // overflow — the scaled paintable the desktop row uses would
            // stretch mismatched ratios instead.
            let pic_weak = pic.downgrade();
            let path = image_path.to_string();
            let set = move |texture: Option<gdk4::Texture>| {
                if let (Some(pic), Some(texture)) = (pic_weak.upgrade(), texture) {
                    pic.set_paintable(Some(&texture));
                }
            };
            match ira_images::cached_texture(image_path) {
                Some(texture) => set(Some(texture)),
                None => ira_images::load_texture_async_with_priority(
                    &path,
                    glib::Priority::DEFAULT,
                    set,
                ),
            }
        } else {
            queue_cover_load_priority(
                pic.clone(),
                image_path.to_string(),
                (w, h),
                game.db_id,
                game.variant_id.unwrap_or(0),
                vbox.clone(),
                glib::Priority::DEFAULT,
            );
        }
    }
    overlay.set_child(Some(&pic));

    if image_path.is_empty() {
        let name_label = gtk4::Label::new(Some(&game.name));
        name_label.set_wrap(true);
        name_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        name_label.set_max_width_chars(15);
        name_label.set_halign(gtk4::Align::Center);
        name_label.set_valign(gtk4::Align::Center);
        name_label.set_margin_start(6);
        name_label.set_margin_end(6);
        name_label.add_css_class(CSS_COVER_NAME_FALLBACK);
        overlay.add_overlay(&name_label);
    }

    vbox.append(&overlay);

    let click_state = state.clone();
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| on_click(&click_state));
    vbox.add_controller(click);

    let sc = state.clone();
    let gc = game.clone();
    let v_weak = vbox.downgrade();
    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    right_click.connect_pressed(move |_, _, x, y| {
        if let Some(v) = v_weak.upgrade() {
            show_game_context_menu(&sc, &gc, &v, x, y, None::<&gtk4::ListBoxRow>);
        }
    });
    vbox.add_controller(right_click);

    vbox.upcast()
}

fn update_scroll_buttons(adj: &gtk4::Adjustment, left: &gtk4::Button, right: &gtk4::Button) {
    let v = adj.value();
    left.set_sensitive(v > 0.5);
    let max = adj.upper() - adj.page_size();
    right.set_sensitive(v < max - 0.5);
}

fn step_target_right(cur: f64, steps: &[i32], max: f64) -> f64 {
    let mut boundary = 0;
    for step in steps {
        boundary += step;
        if (boundary as f64) > cur {
            return ((boundary as f64).min(max)).max(0.0);
        }
    }
    max
}

fn step_target_left(cur: f64, steps: &[i32]) -> f64 {
    let mut previous = 0;
    for step in steps {
        let boundary = previous + step;
        if (boundary as f64) >= cur {
            return previous as f64;
        }
        previous = boundary;
    }
    previous as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_target_right_boundaries() {
        let steps = &[220, 133, 133];
        assert_eq!(step_target_right(0.0, steps, 1000.0), 220.0);
        assert_eq!(step_target_right(220.0, steps, 1000.0), 353.0);
        assert_eq!(step_target_right(221.0, steps, 1000.0), 353.0);
        assert_eq!(step_target_right(2000.0, steps, 500.0), 500.0);
    }

    #[test]
    fn test_step_target_left_boundaries() {
        let steps = &[220, 133, 133];
        assert_eq!(step_target_left(0.0, steps), 0.0);
        assert_eq!(step_target_left(219.0, steps), 0.0);
        assert_eq!(step_target_left(220.0, steps), 0.0);
        assert_eq!(step_target_left(221.0, steps), 220.0);
        assert_eq!(step_target_left(353.0, steps), 220.0);
        assert_eq!(step_target_left(400.0, steps), 353.0);
    }
}
