use super::super::state::SgdbAssetsCacheEntry;
use super::types::{GeometrySync, SgdbPickerCache};
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// Fresh picker window with the persisted zoom and size restored.
/// Returns the window and the zoom it opened at.
pub(super) fn build_picker_window(asset: &str, parent: &adw::Window) -> (adw::Window, i32) {
    // A real resizable window instead of a sheet. Modal over its parent
    // like the settings windows, so focus and keys (Alt+F4 included)
    // belong to it unambiguously; the settings teardown closes cached
    // pickers before dropping their cache.
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
    // Floor the restore: nothing in the UI can produce a smaller window,
    // so anything tinier is a degenerate save that would stick forever.
    let size_w = size_w.max(640);
    let size_h = size_h.max(480);
    let picker = adw::Window::new();
    picker.set_title(Some(&crate::tr!("Pick {}").replacen("{}", asset, 1)));
    picker.set_modal(true);
    picker.set_transient_for(Some(parent));
    // Restore the saved size verbatim: it is user-produced on this
    // machine, so clamping it to the parent would shrink every reopen
    // back to the settings size.
    picker.set_default_size(size_w, size_h);
    (picker, zoom_initial)
}

/// Cache the live window immediately so a reopen (even before the first
/// fetch lands) re-presents it instead of building a fresh one. The fetch
/// handler updates this entry in place.
pub(super) fn register_picker_window(
    sgdb_cache: &Option<SgdbPickerCache>,
    cache_key: String,
    picker: &adw::Window,
) {
    if let Some(cache) = sgdb_cache {
        cache.borrow_mut().insert(
            cache_key,
            SgdbAssetsCacheEntry {
                assets: Vec::new(),
                has_more: true,
                next_page: 0,
                picker: picker.clone(),
            },
        );
    }
}

/// Persist zoom and size on every close — header X, Alt+F4, or the
/// download path closing the window — for the next fresh open. Reopens
/// from the cache keep the live window trivially.
pub(super) fn wire_close_persist(
    picker: &adw::Window,
    asset: &str,
    zoom_level: &Rc<Cell<i32>>,
    on_geometry: &Option<GeometrySync>,
) {
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
