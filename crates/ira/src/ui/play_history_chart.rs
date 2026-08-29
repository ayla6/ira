use super::bar_chart::BarChart;
use super::css::*;
use super::helpers::{clear_children, esc, format_duration};
use adw::prelude::*;
use chrono::Datelike;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

const Y_AXIS_W: i32 = 34;
const SIDEBAR_W: i32 = 260;
const MAX_TOP_GAMES: usize = 5;
const COLOR_HEX: &[&str] = &[
    "#3584e4", "#33d17a", "#ff7800", "#e01b24", "#9141ac", "#f5c211",
];
const OTHER_HEX: &str = "#888a8a";

pub(super) fn color_hex(i: usize) -> &'static str {
    COLOR_HEX[i.min(COLOR_HEX.len() - 1)]
}
pub(super) fn other_hex() -> &'static str {
    OTHER_HEX
}

#[derive(Clone)]
pub(super) struct BarSegment {
    pub value: f64,
    pub color_index: Option<usize>,
    pub label: String,
}
#[derive(Clone)]
pub(super) struct DaySession {
    pub session_id: i64,
    pub label: String,
    pub value: String,
}
#[derive(Clone)]
pub(super) struct DayDetail {
    pub session_id: Option<i64>,
    pub label: String,
    pub value: String,
    pub color_hex: Option<String>,
    pub sessions: Vec<DaySession>,
}
#[derive(Clone)]
pub(super) struct DayData {
    pub date: chrono::NaiveDate,
    pub total: f64,
    pub segments: Vec<BarSegment>,
    pub details: Vec<DayDetail>,
}
#[derive(Clone)]
pub(super) struct WeekData {
    pub week_start: chrono::NaiveDate,
    pub days: Vec<DayData>,
    pub week_total: f64,
}
pub(super) struct GameColorAssignment {
    pub top_games: Vec<i64>,
    pub color_map: HashMap<i64, usize>,
}

pub(super) type DeleteSessionFn = Rc<dyn Fn(i64, bool)>;

pub(super) fn assign_game_colors(s: &[ira_models::PlaySession]) -> GameColorAssignment {
    let mut t: HashMap<i64, f64> = HashMap::new();
    for s in s {
        *t.entry(s.game_id).or_default() += s.duration_seconds as f64;
    }
    let mut v: Vec<(i64, f64)> = t.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top: Vec<i64> = v.iter().take(MAX_TOP_GAMES).map(|(id, _)| *id).collect();
    let map = top.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    GameColorAssignment {
        top_games: top,
        color_map: map,
    }
}

/// Returns (interval_seconds, nice_max_seconds). Prefers smallest interval that fits.
fn axis_config(max_v: f64) -> (f64, f64) {
    const MAX_LINES: usize = 6;
    // 30m interval only when max ≤ 1h
    if max_v <= 3600.0 {
        let interval = 1800.0;
        let nice = ((max_v / interval).ceil() * interval).max(interval);
        return (interval, nice);
    }
    // Try 1h, then 2h
    for &interval in &[3600.0, 7200.0] {
        let nice = ((max_v / interval).ceil() * interval).max(interval);
        let num_lines = (nice / interval) as usize + 1;
        if num_lines <= MAX_LINES {
            return (interval, nice);
        }
    }
    let interval = 7200.0;
    let nice = ((max_v / interval).ceil() * interval).max(interval);
    (interval, nice)
}

struct State {
    weeks: Vec<WeekData>,
    current_idx: usize,
    selected_day: usize,
    is_single_game: bool,
    chart: glib::WeakRef<BarChart>,
    sidebar_header: glib::WeakRef<gtk4::Label>,
    sidebar_list: glib::WeakRef<gtk4::ListBox>,
    week_label: glib::WeakRef<gtk4::Label>,
    prev_w: glib::WeakRef<gtk4::Button>,
    next_w: glib::WeakRef<gtk4::Button>,
    prev_d: glib::WeakRef<gtk4::Button>,
    next_d: glib::WeakRef<gtk4::Button>,
    on_delete: Option<DeleteSessionFn>,
    ctrl_held: Rc<Cell<bool>>,
}

