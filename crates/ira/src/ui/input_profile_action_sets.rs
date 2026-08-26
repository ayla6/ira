//! The Action Sets editor page: which set is being edited, set management
//! (add/rename/delete), layer management, and the virtual-gamepad backend —
//! Steam Input's "Action Sets" header controls. Every action is a full-width
//! row; raw buttons dropped straight into a preferences group both look
//! wrong and trip GTK widget assertions.

use super::helpers::esc;
use super::input_profile_region_pages::PagesCtx;
use adw::prelude::*;
use ira_input::{ActionSet, ActionSetLayer, VirtualGamepadBackend};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Fired when structural changes require rebuilding the region pages (set
/// switch, set add/delete) on top of the usual dirty marking.
pub(crate) type Rebuild = Rc<dyn Fn()>;

/// One "+ action" row: a full-width activatable row with a plus affordance.
fn add_action_row(group: &adw::PreferencesGroup, label: &str, destructive: bool) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    let content = super::helpers::icon_label_button("list-add-symbolic", label);
    if destructive {
        content.add_css_class(super::css::CSS_ERROR);
    }
    row.add_suffix(&content);
    row.set_activatable(true);
    group.add(&row);
    row
}

pub(crate) fn build_sets_page(ctx: &PagesCtx, rebuild: &Rebuild) -> gtk4::Box {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    page.append(&sets_group(ctx, rebuild));
    page.append(&layers_group(ctx, rebuild));
    page.append(&backend_group(ctx));
    page
}

fn sets_group(ctx: &PagesCtx, rebuild: &Rebuild) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Action Sets"));
    group.set_description(Some(&crate::tr!(
        "Each set is a full layout; switch between them from any input"
    )));

    let combo = adw::ComboRow::new();
    combo.set_title(&crate::tr!("Editing set"));
    refill_set_combo(&combo, &ctx.profile.borrow(), ctx.active_set.get());
    group.add(&combo);

    // The rename row always edits the set the combo points at; the shared
    // cell follows combo switches.
    let rename_target = Rc::new(Cell::new(ctx.active_set.get()));
    let rename = adw::EntryRow::new();
    rename.set_title(&crate::tr!("Set name"));
    if let Some(name) = ctx
        .profile
        .borrow()
        .action_sets
        .get(ctx.active_set.get())
        .map(|set| set.name.clone())
    {
        rename.set_text(&name);
    }
    group.add(&rename);

    let ctx_for_combo = ctx.clone();
    let rebuild_for_combo = rebuild.clone();
    let rename_for_combo = rename.clone();
    let target_for_combo = rename_target.clone();
    combo.connect_selected_notify(move |combo| {
        let count = ctx_for_combo.profile.borrow().action_sets.len() as u32;
        if combo.selected() >= count {
            return;
        }
        let set = combo.selected() as usize;
        ctx_for_combo.active_set.set(set);
        target_for_combo.set(set);
        let name = ctx_for_combo
            .profile
            .borrow()
            .action_sets
            .get(set)
            .map(|set| set.name.clone())
            .unwrap_or_default();
        // Guarded so this never re-enters the rename handler with the same
        // text and loops.
        if rename_for_combo.text() != name {
            rename_for_combo.set_text(&name);
        }
        (ctx_for_combo.on_dirty)();
        rebuild_for_combo();
    });

    let ctx_for_rename = ctx.clone();
    let combo_for_rename = combo.clone();
    rename.connect_changed(move |entry| {
        let text = entry.text().to_string();
        let set = rename_target.get();
        let mut profile = ctx_for_rename.profile.borrow_mut();
        let Some(action_set) = profile.action_sets.get_mut(set) else {
            return;
        };
        if action_set.name == text {
            return;
        }
        action_set.name = text;
        drop(profile);
        refill_set_combo(&combo_for_rename, &ctx_for_rename.profile.borrow(), set);
        (ctx_for_rename.on_dirty)();
    });

    let add_row = add_action_row(&group, &crate::tr!("Add set"), false);
    let ctx_for_add = ctx.clone();
    let combo_for_add = combo.clone();
    let rebuild_for_add = rebuild.clone();
    let rename_for_add = rename.clone();
    add_row.connect_activated(move |_| {
        let mut profile = ctx_for_add.profile.borrow_mut();
        let number = profile.action_sets.len() + 1;
        let name = crate::tr!("Set {number}").replace("{number}", &number.to_string());
        profile.action_sets.push(ActionSet {
            name: name.clone(),
            inputs: Vec::new(),
        });
        let last = profile.action_sets.len() - 1;
        drop(profile);
        ctx_for_add.active_set.set(last);
        if rename_for_add.text() != name {
            rename_for_add.set_text(&name);
        }
        refill_set_combo(&combo_for_add, &ctx_for_add.profile.borrow(), last);
        (ctx_for_add.on_dirty)();
        rebuild_for_add();
    });

    let delete_row = add_action_row(&group, &crate::tr!("Delete set"), true);
    delete_row.set_sensitive(ctx.profile.borrow().action_sets.len() > 1);
    let ctx_for_delete = ctx.clone();
    let combo_for_delete = combo.clone();
    let rebuild_for_delete = rebuild.clone();
    delete_row.connect_activated(move |_| {
        let mut profile = ctx_for_delete.profile.borrow_mut();
        // The first set is the default and always stays.
        if profile.action_sets.len() <= 1 {
            return;
        }
        let removing = ctx_for_delete.active_set.get();
        if removing == 0 || removing >= profile.action_sets.len() {
            return;
        }
        profile.action_sets.remove(removing);
        drop(profile);
        ctx_for_delete.active_set.set(0);
        refill_set_combo(&combo_for_delete, &ctx_for_delete.profile.borrow(), 0);
        (ctx_for_delete.on_dirty)();
        rebuild_for_delete();
    });

    group
}

