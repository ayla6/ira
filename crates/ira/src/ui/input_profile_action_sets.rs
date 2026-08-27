//! The Action Sets editor page: which set is being edited, set management
//! (add with layout copy / rename / delete), the layer group, and Steam
//! Input's Global Set Options (auto-switching on mouse cursor visibility).
//! Structural edits run through the profile's cascade methods so layer
//! parenting and binding indices stay consistent.

use super::helpers::{confirm_dialog_with_extra, string_list_from};
use super::input_profile_layers::LayersGroup;
use super::input_profile_region_pages::{EditingTarget, PagesCtx};
use super::input_profile_widgets::SettingGroup;
use adw::prelude::*;
use ira_input::{ActionSet, InputProfile};
use std::cell::Cell;
use std::rc::Rc;

/// Fired when structural changes require rebuilding the region pages (set
/// switch, set add/delete) on top of the usual dirty marking.
pub(crate) type Rebuild = Rc<dyn Fn()>;

pub(crate) fn build_sets_page(ctx: &PagesCtx, rebuild: &Rebuild) -> gtk4::Box {
    // The top-left indicator starts empty until a page touches it.
    ctx.indicator.update(ctx);
    let layers = LayersGroup::build(ctx, rebuild);
    let cursor = CursorOptions::build(ctx);
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    page.append(&sets_group(ctx, rebuild, &layers, &cursor));
    page.append(layers.widget());
    page.append(&cursor.group.root);
    page
}

/// One full-width action row: a labelled button with a leading icon, the
/// boxed-list way to offer "Add set" / "Delete set".
fn button_row(label: &str, icon: &str, destructive: bool) -> adw::ButtonRow {
    let row = adw::ButtonRow::new();
    row.set_title(label);
    row.set_start_icon_name(Some(icon));
    if destructive {
        row.add_css_class(super::css::CSS_DESTRUCTIVE_ACTION);
    }
    row
}

/// The set the combo and rename row follow; editing a layer pins the combo
/// to that layer's parent set.
fn combo_set_index(ctx: &PagesCtx) -> usize {
    let profile = ctx.profile.borrow();
    match ctx.active_target.get() {
        EditingTarget::Set(index) => index,
        EditingTarget::Layer(index) => profile
            .action_layers
            .get(index)
            .map(|layer| layer.parent_set.clone())
            .and_then(|name| {
                profile
                    .action_sets
                    .iter()
                    .position(|set| set.name == name)
            })
            .unwrap_or(0),
    }
}

fn sets_group(
    ctx: &PagesCtx,
    rebuild: &Rebuild,
    layers: &LayersGroup,
    cursor: &CursorOptions,
) -> gtk4::Box {
    let group = SettingGroup::new(
        Some(&crate::tr!("Action Sets")),
        Some(&crate::tr!(
            "Each set is a full layout; switch between them from any input"
        )),
    );

    let combo = adw::ComboRow::new();
    combo.set_title(&crate::tr!("Editing set"));
    refill_set_combo(&combo, &ctx.profile.borrow(), combo_set_index(ctx));
    group.add(&combo);

    // The rename row always edits the set the combo points at; the shared
    // cell follows combo switches and is updated BEFORE any programmatic
    // text change, so the changed handler never renames a stale target.
    let rename_target = Rc::new(Cell::new(combo_set_index(ctx)));
    let rename = adw::EntryRow::new();
    rename.set_title(&crate::tr!("Set name"));
    if let Some(name) = ctx
        .profile
        .borrow()
        .action_sets
        .get(rename_target.get())
        .map(|set| set.name.clone())
    {
        rename.set_text(&name);
    }
    group.add(&rename);

    connect_combo_switch(ctx, &combo, &rename, &rename_target, layers);
    connect_rename(ctx, &combo, &rename, &rename_target, cursor);

    let add_row = button_row(&crate::tr!("Add set"), "list-add-symbolic", false);
    group.add(&add_row);
    {
        let ctx = ctx.clone();
        let combo = combo.clone();
        let rebuild = rebuild.clone();
        let rename = rename.clone();
        let rename_target = rename_target.clone();
        let layers = layers.clone();
        let cursor = cursor.clone();
        add_row.connect_activated(move |_| {
            show_add_set_dialog(&ctx, &combo, &rename, &rename_target, &layers, &cursor, &rebuild);
        });
    }

    let delete_row = button_row(&crate::tr!("Delete set"), "user-trash-symbolic", true);
    group.add(&delete_row);
    delete_row.set_sensitive(ctx.profile.borrow().action_sets.len() > 1);
    {
        let ctx = ctx.clone();
        let combo = combo.clone();
        let rebuild = rebuild.clone();
        let rename = rename.clone();
        let rename_target = rename_target.clone();
        let layers = layers.clone();
        let cursor = cursor.clone();
        delete_row.connect_activated(move |_| {
            delete_active_set(&ctx, &combo, &rename, &rename_target, &layers, &cursor, &rebuild);
        });
    }

    group.root
}

