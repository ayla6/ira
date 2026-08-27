//! The Action Set Layers group: per-set overlay layers, each row opening
//! the layer's bindings in the region pages (layers are first-class binding
//! targets, like Steam Input), with rename and delete alongside. Rows are
//! rebuilt on every structural change so adds, deletes and set switches
//! stay in sync without widget bookkeeping.

use super::css::{CSS_CIRCULAR, CSS_FLAT};
use super::helpers::{confirm_dialog_with_extra, esc};
use super::input_profile_action_sets::Rebuild;
use super::input_profile_region_pages::{EditingTarget, PagesCtx};
use super::input_profile_widgets::SettingGroup;
use adw::prelude::*;
use ira_input::{ActionSetLayer, InputProfile};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub(crate) struct LayersGroup {
    group: SettingGroup,
    rows: Rc<RefCell<Vec<adw::ActionRow>>>,
}

impl LayersGroup {
    pub(crate) fn build(ctx: &PagesCtx, rebuild: &Rebuild) -> Self {
        let group = SettingGroup::new(
            Some(&crate::tr!("Action Set Layers")),
            Some(&crate::tr!(
                "Layers overlay the active set while held or toggled; click one to bind its inputs"
            )),
        );

        let rows = Rc::new(RefCell::new(Vec::new()));
        let add_row = adw::ButtonRow::new();
        add_row.set_title(&crate::tr!("Add layer"));
        add_row.set_start_icon_name(Some("list-add-symbolic"));
        group.add(&add_row);
        {
            let ctx = ctx.clone();
            let rebuild = rebuild.clone();
            let rows = rows.clone();
            let group_for_add = group.clone();
            add_row.connect_activated(move |_| {
                show_add_layer_dialog(&ctx, &rebuild, &group_for_add, &rows);
            });
        }
        let this = Self { group, rows };
        this.refresh(ctx);
        this
    }

    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.group.root
    }

    pub(crate) fn refresh(&self, ctx: &PagesCtx) {
        refill_layer_rows(ctx, &self.group, &self.rows);
    }
}

/// Layers shown for the currently edited set, plus any orphans whose parent
/// set no longer exists (older profiles can carry them; they stay visible
/// and deletable instead of hiding behind a validation error).
fn visible_layers(profile: &InputProfile, active: EditingTarget) -> Vec<(usize, ActionSetLayer)> {
    let active_name = match active {
        EditingTarget::Set(index) => profile.action_sets.get(index).map(|set| set.name.clone()),
        EditingTarget::Layer(index) => profile
            .action_layers
            .get(index)
            .map(|layer| layer.parent_set.clone()),
    };
    let Some(active_name) = active_name else {
        return Vec::new();
    };
    profile
        .action_layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| {
            layer.parent_set == active_name
                || profile
                    .action_sets
                    .iter()
                    .all(|set| set.name != layer.parent_set)
        })
        .map(|(index, layer)| (index, layer.clone()))
        .collect()
}

fn refill_layer_rows(
    ctx: &PagesCtx,
    group: &SettingGroup,
    rows: &Rc<RefCell<Vec<adw::ActionRow>>>,
) {
    for row in rows.borrow_mut().drain(..) {
        group.remove(&row);
    }
    let layers = visible_layers(&ctx.profile.borrow(), ctx.active_target.get());
    let set_names: Vec<String> = ctx
        .profile
        .borrow()
        .action_sets
        .iter()
        .map(|set| set.name.clone())
        .collect();
    for (index, layer) in layers {
        let orphan = set_names.iter().all(|name| name != &layer.parent_set);
        let row = build_layer_row(ctx, group, rows, index, &layer, orphan);
        group.add(&row);
        rows.borrow_mut().push(row);
    }
}

fn build_layer_row(
    ctx: &PagesCtx,
    group: &SettingGroup,
    rows: &Rc<RefCell<Vec<adw::ActionRow>>>,
    index: usize,
    layer: &ActionSetLayer,
    orphan: bool,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(&layer.name));
    row.set_subtitle(&if orphan {
        esc(&crate::tr!("orphaned, was over {parent}").replace("{parent}", &layer.parent_set))
    } else {
        bindings_summary(layer)
    });
    row.set_activatable(true);
    let tooltip = crate::tr!("Edit this layer's bindings");
    row.set_tooltip_text(Some(&tooltip));

    let rename = row_icon_button("document-edit-symbolic", &crate::tr!("Rename layer"));
    let layer_name = layer.name.clone();
    {
        let ctx = ctx.clone();
        let rows = rows.clone();
        let group = group.clone();
        rename.connect_clicked(move |_| {
            show_rename_layer_dialog(&ctx, &group, &rows, index, &layer_name);
        });
    }
    row.add_suffix(&rename);

    let trash = row_icon_button("user-trash-symbolic", &crate::tr!("Delete layer"));
    {
        let ctx = ctx.clone();
        let rows = rows.clone();
        let group = group.clone();
        trash.connect_clicked(move |_| {
            remove_layer(&ctx, index);
            refill_layer_rows(&ctx, &group, &rows);
        });
    }
    row.add_suffix(&trash);

    // Activating the row jumps the region pages (and the top-left
    // indicator) to this layer's bindings.
    {
        let ctx = ctx.clone();
        row.connect_activated(move |_| {
            ctx.active_target.set(EditingTarget::Layer(index));
            ctx.indicator.update(&ctx);
            (ctx.on_dirty)();
        });
    }
    row
}

