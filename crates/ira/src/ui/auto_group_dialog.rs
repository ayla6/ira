//! Create or edit an auto group: a name and a rule tree, edited as
//! groups of rules. The top gate decides how the groups combine
//! (all / any / exactly one); every group carries its own gate over its
//! rules, so `(genre: visual novel AND console: snes) OR (playtime:
//! 2h+)` is two groups under an any-of root. Save rewrites the stored
//! tree; membership follows on the next rebuild, because it is derived,
//! never stored.

use adw::prelude::*;
use ira_models::{AutoCriterion, AutoDimension, AutoGroup, AutoLogic, AutoNode};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::state::SharedState;

pub(super) fn show_auto_group_create(state: &SharedState) {
    show(state, None);
}

pub(super) fn show_auto_group_edit(state: &SharedState, memory_id: i64) {
    match super::auto_groups::find_auto_group(state, memory_id) {
        Some(group) => show(state, Some(group)),
        None => eprintln!("Auto group {memory_id} vanished before its editor opened"),
    }
}

fn show(state: &SharedState, existing: Option<AutoGroup>) {
    let window = state.borrow().window.clone();

    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Auto group"));
    dialog.set_content_width(560);
    dialog.set_content_height(600);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let title = gtk4::Label::new(Some(&crate::tr!("Auto group")));
    title.add_css_class("heading");
    header.set_title_widget(Some(&title));
    toolbar.add_top_bar(&header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // The name, in its own group like every other form in the app.
    let name_group = adw::PreferencesGroup::new();
    let name_row = adw::EntryRow::new();
    name_row.set_title(&crate::tr!("Name"));
    if let Some(group) = &existing {
        name_row.set_text(&group.name);
    }
    name_group.add(&name_row);
    content.append(&name_group);

    // The rule tree, two levels deep: the root gate row plus one card
    // per rule group.
    let groups = GroupsUi::new(state);
    if let Some(AutoNode::Logic { logic, nodes }) = existing.as_ref().map(|g| g.root.clone()) {
        groups.root_gate.set_selected(logic_position(logic));
        for node in nodes {
            if let AutoNode::Logic { logic, nodes } = node {
                let rules = nodes
                    .into_iter()
                    .filter_map(|n| match n {
                        AutoNode::Rule(criterion) => Some(criterion),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                groups.add_group(Some((logic, &rules)));
            }
        }
    }
    content.append(&groups.root);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&content));
    scrolled.set_vexpand(true);
    toolbar.set_content(Some(&scrolled));

    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    buttons.set_halign(gtk4::Align::End);
    buttons.set_margin_top(6);
    buttons.set_margin_bottom(6);
    buttons.set_margin_start(12);
    buttons.set_margin_end(12);
    let cancel = gtk4::Button::with_label(&crate::tr!("Cancel"));
    cancel.add_css_class(super::css::CSS_FLAT);
    let save = gtk4::Button::with_label(&crate::tr!("Save"));
    save.add_css_class(super::css::CSS_SUGGESTED_ACTION);
    buttons.append(&cancel);
    buttons.append(&save);
    toolbar.add_bottom_bar(&buttons);

    dialog.set_child(Some(&toolbar));
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }

    {
        let dialog = dialog.clone();
        let (state, name_row, groups) = (state.clone(), name_row, groups);
        save.connect_clicked(move |_| {
            let name = name_row.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let root = groups.collect_root();
            let db = state.borrow().db.clone();
            match &existing {
                Some(group) => {
                    let db_id = super::auto_groups::to_db_id(group.id);
                    if let Err(e) = ira_db::update_auto_group(&db, db_id, &name, &root) {
                        eprintln!("Failed to update the auto group: {e}");
                        return;
                    }
                    let mut s = state.borrow_mut();
                    if let Some(stored) = s.auto_groups.iter_mut().find(|g| g.id == group.id) {
                        stored.name = name;
                        stored.root = root;
                    }
                }
                None => {
                    let db_id = match ira_db::create_auto_group(&db, &name, &root) {
                        Ok(id) => id,
                        Err(e) => {
                            eprintln!("Failed to create the auto group: {e}");
                            return;
                        }
                    };
                    state.borrow_mut().auto_groups.push(AutoGroup {
                        id: super::auto_groups::to_memory_id(db_id),
                        name,
                        root,
                    });
                }
            }
            dialog.close();
            super::sidebar::rebuild_sidebar(&state);
        });
    }

    dialog.present(Some(&window));
}