fn connect_combo_switch(
    ctx: &PagesCtx,
    combo: &adw::ComboRow,
    rename: &adw::EntryRow,
    rename_target: &Rc<Cell<usize>>,
    layers: &LayersGroup,
) {
    let ctx = ctx.clone();
    let rename = rename.clone();
    let rename_target = rename_target.clone();
    let layers = layers.clone();
    combo.connect_selected_notify(move |combo| {
        let count = ctx.profile.borrow().action_sets.len() as u32;
        if combo.selected() >= count {
            return;
        }
        let set = combo.selected() as usize;
        ctx.active_target.set(EditingTarget::Set(set));
        rename_target.set(set);
        let name = ctx
            .profile
            .borrow()
            .action_sets
            .get(set)
            .map(|set| set.name.clone())
            .unwrap_or_default();
        // Guarded so this never re-enters the rename handler with the same
        // text and loops.
        if rename.text() != name {
            rename.set_text(&name);
        }
        layers.refresh(&ctx);
        ctx.indicator.update(&ctx);
        (ctx.on_dirty)();
    });
}

/// Renaming goes through the profile's cascade so layers keep their parent.
fn connect_rename(
    ctx: &PagesCtx,
    combo: &adw::ComboRow,
    rename: &adw::EntryRow,
    rename_target: &Rc<Cell<usize>>,
    cursor: &CursorOptions,
) {
    let ctx = ctx.clone();
    let combo = combo.clone();
    let rename_target = rename_target.clone();
    let cursor = cursor.clone();
    rename.connect_changed(move |entry| {
        let text = entry.text().trim().to_string();
        let set = rename_target.get();
        {
            let profile = ctx.profile.borrow();
            match profile.action_sets.get(set) {
                Some(action_set) if action_set.name != text => {}
                _ => return,
            }
        }
        ctx.profile.borrow_mut().rename_action_set(set, text);
        refill_set_combo(&combo, &ctx.profile.borrow(), set);
        cursor.refresh(&ctx);
        ctx.indicator.update(&ctx);
        (ctx.on_dirty)();
    });
}

fn show_add_set_dialog(
    ctx: &PagesCtx,
    combo: &adw::ComboRow,
    rename: &adw::EntryRow,
    rename_target: &Rc<Cell<usize>>,
    layers: &LayersGroup,
    cursor: &CursorOptions,
    rebuild: &Rebuild,
) {
    let entry = adw::EntryRow::new();
    entry.set_title(&crate::tr!("Set name"));
    entry.set_text(&free_set_name(ctx));
    let copy = adw::ComboRow::new();
    copy.set_title(&crate::tr!("Copy layout from set"));
    let mut copy_labels = vec![crate::tr!("None")];
    copy_labels.extend(set_names(&ctx.profile.borrow()));
    refill_combo(&copy, &copy_labels, 0);

    let layout_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    layout_box.append(&entry);
    layout_box.append(&copy);

    let ctx = ctx.clone();
    let combo = combo.clone();
    let rebuild = rebuild.clone();
    let rename = rename.clone();
    let rename_target = rename_target.clone();
    let layers = layers.clone();
    let cursor = cursor.clone();
    let window = ctx.window.clone();
    confirm_dialog_with_extra(
        &window,
        &crate::tr!("Add Action Set"),
        &crate::tr!("Enter a name for the new action set"),
        &layout_box,
        move || {
            let name = entry.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let copied = {
                let profile = ctx.profile.borrow();
                (copy.selected() > 0)
                    .then(|| copy.selected() as usize - 1)
                    .and_then(|index| profile.action_sets.get(index))
                    .map(|set| set.inputs.clone())
            };
            ctx.profile.borrow_mut().action_sets.push(ActionSet {
                name: name.clone(),
                inputs: copied.unwrap_or_default(),
            });
            let last = ctx.profile.borrow().action_sets.len() - 1;
            ctx.active_target.set(EditingTarget::Set(last));
            // Order matters: the rename target must follow the new set
            // before the entry text changes, or the changed handler renames
            // whatever set was selected before (the old "Set 2" bug).
            rename_target.set(last);
            if rename.text() != name {
                rename.set_text(&name);
            }
            refill_set_combo(&combo, &ctx.profile.borrow(), last);
            layers.refresh(&ctx);
            cursor.refresh(&ctx);
            ctx.indicator.update(&ctx);
            (ctx.on_dirty)();
            rebuild();
        },
    );
}

