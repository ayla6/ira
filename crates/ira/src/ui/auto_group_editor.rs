//! The rule-tree cards of the auto group dialog: the root gate row, one
//! card per rule group — its gate, its rules, its add-rule row — and
//! the add-group row. A rule picks a dimension and the control rows for
//! that dimension: the values row (which opens the dialog's values
//! page) for named dimensions, from/to date rows for release windows,
//! min/max spin rows for playtime.

use adw::prelude::*;
use ira_models::{AutoCriterion, AutoDimension, AutoGroup, AutoLogic, AutoNode};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::auto_group_values_page::{fill_check_list, picked_values, ValueChecks, ValuesPage};
use super::date_pick::DatePick;

pub(super) struct GroupsUi {
    pub(super) root: gtk4::Box,
    pub(super) root_gate: adw::ComboRow,
    inner: Rc<RefCell<GroupsInner>>,
}

#[derive(Default)]
struct GroupsInner {
    /// One (card, collector) pair per group, in dialog order.
    groups: Vec<(adw::PreferencesGroup, GroupCollector)>,
}

impl GroupsUi {
    /// Builds the editor over `menus`, optionally prefilled from a
    /// saved group (the edit path).
    pub(super) fn new(
        menus: &Rc<HashMap<AutoDimension, Vec<String>>>,
        values_page: &Rc<ValuesPage>,
        existing: Option<&AutoGroup>,
    ) -> Self {
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

        let inner: Rc<RefCell<GroupsInner>> = Default::default();
        let group_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        root.append(&group_box);

        let add_group_group = adw::PreferencesGroup::new();
        let add_group_row = adw::ButtonRow::new();
        add_group_row.set_title(&crate::tr!("Add group"));
        add_group_row.set_start_icon_name(Some("list-add-symbolic"));
        add_group_group.add(&add_group_row);
        root.append(&add_group_group);

        {
            let inner = inner.clone();
            let group_box = group_box.clone();
            let menus = menus.clone();
            let values_page = values_page.clone();
            add_group_row.connect_activated(move |_| {
                add_group_card(&menus, &inner, &values_page, &group_box, None);
            });
        }

        if let Some(group) = existing {
            if let AutoNode::Logic { logic, nodes } = group.root.clone() {
                root_gate.set_selected(logic_position(logic));
                for node in nodes {
                    if let AutoNode::Logic { logic, nodes } = node {
                        let rules = nodes
                            .into_iter()
                            .filter_map(|n| match n {
                                AutoNode::Rule(criterion) => Some(criterion),
                                _ => None,
                            })
                            .collect::<Vec<_>>();
                        add_group_card(
                            menus,
                            &inner,
                            values_page,
                            &group_box,
                            Some((logic, &rules)),
                        );
                    }
                }
            }
        }

        Self {
            root,
            root_gate,
            inner,
        }
    }

