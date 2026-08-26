//! Whole-controller output settings for the profile editor: rumble
//! passthrough on the Controller page and the native-motion transport on
//! the Gyro page. These apply to the pad as a whole rather than any region.

use super::input_profile_widgets::{switch_row, SettingGroup};
use adw::prelude::*;
use ira_input::{InputProfile, VirtualGamepadBackend};
use std::cell::RefCell;
use std::rc::Rc;

/// The profile-level motion transport rows. The editor keeps this handle so
/// [`refresh_motion_rows`] can re-evaluate visibility whenever the profile's
/// backend changes.
#[derive(Clone)]
pub(crate) struct MotionRows {
    pub(crate) native: adw::SwitchRow,
}

impl MotionRows {
    fn apply_backend(&self, backend: VirtualGamepadBackend) {
        // With the DSU backend the cemuhook stream IS the transport for the
        // whole controller, so native motion does not apply.
        self.native
            .set_visible(backend != VirtualGamepadBackend::Dsu);
    }
}

pub(crate) fn refresh_motion_rows(rows: &MotionRows, backend: VirtualGamepadBackend) {
    rows.apply_backend(backend);
}

/// Rumble passthrough, on the Controller page.
pub(crate) fn add_controller_groups(
    page: &gtk4::Box,
    profile: &Rc<RefCell<InputProfile>>,
    on_dirty: &Rc<dyn Fn()>,
) {
    let haptics = SettingGroup::new(
        Some(&crate::tr!("Haptics")),
        Some(&crate::tr!(
            "Rumble the game plays on the virtual controller is replayed on your real controller"
        )),
    );
    let rumble = switch_row(
        &crate::tr!("Enable rumble"),
        None,
        profile.borrow().rumble,
        {
            let profile = profile.clone();
            let on_dirty = on_dirty.clone();
            move |active| {
                profile.borrow_mut().rumble = active;
                on_dirty();
            }
        },
    );
    haptics.add(&rumble);
    page.append(&haptics.root);
}

/// The native motion sensor transport, on the Gyro page next to the rest of
/// the motion settings.
pub(crate) fn add_native_motion_group(
    page: &gtk4::Box,
    profile: &Rc<RefCell<InputProfile>>,
    on_dirty: &Rc<dyn Fn()>,
) -> MotionRows {
    let advanced = SettingGroup::new(
        Some(&crate::tr!("Native motion sensors")),
        Some(&crate::tr!(
            "Expose the sensors as raw evdev axes next to the virtual pad; emulators cannot read them yet until SDL and the kernel support uinput motion"
        )),
    );
    let native_motion = switch_row(
        &crate::tr!("Enable native motion sensors"),
        None,
        profile.borrow().native_motion,
        {
            let profile = profile.clone();
            let on_dirty = on_dirty.clone();
            move |active| {
                profile.borrow_mut().native_motion = active;
                on_dirty();
            }
        },
    );
    advanced.add(&native_motion);
    page.append(&advanced.root);

    let motion_rows = MotionRows {
        native: native_motion,
    };
    motion_rows.apply_backend(profile.borrow().backend);
    motion_rows
}
