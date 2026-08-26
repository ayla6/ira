//! General page for the profile editor: the layout's name and the virtual
//! gamepad backend — the identity settings every other page builds on.

use super::input_profile_region_pages::PagesCtx;
use adw::prelude::*;
use ira_input::VirtualGamepadBackend;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) fn build_general_page(ctx: &PagesCtx, name: &Rc<RefCell<String>>) -> gtk4::Box {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 10);

    let identity = adw::PreferencesGroup::new();
    identity.set_title(&crate::tr!("Profile"));
    let name_row = adw::EntryRow::new();
    name_row.set_title(&crate::tr!("Profile name"));
    name_row.set_text(&name.borrow());
    let name_for_change = name.clone();
    let ctx_for_name = ctx.clone();
    name_row.connect_changed(move |entry| {
        *name_for_change.borrow_mut() = entry.text().to_string();
        (ctx_for_name.on_dirty)();
    });
    identity.add(&name_row);
    page.append(&identity);

    page.append(&backend_group(ctx));
    page
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
