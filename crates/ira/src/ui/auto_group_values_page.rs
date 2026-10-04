//! The value-picking page of the auto group dialog: a search entry over
//! a boxed list of check rows. Rule rows hand their list to
//! [`ValuesPage::open`], which hosts it until the navigation view pops
//! back to the editor; the search entry narrows the hosted rows as you
//! type, matching each option's haystack, not just its shown name.

use adw::prelude::*;
use gtk4::glib;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use ira_models::AutoDimension;

use super::helpers::clear_children;

/// One pickable value of a rule dimension: the stored name plus the
/// haystack its row searches against — lowercased with whitespace
/// dropped, so spacing never matters. For consoles the haystack also
/// carries hidden terms — "ps1" finds "PlayStation 1" — while only the
/// full name shows.
#[derive(Clone)]
pub(super) struct ValueOption {
    pub value: String,
    pub search: String,
}

impl ValueOption {
    /// A plain value whose search haystack is the name itself.
    pub fn plain(value: String) -> Self {
        let search = normalize_query(&value);
        Self { value, search }
    }
}

/// The query-side of the haystack normalization: lowercased, all
/// whitespace dropped.
fn normalize_query(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// The value menus of every dimension, as offered by the dialog's pickers.
pub(super) type ValueMenus = HashMap<ira_models::AutoDimension, Vec<ValueOption>>;

/// One row of a rule's value dimension: the option, its row, and its
/// check. The editor collects the picked names from these.
pub(super) type ValueChecks =
    Rc<RefCell<Vec<(ValueOption, adw::ActionRow, gtk4::CheckButton)>>>;

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
        // The page opens with focus already on the search, so typing
        // just works — with the on-screen keyboard suppressed, since a
        // hardware keyboard is the expected input here.
        if let Some(text) = search.delegate().and_downcast::<gtk4::Text>() {
            text.set_input_hints(gtk4::InputHints::INHIBIT_OSK);
        }
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

    /// Host `list` on this page and come to the front. `checks` drives
    /// the search: each row hides unless its haystack matches the query.
    pub(super) fn open(&self, list: &gtk4::ListBox, checks: &ValueChecks) {
        clear_children(&self.slot);
        self.slot.append(list);
        self.search.set_text("");

        // A fresh filter per open, scoped to the hosted list.
        let old = self.filter_handler.borrow_mut().take();
        if let Some(handler) = old {
            self.search.disconnect(handler);
        }
        let checks = checks.clone();
        let handler = self.search.connect_search_changed(move |search| {
            apply_filter(&checks, &search.text());
        });
        *self.filter_handler.borrow_mut() = Some(handler);

        if let Some(f) = self.navigate.borrow().as_ref() {
            f(true);
        }
        // An idle, so the focus lands after the stack has actually
        // switched pages.
        let search = self.search.clone();
        glib::idle_add_local_once(move || {
            search.grab_focus();
        });
    }
}

fn apply_filter(checks: &ValueChecks, query: &str) {
    let query = normalize_query(query.trim());
    for (opt, row, _) in checks.borrow().iter() {
        row.set_visible(query.is_empty() || opt.search.contains(&query));
    }
}