fn logic_position(logic: AutoLogic) -> u32 {
    match logic {
        AutoLogic::All => 0,
        AutoLogic::Any => 1,
        AutoLogic::One => 2,
    }
}

fn logic_from_position(position: u32) -> AutoLogic {
    match position {
        1 => AutoLogic::Any,
        2 => AutoLogic::One,
        _ => AutoLogic::All,
    }
}

fn logic_model() -> gtk4::StringList {
    let model = gtk4::StringList::new(&[]);
    let labels = [
        crate::tr!("all of"),
        crate::tr!("any of"),
        crate::tr!("exactly one of"),
    ];
    for label in labels {
        model.append(&label);
    }
    model
}

/// One value menu per dimension, fetched once per dialog: the pickers
/// offer what the library has on record (plus the console set, which is
/// closed).
fn dimension_menus(state: &SharedState) -> HashMap<AutoDimension, Vec<String>> {
    let db = state.borrow().db.clone();
    let load = |kind: &str, role: Option<&str>| {
        ira_db::distinct_entity_names(&db, kind, role).unwrap_or_default()
    };
    let mut menus = HashMap::new();
    menus.insert(AutoDimension::Genre, load(ira_db::KIND_GENRE, None));
    menus.insert(AutoDimension::Family, load(ira_db::KIND_FAMILY, None));
    menus.insert(
        AutoDimension::Developer,
        load(ira_db::KIND_COMPANY, Some("is_developer")),
    );
    menus.insert(
        AutoDimension::Publisher,
        load(ira_db::KIND_COMPANY, Some("is_publisher")),
    );
    menus.insert(
        AutoDimension::Console,
        ira_models::all_consoles()
            .map(|c| c.display_name.to_string())
            .collect(),
    );
    menus
}

/// The groups half of the dialog: the root gate row plus one card per
/// rule group. The inner registry is shared with the cards' closures so
/// removals work without index bookkeeping at the call sites.
struct GroupsUi {
    root: gtk4::Box,
    root_gate: adw::ComboRow,
    inner: Rc<RefCell<GroupsInner>>,
    menus: Rc<HashMap<AutoDimension, Vec<String>>>,
}

#[derive(Default)]
struct GroupsInner {
    /// One (card, collector) pair per group, in dialog order.
    groups: Vec<(gtk4::Box, GroupCollector)>,
}

impl GroupsUi {
    fn new(state: &SharedState) -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

        let root_group = adw::PreferencesGroup::new();
        root_group.set_title(&crate::tr!("Match"));
        root_group.set_description(Some(&crate::tr!(
            "A game joins the group when the rule groups below combine as chosen."
        )));
        let root_gate = adw::ComboRow::new();
        root_gate.set_title(&crate::tr!("Groups combine when"));
        root_gate.set_model(Some(&logic_model()));
        root_group.add(&root_gate);
        root.append(&root_group);

        let menus = Rc::new(dimension_menus(state));
        let inner: Rc<RefCell<GroupsInner>> = Default::default();
        let group_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        root.append(&group_box);

        let add_group_btn = gtk4::Button::with_label(&crate::tr!("Add group"));
        add_group_btn.add_css_class(super::css::CSS_FLAT);
        add_group_btn.set_halign(gtk4::Align::Start);
        {
            let inner = inner.clone();
            let group_box = group_box.clone();
            let menus = menus.clone();
            add_group_btn.connect_clicked(move |_| {
                let (card, collector) = build_group_card(&menus, &inner);
                group_box.append(&card);
                inner.borrow_mut().groups.push((card, collector));
            });
        }
        root.append(&add_group_btn);

