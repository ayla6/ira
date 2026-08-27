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
    pub(crate) active_target: EditingTarget,
    pub(crate) source: InputSource,
    pub(crate) device: Option<ira_input::DeviceInfo>,
    pub(crate) backend: ira_input::VirtualGamepadBackend,
    pub(crate) on_changed: OnChanged,
    /// Coalesces deferred rebuilds so a burst of change notifications only
    /// rebuilds once.
    pub(crate) rebuild_pending: Rc<std::cell::Cell<bool>>,
}

/// What the region pages and per-input sheets are editing: an action set,
/// or one of its layers. Layers are first-class binding targets — Steam
/// Input lets you open a layer and bind inputs exactly like a set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EditingTarget {
    Set(usize),
    Layer(usize),
}

impl EditingTarget {
    /// The binding list this target edits, if it still exists (indices can
    /// go stale after structural edits elsewhere).
    pub(crate) fn inputs_mut(
        self,
        profile: &mut ira_input::InputProfile,
    ) -> Option<&mut Vec<InputMapping>> {
        match self {
            Self::Set(index) => profile.action_sets.get_mut(index).map(|set| &mut set.inputs),
            Self::Layer(index) => profile
                .action_layers
                .get_mut(index)
                .map(|layer| &mut layer.inputs),
        }
    }

    /// The mapping bound to `source` inside this target, if any.
    pub(crate) fn find_mapping(
        self,
        profile: &ira_input::InputProfile,
        source: InputSource,
    ) -> Option<InputMapping> {
        let inputs = match self {
            Self::Set(index) => profile.action_sets.get(index)?.inputs.as_slice(),
            Self::Layer(index) => profile.action_layers.get(index)?.inputs.as_slice(),
        };
        inputs.iter().find(|input| input.source == source).cloned()
    }

    pub(crate) fn with_mapping(
        self,
        profile: &mut ira_input::InputProfile,
        source: InputSource,
        apply: impl FnOnce(&mut InputMapping),
    ) {
        let inputs = self.inputs_mut(profile);
        if let Some(input) = inputs
            .and_then(|inputs| inputs.iter_mut().find(|input| input.source == source))
        {
            apply(input);
        }
    }

    /// Name of this target for labels and the editor's set/layer indicator.
    pub(crate) fn name(self, profile: &ira_input::InputProfile) -> String {
        match self {
            Self::Set(index) => profile
                .action_sets
                .get(index)
                .map(|set| set.name.clone())
                .unwrap_or_default(),
            Self::Layer(index) => profile
                .action_layers
                .get(index)
                .map(|layer| layer.name.clone())
                .unwrap_or_default(),
        }
    }

    /// Parent set name of a layer target; `None` for sets.
    pub(crate) fn parent_name(self, profile: &ira_input::InputProfile) -> Option<String> {
        match self {
            Self::Set(_) => None,
            Self::Layer(index) => profile
                .action_layers
                .get(index)
                .map(|layer| layer.parent_set.clone()),
        }
    }
}

pub(crate) fn find_mapping(base: &SheetBase) -> Option<InputMapping> {
    base.active_target
        .find_mapping(&base.profile.borrow(), base.source)
}

pub(crate) fn with_mapping(base: &SheetBase, apply: impl FnOnce(&mut InputMapping)) {
    let mut borrow = base.profile.borrow_mut();
    base.active_target.with_mapping(&mut borrow, base.source, apply);
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
