use super::css::*;
use super::helpers::clear_children;
use super::state::{PendingImage, SgdbAssetsCacheEntry};
use adw::prelude::*;
use ira_api::types::SgdbAsset;
use ira_api::SteamDataClient;
use ira_models::AssetType;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::rc::Weak;
use std::sync::Arc;

/// Tile pixel size for the zoom level: the slider names the tile width
/// and the picker's asset aspect sets the height, like game-grid tiles
/// growing with the viewport instead of fixed squares.
fn tile_size(ctx: &SgdbPickerCtx) -> (i32, i32) {
    let w = ctx.zoom_level.get().clamp(100, 500);
    (w, (w as f64 / ctx.tile_aspect).round().max(1.0) as i32)
}

/// List-row thumbnail for a tile width: a third of the tile, floored so
/// narrow zooms stay tappable, shaped by the same aspect.
fn list_thumb_size(tile_w: i32, aspect: f64) -> (i32, i32) {
    let w = (tile_w / 3).max(48);
    (w, (w as f64 / aspect).round().max(1.0) as i32)
}

/// Paint one card's texture at the tile size: sized paintables on both
/// views plus matching size requests, and the texture stored for zoom
/// rebuilds.
fn paint_card(entry: &mut CardViews, texture: &gdk4::Texture, tile_w: i32, aspect: f64) {
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

fn build_sgdb_asset_card(
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
fn tag_author(tag: &str) -> &str {
    tag.split('\u{1f}').next().unwrap_or(tag)
}

/// The style half of a [`widget_tag`]; empty when the asset has no style.
fn tag_style(tag: &str) -> &str {
    tag.split('\u{1f}').nth(1).unwrap_or("")
}

/// True when the card/row widget's tagged author/style is filtered.
fn card_dimmed(ctx: &SgdbPickerCtx, widget: &gtk4::Widget) -> bool {
    let name = widget.widget_name();
    ctx.steam
        .asset_filtered(tag_author(&name), tag_style(&name))
}

/// Index of the first dimmed child, if any.
fn first_dimmed_pos(container: &impl IsA<gtk4::Widget>, ctx: &SgdbPickerCtx) -> Option<i32> {
    let mut index = 0;
    let mut child = container.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        if card_dimmed(ctx, &w) {
            return Some(index);
        }
        index += 1;
    }
    None
}

/// Detach dimmed children, in order, for re-appending after new cards.
fn detach_dimmed(ctx: &SgdbPickerCtx, container: &impl IsA<gtk4::Widget>) -> Vec<gtk4::Widget> {
    let mut out = Vec::new();
    let mut child = container.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        if card_dimmed(ctx, &w) {
            out.push(w);
        }
    }
    out
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
fn find_filter_button(widget: &gtk4::Widget) -> Option<gtk4::Button> {
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

/// One card's views plus its downloaded texture. Zoom rebuilds the
/// paintables from the stored texture at the new tile size — no
/// re-download — and arrivals size the first paintable from the live
/// zoom level. The thumbnail receiver lives here too: one shared
/// ticker polls every card instead of one timeout per card saturating
/// the main loop.
struct CardViews {
    grid: glib::WeakRef<gtk4::Picture>,
    list: glib::WeakRef<gtk4::Picture>,
    label: glib::WeakRef<gtk4::Label>,
    texture: Option<gdk4::Texture>,
    rx: RefCell<Option<std::sync::mpsc::Receiver<Option<Vec<u8>>>>>,
}

#[derive(Clone)]
struct SgdbPickerCtx {
    id: String,
    save_dir: String,
    steam: Arc<SteamDataClient>,
    asset: String,
    is_steam_id: bool,
    // Weak: ctx clones are owned by closures attached to the picker's own
    // widgets, and the settings-screen cache owns the dialog strongly, so a
    // strong ref here would make dialog -> closure -> dialog a cycle.
    picker: glib::WeakRef<adw::Window>,
    on_done: Rc<dyn Fn()>,
    pending_copies: Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
    dest_dir: Option<String>,
    // Live view state, so asset cards can react to clicks without closures
    // strongly owning the widgets they render into.
    store: Rc<RefCell<Vec<SgdbAsset>>>,
    rendered: Rc<Cell<usize>>,
    zoom_level: Rc<Cell<i32>>,
    /// Uniform tile aspect for this picker's asset type (width over
    /// height): every card takes the type's shape like game-grid tiles,
    /// instead of everything squeezing into squares.
    tile_aspect: f64,
    /// Per-card views in render order; zoom resizes them in place.
    pics: Rc<RefCell<Vec<CardViews>>>,
    all_buttons: Rc<RefCell<Vec<glib::WeakRef<gtk4::Button>>>>,
    flow: glib::WeakRef<gtk4::FlowBox>,
    list_view: glib::WeakRef<gtk4::ListBox>,
    // Persists an author (name, steam64) to the config's filtered-users
    // list; None when the caller has no config to persist to.
    filter_user: Option<Rc<dyn Fn(String, String)>>,
    // Removes an author from the filtered-users list; the eye button's
    // unfilter half. Paired with filter_user above.
    unfilter_user: Option<Rc<dyn Fn(String)>>,
}

impl SgdbPickerCtx {
    /// Re-render every asset currently in the store.
    fn rebuild_all(&self) {
        let (Some(flow), Some(list_view)) = (self.flow.upgrade(), self.list_view.upgrade())
        else {
            return;
        };
        let assets = self.store.borrow();
        full_rebuild(&flow, &list_view, &assets, self);
        self.rendered.set(assets.len());
    }

    /// Render assets fetched since the last render (infinite scroll).
    fn append_new(&self) {
        let start = self.rendered.get();
        if start >= self.store.borrow().len() {
            return;
        }
        let (Some(flow), Some(list_view)) = (self.flow.upgrade(), self.list_view.upgrade())
        else {
            return;
        };
        let assets = self.store.borrow();
        append_assets(&flow, &list_view, &assets, start, self);
        self.rendered.set(assets.len());
    }

    /// Reposition one author's cards after a filter toggle: dimmed to
    /// the end, visible just before the first dimmed — the grid never
    /// stripes visible/dimmed/visible.
    fn settle_author_cards(&self, author: &str) {
        let (Some(flow), Some(list)) = (self.flow.upgrade(), self.list_view.upgrade())
        else {
            return;
        };
        settle_flow_author(self, &flow, author);
        settle_list_author(self, &list, author);
    }

    /// The eye button's click: filter a shown author, unfilter an
    /// already-filtered one. The state is read live so every one of the
    /// author's cards agrees no matter which card was clicked.
    fn toggle_author(&self, author: &str, steam64: &str) {
        if self.steam.user_filtered(author) {
            self.unfilter_author(author);
        } else {
            self.filter_author(author, steam64);
        }
    }

    /// Run `f` over every card and row tag belonging to `author`, in
    /// place — no rebuild, so loaded thumbnails and the scroll position
    /// stay untouched. Shared by the filter and unfilter halves.
    fn for_each_author_card(&self, author: &str, mut f: impl FnMut(&gtk4::Widget)) {
        let (Some(flow), Some(list_view)) = (self.flow.upgrade(), self.list_view.upgrade())
        else {
            return;
        };
        for container in [&flow.upcast::<gtk4::Widget>(), &list_view.upcast()] {
            let mut child = container.first_child();
            while let Some(w) = child {
                child = w.next_sibling();
                if tag_author(&w.widget_name()) == author {
                    f(&w);
                }
            }
        }
    }

    /// Dim one author's art in place: persist the choice, then dim
    /// exactly that author's cards and rows where they stand and flip
    /// their eye buttons to the unfilter state — no rebuild, no reorder,
    /// so loaded thumbnails and the scroll position stay untouched and
    /// the art stays visible, just transparent. The store keeps its
    /// order; later pages render dimmed through the normal path.
    fn filter_author(&self, author: &str, steam64: &str) {
        if let Some(persist) = &self.filter_user {
            persist(author.to_string(), steam64.to_string());
        }
        self.for_each_author_card(author, |w| self.dim_card(w));
        self.settle_author_cards(author);
    }

    /// Dim one card-or-row and flip its eye button to the unfilter
    /// state: the mirror of [`Self::undim_card`] on the same widgets.
    fn dim_card(&self, widget: &gtk4::Widget) {
        widget.add_css_class(CSS_SGDB_DIMMED);
        if let Some(btn) = find_filter_button(widget) {
            btn.set_icon_name("eye-open-negative-filled-symbolic");
            btn.set_tooltip_text(Some(&crate::tr!("Show art by this author")));
        }
    }

    /// Show one author's art again: persist the removal, then undim exactly
    /// that author's cards and rows in place and flip their eye buttons
    /// back to the hide state — no rebuild, so loaded thumbnails and the
    /// scroll position stay untouched. Cards whose style is still filtered
    /// keep their dim (re-checked per card via its tag).
    fn unfilter_author(&self, author: &str) {
        if let Some(persist) = &self.unfilter_user {
            persist(author.to_string());
        }
        self.for_each_author_card(author, |w| self.undim_card(w));
        self.settle_author_cards(author);
    }

    /// Drop one card-or-row's dim when its style is no longer filtered and
    /// flip its eye button back to hide. The author half just left the
    /// filtered-users list, so only the tag's style half still matters.
    fn undim_card(&self, widget: &gtk4::Widget) {
        if !self.steam.style_filtered(tag_style(&widget.widget_name())) {
            widget.remove_css_class(CSS_SGDB_DIMMED);
        }
        if let Some(btn) = find_filter_button(widget) {
            btn.set_icon_name("eye-not-looking-symbolic");
            btn.set_tooltip_text(Some(&crate::tr!("Hide art by this author")));
        }
    }
}

/// Settle one author's flow children: detach, then reinsert dimmed at
/// the end and visible before the first dimmed, all order-preserved.
fn settle_flow_author(ctx: &SgdbPickerCtx, flow: &gtk4::FlowBox, author: &str) {
    let mut mine = Vec::new();
    let mut child = flow.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        if tag_author(&w.widget_name()) == author {
            mine.push(w);
        }
    }
    for w in &mine {
        flow.remove(w);
    }
    let mut pos = first_dimmed_pos(flow, ctx);
    for w in &mine {
        if card_dimmed(ctx, w) {
            flow.append(w);
        } else if let Some(p) = pos {
            flow.insert(w, p);
            pos = Some(p + 1);
        } else {
            flow.append(w);
        }
    }
}

/// Settle one author's list rows, mirroring [`settle_flow_author`].
fn settle_list_author(ctx: &SgdbPickerCtx, list: &gtk4::ListBox, author: &str) {
    let mut mine = Vec::new();
    let mut child = list.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        if tag_author(&w.widget_name()) == author {
            mine.push(w);
        }
    }
    for w in &mine {
        list.remove(w);
    }
    let mut pos = first_dimmed_pos(list, ctx);
    for w in &mine {
        if card_dimmed(ctx, w) {
            list.append(w);
        } else if let Some(p) = pos {
            list.insert(w, p);
            pos = Some(p + 1);
        } else {
            list.append(w);
        }
    }
}