pub(super) fn build_weekly_chart(
    weeks: Vec<WeekData>,
    is_single_game: bool,
    on_delete: Option<DeleteSessionFn>,
    focus_week: Option<chrono::NaiveDate>,
    ctrl_held: Rc<Cell<bool>>,
    empty_hint: Option<String>,
) -> gtk4::Widget {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    container.set_focusable(true);
    let focus_target = container.downgrade();
    container.connect_map(move |_| {
        if let Some(t) = focus_target.upgrade() {
            t.grab_focus();
        }
    });

    let week_label = gtk4::Label::new(Some(""));
    week_label.set_halign(gtk4::Align::Start);
    week_label.set_hexpand(true);
    week_label.set_xalign(0.0);
    week_label.add_css_class(CSS_HEADING);

    let sidebar_header = gtk4::Label::new(Some(""));
    sidebar_header.set_halign(gtk4::Align::Start);
    sidebar_header.set_xalign(0.0);
    sidebar_header.add_css_class(CSS_HEADING);
    sidebar_header.set_width_request(SIDEBAR_W);
    sidebar_header.set_margin_start(12);

    let header_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    header_row.set_hexpand(true);
    header_row.append(&week_label);
    header_row.append(&sidebar_header);
    container.append(&header_row);

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    row.set_hexpand(true);
    row.set_vexpand(true);

    // Chart area: BarChart + day labels + nav (nav centered relative to chart only)
    let chart_area = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    chart_area.set_hexpand(true);
    chart_area.set_vexpand(true);

    let chart = BarChart::new();
    chart.set_hexpand(true);
    chart.set_vexpand(true);

    // Overlay keeps the full chart skeleton visible when there is no data;
    // the hint label ignores input so chart clicks/hover still work.
    let chart_overlay = gtk4::Overlay::new();
    chart_overlay.set_hexpand(true);
    chart_overlay.set_vexpand(true);
    chart_overlay.set_child(Some(&chart));
    if let Some(hint) = &empty_hint {
        let hint_label = gtk4::Label::new(Some(hint));
        hint_label.add_css_class(CSS_DIM_LABEL);
        hint_label.set_halign(gtk4::Align::Center);
        hint_label.set_valign(gtk4::Align::Center);
        hint_label.set_can_target(false);
        chart_overlay.add_overlay(&hint_label);
    }

    let day_labels = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    day_labels.set_hexpand(true);
    day_labels.set_margin_end(Y_AXIS_W);
    for name in &["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] {
        let l = gtk4::Label::new(Some(name));
        l.add_css_class(CSS_DIM_LABEL);
        l.set_hexpand(true);
        day_labels.append(&l);
    }

    let (nav, pw, pd, nd, nw) = build_nav();

    chart_area.append(&chart_overlay);
    chart_area.append(&day_labels);
    chart_area.append(&nav);
    row.append(&chart_area);

    // Sidebar — fills to bottom of window
    let sidebar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_box.set_width_request(SIDEBAR_W);
    sidebar_box.set_valign(gtk4::Align::Fill);
    sidebar_box.set_margin_start(12);

    let sidebar_list = gtk4::ListBox::new();
    sidebar_list.add_css_class(CSS_BOXED_LIST);
    sidebar_list.set_selection_mode(gtk4::SelectionMode::None);
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&sidebar_list));
    scroll.set_vexpand(true);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    sidebar_box.append(&scroll);
    row.append(&sidebar_box);
    container.append(&row);

    let initial_idx = focus_week
        .and_then(|fw| weeks.iter().position(|w| w.week_start == fw))
        .unwrap_or_else(|| weeks.len().saturating_sub(1));

    let state = Rc::new(RefCell::new(State {
        current_idx: initial_idx,
        selected_day: 6,
        is_single_game,
        weeks,
        chart: chart.downgrade(),
        sidebar_header: sidebar_header.downgrade(),
        sidebar_list: sidebar_list.downgrade(),
        week_label: week_label.downgrade(),
        prev_w: pw.downgrade(),
        next_w: nw.downgrade(),
        prev_d: pd.downgrade(),
        next_d: nd.downgrade(),
        on_delete,
        ctrl_held,
    }));

    {
        let st = state.clone();
        chart.connect_day_activated(move |idx| select_day(&st, idx));
    }
    {
        let st = state.clone();
        pw.connect_clicked(move |_| {
            let mut s = st.borrow_mut();
            if s.current_idx > 0 {
                s.current_idx -= 1;
                drop(s);
                rebuild(&st, None);
            }
        });
    }
    {
        let st = state.clone();
        nw.connect_clicked(move |_| {
            let mut s = st.borrow_mut();
            if s.current_idx + 1 < s.weeks.len() {
                s.current_idx += 1;
                drop(s);
                rebuild(&st, None);
            }
        });
    }
    {
        let st = state.clone();
        pd.connect_clicked(move |_| nav_day(&st, -1));
    }
    {
        let st = state.clone();
        nd.connect_clicked(move |_| nav_day(&st, 1));
    }

    {
        let st = state.clone();
        let kc = gtk4::EventControllerKey::new();
        kc.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let ctrl = {
            let s = st.borrow();
            s.ctrl_held.clone()
        };
        let ctrl_pressed = ctrl.clone();
        let ctrl_released = ctrl;
        kc.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Control_L || key == gtk4::gdk::Key::Control_R {
                ctrl_pressed.set(true);
            }
            match key {
                gtk4::gdk::Key::Left | gtk4::gdk::Key::KP_Left => {
                    nav_day(&st, -1);
                    glib::Propagation::Stop
                }
                gtk4::gdk::Key::Right | gtk4::gdk::Key::KP_Right => {
                    nav_day(&st, 1);
                    glib::Propagation::Stop
                }
                gtk4::gdk::Key::Up | gtk4::gdk::Key::KP_Up => {
                    nav_week(&st, -1);
                    glib::Propagation::Stop
                }
                gtk4::gdk::Key::Down | gtk4::gdk::Key::KP_Down => {
                    nav_week(&st, 1);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        kc.connect_key_released(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Control_L || key == gtk4::gdk::Key::Control_R {
                ctrl_released.set(false);
            }
        });
        container.add_controller(kc);
    }

    rebuild(&state, None);
    container.upcast()
}

