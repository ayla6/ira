use super::super::css::*;
use super::types::{CardViews, SgdbPickerCtx};
use adw::prelude::*;
use ira_api::types::SgdbAsset;
use std::cell::RefCell;
use std::rc::Rc;

/// Tile pixel size for the zoom level: the slider names the tile width
/// and the picker's asset aspect sets the height, like game-grid tiles
/// growing with the viewport instead of fixed squares.
pub(super) fn tile_size(ctx: &SgdbPickerCtx) -> (i32, i32) {
    let w = ctx.zoom_level.get().clamp(100, 500);
    (
        w,
        (w as f64 / ctx.tile_aspect).round().max(1.0) as i32,
    )
}

/// List-row thumbnail for a tile width: a third of the tile, floored so
/// narrow zooms stay tappable, shaped by the same aspect.
pub(super) fn list_thumb_size(tile_w: i32, aspect: f64) -> (i32, i32) {
    let w = (tile_w / 3).max(48);
    (w, (w as f64 / aspect).round().max(1.0) as i32)
}

/// Paint one card's texture at the tile size: sized paintables on both
/// views plus matching size requests, and the texture stored for zoom
/// rebuilds.
pub(super) fn paint_card(entry: &mut CardViews, texture: &gdk4::Texture, tile_w: i32, aspect: f64) {
    let tile_h = (tile_w as f64 / aspect).round().max(1.0) as i32;
    let (list_w, list_h) = list_thumb_size(tile_w, aspect);
    if let Some(pic) = entry.grid.upgrade() {
        pic.set_paintable(Some(&ira_images::ScaledPaintable::new(
            texture, tile_w, tile_h,
        )));
        pic.set_size_request(tile_w, tile_h);
    }
    if let Some(pic) = entry.list.upgrade() {
        pic.set_paintable(Some(&ira_images::ScaledPaintable::new(
            texture, list_w, list_h,
        )));
        pic.set_size_request(list_w, list_h);
    }
    entry.texture = Some(texture.clone());
}

