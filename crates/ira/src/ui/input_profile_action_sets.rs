//! The Action Sets editor page: which set is being edited, set management
//! (add/rename/delete), layer management, and the virtual-gamepad backend —
//! Steam Input's "Action Sets" header controls.

use super::helpers::esc;
use super::input_profile_region_pages::PagesCtx;
use adw::prelude::*;
use ira_input::{ActionSet, ActionSetLayer, VirtualGamepadBackend};
use std::cell::Cell;
use std::rc::Rc;

/// Fired when structural changes require rebuilding the region pages (set
/// switch, set add/delete) on top of the usual dirty marking.
pub(crate) type Rebuild = Rc<dyn Fn()>;

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

    let add = gtk4::Button::with_label(&crate::tr!("Add set"));
    add.add_css_class(super::css::CSS_FLAT);
    add.set_halign(gtk4::Align::Start);
    let ctx_for_add = ctx.clone();
    let combo_for_add = combo.clone();
    let rebuild_for_add = rebuild.clone();
    let rename_for_add = rename.clone();
    add.connect_clicked(move |_| {
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
    group.add(&add);

    let delete = gtk4::Button::with_label(&crate::tr!("Delete set"));
    delete.add_css_class(super::css::CSS_FLAT);
    delete.set_halign(gtk4::Align::Start);
    let ctx_for_delete = ctx.clone();
    let combo_for_delete = combo.clone();
    let rebuild_for_delete = rebuild.clone();
    delete.connect_clicked(move |_| {
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
    group.add(&delete);

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

    // Layer rows live in their own container so layer add/delete can clear
    // and refill them without touching the rest of the page.
    let layers_list = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    refill_layer_rows(ctx, rebuild, &layers_list);
    group.add(&layers_list);

    let add = gtk4::Button::with_label(&crate::tr!("Add layer"));
    add.add_css_class(super::css::CSS_FLAT);
    add.set_halign(gtk4::Align::Start);
    let ctx_for_add = ctx.clone();
    let layers_list_for_add = layers_list.clone();
    let rebuild_for_layers = rebuild.clone();
    add.connect_clicked(move |_| {
        let parent = ctx_for_add
            .profile
            .borrow()
            .action_sets
            .get(ctx_for_add.active_set.get())
            .map(|set| set.name.clone())
            .unwrap_or_default();
        let mut profile = ctx_for_add.profile.borrow_mut();
        let number = profile.action_layers.len() + 1;
        profile.action_layers.push(ActionSetLayer {
            name: crate::tr!("Layer {number}").replace("{number}", &number.to_string()),
            parent_set: parent,
            inputs: Vec::new(),
        });
        drop(profile);
        (ctx_for_add.on_dirty)();
        refill_layer_rows(&ctx_for_add, &rebuild_for_layers, &layers_list_for_add);
    });
    group.add(&add);

    group
}

fn refill_layer_rows(ctx: &PagesCtx, rebuild: &Rebuild, container: &gtk4::Box) {
    super::helpers::clear_children(container);
    let layers = ctx.profile.borrow().action_layers.clone();
    for (index, layer) in layers.iter().enumerate() {
        let row = adw::ActionRow::new();
        row.set_title(&esc(&layer.name));
        row.set_subtitle(&esc(
            &crate::tr!("over {parent}").replace("{parent}", &layer.parent_set),
        ));
        let trash = gtk4::Button::from_icon_name("user-trash-symbolic");
        trash.add_css_class(super::css::CSS_FLAT);
        trash.add_css_class(super::css::CSS_SQUARE_BUTTON);
        trash.set_valign(gtk4::Align::Center);
        trash.set_tooltip_text(Some(&crate::tr!("Delete layer")));
        row.add_suffix(&trash);
        let ctx_for_delete = ctx.clone();
        let container_for_delete = container.clone();
        let rebuild_for_delete = rebuild.clone();
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
                &container_for_delete,
            );
        });
        container.append(&row);
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
    ];
    let combo = super::input_profile_activator_sheet::combo_row(
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
    }
}