fn nav_day(st: &Rc<RefCell<State>>, dir: i32) {
    let (wk, dy) = {
        let s = st.borrow();
        let nd = s.selected_day as i32 + dir;
        if nd < 0 {
            if s.current_idx > 0 {
                (s.current_idx - 1, 6usize)
            } else {
                return;
            }
        } else if nd > 6 {
            if s.current_idx + 1 < s.weeks.len() {
                (s.current_idx + 1, 0usize)
            } else {
                return;
            }
        } else {
            (s.current_idx, nd as usize)
        }
    };
    st.borrow_mut().current_idx = wk;
    rebuild(st, Some(dy));
}

fn nav_week(st: &Rc<RefCell<State>>, dir: i32) {
    let mut s = st.borrow_mut();
    let ni = s.current_idx as i32 + dir;
    if ni >= 0 && (ni as usize) < s.weeks.len() {
        s.current_idx = ni as usize;
        drop(s);
        rebuild(st, None);
    }
}

fn rebuild(state: &Rc<RefCell<State>>, forced: Option<usize>) {
    let (week, is_sg, today) = {
        let s = state.borrow();
        (
            s.weeks[s.current_idx].clone(),
            s.is_single_game,
            chrono::Local::now().date_naive(),
        )
    };
    let max_v = week.days.iter().map(|d| d.total).fold(1.0, f64::max);
    let (interval, nmax) = axis_config(max_v);

    let sel = forced.unwrap_or_else(|| {
        week.days
            .iter()
            .position(|d| d.date == today)
            .unwrap_or_else(|| week.days.iter().rposition(|d| d.total > 0.0).unwrap_or(6))
    });
    state.borrow_mut().selected_day = sel;

    {
        let s = state.borrow();
        let Some(chart) = s.chart.upgrade() else {
            return;
        };
        chart.set_data(&week.days, nmax, interval, is_sg);
        chart.set_selected_day(sel, is_sg);
    }

    {
        let s = state.borrow();
        update_stats(&s);
        if let Some(w) = s.prev_w.upgrade() {
            w.set_sensitive(s.current_idx > 0);
        }
        if let Some(w) = s.next_w.upgrade() {
            w.set_sensitive(s.current_idx + 1 < s.weeks.len());
        }
        update_day_btns(&s);
    }
}

