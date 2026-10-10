use super::context_menu::show_game_context_menu;
use super::css::*;
use super::message_helpers::switch_to_game;
use super::recent_carousel::RecentRow;
use super::state::SharedState;
use super::virtual_grid::VirtualGrid;
use crate::Game;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

const COVER_KEY_DATA: &str = "ira-cover-key";
const COVER_ART_DATA: &str = "ira-cover-art";
const COVER_PRESENTATION_DATA: &str = "ira-cover-presentation";
const COVER_NAME_DATA: &str = "ira-cover-name";

pub(super) fn build_recent_row(
    state: &SharedState,
    recent: &[Game],
    cover_height: i32,
) -> gtk4::Widget {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);

    let (title_row, left_btn, right_btn) = build_recent_title_row();
    vbox.append(&title_row);

    let spacing = VirtualGrid::grid_spacing_for_item_w((cover_height as f64 * 2.0 / 3.0) as i32);
    let row = RecentRow::new(spacing, cover_height);
    let game_widths = append_recent_covers(state, &row, recent, cover_height);

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

    wire_recent_scroll(&scrolled, &left_btn, &right_btn, spacing, &game_widths);

    vbox.upcast()
}

fn build_recent_title_row() -> (gtk4::Box, gtk4::Button, gtk4::Button) {
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
    (title_row, left_btn, right_btn)
}

fn append_recent_covers(
    state: &SharedState,
    row: &RecentRow,
    recent: &[Game],
    cover_height: i32,
) -> Vec<i32> {
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
    game_widths
}

fn wire_recent_scroll(
    scrolled: &gtk4::ScrolledWindow,
    left_btn: &gtk4::Button,
    right_btn: &gtk4::Button,
    spacing: i32,
    game_widths: &[i32],
) {
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
}

/// One clickable cover: the artwork (or the game name as fallback) with a
/// hover scale. `on_click` decides what clicking means — the desktop row
/// navigates to the game, the big-picture carousel selects and launches.
/// `square` marks a big-picture capsule: square frame, cover-fit (centered, the
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
    // Any art whose ratio differs from the slot measures taller than its
    // allocation without this (see the big-picture grid cells) — the
    // homogeneous tab stack measures even hidden pages, so one unshrunk
    // cover spams layout warnings for the whole window.
    pic.set_can_shrink(true);
    pic.add_css_class(CSS_GAME_COVER_PIC);
    overlay.set_child(Some(&pic));

    let name_label = gtk4::Label::new(Some(&game.name));
    name_label.set_wrap(true);
    name_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    name_label.set_max_width_chars(15);
    name_label.set_halign(gtk4::Align::Center);
    name_label.set_valign(gtk4::Align::Center);
    name_label.set_margin_start(6);
    name_label.set_margin_end(6);
    name_label.add_css_class(CSS_COVER_NAME_FALLBACK);
    name_label.set_visible(image_path.is_empty());
    overlay.add_overlay(&name_label);

    vbox.append(&overlay);
    unsafe {
        vbox.set_data::<RefCell<String>>(COVER_KEY_DATA, RefCell::new(game.grid_id()));
        vbox.set_data::<RefCell<String>>(COVER_ART_DATA, RefCell::new(image_path.to_string()));
        vbox.set_data::<RefCell<String>>(
            COVER_PRESENTATION_DATA,
            RefCell::new(cover_presentation(square, w, h)),
        );
        vbox.set_data::<gtk4::Label>(COVER_NAME_DATA, name_label.clone());
    }
    set_cover_art(&pic, &vbox, image_path, w, h, square, true);

    let click_state = state.clone();
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| on_click(&click_state));
    vbox.add_controller(click);

    let sc = state.clone();
    let game_key = game.grid_id();
    let fallback_game = game.clone();
    let v_weak = vbox.downgrade();
    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    right_click.connect_pressed(move |_, _, x, y| {
        if let Some(v) = v_weak.upgrade() {
            let game = sc
                .borrow()
                .games
                .iter()
                .find(|game| game.grid_id() == game_key)
                .cloned()
                .unwrap_or_else(|| fallback_game.clone());
            show_game_context_menu(&sc, &game, &v, x, y, None::<&gtk4::ListBoxRow>);
        }
    });
    vbox.add_controller(right_click);

    vbox.upcast()
}