fn build_on_download(ctx: &SgdbPickerCtx, a: &SgdbAsset) -> Rc<dyn Fn()> {
    let dest_dir = ctx.dest_dir.clone().unwrap_or_else(|| {
        let data_subdir = if ctx.is_steam_id {
            "steam".to_string()
        } else {
            "steamgriddb".to_string()
        };
        format!("{}/data/{}/{}", ctx.save_dir, data_subdir, ctx.id)
    });
    let Some(at) = AssetType::from_string(&ctx.asset) else {
        return Rc::new(|| {});
    };
    let file_name = match at {
        AssetType::Icon => {
            let ext = if a.mime.contains("icon") || a.mime.contains("x-icon") {
                "ico"
            } else if a.mime.contains("png") {
                "png"
            } else if a.mime.contains("jpeg") || a.mime.contains("jpg") {
                "jpg"
            } else if a.mime.contains("webp") {
                "webp"
            } else {
                std::path::Path::new(&a.url)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("png")
            };
            format!("{}.{}", at.file_base(), ext)
        }
        AssetType::Hero => format!("{}.jpg", at.file_base()),
        AssetType::Grid => format!("{}.jpg", at.file_base()),
        AssetType::Header => format!("{}.jpg", at.file_base()),
        AssetType::Logo => format!("{}.png", at.file_base()),
        AssetType::Square => format!("{}.png", at.file_base()),
    };
    let dl_url = a.url.clone();
    let steam_dl = ctx.steam.clone();
    let picker_dl = ctx.picker.clone();
    let on_done_dl = ctx.on_done.clone();
    let asset_dl = ctx.asset.clone();
    let pending_dl = ctx.pending_copies.clone();

    Rc::new(move || {
        let _s = tracing::info_span!("on_download", asset = %asset_dl, url = %dl_url).entered();
        if let Some(ref pc) = pending_dl {
            let (tx, rx) = std::sync::mpsc::channel::<(Vec<u8>, bool)>();
            let rx = std::cell::RefCell::new(rx);
            let url = dl_url.clone();
            let steam = steam_dl.clone();
            std::thread::spawn(move || {
                let _s = tracing::info_span!("download_and_convert", url = %url).entered();
                match steam.download_bytes(&url) {
                    Ok(bytes) if !bytes.is_empty() => {
                        let _ = tx.send((bytes.clone(), false));
                        let converted = ira_parser::convert_bytes_to_lossless_webp(&bytes);
                        let _ = tx.send((converted.unwrap_or_else(|| bytes.clone()), true));
                    }
                    _ => {
                        eprintln!("Download failed for {}", url);
                    }
                }
            });
            let pc_c = pc.clone();
            let on_done_c = on_done_dl.clone();
            let picker_weak = picker_dl.clone();
            let asset_c = asset_dl.clone();
            let done = Rc::new(Cell::new(false));
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if picker_weak.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }
                loop {
                    match rx.borrow_mut().try_recv() {
                        Ok((bytes, is_converted)) => {
                            pc_c.borrow_mut().insert(
                                asset_c.clone(),
                                PendingImage::Bytes(gtk4::glib::Bytes::from_owned(bytes)),
                            );
                            if !is_converted {
                                on_done_c();
                                if let Some(picker) = picker_weak.upgrade() {
                                    picker.close();
                                }
                            } else {
                                done.set(true);
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            done.set(true);
                            break;
                        }
                    }
                }
                if done.get() {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        } else {
            let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
            let rx = std::cell::RefCell::new(rx);
            let url = dl_url.clone();
            let steam = steam_dl.clone();
            let dest_dir_c = dest_dir.clone();
            let file_name_c = file_name.clone();
            let asset_at = AssetType::from_string(&asset_dl).unwrap_or(AssetType::Icon);
            std::thread::spawn(move || {
                let _s = tracing::info_span!("download_direct", url = %url).entered();
                // Byte-validated download: decode before anything touches
                // disk and persist WebP only, so a raw .ico can never land
                // in the game's image directory.
                let result = match steam.download_bytes(&url) {
                    Ok(bytes) if !bytes.is_empty() && ira_parser::is_decodable_image(&bytes) => {
                        let webp =
                            ira_parser::convert_bytes_to_lossless_webp(&bytes).unwrap_or(bytes);
                        let _ = std::fs::create_dir_all(&dest_dir_c);
                        // The pick replaces the slot's previous art: without
                        // this sweep an old file under another extension (or
                        // the previous `_small` thumbnail) survives and keeps
                        // the old picture on screen.
                        ira_parser::remove_image_variants(
                            std::path::Path::new(&dest_dir_c),
                            asset_at.file_base(),
                        );
                        let dest = std::path::Path::new(&dest_dir_c)
                            .join(&file_name_c)
                            .with_extension("webp");
                        match std::fs::write(&dest, &webp) {
                            Ok(()) => dest.to_string_lossy().into_owned(),
                            Err(error) => {
                                eprintln!("Failed to write {}: {error}", dest.display());
                                String::new()
                            }
                        }
                    }
                    Ok(_) => {
                        eprintln!("Download is not a decodable image: {url}");
                        String::new()
                    }
                    Err(error) => {
                        eprintln!("Download failed for {url}: {error}");
                        String::new()
                    }
                };
                if !result.is_empty() {
                    let (sw, sh) = asset_at.thumb_dims();
                    ira_parser::ensure_small_image(
                        std::path::Path::new(&dest_dir_c),
                        asset_at.file_base(),
                        sw,
                        sh,
                    );
                }
                let _ = tx.send(if result.is_empty() {
                    None
                } else {
                    Some(result)
                });
            });
            let on_done_c = on_done_dl.clone();
            let picker_weak = picker_dl.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if picker_weak.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }
                if let Ok(result) = rx.borrow_mut().try_recv() {
                    if result.is_some() {
                        on_done_c();
                        if let Some(picker) = picker_weak.upgrade() {
                            picker.close();
                        }
                    }
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }
    })
}

fn append_assets(
    flow: &gtk4::FlowBox,
    list_view: &gtk4::ListBox,
    assets: &[SgdbAsset],
    start: usize,
    ctx: &SgdbPickerCtx,
) {
    // No column clamp: the box wraps by width like the game grid, so the
    // column count follows the dialog size and the zoom level.
    // Dimmed cards always trail: partition the batch and park the
    // previously-settled dimmed tails behind it, so new sections never
    // stripe visible/dimmed/visible.
    let show_filtered = ctx.steam.sgdb_show_filtered();
    let (tile_w, tile_h) = tile_size(ctx);
    let mut new_visible = Vec::new();
    let mut new_dimmed = Vec::new();
    for a in &assets[start..] {
        let filtered = ctx.steam.asset_filtered(&a.author, &a.style);
        if filtered && !show_filtered {
            continue;
        }
        let on_download = build_on_download(ctx, a);
        let (grid_card, list_row) =
            build_sgdb_asset_card(a, ctx, on_download, tile_w, tile_h, filtered);
        if filtered {
            new_dimmed.push((grid_card, list_row));
        } else {
            new_visible.push((grid_card, list_row));
        }
    }
    let old_flow_dimmed = detach_dimmed(ctx, flow);
    for w in &old_flow_dimmed {
        flow.remove(w);
    }
    let old_list_dimmed = detach_dimmed(ctx, list_view);
    for w in &old_list_dimmed {
        list_view.remove(w);
    }
    for (grid_card, list_row) in &new_visible {
        flow.append(grid_card);
        list_view.append(list_row);
    }
    for (grid_card, list_row) in &new_dimmed {
        flow.append(grid_card);
        list_view.append(list_row);
    }
    for w in &old_flow_dimmed {
        flow.append(w);
    }
    for w in &old_list_dimmed {
        list_view.append(w);
    }
}

fn full_rebuild(
    flow: &gtk4::FlowBox,
    list_view: &gtk4::ListBox,
    assets: &[SgdbAsset],
    ctx: &SgdbPickerCtx,
) {
    clear_children(flow);
    clear_children(list_view);
    ctx.all_buttons.borrow_mut().clear();
    ctx.pics.borrow_mut().clear();

    if assets.is_empty() {
        let none = gtk4::Label::new(Some(&crate::tr!("No images found on SteamGridDB")));
        none.add_css_class(CSS_DIM_LABEL);
        flow.append(&none);
        list_view.append(&gtk4::Label::new(Some(&crate::tr!(
            "No images found on SteamGridDB"
        ))));
        return;
    }

    append_assets(flow, list_view, assets, 0, ctx);
}

/// Geometry sync callback (zoom, width, height).
type GeometrySync = Rc<dyn Fn(i32, i32, i32)>;

pub(crate) struct ShowSgdbPickerParams<'a> {
    pub steam: &'a Arc<SteamDataClient>,
    pub id: &'a str,
    pub asset: &'a str,
    pub is_steam_id: bool,
    pub dimensions: &'a [&'a str],
    pub parent: &'a adw::Window,
    pub on_done: Rc<dyn Fn()>,
    pub pending_copies: Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
    pub sgdb_cache: Option<Rc<RefCell<HashMap<String, SgdbAssetsCacheEntry>>>>,
    pub save_dir: &'a str,
    pub dest_dir: Option<&'a str>,
    /// Persists an author (name, steam64) to the filtered-users config;
    /// enables the per-item filter buttons when set.
    pub filter_user: Option<Rc<dyn Fn(String, String)>>,
    /// Removes an author from the filtered-users config; the eye button's
    /// unfilter half. Paired with filter_user above.
    pub unfilter_user: Option<Rc<dyn Fn(String)>>,
    /// Live geometry sync (zoom, width, height): keeps the in-memory
    /// config current so a later settings save can't clobber what the
    /// close-request persists to disk.
    pub on_geometry: Option<GeometrySync>,
}

