use gtk4::prelude::*;
use adw::prelude::*;
use ira_api::SteamDataClient;
use ira_api::types::SgdbAsset;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use super::helpers::clear_children;

fn build_sgdb_asset_card(
    a: &SgdbAsset,
    _asset_type: &str,
    steam: &Arc<SteamDataClient>,
    on_download: Rc<dyn Fn()>,
    save_dir: String,
    thumb_size: i32,
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
    ilbl.add_css_class("dim-label");
    card.append(&ilbl);

    let gdl = gtk4::Button::with_label("Download");
    gdl.add_css_class("suggested-action");
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
    ldl.add_css_class("suggested-action");
    row.append(&ldl);

    let cb_g = on_download.clone();
    let gdl_c = gdl.clone();
    gdl.connect_clicked(move |_| {
        gdl_c.set_label("Downloading…");
        gdl_c.set_sensitive(false);
        cb_g();
    });
    let ldl_c = ldl.clone();
    ldl.connect_clicked(move |_| {
        ldl_c.set_label("Downloading…");
        ldl_c.set_sensitive(false);
        on_download();
    });

    let url_clone = a.url.clone();
    let steam_thumb = steam.clone();
    let thumb_dir = format!("{}/data/.thumbnails", save_dir);
    let _ = std::fs::create_dir_all(&thumb_dir);
    let full_cache_name = format!("{}/{}", thumb_dir, url_clone.rsplit('/').next().unwrap_or("thumb"));
    let thumb_display_name = format!("{}_thumb", full_cache_name);
    let tsize = thumb_size;
    let (tx_thumb, rx_thumb) = std::sync::mpsc::channel::<Option<String>>();
    let rx_thumb = std::cell::RefCell::new(rx_thumb);
    std::thread::spawn(move || {
        let _s = tracing::info_span!("thumbnail_download", url = %url_clone).entered();
        let display_path = if std::path::Path::new(&thumb_display_name).exists() {
            thumb_display_name.clone()
        } else if steam_thumb.download_file(&url_clone, std::path::Path::new(&full_cache_name)).is_ok() {
            let mut src_path = full_cache_name.clone();
            if std::path::Path::new(&full_cache_name).extension().and_then(|e| e.to_str()) == Some("ico") {
                if let Ok(img) = image::open(&full_cache_name) {
                    let png_path = std::path::Path::new(&full_cache_name).with_extension("png");
                    if img.save(&png_path).is_ok() {
                        let _ = std::fs::remove_file(&full_cache_name);
                        src_path = png_path.to_string_lossy().into_owned();
                    }
                }
            }
            if let Ok(img) = image::open(&src_path) {
                let (w, h) = (img.width(), img.height());
                if w > tsize as u32 || h > tsize as u32 {
                    let resized = img.resize(tsize as u32, tsize as u32, image::imageops::FilterType::Lanczos3);
                    let _ = resized.save(&thumb_display_name);
                } else {
                    let _ = std::fs::copy(&src_path, &thumb_display_name);
                }
            }
            thumb_display_name.clone()
        } else {
            String::new()
        };
        let _ = tx_thumb.send(if display_path.is_empty() { None } else { Some(display_path) });
    });
    let tp_g = grid_pic.clone();
    let tp_l = list_pic.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if let Ok(path) = rx_thumb.borrow_mut().try_recv() {
            if let Some(p) = path {
                tp_g.set_filename(Some(&p));
                tp_l.set_filename(Some(&p));
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });

    (card.upcast::<gtk4::Widget>(), row.upcast::<gtk4::Widget>())
}