    /// The saved tree: the root gate over every group card's gate and
    /// rules. A tree with no rules in it matches no game at all.
    pub(super) fn collect_root(&self) -> AutoNode {
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

/// Appends one rule-group card to the card stack, optionally prefilled
/// with a saved gate and rules (the edit path).
fn add_group_card(
    menus: &Rc<HashMap<AutoDimension, Vec<String>>>,
    inner: &Rc<RefCell<GroupsInner>>,
    values_page: &Rc<ValuesPage>,
    group_box: &gtk4::Box,
    existing: Option<(AutoLogic, &[AutoCriterion])>,
) {
    let (card, collector) = build_group_card(menus, inner, values_page);
    if let Some((logic, rules)) = existing {
        collector.gate.set_selected(logic_position(logic));
        for criterion in rules {
            collector.add_rule(Some(criterion));
        }
    }
    group_box.append(&card);
    inner.borrow_mut().groups.push((card, collector));
}

#[derive(Clone)]
struct GroupCollector {
    gate: adw::ComboRow,
    card: adw::PreferencesGroup,
    add_rule_row: adw::ButtonRow,
    menus: Rc<HashMap<AutoDimension, Vec<String>>>,
    values_page: Rc<ValuesPage>,
    rules: Rc<RefCell<Vec<CriterionEditor>>>,
}

impl GroupCollector {
    fn add_rule(&self, existing: Option<&AutoCriterion>) {
        let editor = CriterionEditor::new(
            &self.card,
            &self.add_rule_row,
            &self.menus,
            existing,
            &self.values_page,
        );
        self.rules.borrow_mut().push(editor);
    }
}

/// One rule-group card: its gate row (with the remove button), the rule
/// rows, and the add-rule row. Removals work through the shared
/// registry, so no call site needs to know a card's index.
fn build_group_card(
    menus: &Rc<HashMap<AutoDimension, Vec<String>>>,
    inner: &Rc<RefCell<GroupsInner>>,
    values_page: &Rc<ValuesPage>,
) -> (adw::PreferencesGroup, GroupCollector) {
    let card = adw::PreferencesGroup::new();

    let gate = adw::ComboRow::new();
    gate.set_title(&crate::tr!("Rules match"));
    gate.set_model(Some(&logic_model()));

    let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove.add_css_class(super::css::CSS_FLAT);
    remove.set_valign(gtk4::Align::Center);
    remove.set_tooltip_text(Some(&crate::tr!("Remove this group")));
    gate.add_suffix(&remove);
    card.add(&gate);

    let add_rule_row = adw::ButtonRow::new();
    add_rule_row.set_title(&crate::tr!("Add rule"));
    add_rule_row.set_start_icon_name(Some("list-add-symbolic"));
    card.add(&add_rule_row);

    {
        let card = card.clone();
        let inner = inner.clone();
        remove.connect_clicked(move |_| {
            inner.borrow_mut().groups.retain(|(group, _)| *group != card);
            if let Some(parent) = card.parent().and_downcast::<gtk4::Box>() {
                parent.remove(&card);
            }
        });
    }

    let collector = GroupCollector {
        gate,
        card: card.clone(),
        add_rule_row: add_rule_row.clone(),
        menus: menus.clone(),
        values_page: values_page.clone(),
        rules: Rc::new(RefCell::new(Vec::new())),
    };
    {
        let collector = collector.clone();
        add_rule_row.connect_activated(move |_| collector.add_rule(None));
    }
    (card, collector)
}

/// One rule: a dimension combo plus the control rows for that
/// dimension. All rows sit directly in the group card, so the card's
/// list box gives them row styling and working activation.
struct CriterionEditor {
    dimension: adw::ComboRow,
    controls: ControlRows,
}

impl CriterionEditor {
    fn new(
        card: &adw::PreferencesGroup,
        add_rule_row: &adw::ButtonRow,
        menus: &Rc<HashMap<AutoDimension, Vec<String>>>,
        initial: Option<&AutoCriterion>,
        values_page: &Rc<ValuesPage>,
    ) -> Self {
        let initial = initial.cloned().unwrap_or_default();

        let dimension = adw::ComboRow::new();
        dimension.set_title(&crate::tr!("Match on"));
        dimension.set_model(Some(&dimension_model()));
        if let Some(position) = AutoDimension::ALL
            .iter()
            .position(|d| *d == initial.dimension)
        {
            dimension.set_selected(position as u32);
        }

        let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class(super::css::CSS_FLAT);
        remove.set_valign(gtk4::Align::Center);
        remove.set_tooltip_text(Some(&crate::tr!("Remove this rule")));
        dimension.add_suffix(&remove);

        let controls = ControlRows::new(&initial, values_page);

        // The card gets the rule's rows; the add-rule row stays last.
        let rows: Vec<gtk4::Widget> = [dimension.clone().upcast()]
            .into_iter()
            .chain(controls.rows())
            .collect();
        card.remove(add_rule_row);
        for row in &rows {
            card.add(row);
        }
        card.add(add_rule_row);

        {
            let card = card.clone();
            let rows = rows.clone();
            remove.connect_clicked(move |_| {
                for row in &rows {
                    card.remove(row);
                }
            });
        }

        // Dimension changes show that dimension's control rows; value
        // dimensions refill their check list from scratch, since
        // picked values belong to the dimension they were picked in.
        {
            let controls = controls.clone();
            let menus = menus.clone();
            dimension.connect_selected_notify(move |combo| {
                let dimension = AutoDimension::ALL
                    .get(combo.selected() as usize)
                    .copied()
                    .unwrap_or_default();
                controls.sync(dimension, HashSet::new(), &menus);
            });
        }
        controls.sync(
            initial.dimension,
            initial.values.iter().cloned().collect(),
            menus,
        );

        Self {
            dimension,
            controls,
        }
    }

    fn collect(&self) -> AutoCriterion {
        let dimension = AutoDimension::ALL
            .get(self.dimension.selected() as usize)
            .copied()
            .unwrap_or_default();
        let mut criterion = AutoCriterion {
            dimension,
            ..Default::default()
        };
        match dimension {
            AutoDimension::Released => {
                criterion.from = self.controls.from_pick.typed().unwrap_or_default();
                criterion.to = self.controls.to_pick.typed().unwrap_or_default();
            }
            AutoDimension::Playtime => {
                let min = self.controls.min_hours.value();
                let max = self.controls.max_hours.value();
                criterion.min_hours = (min > 0.0).then_some(min);
                criterion.max_hours = (max > 0.0).then_some(max);
            }
            _ => criterion.values = picked_values(&self.controls.value_checks),
        }
        criterion
    }
}

/// The per-dimension control rows of one rule: the values row (with its
/// unparented check list, hosted by the values page while picking) for
/// named dimensions, from/to date rows for release windows, and
/// min/max spin rows for playtime.
#[derive(Clone)]
struct ControlRows {
    values_list: gtk4::ListBox,
    value_checks: ValueChecks,
    values_row: adw::ActionRow,
    from_pick: Rc<DatePick>,
    from_row: adw::ActionRow,
    to_pick: Rc<DatePick>,
    to_row: adw::ActionRow,
    min_hours: adw::SpinRow,
    max_hours: adw::SpinRow,
}

impl ControlRows {
    fn new(initial: &AutoCriterion, values_page: &Rc<ValuesPage>) -> Self {
        // The check list starts unparented; the values row hands it to
        // the values page when activated and gets it back on return.
        let values_list = gtk4::ListBox::new();
        values_list.add_css_class(super::css::CSS_BOXED_LIST);
        let values_row = adw::ActionRow::new();
        values_row.set_title(&crate::tr!("Values"));
        values_row.set_activatable(true);
        let chevron = gtk4::Image::from_icon_name("go-next-symbolic");
        chevron.add_css_class(super::css::CSS_DIM_LABEL);
        values_row.add_suffix(&chevron);
        {
            let values_page = values_page.clone();
            let list = values_list.clone();
            values_row.connect_activated(move |_| values_page.open(&list));
        }

        let from_pick = Rc::new(DatePick::new(&initial.from));
        from_pick.set_tooltip(&crate::tr!("From"));
        let from_row = date_row(&crate::tr!("From"), &from_pick);
        let to_pick = Rc::new(DatePick::new(&initial.to));
        to_pick.set_tooltip(&crate::tr!("To"));
        let to_row = date_row(&crate::tr!("To"), &to_pick);

        Self {
            values_list,
            value_checks: Default::default(),
            values_row,
            from_pick,
            from_row,
            to_pick,
            to_row,
            min_hours: hours_row(&crate::tr!("Min hours"), initial.min_hours),
            max_hours: hours_row(&crate::tr!("Max hours"), initial.max_hours),
        }
    }

    /// The control rows in card order, for insertion and removal.
    fn rows(&self) -> impl Iterator<Item = gtk4::Widget> {
        [
            self.values_row.clone().upcast(),
            self.from_row.clone().upcast(),
            self.to_row.clone().upcast(),
            self.min_hours.clone().upcast(),
            self.max_hours.clone().upcast(),
        ]
        .into_iter()
    }

    /// Shows the rows `dimension` needs; value dimensions refill the
    /// check list with `picked` pre-checked.
    fn sync(
        &self,
        dimension: AutoDimension,
        picked: HashSet<String>,
        menus: &HashMap<AutoDimension, Vec<String>>,
    ) {
        let named = !matches!(
            dimension,
            AutoDimension::Released | AutoDimension::Playtime
        );
        let dates = dimension == AutoDimension::Released;
        let hours = dimension == AutoDimension::Playtime;
        self.values_row.set_visible(named);
        self.from_row.set_visible(dates);
        self.to_row.set_visible(dates);
        self.min_hours.set_visible(hours);
        self.max_hours.set_visible(hours);
        if named {
            let names = menus.get(&dimension).cloned().unwrap_or_default();
            fill_check_list(
                &self.values_list,
                &self.value_checks,
                &names,
                picked,
                &self.values_row,
            );
        }
    }
}

/// A release-bound row: title and subtitle with a calendar menu button
/// as the suffix; the subtitle follows the pick when the popover
/// closes.
fn date_row(title: &str, pick: &Rc<DatePick>) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.add_suffix(pick.button());
    {
        let row = row.clone();
        let pick = pick.clone();
        pick.popover().connect_closed(move |_| {
            row.set_subtitle(&pick.typed().unwrap_or_else(|| crate::tr!("Any time")));
        });
    }
    row.set_subtitle(&pick.typed().unwrap_or_else(|| crate::tr!("Any time")));
    row
}

fn hours_row(title: &str, initial: Option<f64>) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(0.0, 9999.0, 0.5);
    row.set_digits(1);
    row.set_title(title);
    row.set_value(initial.unwrap_or(0.0));
    row
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

fn dimension_model() -> gtk4::StringList {
    let model = gtk4::StringList::new(&[]);
    for dimension in AutoDimension::ALL {
        model.append(dimension.display_label());
    }
    model
}