        Self {
            root,
            root_gate,
            inner,
            menus,
        }
    }

    /// Add one group card, optionally prefilled with a saved gate and
    /// rules (the edit path).
    fn add_group(&self, existing: Option<(AutoLogic, &[AutoCriterion])>) {
        let (card, collector) = build_group_card(&self.menus, &self.inner);
        if let Some((logic, rules)) = existing {
            collector.gate.set_selected(logic_position(logic));
            let rules_box = collector.rules_box.clone();
            for criterion in rules {
                let editor = CriterionEditor::new(&self.menus, Some(criterion));
                rules_box.append(&editor.root);
                collector.rules.borrow_mut().push(editor);
            }
        }
        // The card goes above the Add-group button, which is the root's
        // last child.
        let last = self.root.last_child();
        match last {
            Some(btn) => {
                self.root.remove(&btn);
                self.root.append(&card);
                self.root.append(&btn);
            }
            None => self.root.append(&card),
        }
        self.inner.borrow_mut().groups.push((card, collector));
    }

    /// The saved tree: the root gate over every group card's gate and
    /// rules. A tree with no rules in it matches no game at all.
    fn collect_root(&self) -> AutoNode {
        let logic = logic_from_position(self.root_gate.selected());
        let nodes: Vec<AutoNode> = self
            .inner
            .borrow()
            .groups
            .iter()
            .map(|(_, collector)| AutoNode::Logic {
                logic: logic_from_position(collector.gate.selected()),
                nodes: collector
                    .rules
                    .borrow()
                    .iter()
                    .map(|editor| AutoNode::Rule(editor.collect()))
                    .collect(),
            })
            .collect();
        AutoNode::Logic { logic, nodes }
    }
}

#[derive(Clone)]
struct GroupCollector {
    gate: adw::ComboRow,
    rules_box: gtk4::Box,
    rules: Rc<RefCell<Vec<CriterionEditor>>>,
}

/// One rule-group card: its gate row (with the remove button), the rule
/// rows, and the add-rule button. Removals work through the shared
/// registry, so no call site needs to know a card's index.
fn build_group_card(
    menus: &Rc<HashMap<AutoDimension, Vec<String>>>,
    inner: &Rc<RefCell<GroupsInner>>,
) -> (gtk4::Box, GroupCollector) {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class(super::css::CSS_BOXED_LIST);
    card.set_margin_top(6);
    card.set_margin_bottom(6);

    let gate = adw::ComboRow::new();
    gate.set_title(&crate::tr!("Rules match"));
    gate.set_model(Some(&logic_model()));

    let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove.add_css_class(super::css::CSS_FLAT);
    remove.set_valign(gtk4::Align::Center);
    remove.set_tooltip_text(Some(&crate::tr!("Remove this group")));
    gate.add_suffix(&remove);

    card.append(&gate);

    let rules_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    rules_box.set_margin_top(6);
    rules_box.set_margin_bottom(6);
    rules_box.set_margin_start(6);
    rules_box.set_margin_end(6);
    card.append(&rules_box);

    let add_rule = gtk4::Button::with_label(&crate::tr!("Add rule"));
    add_rule.add_css_class(super::css::CSS_FLAT);
    add_rule.set_halign(gtk4::Align::Start);
    add_rule.set_margin_bottom(6);
    card.append(&add_rule);

    {
        let card = card.clone();
        let inner = inner.clone();
        remove.connect_clicked(move |_| {
            inner.borrow_mut().groups.retain(|(box_, _)| *box_ != card);
            if let Some(parent) = card.parent().and_downcast::<gtk4::Box>() {
                parent.remove(&card);
            }
        });
    }

    let rules: Rc<RefCell<Vec<CriterionEditor>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let menus = menus.clone();
        let rules_box = rules_box.clone();
        let rules = rules.clone();
        add_rule.connect_clicked(move |_| {
            let editor = CriterionEditor::new(&menus, None);
            rules_box.append(&editor.root);
            rules.borrow_mut().push(editor);
        });
    }

    let collector = GroupCollector {
        gate,
        rules_box,
        rules,
    };
    (card, collector)
}