pub fn show_sgdb_picker(params: ShowSgdbPickerParams) {
    let ShowSgdbPickerParams {
        steam,
        id,
        asset,
        is_steam_id,
        dimensions,
        parent,
        on_done,
        pending_copies,
        sgdb_cache,
        save_dir,
        dest_dir,
        filter_user,
        unfilter_user,
        on_geometry,
    } = params;
    // Key the settings-screen cache by the full SGDB query, not just the
    // asset type: a Steam game's picker runs against its Steam id until it's
    // matched to an SGDB id, and reusing one entry across both would serve
    // stale results (and resume pagination from the wrong stream).
    let cache_key = format!("{}|{}|{}|{}", id, is_steam_id, asset, dimensions.join(","));

    // If a picker dialog for this exact query is already registered (open,
    // or closed but still cache-owned), re-present it with its loaded
    // thumbnails and scroll intact instead of rebuilding everything and
    // refetching the list.
    let cached_picker = sgdb_cache
        .as_ref()
        .and_then(|c| c.borrow().get(&cache_key).map(|entry| entry.picker.clone()));
    if let Some(picker) = cached_picker {
        // No refit: the live window keeps whatever size it has, which is
        // the whole point of persisting it.
        picker.present();
        return;
    }

    // A real window, not a sheet: sheets cannot be resized by the user,
    // and this grid is meant to grow with its viewport like the game
    // grid does. Modal over its parent like the settings windows, so
    // focus and keys (Alt+F4 included) belong to it unambiguously; a
    // modeless window under a modal parent leaves focus delivery up to
    // the compositor. Still freely resizable; the settings teardown
    // closes cached pickers before dropping their cache.
    let persisted = ira_config::load_config();
    let zoom_initial = persisted
        .sgdb_picker_zoom
        .get(asset)
        .copied()
        .unwrap_or(300)
        .clamp(100, 500);
    let (size_w, size_h) = persisted
        .sgdb_picker_size
        .get(asset)
        .copied()
        .unwrap_or((900, 700));
    // Floor the restore: degenerate saved sizes (from older builds that
    // measured mid-teardown) would otherwise stick forever, and nothing
    // in the UI can even produce a legitimately smaller window.
    let size_w = size_w.max(640);
    let size_h = size_h.max(480);
    let picker = adw::Window::new();
    picker.set_title(Some(&crate::tr!("Pick {}").replacen("{}", asset, 1)));
    picker.set_modal(true);
    picker.set_transient_for(Some(parent));
    // Restore the saved size verbatim: it is user-produced on this
    // machine, so clamping it to the parent (like dialog fits do)
    // would shrink every reopen back to the settings size.
    picker.set_default_size(size_w, size_h);

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
    // columns follow the dialog and zoom like the game grid does.
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

    let loading_label = gtk4::Label::new(Some(&crate::tr!("Loading\u{2026}")));
    loading_label.add_css_class(CSS_DIM_LABEL);
    flow.append(&loading_label);
    let loading_label2 = gtk4::Label::new(Some(&crate::tr!("Loading\u{2026}")));
    loading_label2.add_css_class(CSS_DIM_LABEL);
    list_view.append(&loading_label2);

    scrolled.set_child(Some(&stack));
    outer.append(&scrolled);

    picker.set_content(Some(&outer));
    picker.present();

    // Closing an AdwDialog only unmaps it; the cache entry registered below
    // is what owns the dialog from here on. Without that strong ref the
    // dialog would be finalized on close, because every handler holds it
    // weakly and this function's own ref dies with the call frame.

    // Register the live dialog in the settings-screen cache immediately so a
    // reopen (even before the first fetch lands) can re-present it instead of
    // building a fresh one. The fetch handler updates this entry in place.
    if let Some(ref cache) = sgdb_cache {
        cache.borrow_mut().insert(
            cache_key.clone(),
            SgdbAssetsCacheEntry {
                assets: Vec::new(),
                has_more: true,
                next_page: 0,
                picker: picker.clone(),
            },
        );
    }

    let assets_store: Rc<RefCell<Vec<SgdbAsset>>> = Rc::new(RefCell::new(Vec::new()));
    let zoom_level = Rc::new(Cell::new(zoom_initial));
    // Closing — header X, Alt+F4, or the download path closing the
    // window — persists zoom and size for the next fresh open. Reopens
    // from the cache keep the live window trivially.
    {
        let geo_asset = asset.to_string();
        let geo_zoom = zoom_level.clone();
        let geo_sync = on_geometry.clone();
        picker.connect_close_request(move |win| {
            // Record geometry only from a mapped window: teardown and the
            // download path also close already-hidden pickers, whose size
            // reads are meaningless and would overwrite real values.
            if win.is_visible() {
                let zoom = geo_zoom.get();
                let (w, h) = (win.width(), win.height());
                if let Some(ref sync) = geo_sync {
                    sync(zoom, w, h);
                }
                let mut cfg = ira_config::load_config();
                cfg.sgdb_picker_zoom.insert(geo_asset.clone(), zoom);
                cfg.sgdb_picker_size.insert(geo_asset.clone(), (w, h));
                if let Err(e) = cfg.save() {
                    eprintln!("Failed to save picker geometry: {e}");
                }
            }
            // Stop: the default handler DESTROYS the window, which strands
            // the cache holding a destroyed widget that re-presents as an
            // unresponsive zombie and never closes again. Hiding keeps the
            // live window for the next open; the cache drop destroys it.
            win.set_visible(false);
            glib::Propagation::Stop
        });
    }
    let current_page: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let has_more: Rc<Cell<bool>> = Rc::new(Cell::new(true));
    let loading_more: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let rendered_count: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let all_buttons: Rc<RefCell<Vec<glib::WeakRef<gtk4::Button>>>> =
        Rc::new(RefCell::new(Vec::new()));
    let is_initial_load: Rc<Cell<bool>> = Rc::new(Cell::new(true));

    let picker_ctx = SgdbPickerCtx {
        id: id.to_string(),
        save_dir: save_dir.to_string(),
        steam: steam.clone(),
        asset: asset.to_string(),
        is_steam_id,
        picker: picker.downgrade(),
        on_done: on_done.clone(),
        pending_copies: pending_copies.clone(),
        dest_dir: dest_dir.map(|s| s.to_string()),
        store: assets_store.clone(),
        rendered: rendered_count.clone(),
        zoom_level: zoom_level.clone(),
        tile_aspect: AssetType::from_string(asset)
            .map(|at| at.tile_aspect())
            .unwrap_or(1.0),
        pics: Rc::new(RefCell::new(Vec::new())),
        all_buttons: all_buttons.clone(),
        flow: flow.downgrade(),
        list_view: list_view.downgrade(),
        filter_user,
        unfilter_user,
    };

    // Re-showing a hidden picker resets any buttons left disabled by a
    // previous download so the same window is ready to pick again.
    let reset_buttons = all_buttons.clone();
    picker.connect_map(move |_| {
        for b in reset_buttons.borrow().iter() {
            if let Some(b) = b.upgrade() {
                b.set_sensitive(true);
                b.set_label(&crate::tr!("Download"));
            }
        }
    });

    let do_full_rebuild = {
        let ctx = picker_ctx.clone();
        Rc::new(move || ctx.rebuild_all()) as Rc<dyn Fn()>
    };

    let do_append = {
        let ctx = picker_ctx.clone();
        Rc::new(move || ctx.append_new()) as Rc<dyn Fn()>
    };

    let stack_toggle = stack.clone();
    toggle_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            stack_toggle.set_visible_child_name("list");
            btn.set_icon_name("view-grid-symbolic");
            btn.set_tooltip_text(Some(&crate::tr!("Switch to grid view")));
        } else {
            stack_toggle.set_visible_child_name("grid");
            btn.set_icon_name("view-list-symbolic");
            btn.set_tooltip_text(Some(&crate::tr!("Switch to list view")));
        }
    });

    if let Some(entry) = sgdb_cache
        .as_ref()
        .and_then(|c| c.borrow().get(&cache_key).cloned())
        .filter(|e| !e.assets.is_empty())
    {
        // Cache hit: render straight from the settings-screen cache instead of
        // re-issuing the network request. Pagination continues from the
        // page where the previous session left off.
        *assets_store.borrow_mut() = entry.assets;
        has_more.set(entry.has_more);
        current_page.set(entry.next_page);
        is_initial_load.set(false);
        loading_label.set_text("");
        loading_label.set_visible(false);
        loading_label2.set_text("");
        loading_label2.set_visible(false);
        do_full_rebuild();
    } else {
        let (tx, rx) = std::sync::mpsc::channel::<(Vec<SgdbAsset>, bool)>();
        let rx = std::cell::RefCell::new(rx);
        let steam_c = steam.clone();
        let id_c = id.to_string();
        let asset_at = AssetType::from_string(asset).unwrap_or(AssetType::Icon);
        let dims: Vec<String> = dimensions.iter().map(|s| s.to_string()).collect();
        std::thread::spawn(move || {
            let dims_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
            let results = steam_c.list_sgdb_assets(&id_c, asset_at, is_steam_id, &dims_refs, 0);
            let _ = tx.send(results);
        });

        let assets_store_t = assets_store.clone();
        let do_append_t = do_append.clone();
        let do_full_rebuild_t = do_full_rebuild.clone();
        let has_more_t = has_more.clone();
        let current_page_t = current_page.clone();
        let loading_more_t = loading_more.clone();
        let is_initial_load_t = is_initial_load.clone();
        let loading_label = loading_label.clone();
        let loading_label2 = loading_label2.clone();
        let picker_weak = picker.downgrade();
        let sgdb_cache_t = sgdb_cache.as_ref().map(Rc::downgrade);
        let cache_key_t = cache_key.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if picker_weak.upgrade().is_none() {
                return glib::ControlFlow::Break;
            }
            if let Ok((new_assets, more)) = rx.borrow_mut().try_recv() {
                if is_initial_load_t.get() {
                    *assets_store_t.borrow_mut() = new_assets.clone();
                    is_initial_load_t.set(false);
                    loading_label.set_text("");
                    loading_label.set_visible(false);
                    loading_label2.set_text("");
                    loading_label2.set_visible(false);
                    do_full_rebuild_t();
                    if let Some(ref cache) = sgdb_cache_t.as_ref().and_then(Weak::upgrade) {
                        if let Some(entry) = cache.borrow_mut().get_mut(&cache_key_t) {
                            entry.assets = new_assets;
                            entry.has_more = more;
                            entry.next_page = 1;
                        } else if let Some(picker) = picker_weak.upgrade() {
                            cache.borrow_mut().insert(
                                cache_key_t.clone(),
                                SgdbAssetsCacheEntry {
                                    assets: new_assets,
                                    has_more: more,
                                    next_page: 1,
                                    picker,
                                },
                            );
                        }
                    }
                } else {
                    assets_store_t.borrow_mut().extend(new_assets);
                    do_append_t();
                }
                has_more_t.set(more);
                current_page_t.set(current_page_t.get() + 1);
                loading_more_t.set(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    let zoom_level_z = zoom_level.clone();
    let zoom_pics = picker_ctx.pics.clone();
    let zoom_aspect = picker_ctx.tile_aspect;
    zoom_scale.connect_value_changed(move |s| {
        // Resize in place: rebuilding would respawn every thumbnail
        // download on each slider tick.
        let w = (s.value() as i32).clamp(100, 500);
        zoom_level_z.set(w);
        let h = (w as f64 / zoom_aspect).round().max(1.0) as i32;
        let (lw, lh) = list_thumb_size(w, zoom_aspect);
        for entry in zoom_pics.borrow_mut().iter_mut() {
            if let Some(texture) = entry.texture.clone() {
                paint_card(entry, &texture, w, zoom_aspect);
            } else {
                if let Some(pic) = entry.grid.upgrade() {
                    pic.set_size_request(w, h);
                }
                if let Some(pic) = entry.list.upgrade() {
                    pic.set_size_request(lw, lh);
                }
            }
            if let Some(lbl) = entry.label.upgrade() {
                lbl.set_size_request(w, -1);
            }
        }
    });

    // One ticker polls every card's thumbnail receiver. A timeout per
    // card saturated the main loop past a few hundred cards — starving
    // input and window close — while one callback sipping try_recv is
    // nothing. It lives as long as the window; hidden cached windows
    // just idle through it.
    {
        let tick_ctx = picker_ctx.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            let Some(_) = tick_ctx.picker.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let w = tick_ctx.zoom_level.get().clamp(100, 500);
            let aspect = tick_ctx.tile_aspect;
            for entry in tick_ctx.pics.borrow_mut().iter_mut() {
                let rx = entry.rx.borrow_mut().take();
                let Some(rx) = rx else { continue };
                match rx.try_recv() {
                    Ok(Some(bytes)) => {
                        if let Ok(texture) =
                            gdk4::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
                        {
                            paint_card(entry, &texture, w, aspect);
                        }
                    }
                    Ok(None) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        entry.rx.borrow_mut().replace(rx);
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
                }
            }
            glib::ControlFlow::Continue
        });
    }

/// Bottom edge of the lowest visible flow card, in flow coordinates.
/// Dimmed cards don't count: prefetching off them forces scrolling
/// through the whole hidden tail first.
fn last_visible_bottom(flow: &gtk4::FlowBox, steam: &SteamDataClient) -> Option<f64> {
    let mut bottom: Option<f64> = None;
    let mut child = flow.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        let name = w.widget_name();
        if steam.asset_filtered(tag_author(&name), tag_style(&name)) {
            continue;
        }
        if let Some(rect) = w.compute_bounds(flow) {
            bottom = Some(bottom.unwrap_or(0.0).max((rect.y() + rect.height()) as f64));
        }
    }
    bottom
}

    let vadj = scrolled.vadjustment();
    let steam_scroll = steam.clone();
    let id_scroll = id.to_string();
    let asset_scroll = asset.to_string();
    let dims_scroll: Vec<String> = dimensions.iter().map(|s| s.to_string()).collect();
    let assets_store_scroll = assets_store.clone();
    let do_append_scroll = do_append.clone();
    let has_more_scroll = has_more.clone();
    let loading_more_scroll = loading_more.clone();
    let current_page_scroll = current_page.clone();
    let picker_scroll = picker.downgrade();
    let sgdb_cache_scroll = sgdb_cache.as_ref().map(Rc::downgrade);
    let cache_key_scroll = cache_key.clone();
    let is_initial_load_scroll = is_initial_load.clone();
    let flow_scroll = flow.downgrade();
    let view_toggle = toggle_btn.clone();

    vadj.connect_value_changed(move |adj| {
        if is_initial_load_scroll.get() {
            return;
        }
        if !has_more_scroll.get() || loading_more_scroll.get() {
            return;
        }
        let Some(picker_vis) = picker_scroll.upgrade() else {
            return;
        };
        if !picker_vis.is_visible() {
            return;
        }

        let value = adj.value();
        let page_size = adj.page_size();
        // Prefetch off the last VISIBLE card, not the absolute bottom:
        // the dimmed tail keeps growing and scrolling through all of it
        // to trigger the next page is the complaint. The hidden list
        // view has no valid flow allocation, so it keeps the old
        // absolute check.
        let prefetch = if view_toggle.is_active() {
            adj.upper() - value - page_size <= 800.0
        } else if let Some(flow) = flow_scroll.upgrade() {
            match last_visible_bottom(&flow, &steam_scroll) {
                Some(bottom) => value + page_size >= bottom - 800.0,
                None => true,
            }
        } else {
            false
        };
        if !prefetch {
            return;
        }

        loading_more_scroll.set(true);
        let next_page = current_page_scroll.get();
        let (tx_more, rx_more) = std::sync::mpsc::channel::<(Vec<SgdbAsset>, bool)>();
        let rx_more = std::cell::RefCell::new(rx_more);
        let steam_m = steam_scroll.clone();
        let id_m = id_scroll.clone();
        let asset_at_m = AssetType::from_string(&asset_scroll).unwrap_or(AssetType::Icon);
        let dims_m = dims_scroll.clone();
        std::thread::spawn(move || {
            let dims_refs: Vec<&str> = dims_m.iter().map(|s| s.as_str()).collect();
            let results =
                steam_m.list_sgdb_assets(&id_m, asset_at_m, is_steam_id, &dims_refs, next_page);
            let _ = tx_more.send(results);
        });

        let do_append_m = do_append_scroll.clone();
        let current_page_m = current_page_scroll.clone();
        let has_more_m = has_more_scroll.clone();
        let loading_more_m = loading_more_scroll.clone();
        let assets_store_m = assets_store_scroll.clone();
        let picker_weak = picker_scroll.clone();
        let sgdb_cache_m = sgdb_cache_scroll.clone();
        let cache_key_m = cache_key_scroll.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            if picker_weak.upgrade().is_none() {
                loading_more_m.set(false);
                return glib::ControlFlow::Break;
            }
            if let Ok((new_assets, more)) = rx_more.borrow_mut().try_recv() {
                if !new_assets.is_empty() {
                    assets_store_m.borrow_mut().extend(new_assets.clone());
                    do_append_m();
                    if let Some(ref cache) = sgdb_cache_m.as_ref().and_then(Weak::upgrade) {
                        if let Some(entry) = cache.borrow_mut().get_mut(&cache_key_m) {
                            entry.assets.extend(new_assets);
                            entry.has_more = more;
                            entry.next_page = current_page_m.get() + 1;
                        }
                    }
                }
                has_more_m.set(more);
                current_page_m.set(current_page_m.get() + 1);
                loading_more_m.set(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    });
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

    // Display-bound probes, merged into one test: GTK lives on a single
    // thread, so separate display tests starve each other. Ignored by
    // default (needs a display); run under xvfb-run. HOME is sandboxed
    // so the close-request geometry save can't touch the real config.
    // Still compiled, so API drift breaks the build.
    #[test]
    #[ignore]
    fn test_picker_display_probes() {
        use adw::prelude::*;
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            adw::init().expect("adw init");
        });
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
        let save_dir = home.path().join("save");
        std::fs::create_dir_all(&save_dir).unwrap();
        let save_dir = save_dir.to_str().unwrap().to_string();
        let steam = std::sync::Arc::new(ira_api::SteamDataClient::new(
            String::new(),
            String::new(),
            &save_dir,
        ));

        // Phase 1: tiles measure tile-sized no matter how large the
        // downloaded texture is — a raw texture paintable reports its
        // own size and blows out the columns and the window minimum.
        let win = adw::Window::new();
        let ctx = SgdbPickerCtx {
            id: "1".to_string(),
            save_dir: save_dir.clone(),
            steam: steam.clone(),
            asset: "grid".to_string(),
            is_steam_id: false,
            picker: win.downgrade(),
            on_done: Rc::new(|| {}),
            pending_copies: None,
            dest_dir: None,
            store: Rc::new(RefCell::new(Vec::new())),
            rendered: Rc::new(Cell::new(0)),
            zoom_level: Rc::new(Cell::new(300)),
            tile_aspect: 2.0 / 3.0,
            pics: Rc::new(RefCell::new(Vec::new())),
            all_buttons: Rc::new(RefCell::new(Vec::new())),
            flow: glib::WeakRef::new(),
            list_view: glib::WeakRef::new(),
            filter_user: None,
            unfilter_user: None,
        };
        let asset = SgdbAsset {
            url: String::new(),
            thumb: String::new(),
            width: 1024,
            height: 1024,
            style: String::new(),
            author: "t".to_string(),
            author_steam64: String::new(),
            mime: String::new(),
        };
        let (card, _row) = build_sgdb_asset_card(&asset, &ctx, Rc::new(|| {}), 300, 450, false);
        let (min_w, ..) = card.measure(gtk4::Orientation::Horizontal, -1);
        assert_eq!(min_w, 300);
        // Sensitivity: a raw texture paintable blows the same card out.
        let bytes = vec![128u8; 1024 * 1024 * 4];
        let tex = gdk4::MemoryTexture::new(
            1024,
            1024,
            gdk4::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(bytes),
            1024 * 4,
        );
        let overlay = card
            .first_child()
            .and_then(|c| c.first_child())
            .and_then(|c| c.downcast::<gtk4::Overlay>().ok())
            .expect("card overlay");
        let pic = overlay
            .child()
            .and_then(|c| c.downcast::<gtk4::Picture>().ok())
            .expect("tile picture");
        pic.set_paintable(Some(&tex));
        let (_, raw_nat_w, ..) = card.measure(gtk4::Orientation::Horizontal, -1);
        assert!(raw_nat_w >= 1024, "probe insensitive: {raw_nat_w}");
        // The fix: sized paintable keeps the tile at tile size.
        let tex_obj = tex.upcast::<gdk4::Texture>();
        pic.set_paintable(Some(&ira_images::ScaledPaintable::new(&tex_obj, 300, 450)));
        pic.set_size_request(300, 450);
        let (fix_min_w, fix_nat_w, ..) = card.measure(gtk4::Orientation::Horizontal, -1);
        assert_eq!(fix_min_w, 300);
        assert_eq!(fix_nat_w, 300);

        // Phase 2: open, close, reopen from the cache, close again —
        // every close must hide the window.
        let parent = adw::Window::new();
        parent.present();
        let cache: Rc<RefCell<HashMap<String, SgdbAssetsCacheEntry>>> = Default::default();
        let open = || {
            show_sgdb_picker(ShowSgdbPickerParams {
                steam: &steam,
                id: "1",
                asset: "grid",
                is_steam_id: false,
                dimensions: &["600x900"],
                parent: &parent,
                on_done: Rc::new(|| {}),
                pending_copies: None,
                sgdb_cache: Some(cache.clone()),
                save_dir: &save_dir,
                dest_dir: None,
                filter_user: None,
                unfilter_user: None,
                on_geometry: None,
            });
        };
        let pump = || {
            for _ in 0..100 {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        let picker = || {
            cache
                .borrow()
                .values()
                .next()
                .map(|entry| entry.picker.clone())
        };
        open();
        pump();
        let win = picker().expect("picker cached after open");
        assert!(win.is_visible(), "fresh picker never became visible");
        // Fresh config: default size applies.
        assert_eq!(win.width(), 900);
        assert_eq!(win.height(), 700);
        win.close();
        pump();
        assert!(!win.is_visible(), "first close did not hide");
        open();
        pump();
        assert!(win.is_visible(), "cache-hit reopen never became visible");
        // Resize, close, and prove the geometry round-trips through the
        // config file into a fresh window. Default size stands in for a
        // user resize (headless has no compositor to drag against).
        win.close();
        pump();
        assert!(!win.is_visible(), "second close did not hide");
        let config_path = home
            .path()
            .join(".config")
            .join("ira")
            .join("config.json");
        let config_text = std::fs::read_to_string(&config_path)
            .expect("config file written by close");
        let config_json: serde_json::Value =
            serde_json::from_str(&config_text).expect("config parses");
        assert_eq!(
            config_json["sgdb_picker_size"]["grid"],
            serde_json::json!([900, 700])
        );
        assert!(
            config_json["sgdb_picker_zoom"]["grid"]
                .as_i64()
                .is_some_and(|z| (100..=500).contains(&z))
        );
        // Fresh cache: rebuild reads the saved geometry back.
        let cache2: Rc<RefCell<HashMap<String, SgdbAssetsCacheEntry>>> = Default::default();
        show_sgdb_picker(ShowSgdbPickerParams {
            steam: &steam,
            id: "1",
            asset: "grid",
            is_steam_id: false,
            dimensions: &["600x900"],
            parent: &parent,
            on_done: Rc::new(|| {}),
            pending_copies: None,
            sgdb_cache: Some(cache2.clone()),
            save_dir: &save_dir,
            dest_dir: None,
            filter_user: None,
            unfilter_user: None,
            on_geometry: None,
        });
        pump();
        let win2 = cache2
            .borrow()
            .values()
            .next()
            .map(|entry| entry.picker.clone())
            .expect("fresh picker cached");
        assert!(win2.is_visible(), "fresh reopen never became visible");
        assert_eq!((win2.width(), win2.height()), (900, 700));

        // Phase 3: rows stay content-sized instead of stretching to the
        // viewport. Three fixture cards go in a 1000-tall window; at a
        // ~950 viewport a stretching row would fill it, a content-sized
        // one stays near its 516 card.
        let measure_win = adw::Window::new();
        measure_win.set_default_size(1000, 1000);
        let measure_ctx = SgdbPickerCtx {
            id: "1".to_string(),
            save_dir: save_dir.clone(),
            steam: steam.clone(),
            asset: "grid".to_string(),
            is_steam_id: false,
            picker: measure_win.downgrade(),
            on_done: Rc::new(|| {}),
            pending_copies: None,
            dest_dir: None,
            store: Rc::new(RefCell::new(Vec::new())),
            rendered: Rc::new(Cell::new(0)),
            zoom_level: Rc::new(Cell::new(300)),
            tile_aspect: 2.0 / 3.0,
            pics: Rc::new(RefCell::new(Vec::new())),
            all_buttons: Rc::new(RefCell::new(Vec::new())),
            flow: glib::WeakRef::new(),
            list_view: glib::WeakRef::new(),
            filter_user: None,
            unfilter_user: None,
        };
        let measure_flow = gtk4::FlowBox::new();
        measure_flow.set_selection_mode(gtk4::SelectionMode::None);
        measure_flow.set_homogeneous(false);
        measure_flow.set_min_children_per_line(1);
        measure_flow.set_row_spacing(12);
        measure_flow.set_column_spacing(12);
        measure_flow.set_halign(gtk4::Align::Fill);
        measure_flow.set_valign(gtk4::Align::Start);
        measure_win.set_content(Some(&measure_flow));
        measure_win.present();
        for author in ["MrSandman7337", "Neptune13", "GabiShy"] {
            let asset = SgdbAsset {
                url: String::new(),
                thumb: String::new(),
                width: 600,
                height: 900,
                style: "alternate".to_string(),
                author: author.to_string(),
                author_steam64: String::new(),
                mime: String::new(),
            };
            let (card, _row) =
                build_sgdb_asset_card(&asset, &measure_ctx, Rc::new(|| {}), 300, 450, false);
            measure_flow.append(&card);
        }
        pump();
        let height_of = |w: &gtk4::Widget| {
            w.compute_bounds(w).map(|r| r.height() as i32).unwrap_or(-1)
        };
        let first = measure_flow
            .first_child()
            .and_then(|c| c.downcast::<gtk4::FlowBoxChild>().ok())
            .expect("flow has children");
        let card = first.first_child().expect("child has card");
        let (child_h, card_h) = (height_of(first.upcast_ref()), height_of(&card));
        assert!(
            child_h >= card_h && child_h < 600,
            "row stretched to viewport: {child_h} vs card {card_h}"
        );
    }
}
