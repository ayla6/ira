use super::cards::{list_thumb_size, paint_card};
use super::fetch::{
    fetch_first_page, render_cached_entry, start_infinite_scroll, start_thumbnail_ticker,
};
use super::geometry::{build_picker_window, register_picker_window, wire_close_persist};
use super::types::{SgdbPickerCache, SgdbPickerCtx, ShowSgdbPickerParams, ThumbPool};
use super::views::build_picker_views;
use adw::prelude::*;
use ira_api::types::SgdbAsset;
use ira_models::AssetType;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Re-present the cached picker for this exact query (open, or closed
/// but still cache-owned), with its loaded thumbnails and scroll intact
/// instead of rebuilding everything and refetching the list.
/// True when there was anything to re-present.
fn reopen_cached_picker(sgdb_cache: &Option<SgdbPickerCache>, cache_key: &str) -> bool {
    let cached = sgdb_cache
        .as_ref()
        .and_then(|c| c.borrow().get(cache_key).map(|entry| entry.picker.clone()));
    if let Some(picker) = cached {
        // No refit: the live window keeps whatever size it has.
        picker.present();
        true
    } else {
        false
    }
}

/// Re-showing a hidden picker resets any buttons left disabled by a
/// previous download so the same window is ready to pick again.
fn wire_reopen_reset(
    picker: &adw::Window,
    all_buttons: &Rc<RefCell<Vec<glib::WeakRef<gtk4::Button>>>>,
) {
    let reset_buttons = all_buttons.clone();
    picker.connect_map(move |_| {
        for b in reset_buttons.borrow().iter() {
            if let Some(b) = b.upgrade() {
                b.set_sensitive(true);
                b.set_label(&crate::tr!("Download"));
            }
        }
    });
}

fn wire_view_toggle(toggle_btn: &gtk4::ToggleButton, stack: &gtk4::Stack) {
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
}

/// Zoom slider resizes cards in place: rebuilding would respawn every
/// thumbnail download on each slider tick.
fn wire_zoom(zoom_scale: &gtk4::Scale, ctx: &SgdbPickerCtx) {
    let zoom_level_z = ctx.zoom_level.clone();
    let zoom_pics = ctx.pics.clone();
    let zoom_aspect = ctx.tile_aspect;
    zoom_scale.connect_value_changed(move |s| {
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
}

pub(crate) fn show_sgdb_picker(params: ShowSgdbPickerParams) {
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

    if reopen_cached_picker(&sgdb_cache, &cache_key) {
        return;
    }

    let (picker, zoom_initial) = build_picker_window(asset, parent);
    let (outer, views) = build_picker_views(asset, zoom_initial);

    picker.set_content(Some(&outer));
    picker.present();

    register_picker_window(&sgdb_cache, cache_key.clone(), &picker);

    let assets_store: Rc<RefCell<Vec<SgdbAsset>>> = Rc::new(RefCell::new(Vec::new()));
    let zoom_level = Rc::new(Cell::new(zoom_initial));
    wire_close_persist(&picker, asset, &zoom_level, &on_geometry);
    let rendered_count: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let all_buttons: Rc<RefCell<Vec<glib::WeakRef<gtk4::Button>>>> =
        Rc::new(RefCell::new(Vec::new()));

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
        has_more: Rc::new(Cell::new(true)),
        current_page: Rc::new(Cell::new(0)),
        loading_more: Rc::new(Cell::new(false)),
        is_initial_load: Rc::new(Cell::new(true)),
        tile_aspect: AssetType::from_string(asset)
            .map(|at| at.tile_aspect())
            .unwrap_or(1.0),
        pool: ThumbPool::new(steam),
        pics: Rc::new(RefCell::new(Vec::new())),
        all_buttons: all_buttons.clone(),
        flow: views.flow.downgrade(),
        list_view: views.list_view.downgrade(),
        filter_user,
        unfilter_user,
    };

    wire_reopen_reset(&picker, &all_buttons);

    let do_full_rebuild = {
        let ctx = picker_ctx.clone();
        Rc::new(move || ctx.rebuild_all()) as Rc<dyn Fn()>
    };

    let do_append = {
        let ctx = picker_ctx.clone();
        Rc::new(move || ctx.append_new()) as Rc<dyn Fn()>
    };

    wire_view_toggle(&views.toggle_btn, &views.stack);

    if let Some(entry) = sgdb_cache
        .as_ref()
        .and_then(|c| c.borrow().get(&cache_key).cloned())
        .filter(|e| !e.assets.is_empty())
    {
        // Pagination continues where the previous session left off:
        // render straight from the settings-screen cache.
        render_cached_entry(
            &picker_ctx,
            &assets_store,
            entry,
            &do_full_rebuild,
            &views.loading_grid,
            &views.loading_list,
        );
    } else {
        fetch_first_page(
            &picker_ctx,
            dimensions,
            &views.loading_grid,
            &views.loading_list,
            &sgdb_cache,
            &cache_key,
        );
    }

    wire_zoom(&views.zoom_scale, &picker_ctx);
    start_thumbnail_ticker(&picker_ctx);
    start_infinite_scroll(
        &picker_ctx,
        &views.scrolled,
        dimensions,
        &do_append,
        &sgdb_cache,
        &cache_key,
        &views.toggle_btn,
    );
}

#[cfg(test)]
mod tests {
    use super::super::cards::build_sgdb_asset_card;
    use crate::ui::state::SgdbAssetsCacheEntry;
    use super::*;
    use ira_api::types::SgdbAsset;
    use std::collections::HashMap;

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
            has_more: Rc::new(Cell::new(true)),
            current_page: Rc::new(Cell::new(0)),
            loading_more: Rc::new(Cell::new(false)),
            is_initial_load: Rc::new(Cell::new(true)),
            tile_aspect: 2.0 / 3.0,
            pool: ThumbPool::new(&steam),
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
        assert_eq!(win.width(), 900);
        assert_eq!(win.height(), 700);
        win.close();
        pump();
        assert!(!win.is_visible(), "first close did not hide");
        open();
        pump();
        assert!(win.is_visible(), "cache-hit reopen never became visible");
        // Default size stands in for a user resize: headless has no
        // compositor to drag against.
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
            has_more: Rc::new(Cell::new(true)),
            current_page: Rc::new(Cell::new(0)),
            loading_more: Rc::new(Cell::new(false)),
            is_initial_load: Rc::new(Cell::new(true)),
            tile_aspect: 2.0 / 3.0,
            pool: ThumbPool::new(&steam),
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