#[allow(clippy::too_many_arguments)]
fn rebuild_assets_view(
    flow: &gtk4::FlowBox,
    list_view: &gtk4::ListBox,
    assets: &[SgdbAsset],
    thumb_size: i32,
    id_clone: &str,
    save_dir_clone: &str,
    steam_clone: &Arc<SteamDataClient>,
    asset_clone: &str,
    is_steam_id: bool,
    picker_clone: &adw::Window,
    on_done: &Rc<dyn Fn()>,
    pending_copies: &Option<Rc<RefCell<HashMap<String, String>>>>,
) {
    clear_children(flow);
    clear_children(list_view);

    if assets.is_empty() {
        let none = gtk4::Label::new(Some("No images found on SteamGridDB"));
        none.add_css_class("dim-label");
        flow.append(&none);
        list_view.append(&gtk4::Label::new(Some("No images found on SteamGridDB")));
        return;
    }

    flow.set_max_children_per_line((900 / (thumb_size + 20)).clamp(1, 8) as u32);

    for a in assets {
        let data_subdir = if is_steam_id { "steam".to_string() } else { "steamgriddb".to_string() };
        let dest_dir = format!("{}/data/{}/{}", save_dir_clone, data_subdir, id_clone);
        let file_name = match asset_clone {
            "icon" => {
                let ext = if a.mime.contains("icon") || a.mime.contains("x-icon") { "ico" }
                else if a.mime.contains("png") { "png" }
                else if a.mime.contains("jpeg") || a.mime.contains("jpg") { "jpg" }
                else if a.mime.contains("webp") { "webp" }
                else { std::path::Path::new(&a.url).extension().and_then(|e| e.to_str()).unwrap_or("png") };
                format!("icon.{}", ext)
            }
            "hero" => "hero.jpg".to_string(),
            "grid" => "vertical.jpg".to_string(),
            "header" => "header.jpg".to_string(),
            "logo" => "logo.png".to_string(),
            _ => continue,
        };
        let _dest = format!("{}/{}", dest_dir, file_name);
        let dl_url = a.url.clone();
        let steam_dl = steam_clone.clone();
        let picker_dl = picker_clone.clone();
        let on_done_dl = on_done.clone();
        let asset_dl = asset_clone.to_string();
        let pending_dl = pending_copies.clone();
        let save_dir_dl = save_dir_clone.to_string();
        let on_download: Rc<dyn Fn()> = Rc::new(move || {
            let _s = tracing::info_span!("on_download", asset = %asset_dl, url = %dl_url).entered();
            if let Some(ref pc) = pending_dl {
                let thumb_dir = format!("{}/data/.thumbnails", save_dir_dl);
                let cache_name = format!("{}/{}", thumb_dir, dl_url.rsplit('/').next().unwrap_or("thumb"));
                let cache_path = std::path::Path::new(&cache_name);
                let png_cache = cache_path.with_extension("png");
                let cached = if cache_path.is_file() {
                    Some(cache_path.to_path_buf())
                } else if png_cache.is_file() {
                    Some(png_cache)
                } else {
                    None
                };

                if let Some(cached_path) = cached {
                    let ext = cached_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                    let final_path = if ext == "png" || ext == "ico" {
                        ira_parser::convert_to_lossless_webp(&cached_path);
                        let webp = cached_path.with_extension("webp");
                        if webp.is_file() { webp.to_string_lossy().into_owned() }
                        else { cached_path.to_string_lossy().into_owned() }
                    } else {
                        cached_path.to_string_lossy().into_owned()
                    };
                    pc.borrow_mut().insert(asset_dl.clone(), final_path);
                    on_done_dl();
                    picker_dl.close();
                    return;
                }

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

        let (grid_card, list_row) = build_sgdb_asset_card(a, asset_clone, steam_clone, on_download, save_dir_clone.to_string(), thumb_size);
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
    toggle_btn.add_css_class("flat");

    let zoom_adj = gtk4::Adjustment::new(300.0, 100.0, 500.0, 50.0, 100.0, 0.0);
    let zoom_scale = gtk4::Scale::new(gtk4::Orientation::Horizontal, Some(&zoom_adj));
    zoom_scale.set_tooltip_text(Some("Zoom"));
    zoom_scale.set_width_request(120);
    zoom_scale.set_draw_value(false);
    zoom_scale.add_css_class("flat");

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
    loading.add_css_class("dim-label");
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
    let asset_c = asset.to_string();
    let dims: Vec<String> = dimensions.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let dims_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let results = steam_c.list_sgdb_assets(&id_c, &asset_c, is_steam_id, &dims_refs);
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
        let steam_clone = steam_clone.clone();
        let id_clone = id_clone.clone();
        let asset_clone = asset_clone.clone();
        let save_dir_clone = save_dir_clone.clone();
        let picker_clone = picker_clone.clone();
        let on_done = on_done.clone();
        let pending_copies = pending_copies.clone();

        Rc::new(move || {
            let assets = assets_store.borrow();
            let thumb_size = zoom_level.get();
            rebuild_assets_view(
                &flow,
                &list_view,
                &assets,
                thumb_size,
                &id_clone,
                &save_dir_clone,
                &steam_clone,
                &asset_clone,
                is_steam_id,
                &picker_clone,
                &on_done,
                &pending_copies,
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
