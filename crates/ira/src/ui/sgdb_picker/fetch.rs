use super::super::state::SgdbAssetsCacheEntry;
use super::cards::paint_card;
use super::types::{SgdbPickerCache, SgdbPickerCtx};
use adw::prelude::*;
use ira_api::types::SgdbAsset;
use ira_api::SteamDataClient;
use ira_models::AssetType;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// Render straight from the settings-screen cache instead of re-issuing
/// the network request. Pagination continues where the previous session
/// left off.
pub(super) fn render_cached_entry(
    ctx: &SgdbPickerCtx,
    assets_store: &Rc<RefCell<Vec<SgdbAsset>>>,
    entry: SgdbAssetsCacheEntry,
    do_full_rebuild: &Rc<dyn Fn()>,
    loading_grid: &gtk4::Label,
    loading_list: &gtk4::Label,
) {
    *assets_store.borrow_mut() = entry.assets;
    ctx.has_more.set(entry.has_more);
    ctx.current_page.set(entry.next_page);
    ctx.is_initial_load.set(false);
    loading_grid.set_text("");
    loading_grid.set_visible(false);
    loading_list.set_text("");
    loading_list.set_visible(false);
    do_full_rebuild();
}

/// First SGDB page load: fetch on a worker thread, then a short-interval
/// poller receives the list once and renders it. The poller dies with
/// the window.
pub(super) fn fetch_first_page(
    ctx: &SgdbPickerCtx,
    dimensions: &[&str],
    loading_grid: &gtk4::Label,
    loading_list: &gtk4::Label,
    sgdb_cache: &Option<SgdbPickerCache>,
    cache_key: &str,
) {
    let (tx, rx) = std::sync::mpsc::channel::<(Vec<SgdbAsset>, bool)>();
    let rx = std::cell::RefCell::new(rx);
    let steam_c = ctx.steam.clone();
    let id_c = ctx.id.clone();
    let asset_at = AssetType::from_string(&ctx.asset).unwrap_or(AssetType::Icon);
    let is_steam_id = ctx.is_steam_id;
    let dims: Vec<String> = dimensions.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let dims_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let results = steam_c.list_sgdb_assets(&id_c, asset_at, is_steam_id, &dims_refs, 0);
        let _ = tx.send(results);
    });

    let assets_store_t = ctx.store.clone();
    let do_append_t = {
        let ctx = ctx.clone();
        Rc::new(move || ctx.append_new()) as Rc<dyn Fn()>
    };
    let do_full_rebuild_t = {
        let ctx = ctx.clone();
        Rc::new(move || ctx.rebuild_all()) as Rc<dyn Fn()>
    };
    let has_more_t = ctx.has_more.clone();
    let current_page_t = ctx.current_page.clone();
    let loading_more_t = ctx.loading_more.clone();
    let is_initial_load_t = ctx.is_initial_load.clone();
    let loading_grid = loading_grid.clone();
    let loading_list = loading_list.clone();
    let picker_weak = ctx.picker.clone();
    let sgdb_cache_t = sgdb_cache.as_ref().map(Rc::downgrade);
    let cache_key_t = cache_key.to_string();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if picker_weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        if let Ok((new_assets, more)) = rx.borrow_mut().try_recv() {
            if is_initial_load_t.get() {
                *assets_store_t.borrow_mut() = new_assets.clone();
                is_initial_load_t.set(false);
                loading_grid.set_text("");
                loading_grid.set_visible(false);
                loading_list.set_text("");
                loading_list.set_visible(false);
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

/// One ticker polls every card's thumbnail receiver. A timeout per card
/// saturated the main loop past a few hundred cards — starving input and
/// window close — while one callback sipping try_recv is nothing. It
/// lives as long as the window; hidden cached windows just idle
/// through it.
pub(super) fn start_thumbnail_ticker(ctx: &SgdbPickerCtx) {
    let tick_ctx = ctx.clone();
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
        if steam.asset_filtered(
            super::cards::tag_author(&name),
            super::cards::tag_style(&name),
        ) {
            continue;
        }
        if let Some(rect) = w.compute_bounds(flow) {
            bottom = Some(bottom.unwrap_or(0.0).max((rect.y() + rect.height()) as f64));
        }
    }
    bottom
}

/// Infinite scroll: near the bottom, fetch the next SGDB page in the
/// background and append it. The guards (initial load, exhaustion, one
/// flight at a time, live visible window) keep it from stampeding, and
/// pagination resumes in the settings-screen cache.
pub(super) fn start_infinite_scroll(
    ctx: &SgdbPickerCtx,
    scrolled: &gtk4::ScrolledWindow,
    dimensions: &[&str],
    do_append: &Rc<dyn Fn()>,
    sgdb_cache: &Option<SgdbPickerCache>,
    cache_key: &str,
    toggle_btn: &gtk4::ToggleButton,
) {
    let vadj = scrolled.vadjustment();
    let steam_scroll = ctx.steam.clone();
    let id_scroll = ctx.id.clone();
    let asset_scroll = ctx.asset.clone();
    let is_steam_id = ctx.is_steam_id;
    let dims_scroll: Vec<String> = dimensions.iter().map(|s| s.to_string()).collect();
    let assets_store_scroll = ctx.store.clone();
    let do_append_scroll = do_append.clone();
    let has_more_scroll = ctx.has_more.clone();
    let loading_more_scroll = ctx.loading_more.clone();
    let current_page_scroll = ctx.current_page.clone();
    let picker_scroll = ctx.picker.clone();
    let sgdb_cache_scroll = sgdb_cache.as_ref().map(Rc::downgrade);
    let cache_key_scroll = cache_key.to_string();
    let is_initial_load_scroll = ctx.is_initial_load.clone();
    let flow_scroll = ctx.flow.clone();
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