/// Update a retained cover without replacing its widget or current paintable.
/// Keeping old art visible while a changed path decodes avoids a blank frame;
/// callbacks validate the current key and path before painting.
pub(super) fn update_cover(
    widget: &gtk4::Widget,
    game: &Game,
    image_path: &str,
    w: i32,
    h: i32,
    square: bool,
) {
    let Some(vbox) = widget.downcast_ref::<gtk4::Box>() else {
        return;
    };
    let Some(overlay) = vbox
        .first_child()
        .and_then(|child| child.downcast::<gtk4::Overlay>().ok())
    else {
        return;
    };
    let Some(pic) = overlay
        .child()
        .and_then(|child| child.downcast::<gtk4::Picture>().ok())
    else {
        return;
    };
    let old_key = cover_data(vbox, COVER_KEY_DATA);
    let old_art = cover_data(vbox, COVER_ART_DATA);
    let old_presentation = cover_data(vbox, COVER_PRESENTATION_DATA);
    let key = game.grid_id();
    let key_changed = old_key != key;
    let presentation = cover_presentation(square, w, h);
    let changed = old_key != key || old_art != image_path || old_presentation != presentation;

    vbox.set_size_request(w, h);
    pic.set_size_request(w, h);
    if square {
        vbox.add_css_class(CSS_BP_SQ);
    } else {
        vbox.remove_css_class(CSS_BP_SQ);
    }
    if let Some(name_label) = unsafe { vbox.data::<gtk4::Label>(COVER_NAME_DATA) }
        .map(|ptr| unsafe { ptr.as_ref() }.clone())
    {
        name_label.set_text(&game.name);
        name_label.set_visible(image_path.is_empty());
    }
    set_cover_data(vbox, COVER_KEY_DATA, key.clone());
    set_cover_data(vbox, COVER_ART_DATA, image_path.to_string());
    set_cover_data(vbox, COVER_PRESENTATION_DATA, presentation);
    if changed {
        set_cover_art(
            &pic,
            vbox,
            image_path,
            w,
            h,
            square,
            key_changed || image_path.is_empty(),
        );
    }
}

fn set_cover_data(vbox: &gtk4::Box, name: &str, value: String) {
    if let Some(ptr) = unsafe { vbox.data::<RefCell<String>>(name) } {
        unsafe { ptr.as_ref() }.replace(value);
    }
}

fn cover_data(vbox: &gtk4::Box, name: &str) -> String {
    unsafe { vbox.data::<RefCell<String>>(name) }
        .map(|ptr| unsafe { ptr.as_ref() }.borrow().clone())
        .unwrap_or_default()
}

fn cover_is_current(vbox: &gtk4::Box, key: &str, art: &str, presentation: &str) -> bool {
    cover_data(vbox, COVER_KEY_DATA) == key
        && cover_data(vbox, COVER_ART_DATA) == art
        && cover_data(vbox, COVER_PRESENTATION_DATA) == presentation
}

fn cover_presentation(square: bool, w: i32, h: i32) -> String {
    format!("{square}:{w}:{h}")
}

fn set_cover_art(
    pic: &gtk4::Picture,
    vbox: &gtk4::Box,
    path: &str,
    w: i32,
    h: i32,
    square: bool,
    clear_first: bool,
) {
    if clear_first {
        let empty = ira_images::ScaledPaintable::new_empty(w, h);
        pic.set_paintable(Some(&empty));
    }
    if path.is_empty() {
        return;
    }

    let key = cover_data(vbox, COVER_KEY_DATA);
    let path_owned = path.to_string();
    let presentation = cover_data(vbox, COVER_PRESENTATION_DATA);
    let pic_weak = pic.downgrade();
    let vbox_weak = vbox.downgrade();
    let set = move |texture: Option<gdk4::Texture>| {
        let Some(pic) = pic_weak.upgrade() else {
            return;
        };
        let Some(vbox) = vbox_weak.upgrade() else {
            return;
        };
        if !cover_is_current(&vbox, &key, &path_owned, &presentation) {
            return;
        }
        match texture {
            Some(texture) if square => pic.set_paintable(Some(&texture)),
            Some(texture) => {
                let paintable = ira_images::ScaledPaintable::new(&texture, w, h);
                pic.set_paintable(Some(&paintable));
            }
            None => {
                pic.set_paintable(None::<&gdk4::Paintable>);
                if let Some(ptr) = unsafe { vbox.data::<gtk4::Label>(COVER_NAME_DATA) } {
                    unsafe { ptr.as_ref() }.set_visible(true);
                }
            }
        }
    };
    match ira_images::cached_texture(path) {
        Some(texture) => set(Some(texture)),
        None => ira_images::load_texture_async_with_priority(
            path,
            glib::Priority::DEFAULT,
            set,
        ),
    }
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
