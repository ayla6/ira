//! The value-picking page of the auto group dialog: a search entry over
//! a boxed list of check rows. Rule rows hand their list to
//! [`ValuesPage::open`], which hosts it until the back button returns
//! to the editor; the search entry narrows the hosted rows as you type.

use adw::prelude::*;
use gtk4::glib;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// The check rows of one rule's value dimension: the name, its row, and
/// its check. The editor collects the picked names from these.
pub(super) type ValueChecks =
    Rc<RefCell<Vec<(String, adw::ActionRow, gtk4::CheckButton)>>>;

/// The page-switch callback: `true` when the values page came up.
pub(super) type NavigateFn = Rc<dyn Fn(bool)>;

pub(super) struct ValuesPage {
    root: gtk4::Box,
    search: gtk4::SearchEntry,
    slot: gtk4::Box,
    filter_handler: RefCell<Option<glib::SignalHandlerId>>,
    navigate: RefCell<Option<NavigateFn>>,
}

impl ValuesPage {
    pub(super) fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        root.set_margin_top(12);
        root.set_margin_bottom(12);
        root.set_margin_start(12);
        root.set_margin_end(12);

        let search = gtk4::SearchEntry::new();
        search.set_hexpand(true);
        root.append(&search);

        let slot = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_child(Some(&slot));
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        root.append(&scrolled);

        Self {
            root,
            search,
            slot,
            filter_handler: RefCell::new(None),
            navigate: RefCell::new(None),
        }
    }

    pub(super) fn root(&self) -> gtk4::Box {
        self.root.clone()
    }

    /// The page-switch callback: `true` when the values page came up.
    pub(super) fn set_navigation(&self, f: NavigateFn) {
        *self.navigate.borrow_mut() = Some(f);
    }

    /// Host `list` on this page and come to the front.
    pub(super) fn open(&self, list: &gtk4::ListBox) {
        while let Some(child) = self.slot.first_child() {
            self.slot.remove(&child);
        }
        self.slot.append(list);
        self.search.set_text("");

        // A fresh filter per open, scoped to the hosted list.
        let old = self.filter_handler.borrow_mut().take();
        if let Some(handler) = old {
            self.search.disconnect(handler);
        }
        let list = list.clone();
        let handler = self.search.connect_search_changed(move |search| {
            apply_filter(&list, &search.text());
        });
        *self.filter_handler.borrow_mut() = Some(handler);

        if let Some(f) = self.navigate.borrow().as_ref() {
            f(true);
        }
    }

    /// Return to the editor page.
    pub(super) fn close(&self) {
        if let Some(f) = self.navigate.borrow().as_ref() {
            f(false);
        }
    }
}

fn apply_filter(list: &gtk4::ListBox, query: &str) {
    let query = query.trim().to_lowercase();
    let mut child = list.first_child();
    while let Some(row) = child {
        let next = row.next_sibling();
        let show = query.is_empty()
            || row
                .downcast_ref::<adw::ActionRow>()
                .is_some_and(|r| r.title().to_lowercase().contains(&query));
        row.set_visible(show);
        child = next;
    }
}

/// Builds one rule's check list for a value dimension: a boxed list of
/// one row per known name, `picked` pre-checked. Activating a row
/// toggles its check — the row is in the list box, so activation
/// actually fires — and every change refreshes the rule row's subtitle.
pub(super) fn fill_check_list(
    list: &gtk4::ListBox,
    checks: &ValueChecks,
    names: &[String],
    picked: HashSet<String>,
    values_row: &adw::ActionRow,
) {
    checks.borrow_mut().clear();
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for name in names {
        let check = gtk4::CheckButton::new();
        check.set_active(picked.contains(name));
        check.set_valign(gtk4::Align::Center);
        let row = adw::ActionRow::new();
        row.set_title(name);
        row.add_suffix(&check);
        row.set_activatable(true);
        {
            let check = check.clone();
            row.connect_activated(move |_| check.set_active(!check.is_active()));
        }
        {
            let checks = checks.clone();
            let values_row = values_row.clone();
            check.connect_toggled(move |_| refresh_values_row(&checks, &values_row));
        }
        list.append(&row);
        checks.borrow_mut().push((name.clone(), row, check));
    }
    if names.is_empty() {
        let empty = adw::ActionRow::new();
        empty.set_title(&crate::tr!("Nothing on record for this rule yet"));
        empty.add_css_class(super::css::CSS_DIM_LABEL);
        empty.set_activatable(false);
        list.append(&empty);
    }
    refresh_values_row(checks, values_row);
}

/// The rule row's subtitle: the picked names, or the any-value hint.
pub(super) fn refresh_values_row(checks: &ValueChecks, row: &adw::ActionRow) {
    let picked = picked_values(checks);
    row.set_subtitle(&if picked.is_empty() {
        crate::tr!("Any value").to_string()
    } else {
        picked.join(", ")
    });
}

/// The names whose checks are on, in list order.
pub(super) fn picked_values(checks: &ValueChecks) -> Vec<String> {
    checks
        .borrow()
        .iter()
        .filter(|(_, _, check)| check.is_active())
        .map(|(name, _, _)| name.clone())
        .collect()
}