fn refill_set_combo(combo: &adw::ComboRow, profile: &ira_input::InputProfile, selected: usize) {
    let names: Vec<String> = profile
        .action_sets
        .iter()
        .map(|set| set.name.clone())
        .collect();
    combo.set_model(Some(&super::helpers::string_list_from(&names)));
    combo.set_selected(selected.min(names.len().saturating_sub(1)) as u32);
}

fn layers_group(ctx: &PagesCtx, rebuild: &Rebuild) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Action Set Layers"));
    group.set_description(Some(&crate::tr!(
        "Layers overlay the active set while held or toggled"
    )));

    let add_row = add_action_row(&group, &crate::tr!("Add layer"), false);
    let ctx_for_add = ctx.clone();
    let rebuild_for_layers = rebuild.clone();
    add_row.connect_activated(move |_| {
        let parent = ctx_for_add
            .profile
            .borrow()
            .action_sets
            .get(ctx_for_add.active_set.get())
            .map(|set| set.name.clone())
            .unwrap_or_default();
        {
            let mut profile = ctx_for_add.profile.borrow_mut();
            let number = profile.action_layers.len() + 1;
            profile.action_layers.push(ActionSetLayer {
                name: crate::tr!("Layer {number}").replace("{number}", &number.to_string()),
                parent_set: parent,
                inputs: Vec::new(),
            });
        }
        (ctx_for_add.on_dirty)();
        rebuild_for_layers();
    });

    // Layer rows are rebuilt on every structural change so adds and deletes
    // stay consistent without juggling a separate container widget.
    let layer_rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    refill_layer_rows(ctx, rebuild, &group.clone(), &layer_rows, &add_row);

    group
}

fn refill_layer_rows(
    ctx: &PagesCtx,
    rebuild: &Rebuild,
    group: &adw::PreferencesGroup,
    layer_rows: &Rc<RefCell<Vec<adw::ActionRow>>>,
    add_row: &adw::ActionRow,
) {
    for row in layer_rows.borrow_mut().drain(..) {
        group.remove(&row);
    }
    let layers = ctx.profile.borrow().action_layers.clone();
    for (index, layer) in layers.iter().enumerate() {
        let row = adw::ActionRow::new();
        row.set_title(&esc(&layer.name));
        row.set_subtitle(&esc(
            &crate::tr!("over {parent}").replace("{parent}", &layer.parent_set)
        ));
        let trash = gtk4::Button::from_icon_name("user-trash-symbolic");
        trash.add_css_class(super::css::CSS_FLAT);
        trash.add_css_class(super::css::CSS_SQUARE_BUTTON);
        trash.set_valign(gtk4::Align::Center);
        trash.set_tooltip_text(Some(&crate::tr!("Delete layer")));
        row.add_suffix(&trash);
        let ctx_for_delete = ctx.clone();
        let rebuild_for_delete = rebuild.clone();
        let layer_rows_for_delete = layer_rows.clone();
        let add_row_for_delete = add_row.clone();
        let group_for_delete = group.clone();
        trash.connect_clicked(move |_| {
            let mut profile = ctx_for_delete.profile.borrow_mut();
            if index < profile.action_layers.len() {
                profile.action_layers.remove(index);
            }
            drop(profile);
            (ctx_for_delete.on_dirty)();
            refill_layer_rows(
                &ctx_for_delete,
                &rebuild_for_delete,
                &group_for_delete,
                &layer_rows_for_delete,
                &add_row_for_delete,
            );
        });
        group.add(&row);
        layer_rows.borrow_mut().push(row);
    }
}

/// Backend only changes which virtual device Ira creates for this layout;
/// it never wipes anything, so no confirmation is needed.
fn backend_group(ctx: &PagesCtx) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Virtual gamepad"));
    group.set_description(Some(&crate::tr!("Which controller type the game sees")));

    let labels = [
        crate::tr!("XInput (Xbox)"),
        crate::tr!("DirectInput"),
        crate::tr!("Switch Pro"),
        crate::tr!("DualShock 4"),
        crate::tr!("DualSense"),
        crate::tr!("DSU (cemuhook)"),
    ];
    let combo = super::input_profile_sheet_base::combo_row(
        &labels,
        backend_index(ctx.profile.borrow().backend),
    );
    combo.set_title(&crate::tr!("Backend"));
    group.add(&combo);

    let ctx_for_backend = ctx.clone();
    combo.connect_selected_notify(move |combo| {
        let backend = match combo.selected() {
            1 => VirtualGamepadBackend::DirectInput,
            2 => VirtualGamepadBackend::SwitchPro,
            3 => VirtualGamepadBackend::DualShock4,
            4 => VirtualGamepadBackend::DualSense,
            5 => VirtualGamepadBackend::Dsu,
            _ => VirtualGamepadBackend::XInput,
        };
        ctx_for_backend.profile.borrow_mut().backend = backend;
        (ctx_for_backend.on_dirty)();
    });

    group
}

fn backend_index(backend: VirtualGamepadBackend) -> u32 {
    match backend {
        VirtualGamepadBackend::XInput => 0,
        VirtualGamepadBackend::DirectInput => 1,
        VirtualGamepadBackend::SwitchPro => 2,
        VirtualGamepadBackend::DualShock4 => 3,
        VirtualGamepadBackend::DualSense => 4,
        VirtualGamepadBackend::Dsu => 5,
    }
}
