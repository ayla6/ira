//! Shared plumbing for the per-input binding sheet and its sub-editors:
//! the cloneable sheet state, profile accessors that route through the
//! active action set, and the combo/spin row helpers every group uses.

use adw::prelude::*;
use ira_input::{GamepadAxis, InputMapping, InputSource};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) type ProfileRc = Rc<RefCell<ira_input::InputProfile>>;
pub(crate) type OnChanged = Rc<dyn Fn()>;
/// Structural edits rebuild the sheet contents through this hook.
pub(crate) type Reopen = Rc<dyn Fn()>;

/// Cloneable sheet state shared by every edit closure. The sheet derives
/// its own reopen hook from this so any depth of rebuilds keeps working.
#[derive(Clone)]
pub(crate) struct SheetBase {
    pub(crate) content: gtk4::Box,
    pub(crate) profile: ProfileRc,
    pub(crate) active_set: usize,
    pub(crate) source: InputSource,
    pub(crate) device: Option<ira_input::DeviceInfo>,
    pub(crate) backend: ira_input::VirtualGamepadBackend,
    pub(crate) on_changed: OnChanged,
    /// Coalesces deferred rebuilds so a burst of change notifications only
    /// rebuilds once.
    pub(crate) rebuild_pending: Rc<std::cell::Cell<bool>>,
}

pub(crate) fn find_mapping(base: &SheetBase) -> Option<InputMapping> {
    base.profile
        .borrow()
        .action_sets
        .get(base.active_set)?
        .inputs
        .iter()
        .find(|input| input.source == base.source)
        .cloned()
}

pub(crate) fn with_mapping(base: &SheetBase, apply: impl FnOnce(&mut InputMapping)) {
    let mut borrow = base.profile.borrow_mut();
    if let Some(set) = borrow.action_sets.get_mut(base.active_set) {
        if let Some(input) = set
            .inputs
            .iter_mut()
            .find(|input| input.source == base.source)
        {
            apply(input);
        }
    }
}

/// Trigger axes are the one analog source with dual-stage activators.
pub(crate) fn is_trigger_axis(source: InputSource) -> bool {
    matches!(
        source,
        InputSource::Axis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
    )
}

/// A full-width libadwaita combo row; callers add it directly instead of
/// nesting a compact dropdown inside another row's suffix.
pub(crate) fn combo_row(labels: &[String], selected: u32) -> adw::ComboRow {
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let row = adw::ComboRow::new();
    row.set_model(Some(&gtk4::StringList::new(&refs)));
    row.set_selected(selected);
    row
}
