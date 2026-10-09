use super::super::css::*;
use super::super::helpers::clear_children;
use super::cards::{build_sgdb_asset_card, tile_size};
use super::download::build_on_download;
use super::filter::detach_dimmed;
use super::types::SgdbPickerCtx;
use adw::prelude::*;
use ira_api::types::SgdbAsset;

pub(super) fn append_assets(
    flow: &gtk4::FlowBox,
    list_view: &gtk4::ListBox,
    assets: &[SgdbAsset],
    start: usize,
    ctx: &SgdbPickerCtx,
) {
    // The box wraps by width like the game grid, so the column count
    // follows the window size and the zoom level.
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

pub(super) fn full_rebuild(
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