fn update_day_btns(s: &State) {
    if let Some(w) = s.prev_d.upgrade() {
        w.set_sensitive(s.current_idx > 0 || s.selected_day > 0);
    }
    if let Some(w) = s.next_d.upgrade() {
        w.set_sensitive(s.current_idx + 1 < s.weeks.len() || s.selected_day < 6);
    }
}

fn select_day(state: &Rc<RefCell<State>>, idx: usize) {
    let is_sg = {
        let mut s = state.borrow_mut();
        s.selected_day = idx;
        s.is_single_game
    };
    {
        let s = state.borrow();
        let Some(chart) = s.chart.upgrade() else {
            return;
        };
        chart.set_selected_day(idx, is_sg);
        update_stats(&s);
        update_day_btns(&s);
    }
}

fn build_nav() -> (
    gtk4::Box,
    gtk4::Button,
    gtk4::Button,
    gtk4::Button,
    gtk4::Button,
) {
    let b = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    b.set_halign(gtk4::Align::Center);
    b.set_margin_top(4);
    let pw = gtk4::Button::from_icon_name("go-first-symbolic");
    pw.add_css_class(CSS_CIRCULAR);
    pw.set_tooltip_text(Some(&crate::tr!("Previous week")));
    let pd = gtk4::Button::from_icon_name("go-previous-symbolic");
    pd.add_css_class(CSS_CIRCULAR);
    pd.set_tooltip_text(Some(&crate::tr!("Previous day")));
    let nd = gtk4::Button::from_icon_name("go-next-symbolic");
    nd.add_css_class(CSS_CIRCULAR);
    nd.set_tooltip_text(Some(&crate::tr!("Next day")));
    let nw = gtk4::Button::from_icon_name("go-last-symbolic");
    nw.add_css_class(CSS_CIRCULAR);
    nw.set_tooltip_text(Some(&crate::tr!("Next week")));
    b.append(&pw);
    b.append(&pd);
    b.append(&nd);
    b.append(&nw);
    (b, pw, pd, nd, nw)
}

fn update_stats(s: &State) {
    let week = &s.weeks[s.current_idx];
    let day = &week.days[s.selected_day];
    let today = chrono::Local::now().date_naive();
    let we = week.week_start + chrono::Duration::days(6);
    let dash = "<span weight=\"normal\">\u{2014}</span>";

    let wt = if week.week_start <= today && today <= we {
        format!(
            "This week {} {}",
            dash,
            format_duration(week.week_total as i64)
        )
    } else if week.week_start.month() == we.month() {
        format!(
            "{} \u{2013} {} {} {}",
            week.week_start.format("%b %-d"),
            we.format("%-d"),
            dash,
            format_duration(week.week_total as i64)
        )
    } else {
        format!(
            "{} \u{2013} {} {} {}",
            week.week_start.format("%b %-d"),
            we.format("%b %-d"),
            dash,
            format_duration(week.week_total as i64)
        )
    };
    if let Some(w) = s.week_label.upgrade() {
        w.set_markup(&wt);
    }

    let ht = if day.total > 0.0 {
        format!(
            "{} {} {}",
            day.date.format("%a, %b %d"),
            dash,
            format_duration(day.total as i64)
        )
    } else {
        day.date.format("%a, %b %d").to_string()
    };
    if let Some(w) = s.sidebar_header.upgrade() {
        w.set_markup(&ht);
    }

    update_sidebar(s, day);
}

