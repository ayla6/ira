//! The editor's top-left display of what is being edited, mirroring Steam
//! Input: an "Action set / Layer" caption above the current name, with the
//! layer count (or, for layers, the parent set) as detail. Lives above the
//! editor sidebar and refreshes from every page that moves the editing
//! target.

use super::css::{CSS_CAPTION, CSS_DIM_LABEL, CSS_HEADING};
use super::input_profile_region_pages::{EditingTarget, PagesCtx};
use adw::prelude::*;

#[derive(Clone)]
pub(crate) struct SetIndicator {
    pub(crate) root: gtk4::Box,
    kind: gtk4::Label,
    name: gtk4::Label,
    detail: gtk4::Label,
}

impl SetIndicator {
    pub(crate) fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.set_margin_top(10);
        root.set_margin_bottom(8);
        root.set_margin_start(12);
        root.set_margin_end(12);

        let kind = gtk4::Label::new(None);
        kind.set_xalign(0.0);
        kind.add_css_class(CSS_CAPTION);
        kind.add_css_class(CSS_DIM_LABEL);

        let name = gtk4::Label::new(None);
        name.set_xalign(0.0);
        name.add_css_class(CSS_HEADING);
        name.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let detail = gtk4::Label::new(None);
        detail.set_xalign(0.0);
        detail.add_css_class(CSS_CAPTION);
        detail.add_css_class(CSS_DIM_LABEL);
        detail.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        root.append(&kind);
        root.append(&name);
        root.append(&detail);
        Self {
            root,
            kind,
            name,
            detail,
        }
    }

    /// Re-reads the editing target from the shared state; cheap enough to
    /// call after every mutation instead of tracking change reasons.
    pub(crate) fn update(&self, ctx: &PagesCtx) {
        let profile = ctx.profile.borrow();
        let target = ctx.active_target.get();
        self.kind.set_text(&match target {
            EditingTarget::Set(_) => crate::tr!("Action set"),
            EditingTarget::Layer(_) => crate::tr!("Layer"),
        });
        self.name.set_text(&target.name(&profile));
        self.detail.set_text(&match target {
            EditingTarget::Set(index) => {
                let Some(set) = profile.action_sets.get(index) else {
                    self.detail.set_visible(false);
                    return;
                };
                let count = profile
                    .action_layers
                    .iter()
                    .filter(|layer| layer.parent_set == set.name)
                    .count();
                let text = match count {
                    0 => crate::tr!("No layers"),
                    1 => crate::tr!("1 layer"),
                    _ => crate::tr!("{count} layers").replace("{count}", &count.to_string()),
                };
                self.detail.set_visible(true);
                text
            }
            EditingTarget::Layer(_) => {
                let text = match target.parent_name(&profile) {
                    Some(parent) => {
                        crate::tr!("over {parent}").replace("{parent}", &parent)
                    }
                    None => String::new(),
                };
                self.detail.set_visible(!text.is_empty());
                text
            }
        });
    }
}