pub(super) fn build_sgdb_asset_card(
    a: &SgdbAsset,
    ctx: &SgdbPickerCtx,
    on_download: Rc<dyn Fn()>,
    tile_w: i32,
    tile_h: i32,
    dimmed: bool,
) -> (gtk4::Widget, gtk4::Widget) {
    let mut info = String::new();
    if a.width > 0 && a.height > 0 {
        info = format!("{}\u{d7}{}", a.width, a.height);
    }
    if !a.style.is_empty() {
        if !info.is_empty() {
            info = format!("{} \u{b7} {}", info, a.style);
        } else {
            info = a.style.clone();
        }
    }
    if !a.author.is_empty() {
        if !info.is_empty() {
            info = format!("{} \u{b7} by {}", info, a.author);
        } else {
            info = format!("by {}", a.author);
        }
    }

    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    card.set_halign(gtk4::Align::Center);
    card.set_valign(gtk4::Align::Start);
    // No outer margins: row/column spacing alone sets the rhythm (like
    // the game grid), so gaps can't read uneven next to the tight
    // in-card spacing.

    // Thumbnail with the author eye button overlaid top-right; the button
    // is revealed by hovering anywhere on the card (see the sgdb-filter
    // CSS rules).
    let thumb_overlay = gtk4::Overlay::new();
    let grid_pic = gtk4::Picture::new();
    // Sized paintables, not sized Pictures: a Picture reports its
    // texture's natural size and ignores the request, so full-size art
    // blows out the tile, the columns and the window minimum. The
    // ScaledPaintable reports the tile instead and draws into it.
    grid_pic.set_content_fit(gtk4::ContentFit::Contain);
    grid_pic.set_size_request(tile_w, tile_h);
    grid_pic.set_paintable(Some(&ira_images::ScaledPaintable::new_empty(
        tile_w, tile_h,
    )));
    thumb_overlay.set_child(Some(&grid_pic));
    if let Some(btn) = build_filter_button(ctx, a) {
        btn.set_halign(gtk4::Align::End);
        btn.set_valign(gtk4::Align::Start);
        btn.set_margin_top(6);
        btn.set_margin_end(6);
        thumb_overlay.add_overlay(&btn);
        attach_hover_reveal(&card, &btn);
    }
    card.append(&thumb_overlay);

    let ilbl = gtk4::Label::new(Some(&info));
    ilbl.set_xalign(0.5);
    // Wrap at the tile width: a fixed char cap is wider than narrow
    // tiles and stretches them, while an unconstrained label takes the
    // full unwrapped line width.
    ilbl.set_size_request(tile_w, -1);
    ilbl.set_wrap(true);
    ilbl.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    // Fixed two lines: wrapping to 1-3 lines per card ragged-rows the
    // whole flow at small zooms. Overflow ellipsizes on the second line.
    ilbl.set_lines(2);
    ilbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    ilbl.add_css_class(CSS_DIM_LABEL);
    card.append(&ilbl);

    let gdl = gtk4::Button::with_label(&crate::tr!("Download"));
    gdl.add_css_class(CSS_SUGGESTED_ACTION);
    gdl.set_hexpand(true);
    card.append(&gdl);

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.set_margin_top(4);
    row.set_margin_bottom(4);

    let list_pic = gtk4::Picture::new();
    list_pic.set_content_fit(gtk4::ContentFit::Contain);
    let (list_w, list_h) = list_thumb_size(tile_w, ctx.tile_aspect);
    list_pic.set_size_request(list_w, list_h);
    list_pic.set_paintable(Some(&ira_images::ScaledPaintable::new_empty(
        list_w, list_h,
    )));
    row.append(&list_pic);

    let rlbl = gtk4::Label::new(Some(&info));
    rlbl.set_xalign(0.0);
    rlbl.set_hexpand(true);
    rlbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    row.append(&rlbl);

    if let Some(btn) = build_filter_button(ctx, a) {
        row.append(&btn);
        attach_hover_reveal(&row, &btn);
    }
    let ldl = gtk4::Button::with_label(&crate::tr!("Download"));
    ldl.add_css_class(CSS_SUGGESTED_ACTION);
    row.append(&ldl);

    all_buttons_push(ctx, &gdl);
    all_buttons_push(ctx, &ldl);

    let cb_g = on_download.clone();
    let buttons_g = ctx.all_buttons.clone();
    gdl.connect_clicked(move |_| {
        for b in buttons_g.borrow().iter() {
            if let Some(b) = b.upgrade() {
                b.set_sensitive(false);
                b.set_label(&crate::tr!("Downloading\u{2026}"));
            }
        }
        cb_g();
    });
    let buttons_l = ctx.all_buttons.clone();
    ldl.connect_clicked(move |_| {
        for b in buttons_l.borrow().iter() {
            if let Some(b) = b.upgrade() {
                b.set_sensitive(false);
                b.set_label(&crate::tr!("Downloading\u{2026}"));
            }
        }
        on_download();
    });

    let thumb_url = if a.thumb.is_empty() {
        a.url.clone()
    } else {
        a.thumb.clone()
    };
    let steam_thumb = ctx.steam.clone();
    let (tx_thumb, rx_thumb) = std::sync::mpsc::channel::<Option<Vec<u8>>>();
    std::thread::spawn(move || {
        let _s = tracing::info_span!("thumbnail_download", url = %thumb_url).entered();
        let bytes = steam_thumb
            .download_bytes(&thumb_url)
            .ok()
            .filter(|b| !b.is_empty());
        let _ = tx_thumb.send(bytes);
    });
    let tp_g = grid_pic.downgrade();
    let tp_l = list_pic.downgrade();
    let tp_lbl = ilbl.downgrade();
    {
        let mut pics = ctx.pics.borrow_mut();
        pics.push(CardViews {
            grid: tp_g,
            list: tp_l,
            label: tp_lbl,
            texture: None,
            rx: RefCell::new(Some(rx_thumb)),
        });
    }

    // Wrapped and tagged by author and style so a filter click can dim
    // exactly these widgets in place without rebuilding anything, and
    // an unfilter click can re-evaluate each card (art that is still
    // style-filtered stays dim). Split with tag_author/tag_style.
    let flow_child = gtk4::FlowBoxChild::new();
    flow_child.set_widget_name(&widget_tag(&a.author, &a.style));
    flow_child.set_child(Some(&card));
    if dimmed {
        flow_child.add_css_class(CSS_SGDB_DIMMED);
    }
    let list_row = gtk4::ListBoxRow::new();
    list_row.set_widget_name(&widget_tag(&a.author, &a.style));
    list_row.set_child(Some(&row));
    if dimmed {
        list_row.add_css_class(CSS_SGDB_DIMMED);
    }
    (
        flow_child.upcast::<gtk4::Widget>(),
        list_row.upcast::<gtk4::Widget>(),
    )
}

