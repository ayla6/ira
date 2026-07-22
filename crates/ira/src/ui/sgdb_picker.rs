use gtk4::prelude::*;
use adw::prelude::*;
use ira_api::SteamDataClient;
use ira_api::types::SgdbAsset;
use ira_models::AssetType;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use super::helpers::clear_children;
use super::css::*;

fn build_sgdb_asset_card(
    a: &SgdbAsset,
    _asset_type: &str,
    steam: &Arc<SteamDataClient>,
    on_download: Rc<dyn Fn()>,
    thumb_size: i32,
    all_buttons: Rc<RefCell<Vec<gtk4::Button>>>,
) -> (gtk4::Widget, gtk4::Widget) {

    let mut info = String::new();
    if a.width > 0 && a.height > 0 {
        info = format!("{}\u{d7}{}", a.width, a.height);
    }
    if !a.style.is_empty() {
        if !info.is_empty() { info = format!("{} \u{b7} {}", info, a.style); }
        else { info = a.style.clone(); }
    }
    if !a.author.is_empty() {
        if !info.is_empty() { info = format!("{} \u{b7} by {}", info, a.author); }
        else { info = format!("by {}", a.author); }
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

    let gdl = gtk4::Button::with_label("Download");
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

    let ldl = gtk4::Button::with_label("Download");
    ldl.add_css_class(CSS_SUGGESTED_ACTION);
    row.append(&ldl);

    all_buttons.borrow_mut().push(gdl.clone());
    all_buttons.borrow_mut().push(ldl.clone());

    let cb_g = on_download.clone();
    let buttons_g = all_buttons.clone();
    gdl.connect_clicked(move |_| {
        for b in buttons_g.borrow().iter() {
            b.set_sensitive(false);
            b.set_label("Downloading…");
        }
        cb_g();
    });
    let buttons_l = all_buttons.clone();
    ldl.connect_clicked(move |_| {
        for b in buttons_l.borrow().iter() {
            b.set_sensitive(false);
            b.set_label("Downloading…");
        }
        on_download();
    });

    let thumb_url = if a.thumb.is_empty() { a.url.clone() } else { a.thumb.clone() };
    let steam_thumb = steam.clone();
    let (tx_thumb, rx_thumb) = std::sync::mpsc::channel::<Option<Vec<u8>>>();
    let rx_thumb = std::cell::RefCell::new(rx_thumb);
    std::thread::spawn(move || {
        let _s = tracing::info_span!("thumbnail_download", url = %thumb_url).entered();
        let bytes = steam_thumb.download_bytes(&thumb_url).ok().filter(|b| !b.is_empty());
        let _ = tx_thumb.send(bytes);
    });
    let tp_g = grid_pic.clone();
    let tp_l = list_pic.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
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

struct SgdbPickerCtx {
    id: String,
    save_dir: String,
    steam: Arc<SteamDataClient>,
    asset: String,
    is_steam_id: bool,
    picker: adw::Window,
    on_done: Rc<dyn Fn()>,
    pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
}

fn rebuild_assets_view(
    flow: &gtk4::FlowBox,
    list_view: &gtk4::ListBox,
    assets: &[SgdbAsset],
    thumb_size: i32,
    ctx: &SgdbPickerCtx,
) {
    clear_children(flow);
    clear_children(list_view);

    if assets.is_empty() {
        let none = gtk4::Label::new(Some("No images found on SteamGridDB"));
        none.add_css_class(CSS_DIM_LABEL);
        flow.append(&none);
        list_view.append(&gtk4::Label::new(Some("No images found on SteamGridDB")));
        return;
    }

    flow.set_max_children_per_line((900 / (thumb_size + 20)).clamp(1, 8) as u32);

    let all_buttons: Rc<RefCell<Vec<gtk4::Button>>> = Rc::new(RefCell::new(Vec::new()));

    for a in assets {
        let data_subdir = if ctx.is_steam_id { "steam".to_string() } else { "steamgriddb".to_string() };
        let dest_dir = format!("{}/data/{}/{}", ctx.save_dir, data_subdir, ctx.id);
        let Some(at) = AssetType::from_string(&ctx.asset) else { continue; };
        let file_name = match at {
            AssetType::Icon => {
                let ext = if a.mime.contains("icon") || a.mime.contains("x-icon") { "ico" }
                else if a.mime.contains("png") { "png" }
                else if a.mime.contains("jpeg") || a.mime.contains("jpg") { "jpg" }
                else if a.mime.contains("webp") { "webp" }
                else { std::path::Path::new(&a.url).extension().and_then(|e| e.to_str()).unwrap_or("png") };
                format!("{}.{}", at.file_base(), ext)
            }
            AssetType::Hero => format!("{}.jpg", at.file_base()),
            AssetType::Grid => format!("{}.jpg", at.file_base()),
            AssetType::Header => format!("{}.jpg", at.file_base()),
            AssetType::Logo => format!("{}.png", at.file_base()),
        };
        let _dest = format!("{}/{}", dest_dir, file_name);
        let dl_url = a.url.clone();
        let steam_dl = ctx.steam.clone();
        let picker_dl = ctx.picker.clone();
        let on_done_dl = ctx.on_done.clone();
        let asset_dl = ctx.asset.clone();
        let pending_dl = ctx.pending_copies.clone();
        let on_download: Rc<dyn Fn()> = Rc::new(move || {
            let _s = tracing::info_span!("on_download", asset = %asset_dl, url = %dl_url).entered();
            if let Some(ref pc) = pending_dl {
                let tmp = {
                    let url_path = std::path::Path::new(&dl_url);
                    let e = url_path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if e.is_empty() {
                        std::env::temp_dir().join(format!("sgdb_{}", asset_dl))
                    } else {
                        std::env::temp_dir().join(format!("sgdb_{}.{}", asset_dl, e))
                    }
                };
                let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
                let rx = std::cell::RefCell::new(rx);
                let url = dl_url.clone();
                let steam = steam_dl.clone();
                let tmp_c = tmp.clone();
                std::thread::spawn(move || {
                    let _s = tracing::info_span!("download_and_convert", url = %url).entered();
                    let result = if steam.download_file(&url, &tmp_c).is_ok() {
                        let ext = tmp_c.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                        if ext == "png" || ext == "ico" {
                            ira_parser::convert_to_lossless_webp(&tmp_c);
                            let webp = tmp_c.with_extension("webp");
                            if webp.is_file() { webp.to_string_lossy().into_owned() }
                            else { tmp_c.to_string_lossy().into_owned() }
                        } else {
                            tmp_c.to_string_lossy().into_owned()
                        }
                    } else {
                        eprintln!("Download failed for {}", url);
                        String::new()
                    };
                    let _ = tx.send(if result.is_empty() { None } else { Some(result) });
                });
                let pc_c = pc.clone();
                let on_done_c = on_done_dl.clone();
                let picker_c = picker_dl.clone();
                let asset_c = asset_dl.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                    if let Ok(result) = rx.borrow_mut().try_recv() {
                        if let Some(path) = result {
                            pc_c.borrow_mut().insert(asset_c.clone(), path);
                            on_done_c();
                            picker_c.close();
                        }
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                });
            }
        });

        let (grid_card, list_row) = build_sgdb_asset_card(a, &ctx.asset, &ctx.steam, on_download, thumb_size, all_buttons.clone());
        flow.append(&grid_card);
        list_view.append(&list_row);
    }
}

pub(crate) struct ShowSgdbPickerParams<'a> {
    pub steam: &'a Arc<SteamDataClient>,
    pub id: &'a str,
    pub asset: &'a str,
    pub is_steam_id: bool,
    pub dimensions: &'a [&'a str],
    pub parent: &'a adw::Window,
    pub on_done: Rc<dyn Fn()>,
    pub pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
    pub save_dir: &'a str,
}