/// One rule row: the dimension picker and remove button on top, that
/// dimension's control underneath.
struct CriterionEditor {
    root: gtk4::Box,
    dropdown: gtk4::DropDown,
    /// The check rows of the current value dimension, name in list order.
    value_checks: Rc<RefCell<Vec<(String, gtk4::CheckButton)>>>,
    from_pick: Rc<super::date_pick::DatePick>,
    to_pick: Rc<super::date_pick::DatePick>,
    min_hours: gtk4::SpinButton,
    max_hours: gtk4::SpinButton,
}

impl CriterionEditor {
    fn new(
        menus: &Rc<HashMap<AutoDimension, Vec<String>>>,
        initial: Option<&AutoCriterion>,
    ) -> Self {
        let initial = initial.cloned().unwrap_or_default();
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        root.add_css_class("card");
        root.set_margin_top(4);
        root.set_margin_bottom(4);

        let top = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let dimensions = AutoDimension::ALL;
        let model = gtk4::StringList::new(&[]);
        for dimension in dimensions {
            model.append(dimension.display_label());
        }
        let dropdown = gtk4::DropDown::new(Some(model), None::<gtk4::Expression>);
        dropdown.set_hexpand(true);
        if let Some(position) = dimensions.iter().position(|d| *d == initial.dimension) {
            dropdown.set_selected(position as u32);
        }
        top.append(&dropdown);

        let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class(super::css::CSS_FLAT);
        remove.set_valign(gtk4::Align::Center);
        remove.set_tooltip_text(Some(&crate::tr!("Remove this rule")));
        {
            let root = root.clone();
            remove.connect_clicked(move |_| {
                if let Some(parent) = root.parent().and_downcast::<gtk4::Box>() {
                    parent.remove(&root);
                }
            });
        }
        top.append(&remove);
        root.append(&top);

        let value_checks: Rc<RefCell<Vec<(String, gtk4::CheckButton)>>> =
            Rc::new(RefCell::new(Vec::new()));
        let from_pick = Rc::new(super::date_pick::DatePick::new(&initial.from));
        from_pick.set_tooltip(&crate::tr!("From"));
        let to_pick = Rc::new(super::date_pick::DatePick::new(&initial.to));
        to_pick.set_tooltip(&crate::tr!("To"));
        let min_hours = hours_spin(initial.min_hours);
        min_hours.set_tooltip_text(Some(&crate::tr!(
            "Played at least this many hours; 0 is no floor"
        )));
        let max_hours = hours_spin(initial.max_hours);
        max_hours.set_tooltip_text(Some(&crate::tr!(
            "Played at most this many hours; 0 is no ceiling"
        )));

        // The value picker: a popover of check buttons over the
        // dimension's names; the button label counts the picked ones.
        let value_label = gtk4::Label::new(Some(&crate::tr!("Pick values")));
        let list = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        let value_button = {
            let button = gtk4::MenuButton::new();
            button.set_child(Some(&value_label));
            button.add_css_class(super::css::CSS_FLAT);
            let scrolled = gtk4::ScrolledWindow::new();
            scrolled.set_child(Some(&list));
            scrolled.set_max_content_height(320);
            scrolled.set_propagate_natural_height(true);
            let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            popover_box.set_margin_top(6);
            popover_box.set_margin_bottom(6);
            popover_box.set_margin_start(6);
            popover_box.set_margin_end(6);
            popover_box.append(&scrolled);
            let popover = gtk4::Popover::new();
            popover.set_child(Some(&popover_box));
            button.set_popover(Some(&popover));
            button
        };

        let set_label = {
            let value_checks = value_checks.clone();
            let value_label = value_label.clone();
            move || {
                let picked = value_checks
                    .borrow()
                    .iter()
                    .filter(|(_, check)| check.is_active())
                    .count();
                value_label.set_text(&if picked == 0 {
                    crate::tr!("Pick values").to_string()
                } else {
                    crate::tr!("{} picked").replacen("{}", &picked.to_string(), 1)
                });
            }
        };
        let fill_values = {
            let value_checks = value_checks.clone();
            let list = list.clone();
            let set_label = set_label.clone();
            let menus = menus.clone();
            move |dimension: AutoDimension| {
                let names = menus.get(&dimension).cloned().unwrap_or_default();
                let picked: HashSet<String> = value_checks
                    .borrow()
                    .iter()
                    .filter(|(_, check)| check.is_active())
                    .map(|(name, _)| name.clone())
                    .collect();
                value_checks.borrow_mut().clear();
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                for name in &names {
                    let check = gtk4::CheckButton::with_label(name);
                    check.set_active(picked.contains(name));
                    let recount = set_label.clone();
                    check.connect_toggled(move |_| recount());
                    list.append(&check);
                    value_checks.borrow_mut().push((name.clone(), check));
                }
                if names.is_empty() {
                    let empty = gtk4::Label::new(Some(&crate::tr!(
                        "Nothing on record for this rule yet"
                    )));
                    empty.add_css_class(super::css::CSS_DIM_LABEL);
                    list.append(&empty);
                }
                set_label();
            }
        };

        let stack = gtk4::Stack::new();
        stack.add_named(&value_button, Some("values"));
        let dates = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        dates.append(from_pick.button());
        dates.append(to_pick.button());
        stack.add_named(&dates, Some("released"));
        let hours = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        hours.append(&min_hours);
        hours.append(&max_hours);
        stack.add_named(&hours, Some("playtime"));
        root.append(&stack);

        // The dimension switch re-fills the value list and shows the
        // matching control.
        {
            let stack = stack.clone();
            let dates = dates.clone();
            let hours = hours.clone();
            let value_button = value_button.clone();
            let fill_values = fill_values.clone();
            dropdown.connect_selected_notify(move |dropdown| {
                let dimension = AutoDimension::ALL
                    .get(dropdown.selected() as usize)
                    .copied()
                    .unwrap_or_default();
                sync_dimension(
                    &stack,
                    dates.upcast_ref(),
                    hours.upcast_ref(),
                    value_button.upcast_ref(),
                    dimension,
                );
                if dimension.takes_values() {
                    fill_values(dimension);
                }
            });
        }
        // Sync the initial dimension directly — there is no signal to
        // emit, and none is needed.
        sync_dimension(
            &stack,
            dates.upcast_ref(),
            hours.upcast_ref(),
            value_button.upcast_ref(),
            initial.dimension,
        );
        if initial.dimension.takes_values() {
            fill_values(initial.dimension);
        }

        Self {
            root,
            dropdown,
            value_checks,
            from_pick,
            to_pick,
            min_hours,
            max_hours,
        }
    }

