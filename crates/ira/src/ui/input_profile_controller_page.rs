//! Whole-controller output settings for the profile editor: rumble
//! passthrough on the Controller page. This applies to the pad as a whole
//! rather than any region; the native-motion transport lives on the Gyro
//! page as the gyro output choice.

use super::input_profile_widgets::{switch_row, SettingGroup};
use adw::prelude::*;
use ira_input::InputProfile;
use std::cell::RefCell;
use std::rc::Rc;

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