fn bindings_summary(layer: &ActionSetLayer) -> String {
    let count = layer.inputs.len();
    match count {
        0 => crate::tr!("No bindings yet"),
        1 => crate::tr!("1 binding"),
        _ => crate::tr!("{count} bindings").replace("{count}", &count.to_string()),
    }
}

fn row_icon_button(icon: &str, tooltip: &str) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name(icon);
    button.add_css_class(CSS_FLAT);
    button.add_css_class(CSS_CIRCULAR);
    button.set_valign(gtk4::Align::Center);
    button.set_tooltip_text(Some(tooltip));
    button
}

/// Removing a layer shifts later layer indices everywhere, so the profile
/// cascade remaps the EnableLayer outputs; an edit session left pointing at
/// the removed layer falls back to its parent set.
fn remove_layer(ctx: &PagesCtx, index: usize) {
    let parent = ctx
        .profile
        .borrow()
        .action_layers
        .get(index)
        .map(|layer| layer.parent_set.clone());
    ctx.profile.borrow_mut().remove_action_layer(index);
    if ctx.active_target.get() == EditingTarget::Layer(index) {
        let parent_index = parent
            .and_then(|name| {
                ctx.profile
                    .borrow()
                    .action_sets
                    .iter()
                    .position(|set| set.name == name)
            })
            .unwrap_or(0);
        ctx.active_target.set(EditingTarget::Set(parent_index));
        ctx.indicator.update(ctx);
    }
    (ctx.on_dirty)();
}

fn show_add_layer_dialog(
    ctx: &PagesCtx,
    rebuild: &Rebuild,
    group: &SettingGroup,
    rows: &Rc<RefCell<Vec<adw::ActionRow>>>,
) {
    let parent_name = parent_set_name(ctx);
    let entry = adw::EntryRow::new();
    entry.set_title(&crate::tr!("Layer name"));
    entry.set_text(&free_layer_name(ctx));
    {
        let ctx = ctx.clone();
        let rebuild = rebuild.clone();
        let rows = rows.clone();
        let group = group.clone();
        let window = ctx.window.clone();
        let entry_for_confirm = entry.clone();
        confirm_dialog_with_extra(
            &window,
            &crate::tr!("Add Layer to set {set}").replace("{set}", &parent_name),
            &crate::tr!("Enter a name for the new layer"),
            &entry,
            move || {
                let name = entry_for_confirm.text().trim().to_string();
                if name.is_empty() {
                    return;
                }
                ctx.profile.borrow_mut().action_layers.push(ActionSetLayer {
                    name,
                    parent_set: parent_name.clone(),
                    inputs: Vec::new(),
                });
                refill_layer_rows(&ctx, &group, &rows);
                ctx.indicator.update(&ctx);
                (ctx.on_dirty)();
                rebuild();
            },
        );
    }
}

fn show_rename_layer_dialog(
    ctx: &PagesCtx,
    group: &SettingGroup,
    rows: &Rc<RefCell<Vec<adw::ActionRow>>>,
    index: usize,
    current: &str,
) {
    let entry = adw::EntryRow::new();
    entry.set_title(&crate::tr!("Layer name"));
    entry.set_text(current);
    {
        let ctx = ctx.clone();
        let rows = rows.clone();
        let group = group.clone();
        let current = current.to_string();
        let window = ctx.window.clone();
        let entry_for_confirm = entry.clone();
        confirm_dialog_with_extra(
            &window,
            &crate::tr!("Rename layer"),
            &crate::tr!("Enter a new name"),
            &entry,
            move || {
                let name = entry_for_confirm.text().trim().to_string();
                if name.is_empty() || name == current {
                    return;
                }
                if let Some(layer) = ctx.profile.borrow_mut().action_layers.get_mut(index) {
                    layer.name = name;
                }
                refill_layer_rows(&ctx, &group, &rows);
                ctx.indicator.update(&ctx);
                (ctx.on_dirty)();
            },
        );
    }
}

/// The set newly created layers belong to: whatever the region pages are
/// editing — a layer target counts as its parent set.
fn parent_set_name(ctx: &PagesCtx) -> String {
    let profile = ctx.profile.borrow();
    match ctx.active_target.get() {
        EditingTarget::Set(index) => profile
            .action_sets
            .get(index)
            .map(|set| set.name.clone())
            .unwrap_or_default(),
        EditingTarget::Layer(index) => profile
            .action_layers
            .get(index)
            .map(|layer| layer.parent_set.clone())
            .unwrap_or_default(),
    }
}

/// First "Layer N" that does not collide with an existing layer.
fn free_layer_name(ctx: &PagesCtx) -> String {
    let profile = ctx.profile.borrow();
    let mut number = 1;
    loop {
        let candidate = crate::tr!("Layer {number}").replace("{number}", &number.to_string());
        if profile.is_free_layer_name(&candidate) {
            return candidate;
        }
        number += 1;
    }
}