    fn collect(&self) -> AutoCriterion {
        let dimension = AutoDimension::ALL
            .get(self.dropdown.selected() as usize)
            .copied()
            .unwrap_or_default();
        let mut criterion = AutoCriterion {
            dimension,
            ..Default::default()
        };
        match dimension {
            AutoDimension::Released => {
                criterion.from = self.from_pick.typed().unwrap_or_default();
                criterion.to = self.to_pick.typed().unwrap_or_default();
            }
            AutoDimension::Playtime => {
                let min = self.min_hours.value();
                let max = self.max_hours.value();
                criterion.min_hours = (min > 0.0).then_some(min);
                criterion.max_hours = (max > 0.0).then_some(max);
            }
            _ => {
                criterion.values = self
                    .value_checks
                    .borrow()
                    .iter()
                    .filter(|(_, check)| check.is_active())
                    .map(|(name, _)| name.clone())
                    .collect();
            }
        }
        criterion
    }
}

fn sync_dimension(
    stack: &gtk4::Stack,
    dates: &gtk4::Widget,
    hours: &gtk4::Widget,
    values: &gtk4::Widget,
    dimension: AutoDimension,
) {
    match dimension {
        AutoDimension::Released => stack.set_visible_child(dates),
        AutoDimension::Playtime => stack.set_visible_child(hours),
        _ => stack.set_visible_child(values),
    }
}

fn hours_spin(initial: Option<f64>) -> gtk4::SpinButton {
    let spin = gtk4::SpinButton::with_range(0.0, 9999.0, 0.5);
    spin.set_value(initial.unwrap_or(0.0));
    spin.set_digits(1);
    spin
}