fn delete_active_set(
    ctx: &PagesCtx,
    combo: &adw::ComboRow,
    rename: &adw::EntryRow,
    rename_target: &Rc<Cell<usize>>,
    layers: &LayersGroup,
    cursor: &CursorOptions,
    rebuild: &Rebuild,
) {
    let removing = rename_target.get();
    {
        let profile = ctx.profile.borrow();
        // The first set is the default and always stays.
        if profile.action_sets.len() <= 1 || removing == 0 || removing >= profile.action_sets.len()
        {
            return;
        }
    }
    ctx.profile.borrow_mut().remove_action_set(removing);
    let default_name = ctx
        .profile
        .borrow()
        .action_sets
        .first()
        .map(|set| set.name.clone())
        .unwrap_or_default();
    ctx.active_target.set(EditingTarget::Set(0));
    rename_target.set(0);
    if rename.text() != default_name {
        rename.set_text(&default_name);
    }
    refill_set_combo(combo, &ctx.profile.borrow(), 0);
    layers.refresh(ctx);
    cursor.refresh(ctx);
    ctx.indicator.update(ctx);
    (ctx.on_dirty)();
    rebuild();
}

fn refill_set_combo(combo: &adw::ComboRow, profile: &InputProfile, selected: usize) {
    let names = set_names(profile);
    refill_combo(combo, &names, selected);
}

fn refill_combo(combo: &adw::ComboRow, names: &[String], selected: usize) {
    combo.set_model(Some(&string_list_from(names)));
    combo.set_selected(selected.min(names.len().saturating_sub(1)) as u32);
}

fn set_names(profile: &InputProfile) -> Vec<String> {
    profile
        .action_sets
        .iter()
        .map(|set| set.name.clone())
        .collect()
}

/// First "Set N" that does not collide with an existing set.
fn free_set_name(ctx: &PagesCtx) -> String {
    let profile = ctx.profile.borrow();
    let mut number = 2;
    loop {
        let candidate = crate::tr!("Set {number}").replace("{number}", &number.to_string());
        if profile.is_free_set_name(&candidate) {
            return candidate;
        }
        number += 1;
    }
}

/// Steam Input's Global Set Options: switch to a chosen action set whenever
/// the game shows or hides the mouse cursor, driven by the engine's cursor
/// watcher.
#[derive(Clone)]
struct CursorOptions {
    group: SettingGroup,
    shown: adw::ComboRow,
    hidden: adw::ComboRow,
}

const CURSOR_NONE_LABEL: &str = "None";

impl CursorOptions {
    fn build(ctx: &PagesCtx) -> Self {
        let group = SettingGroup::new(
            Some(&crate::tr!("Global Set Options")),
            Some(&crate::tr!(
                "Games show the mouse cursor when they expect you to interact with an interface, \
                 like an inventory screen. The chosen action sets switch in automatically."
            )),
        );

        let shown = adw::ComboRow::new();
        shown.set_title(&crate::tr!("Action set when cursor shown"));
        let hidden = adw::ComboRow::new();
        hidden.set_title(&crate::tr!("Action set when cursor hidden"));
        let this = Self { group, shown, hidden };
        this.refresh(ctx);

        for (combo, field) in [
            (this.shown.clone(), CursorField::Shown),
            (this.hidden.clone(), CursorField::Hidden),
        ] {
            let ctx = ctx.clone();
            combo.connect_selected_notify(move |combo| {
                let value = (combo.selected() > 0).then(|| combo.selected() as usize - 1);
                let mut profile = ctx.profile.borrow_mut();
                let current = match field {
                    CursorField::Shown => profile.action_set_when_cursor_shown,
                    CursorField::Hidden => profile.action_set_when_cursor_hidden,
                };
                if current == value {
                    return;
                }
                match field {
                    CursorField::Shown => profile.action_set_when_cursor_shown = value,
                    CursorField::Hidden => profile.action_set_when_cursor_hidden = value,
                }
                drop(profile);
                (ctx.on_dirty)();
            });
        }
        this
    }

    /// Rebuilds both option lists from the current set names, keeping the
    /// stored targets selected. Values are copied out first: updating the
    /// combos fires their change handlers, which borrow the profile.
    fn refresh(&self, ctx: &PagesCtx) {
        let (labels, shown, hidden) = {
            let profile = ctx.profile.borrow();
            let mut labels = vec![String::from(CURSOR_NONE_LABEL)];
            labels.extend(set_names(&profile));
            (
                labels,
                profile.action_set_when_cursor_shown,
                profile.action_set_when_cursor_hidden,
            )
        };
        for (combo, value) in [(&self.shown, shown), (&self.hidden, hidden)] {
            let selected =
                (value.map(|target| target + 1).unwrap_or(0) as u32).min(labels.len() as u32 - 1);
            combo.set_model(Some(&string_list_from(&labels)));
            combo.set_selected(selected);
        }
    }
}

enum CursorField {
    Shown,
    Hidden,
}