/// Builds one rule's check list for a value dimension: a boxed list of
/// one row per known option, `picked` pre-checked. The check toggles on
/// its own clicks; the row opens the entity editor wherever the option
/// names a cache entry (genre/family/company — never console/date
/// dimensions, which have nothing to edit).
pub(super) fn fill_check_list(
    list: &gtk4::ListBox,
    checks: &ValueChecks,
    menus: &Rc<ValueMenus>,
    dimension: AutoDimension,
    picked: HashSet<String>,
    values_row: &adw::ActionRow,
    state: &super::state::SharedState,
) {
    let options = menus.get(&dimension).cloned().unwrap_or_default();
    let entity_kind = match dimension {
        AutoDimension::Genre => Some(ira_db::KIND_GENRE),
        AutoDimension::Family => Some(ira_db::KIND_FAMILY),
        AutoDimension::Developer | AutoDimension::Publisher => Some(ira_db::KIND_COMPANY),
        _ => None,
    };
    checks.borrow_mut().clear();
    clear_children(list);
    // Entity dimensions resolve names to cache ids once, so rows can
    // open the editor; other dimensions have nothing editable.
    let ids: std::collections::HashMap<String, i64> = entity_kind
        .map(|kind| {
            let db = state.borrow().db.clone();
            ira_db::list_entities(&db, kind, "")
                .unwrap_or_default()
                .into_iter()
                .map(|(id, name)| (name, id))
                .collect()
        })
        .unwrap_or_default();
    let refill = RefillCtx {
        list: list.clone(),
        checks: checks.clone(),
        menus: menus.clone(),
        dimension,
        values_row: values_row.clone(),
        state: state.clone(),
    };
    for opt in &options {
        let check = gtk4::CheckButton::new();
        check.set_active(picked.contains(&opt.value));
        check.set_valign(gtk4::Align::Center);
        let row = adw::ActionRow::new();
        // Entity names are shown as typed — an "&" in "Run & Jump" is
        // not markup.
        row.set_use_markup(false);
        row.set_title(&opt.value);
        if let (Some(kind), Some(id)) = (entity_kind, ids.get(&opt.value).copied()) {
            // The pencil opens the editor; saving there refills this
            // list so renames land. The row and the check itself
            // still toggle.
            row.add_suffix(&entity_edit_button(&refill, &row, kind, id, &opt.value));
        }
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
        checks
            .borrow_mut()
            .push((ValueOption {
                value: opt.value.clone(),
                search: opt.search.clone(),
            }, row, check));
    }
    if options.is_empty() {
        let empty = adw::ActionRow::new();
        empty.set_title(&crate::tr!("Nothing on record for this rule yet"));
        empty.set_use_markup(false);
        empty.add_css_class(super::css::CSS_DIM_LABEL);
        empty.set_activatable(false);
        list.append(&empty);
    }
    refresh_values_row(checks, values_row);
}

/// Everything the entity pencil needs to refill its value list after an
/// editor save: the hosted list, its checks, the menus, the dimension,
/// the rule row showing the picks, and the state for entity lookups.
#[derive(Clone)]
struct RefillCtx {
    list: gtk4::ListBox,
    checks: ValueChecks,
    menus: Rc<ValueMenus>,
    dimension: AutoDimension,
    values_row: adw::ActionRow,
    state: super::state::SharedState,
}

/// The pencil opening the entity editor for one entity-dimension row.
/// Saving there refills the same list so renames land; the row and the
/// check itself still toggle on their own clicks.
fn entity_edit_button(
    refill: &RefillCtx,
    row: &adw::ActionRow,
    kind: &'static str,
    id: i64,
    name: &str,
) -> gtk4::Button {
    let pen = gtk4::Button::from_icon_name("document-edit-symbolic");
    pen.add_css_class(super::css::CSS_FLAT);
    pen.set_valign(gtk4::Align::Center);
    pen.set_tooltip_text(Some(&crate::tr!("Manage")));
    let refill = refill.clone();
    let row_c = row.clone();
    let name = name.to_string();
    pen.connect_clicked(move |_| {
        let on_saved: Rc<dyn Fn()> = Rc::new({
            let refill = refill.clone();
            move || {
                fill_check_list(
                    &refill.list,
                    &refill.checks,
                    &refill.menus,
                    refill.dimension,
                    picked_values(&refill.checks).into_iter().collect(),
                    &refill.values_row,
                    &refill.state,
                );
            }
        });
        super::edit_game_entity_dialog::show_entity_editor(
            &refill.state, &row_c, kind, id, &name, on_saved,
        );
    });
    pen
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
        .map(|(opt, _, _)| opt.value.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_option_plain_searches_its_own_name() {
        let opt = ValueOption::plain("Run & Jump".to_string());
        assert_eq!(opt.value, "Run & Jump");
        assert_eq!(opt.search, "run&jump");
    }

    #[test]
    fn test_normalize_query_drops_spacing() {
        assert_eq!(normalize_query("  Game Boy Advance "), "gameboyadvance");
        assert_eq!(normalize_query("   "), "");
    }
}
