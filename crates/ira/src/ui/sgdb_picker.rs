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

fn build_sgdb_asset_card(
    a: &SgdbAsset,
    _asset_type: &str,
    steam: &Arc<SteamDataClient>,
    on_download: Rc<dyn Fn()>,
    thumb_size: i32,
    all_buttons: Rc<RefCell<Vec<glib::WeakRef<gtk4::Button>>>>,
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
    card.set_margin_top(8);
    card.set_margin_bottom(8);

    let grid_pic = gtk4::Picture::new();
    grid_pic.set_content_fit(gtk4::ContentFit::ScaleDown);
    grid_pic.set_size_request(thumb_size, thumb_size);
    card.append(&grid_pic);

    let ilbl = gtk4::Label::new(Some(&info));
    ilbl.set_xalign(0.5);
    ilbl.set_max_width_chars(20);
    ilbl.set_wrap(true);
    ilbl.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
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
    list_pic.set_content_fit(gtk4::ContentFit::ScaleDown);
    let list_thumb = (thumb_size / 3).max(48);
    list_pic.set_size_request(list_thumb, list_thumb);
    row.append(&list_pic);

    let rlbl = gtk4::Label::new(Some(&info));
    rlbl.set_xalign(0.0);
    rlbl.set_hexpand(true);
    rlbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    row.append(&rlbl);

    let ldl = gtk4::Button::with_label(&crate::tr!("Download"));
    ldl.add_css_class(CSS_SUGGESTED_ACTION);
    row.append(&ldl);

    all_buttons.borrow_mut().push(gdl.downgrade());
    all_buttons.borrow_mut().push(ldl.downgrade());

    let cb_g = on_download.clone();
    let buttons_g = all_buttons.clone();
    gdl.connect_clicked(move |_| {
        for b in buttons_g.borrow().iter() {
            if let Some(b) = b.upgrade() {
                b.set_sensitive(false);
                b.set_label(&crate::tr!("Downloading\u{2026}"));
            }
        }
        cb_g();
    });
    let buttons_l = all_buttons.clone();
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
    let steam_thumb = steam.clone();
    let (tx_thumb, rx_thumb) = std::sync::mpsc::channel::<Option<Vec<u8>>>();
    let rx_thumb = std::cell::RefCell::new(rx_thumb);
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
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        let (Some(tp_g), Some(tp_l)) = (tp_g.upgrade(), tp_l.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        if let Ok(bytes) = rx_thumb.borrow_mut().try_recv() {
            if let Some(bytes) = bytes {
                let texture = gdk4::Texture::from_bytes(&glib::Bytes::from_owned(bytes));
                if let Ok(texture) = texture {
                    tp_g.set_paintable(Some(&texture));
                    tp_l.set_paintable(Some(&texture));
                }
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });

    (card.upcast::<gtk4::Widget>(), row.upcast::<gtk4::Widget>())
}

#[derive(Clone)]
struct SgdbPickerCtx {
    id: String,
    save_dir: String,
    steam: Arc<SteamDataClient>,
    asset: String,
    is_steam_id: bool,
    picker: glib::WeakRef<adw::Dialog>,
    on_done: Rc<dyn Fn()>,
    pending_copies: Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
    dest_dir: Option<String>,
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
    thumb_size: i32,
    ctx: &SgdbPickerCtx,
    all_buttons: &Rc<RefCell<Vec<glib::WeakRef<gtk4::Button>>>>,
) {
    flow.set_max_children_per_line((900 / (thumb_size + 20)).clamp(1, 8) as u32);
    for a in &assets[start..] {
        let on_download = build_on_download(ctx, a);
        let (grid_card, list_row) = build_sgdb_asset_card(
            a,
            &ctx.asset,
            &ctx.steam,
            on_download,
            thumb_size,
            all_buttons.clone(),
        );
        flow.append(&grid_card);
        list_view.append(&list_row);
    }
}

fn full_rebuild(
    flow: &gtk4::FlowBox,
    list_view: &gtk4::ListBox,
    assets: &[SgdbAsset],
    thumb_size: i32,
    ctx: &SgdbPickerCtx,
    all_buttons: &Rc<RefCell<Vec<glib::WeakRef<gtk4::Button>>>>,
) {
    clear_children(flow);
    clear_children(list_view);
    all_buttons.borrow_mut().clear();

    if assets.is_empty() {
        let none = gtk4::Label::new(Some(&crate::tr!("No images found on SteamGridDB")));
        none.add_css_class(CSS_DIM_LABEL);
        flow.append(&none);
        list_view.append(&gtk4::Label::new(Some(&crate::tr!(
            "No images found on SteamGridDB"
        ))));
        return;
    }

    append_assets(flow, list_view, assets, 0, thumb_size, ctx, all_buttons);
}

pub(crate) struct ShowSgdbPickerParams<'a> {
    pub steam: &'a Arc<SteamDataClient>,
    pub id: &'a str,
    pub asset: &'a str,
    pub is_steam_id: bool,
    pub dimensions: &'a [&'a str],
    pub parent: &'a adw::Dialog,
    pub on_done: Rc<dyn Fn()>,
    pub pending_copies: Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
    pub sgdb_cache: Option<Rc<RefCell<HashMap<String, SgdbAssetsCacheEntry>>>>,
    pub save_dir: &'a str,
    pub dest_dir: Option<&'a str>,
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
    } = params;
    // Key the settings-screen cache by the full SGDB query, not just the
    // asset type: a Steam game's picker runs against its Steam id until it's
    // matched to an SGDB id, and reusing one entry across both would serve
    // stale results (and resume pagination from the wrong stream).
    let cache_key = format!("{}|{}|{}|{}", id, is_steam_id, asset, dimensions.join(","));

    // If a picker window for this exact query is still alive (hidden, not
    // destroyed), re-present it with its loaded thumbnails and scroll intact
    // instead of rebuilding everything and refetching the list.
    if let Some(c) = sgdb_cache.as_ref() {
        if let Some(entry) = c.borrow().get(&cache_key) {
            if let Some(w) = entry.picker.upgrade() {
                super::helpers::fit_dialog_height(&w, parent, 700);
                w.present(Some(parent));
            }
            return;
        }
    }

    let picker = adw::Dialog::new();
    picker.set_content_width(900);
    picker.set_content_height(700);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header_bar = adw::HeaderBar::new();
    let title = crate::tr!("Pick {}").replacen("{}", asset, 1);
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(&title))));

    let toggle_btn = gtk4::ToggleButton::new();
    toggle_btn.set_icon_name("view-list-symbolic");
    toggle_btn.set_tooltip_text(Some(&crate::tr!("Switch to list view")));
    toggle_btn.add_css_class(CSS_FLAT);

    let zoom_adj = gtk4::Adjustment::new(300.0, 100.0, 500.0, 50.0, 100.0, 0.0);
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
    flow.set_homogeneous(true);
    flow.set_min_children_per_line(1);
    flow.set_max_children_per_line(3);
    flow.set_row_spacing(8);
    flow.set_column_spacing(8);
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

    let close_btn = gtk4::Button::with_label(&crate::tr!("Close"));
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win = picker.downgrade();
    close_btn.connect_clicked(move |_| {
        if let Some(win) = win.upgrade() {
            win.close();
        }
    });
    outer.append(&close_btn);

    picker.set_child(Some(&outer));
    // 700 content + sheet chrome overflows shorter windows; libadwaita
    // warns and clips floating sheets that ask for more than their
    // presenter has.
    super::helpers::fit_dialog_height(&picker, parent, 700);
    picker.present(Some(parent));

    // Closing an AdwDialog only unmaps it, so the picker object — and its
    // loaded thumbnails — survive until the settings screen tears down.

    // Register the live window in the settings-screen cache immediately so a
    // reopen (even before the first fetch lands) can re-present it instead of
    // building a fresh one. The fetch handler updates this entry in place.
    if let Some(ref cache) = sgdb_cache {
        cache.borrow_mut().insert(
            cache_key.clone(),
            SgdbAssetsCacheEntry {
                assets: Vec::new(),
                has_more: true,
                next_page: 0,
                picker: picker.downgrade(),
            },
        );
    }

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
    };

    let assets_store: Rc<RefCell<Vec<SgdbAsset>>> = Rc::new(RefCell::new(Vec::new()));
    let zoom_level = Rc::new(Cell::new(300));
    let current_page: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let has_more: Rc<Cell<bool>> = Rc::new(Cell::new(true));
    let loading_more: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let rendered_count: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let all_buttons: Rc<RefCell<Vec<glib::WeakRef<gtk4::Button>>>> =
        Rc::new(RefCell::new(Vec::new()));
    let is_initial_load: Rc<Cell<bool>> = Rc::new(Cell::new(true));

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
        let assets_store = assets_store.clone();
        let zoom_level = zoom_level.clone();
        let flow = flow.clone();
        let list_view = list_view.clone();
        let ctx = picker_ctx.clone();
        let all_buttons = all_buttons.clone();
        let rendered_count = rendered_count.clone();

        Rc::new(move || {
            let assets = assets_store.borrow();
            let thumb_size = zoom_level.get();
            full_rebuild(&flow, &list_view, &assets, thumb_size, &ctx, &all_buttons);
            rendered_count.set(assets.len());
        }) as Rc<dyn Fn()>
    };

    let do_append = {
        let assets_store = assets_store.clone();
        let zoom_level = zoom_level.clone();
        let flow = flow.clone();
        let list_view = list_view.clone();
        let ctx = picker_ctx.clone();
        let all_buttons = all_buttons.clone();
        let rendered_count = rendered_count.clone();

        Rc::new(move || {
            let assets = assets_store.borrow();
            let thumb_size = zoom_level.get();
            let start = rendered_count.get();
            if start < assets.len() {
                append_assets(
                    &flow,
                    &list_view,
                    &assets,
                    start,
                    thumb_size,
                    &ctx,
                    &all_buttons,
                );
                rendered_count.set(assets.len());
            }
        }) as Rc<dyn Fn()>
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
                                    picker: picker.downgrade(),
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
    let do_full_rebuild_z = do_full_rebuild.clone();
    zoom_scale.connect_value_changed(move |s| {
        zoom_level_z.set(s.value() as i32);
        do_full_rebuild_z();
    });

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
        let upper = adj.upper();
        let page_size = adj.page_size();
        let distance_to_bottom = upper - value - page_size;

        if distance_to_bottom > 800.0 {
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