/// Tag pairing an asset's author with its style on its card and row
/// widgets: author names alone can't re-evaluate one card's dim state,
/// and no widget↔asset map is kept (see [`tag_author`], [`tag_style`]).
fn widget_tag(author: &str, style: &str) -> String {
    format!("{author}\u{1f}{style}")
}

/// The author half of a [`widget_tag`]; the pluck match, kept exact like
/// the previous bare-name comparison.
pub(super) fn tag_author(tag: &str) -> &str {
    tag.split('\u{1f}').next().unwrap_or(tag)
}

/// The style half of a [`widget_tag`]; empty when the asset has no style.
pub(super) fn tag_style(tag: &str) -> &str {
    tag.split('\u{1f}').nth(1).unwrap_or("")
}

/// The author eye button, overlaid on thumbnails and revealed on hover
/// (rows included): a crossed eye to filter a shown author, the open eye
/// to unfilter an already-filtered one. Both follow the theme text color.
fn build_filter_button(ctx: &SgdbPickerCtx, a: &SgdbAsset) -> Option<gtk4::Button> {
    if a.author.is_empty() || ctx.filter_user.is_none() {
        return None;
    }
    let btn = if ctx.steam.user_filtered(&a.author) {
        let btn = gtk4::Button::from_icon_name("eye-open-negative-filled-symbolic");
        btn.set_tooltip_text(Some(&crate::tr!("Show art by this author")));
        btn
    } else {
        let btn = gtk4::Button::from_icon_name("eye-not-looking-symbolic");
        btn.set_tooltip_text(Some(&crate::tr!("Hide art by this author")));
        btn
    };
    btn.add_css_class(CSS_FLAT);
    btn.add_css_class(CSS_SGDB_FILTER);
    let ctx = ctx.clone();
    let author = a.author.clone();
    let steam64 = a.author_steam64.clone();
    btn.connect_clicked(move |_| ctx.toggle_author(&author, &steam64));
    Some(btn)
}

/// The first descendant carrying the filter-button class, if any: cards
/// hold two buttons (filter eye, Download) and rows do too, so the class
/// is the only reliable marker.
pub(super) fn find_filter_button(widget: &gtk4::Widget) -> Option<gtk4::Button> {
    let mut child = widget.first_child();
    while let Some(c) = child {
        child = c.next_sibling();
        if let Some(btn) = c.downcast_ref::<gtk4::Button>() {
            if btn.has_css_class(CSS_SGDB_FILTER) {
                return Some(btn.clone());
            }
        }
        if let Some(found) = find_filter_button(&c) {
            return Some(found);
        }
    }
    None
}

fn all_buttons_push(ctx: &SgdbPickerCtx, btn: &gtk4::Button) {
    ctx.all_buttons.borrow_mut().push(btn.downgrade());
}

/// Reveal `target` while the pointer is anywhere over `area` (the whole
/// card or row), not just over the button itself — CSS :hover on ancestor
/// boxes is not dependable, so track motion explicitly.
fn attach_hover_reveal<T: IsA<gtk4::Widget>, U: IsA<gtk4::Widget>>(
    area: &T,
    target: &U,
) {
    let target = target.downgrade();
    let ctrl = gtk4::EventControllerMotion::new();
    ctrl.connect_enter({
        let target = target.clone();
        move |_, _, _| {
            if let Some(t) = target.upgrade() {
                t.add_css_class(CSS_SGDB_REVEAL);
            }
        }
    });
    ctrl.connect_leave(move |_| {
        if let Some(t) = target.upgrade() {
            t.remove_css_class(CSS_SGDB_REVEAL);
        }
    });
    area.add_controller(ctrl);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_widget_tag_roundtrips_author_and_style() {
        let tag = widget_tag("Some Artist", "blurred");
        assert_eq!(tag_author(&tag), "Some Artist");
        assert_eq!(tag_style(&tag), "blurred");
    }

    #[test]
    fn test_tag_style_empty_without_style() {
        let tag = widget_tag("Some Artist", "");
        assert_eq!(tag_author(&tag), "Some Artist");
        assert_eq!(tag_style(&tag), "");
    }
}