pub fn show_sgdb_picker(params: ShowSgdbPickerParams) {
    let ShowSgdbPickerParams { steam, id, asset, is_steam_id, dimensions, parent, on_done, pending_copies, save_dir } = params;
    let picker = adw::Window::new();
    picker.set_default_width(900);
    picker.set_default_height(700);
    picker.set_transient_for(Some(parent));
    picker.set_modal(true);
    let save_dir_owned = save_dir.to_string();

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(&format!("Pick {}", asset)))));

    let toggle_btn = gtk4::ToggleButton::new();
    toggle_btn.set_icon_name("view-list-symbolic");
    toggle_btn.set_tooltip_text(Some("Switch to list view"));
    toggle_btn.add_css_class(CSS_FLAT);

    let zoom_adj = gtk4::Adjustment::new(300.0, 100.0, 500.0, 50.0, 100.0, 0.0);
    let zoom_scale = gtk4::Scale::new(gtk4::Orientation::Horizontal, Some(&zoom_adj));
    zoom_scale.set_tooltip_text(Some("Zoom"));
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

    let loading = gtk4::Label::new(Some("Loading\u{2026}"));
    loading.add_css_class(CSS_DIM_LABEL);
    flow.append(&loading);
    list_view.append(&gtk4::Label::new(Some("Loading\u{2026}")));

    scrolled.set_child(Some(&stack));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win = picker.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    picker.set_content(Some(&outer));
    picker.present();

    let (tx, rx) = std::sync::mpsc::channel::<Vec<SgdbAsset>>();
    let rx = std::cell::RefCell::new(rx);
    let steam_c = steam.clone();
    let id_c = id.to_string();
    let asset_at = AssetType::from_string(asset).unwrap_or(AssetType::Icon);
    let dims: Vec<String> = dimensions.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let dims_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let results = steam_c.list_sgdb_assets(&id_c, asset_at, is_steam_id, &dims_refs);
        let _ = tx.send(results);
    });

    let steam_clone = steam.clone();
    let id_clone = id.to_string();
    let asset_clone = asset.to_string();
    let save_dir_clone = save_dir_owned.clone();
    let picker_clone = picker.clone();
    let on_done = on_done.clone();
    let pending_copies = pending_copies.clone();

    let assets_store: Rc<RefCell<Vec<SgdbAsset>>> = Rc::new(RefCell::new(Vec::new()));
    let zoom_level = Rc::new(Cell::new(300));

    let rebuild: Rc<dyn Fn()> = {
        let assets_store = assets_store.clone();
        let zoom_level = zoom_level.clone();
        let flow = flow.clone();
        let list_view = list_view.clone();
        let picker_ctx = SgdbPickerCtx {
            id: id_clone,
            save_dir: save_dir_clone,
            steam: steam_clone,
            asset: asset_clone,
            is_steam_id,
            picker: picker_clone,
            on_done,
            pending_copies,
        };

        Rc::new(move || {
            let assets = assets_store.borrow();
            let thumb_size = zoom_level.get();
            rebuild_assets_view(
                &flow,
                &list_view,
                &assets,
                thumb_size,
                &picker_ctx,
            );
        })
    };

    let stack_toggle = stack.clone();
    toggle_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            stack_toggle.set_visible_child_name("list");
            btn.set_icon_name("view-grid-symbolic");
            btn.set_tooltip_text(Some("Switch to grid view"));
        } else {
            stack_toggle.set_visible_child_name("grid");
            btn.set_icon_name("view-list-symbolic");
            btn.set_tooltip_text(Some("Switch to list view"));
        }
    });

    let assets_store_t = assets_store.clone();
    let rebuild_t = rebuild.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok(assets) = rx.borrow_mut().try_recv() {
            *assets_store_t.borrow_mut() = assets;
            rebuild_t();
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });

    let rebuild_z = rebuild.clone();
    zoom_scale.connect_value_changed(move |s| {
        zoom_level.set(s.value() as i32);
        rebuild_z();
    });
}
