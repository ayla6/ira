use ira_input::{ControllerFamily, DeviceInfo, InputSource, OutputAction, VirtualGamepadBackend};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(crate) struct BindingRow {
    pub container: adw::ExpanderRow,
    pub source_options: Vec<(InputSource, String)>,
    pub activator_options: Vec<(InputSource, String)>,
    pub source: gtk4::DropDown,
    pub output: gtk4::DropDown,
    pub output_action: Rc<RefCell<OutputAction>>,
    pub activation: gtk4::DropDown,
    pub activator: gtk4::DropDown,
    pub chord: adw::EntryRow,
    pub dead_zone: gtk4::SpinButton,
    pub sensitivity: gtk4::SpinButton,
    pub exponent: gtk4::SpinButton,
    pub invert: gtk4::CheckButton,
}

#[derive(Clone)]
pub(super) struct SourceChangeContext {
    pub source_options: Vec<(InputSource, String)>,
    pub family: ControllerFamily,
    pub fallback: gtk4::Label,
    pub asset: gtk4::Image,
    pub current_source: Rc<Cell<InputSource>>,
    pub output: Rc<RefCell<OutputAction>>,
    pub row: adw::ExpanderRow,
    pub dead_zone_row: adw::ActionRow,
    pub sensitivity_row: adw::ActionRow,
    pub exponent_row: adw::ActionRow,
    pub invert_row: adw::ActionRow,
}

#[derive(Clone)]
pub(super) struct OutputChangeContext {
    pub source: gtk4::DropDown,
    pub output: Rc<RefCell<OutputAction>>,
    pub row: adw::ExpanderRow,
    pub on_dirty: Rc<dyn Fn()>,
    pub backend: VirtualGamepadBackend,
}

pub(crate) type SectionGroups = Rc<RefCell<Vec<Vec<(String, adw::PreferencesGroup)>>>>;

#[derive(Clone)]
pub(crate) struct BindingRowContext {
    pub(crate) page: gtk4::Box,
    pub(crate) section_groups: SectionGroups,
    pub(crate) rows: Rc<RefCell<Vec<BindingRow>>>,
    pub(crate) device: Option<DeviceInfo>,
    pub(crate) backend: VirtualGamepadBackend,
    pub(crate) on_dirty: Rc<dyn Fn()>,
}