fn update_sidebar(s: &State, day: &DayData) {
    let Some(sidebar_list) = s.sidebar_list.upgrade() else {
        return;
    };
    clear_children(&sidebar_list);
    if day.details.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title(&crate::tr!("No sessions"));
        r.add_css_class(CSS_DIM_LABEL);
        sidebar_list.append(&r);
        return;
    }
    for d in &day.details {
        if d.sessions.is_empty() {
            let r = adw::ActionRow::new();
            r.set_title(&esc(&d.label));
            let v = gtk4::Label::new(Some(&d.value));
            v.add_css_class(CSS_DIM_LABEL);
            r.add_suffix(&v);
            if let Some(h) = &d.color_hex {
                let sw = gtk4::Label::new(None);
                sw.set_markup(&format!("<span foreground=\"{}\">\u{25A0}</span>", h));
                r.add_prefix(&sw);
            }
            if let (Some(sid), Some(on_delete)) = (d.session_id, &s.on_delete) {
                r.add_suffix(&make_delete_button(sid, on_delete, &s.ctrl_held));
            }
            sidebar_list.append(&r);
        } else {
            let ex = adw::ExpanderRow::new();
            ex.set_title(&esc(&d.label));
            let v = gtk4::Label::new(Some(&d.value));
            v.add_css_class(CSS_DIM_LABEL);
            ex.add_suffix(&v);
            if let Some(h) = &d.color_hex {
                let sw = gtk4::Label::new(None);
                sw.set_markup(&format!("<span foreground=\"{}\">\u{25A0}</span>", h));
                ex.add_prefix(&sw);
            }
            for ses in &d.sessions {
                let sub = adw::ActionRow::new();
                sub.set_title(&ses.label);
                let sv = gtk4::Label::new(Some(&ses.value));
                sv.add_css_class(CSS_DIM_LABEL);
                sub.add_suffix(&sv);
                if let Some(on_delete) = &s.on_delete {
                    sub.add_suffix(&make_delete_button(ses.session_id, on_delete, &s.ctrl_held));
                }
                ex.add_row(&sub);
            }
            sidebar_list.append(&ex);
        }
    }
}

fn make_delete_button(
    session_id: i64,
    on_delete: &DeleteSessionFn,
    ctrl_held: &Rc<Cell<bool>>,
) -> gtk4::Widget {
    let btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    btn.add_css_class(CSS_FLAT);
    btn.add_css_class(CSS_SESSION_DELETE);
    btn.set_valign(gtk4::Align::Center);
    btn.set_tooltip_text(Some(&crate::tr!(
        "Delete session (hold Ctrl to skip confirmation)"
    )));
    let on_delete = on_delete.clone();
    let ctrl_held = ctrl_held.clone();
    btn.connect_clicked(move |_| {
        on_delete(session_id, ctrl_held.get());
    });
    btn.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ira_models::PlaySession;

    #[test]
    fn test_color_hex() {
        assert_eq!(color_hex(0), "#3584e4");
        assert_eq!(color_hex(10), color_hex(5));
    }

    #[test]
    fn test_axis_config() {
        assert_eq!(axis_config(900.0), (1800.0, 1800.0)); // ≤1h → 30m interval
        assert_eq!(axis_config(2400.0), (1800.0, 3600.0)); // ≤1h → 30m, max 1h
        assert_eq!(axis_config(5400.0), (3600.0, 7200.0)); // >1h → 1h, max 2h (3 lines)
        assert_eq!(axis_config(10800.0), (3600.0, 10800.0)); // 3h → 1h (4 lines)
        assert_eq!(axis_config(18000.0), (3600.0, 18000.0)); // 5h → 1h (6 lines, max)
        assert_eq!(axis_config(21600.0), (7200.0, 21600.0)); // 6h → 2h (1h would be 7 lines)
        assert_eq!(axis_config(28800.0), (7200.0, 28800.0)); // 8h → 2h (5 lines)
    }

    #[test]
    fn test_assign_colors() {
        let s: Vec<PlaySession> = (1..=8i64)
            .map(|i| PlaySession {
                id: i,
                game_id: i,
                variant_id: None,
                started_at: 1000 + i,
                ended_at: 1000 + i + 100 * (9 - i),
                duration_seconds: 100 * (9 - i),
            })
            .collect();
        let a = assign_game_colors(&s);
        assert_eq!(a.top_games.len(), 5);
        assert!(!a.color_map.contains_key(&8));
    }
}
