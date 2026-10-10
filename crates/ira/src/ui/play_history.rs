use super::helpers::{clear_children, format_duration};
use super::play_history_chart::{
    build_weekly_chart, color_hex, color_index_for_game, BarSegment, ChartFocus, DayData,
    DayDetail, DaySession, WeekData,
};
use super::state::SharedState;
use adw::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

type RebuildFn = std::rc::Rc<dyn Fn(Option<ChartFocus>)>;
type ToastFn = Rc<dyn Fn(String)>;

struct DeletedSession {
    session: ira_models::PlaySession,
    focus: ChartFocus,
}

#[derive(Clone, Copy)]
struct SessionEdit {
    session_id: i64,
    started_at: i64,
    pending: bool,
}

#[derive(Default)]
struct SessionEditHistory {
    pending: HashSet<i64>,
    undo: Vec<SessionEdit>,
    redo: Vec<SessionEdit>,
}

impl SessionEditHistory {
    fn toggle(&mut self, session_id: i64, started_at: i64) -> SessionEdit {
        let pending = !self.pending.contains(&session_id);
        self.set_pending(session_id, pending);
        let edit = SessionEdit {
            session_id,
            started_at,
            pending,
        };
        self.undo.push(edit);
        self.redo.clear();
        edit
    }

    fn undo(&mut self) -> Option<SessionEdit> {
        let edit = self.undo.pop()?;
        self.set_pending(edit.session_id, !edit.pending);
        self.redo.push(edit);
        Some(edit)
    }

    fn redo(&mut self) -> Option<SessionEdit> {
        let edit = self.redo.pop()?;
        self.set_pending(edit.session_id, edit.pending);
        self.undo.push(edit);
        Some(edit)
    }

    fn set_pending(&mut self, session_id: i64, pending: bool) {
        if pending {
            self.pending.insert(session_id);
        } else {
            self.pending.remove(&session_id);
        }
    }
}

fn session_edit_message(pending: bool, started_at: i64) -> String {
    let timestamp = format!("{} {}", ts_to_date(started_at), format_time(started_at));
    if pending {
        crate::tr!("Deleted session: {}").replacen("{}", &timestamp, 1)
    } else {
        crate::tr!("Restored session: {}").replacen("{}", &timestamp, 1)
    }
}

/// An open history window: a per-game dialog (`Some` game id) or the daily
/// dialog (`None`, matches every game). Unregistered on close; the rebuild
/// owns its widgets, so a forgotten unregister would leak the window.
pub struct HistoryDialog {
    game_id: Option<i64>,
    rebuild: RebuildFn,
}

/// Whether an open history window shows sessions of the changed game: the
/// daily window shows everything, a per-game window shows its own game and
/// its playtime-linked members.
fn history_dialog_matches(
    entry_game_id: Option<i64>,
    linked: &[i64],
    changed_game_id: i64,
) -> bool {
    entry_game_id.is_none_or(|id| id == changed_game_id || linked.contains(&changed_game_id))
}

/// Cursor placement for a changed session: its own day, so an insert
/// reveals the new entry and a delete stays where the entry was (falling
/// back through the usual focus resolution when that week is gone).
pub(super) fn session_focus(started_at: i64) -> ChartFocus {
    let day = ts_to_date(started_at);
    ChartFocus {
        week: week_start(day),
        day,
    }
}

/// Rebuild every open history window showing the changed game's sessions,
/// placed on the changed entry. Call after a session row is inserted (game
/// close) or deleted (from another window).
pub(super) fn notify_sessions_changed(
    state: &SharedState,
    changed_game_id: i64,
    focus: Option<ChartFocus>,
) {
    // Snapshot the registry first: matching borrows state, and each rebuild
    // below borrows it too — nothing may be held across those calls.
    let entries: Vec<(Option<i64>, RebuildFn)> = {
        state
            .borrow()
            .history_dialogs
            .borrow()
            .iter()
            .map(|d| (d.game_id, d.rebuild.clone()))
            .collect()
    };
    for (game_id, rebuild) in entries {
        let linked = game_id
            .map(|id| super::helpers::linked_members(state, id))
            .unwrap_or_default();
        if !history_dialog_matches(game_id, &linked, changed_game_id) {
            continue;
        }
        rebuild(focus);
    }
}

/// Some sources store milliseconds; normalize everything to seconds.
fn ts_secs(timestamp: i64) -> i64 {
    if timestamp > 1_000_000_000_000 {
        timestamp / 1000
    } else {
        timestamp
    }
}

fn format_time(timestamp: i64) -> String {
    super::helpers::local_datetime(ts_secs(timestamp))
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn ts_to_date(ts: i64) -> chrono::NaiveDate {
    super::helpers::local_datetime(ts_secs(ts))
        .map(|dt| dt.date_naive())
        .unwrap_or_default()
}

fn week_start(date: chrono::NaiveDate) -> chrono::NaiveDate {
    use chrono::Datelike;
    let weekday = date.weekday().num_days_from_monday() as i64;
    date - chrono::Duration::days(weekday)
}

fn current_week_start() -> chrono::NaiveDate {
    week_start(chrono::Local::now().date_naive())
}

fn generate_week_days(ws: chrono::NaiveDate) -> Vec<chrono::NaiveDate> {
    (0..7).map(|i| ws + chrono::Duration::days(i)).collect()
}

fn rebuild_game_history(
    state: &SharedState,
    box_: &gtk4::Box,
    game_id: i64,
    variant_id: Option<i64>,
    game_name: &str,
    history: &Rc<RefCell<SessionEditHistory>>,
    focus: Option<ChartFocus>,
) {
    let db = state.borrow().db.clone();
    let mut sessions = Vec::new();
    for member in super::helpers::linked_members(state, game_id) {
        let variant = if member == game_id { variant_id } else { None };
        sessions.extend(
            ira_db::get_sessions_for_game(&db, member, variant).unwrap_or_else(|e| {
                eprintln!("Failed to read sessions for game {}: {e}", member);
                Vec::new()
            }),
        );
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.started_at));
    clear_children(box_);
    let session_starts: HashMap<i64, i64> = sessions
        .iter()
        .map(|session| (session.id, session.started_at))
        .collect();
    let on_delete: super::play_history_chart::DeleteSessionFn = {
        let state = state.clone();
        let box_ = box_.clone();
        let history = history.clone();
        let game_name = game_name.to_string();
        Rc::new(move |session_id: i64| {
            let Some(started_at) = session_starts.get(&session_id).copied() else {
                return;
            };
            history.borrow_mut().toggle(session_id, started_at);
            // Other history windows show committed database state until close.
            rebuild_game_history(
                &state,
                &box_,
                game_id,
                variant_id,
                &game_name,
                &history,
                Some(session_focus(started_at)),
            );
        })
    };
    // Always render the full chart (axes, day labels, nav, sidebar); when
    // nothing was ever recorded it doubles as the empty state.
    let empty_hint = sessions
        .is_empty()
        .then(|| crate::tr!("No play sessions recorded yet"));
    let pending = history.borrow().pending.clone();
    let weeks = compute_game_weeks(&sessions, game_name, &pending);
    box_.append(&build_weekly_chart(
        weeks,
        true,
        Some(on_delete),
        focus,
        empty_hint,
    ));
}

fn history_shortcut_redo(key: gtk4::gdk::Key, modifiers: gtk4::gdk::ModifierType) -> Option<bool> {
    if !matches!(key, gtk4::gdk::Key::z | gtk4::gdk::Key::Z)
        || !modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK)
    {
        return None;
    }
    Some(modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK))
}

fn wire_history_keys(
    dialog: &adw::Dialog,
    history: Rc<RefCell<SessionEditHistory>>,
    rebuild: RebuildFn,
    toast: ToastFn,
) {
    let key = gtk4::EventControllerKey::new();
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(redo) = history_shortcut_redo(key, modifiers) else {
            return glib::Propagation::Proceed;
        };
        let edit = if redo {
            history.borrow_mut().redo()
        } else {
            history.borrow_mut().undo()
        };
        let Some(edit) = edit else {
            return glib::Propagation::Proceed;
        };
        toast(session_edit_message(
            if redo { edit.pending } else { !edit.pending },
            edit.started_at,
        ));
        rebuild(Some(session_focus(edit.started_at)));
        glib::Propagation::Stop
    });
    dialog.add_controller(key);
}

pub fn show_play_history_dialog(
    state: &SharedState,
    game_id: i64,
    variant_id: Option<i64>,
) -> adw::Dialog {
    let game_name = state
        .borrow()
        .find_game(game_id, variant_id)
        .map(|g| g.name)
        .unwrap_or_default();

    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Play history for {}").replacen("{}", &game_name, 1));
    dialog.set_content_width(820);
    dialog.set_content_height(500);

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);
    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&toolbar_view));
    let toast: ToastFn = {
        let overlay = toast_overlay.clone();
        Rc::new(move |message| {
            let toast = adw::Toast::new(&message);
            toast.set_timeout(3);
            overlay.add_toast(toast);
        })
    };
    let history = Rc::new(RefCell::new(SessionEditHistory::default()));

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);

    // Stored strongly in the registry (unregistered on close).
    let rebuild: RebuildFn = std::rc::Rc::new({
        let state = state.clone();
        let box_ = box_.clone();
        let game_name = game_name.clone();
        let history = history.clone();
        move |focus: Option<ChartFocus>| {
            rebuild_game_history(
                &state, &box_, game_id, variant_id, &game_name, &history, focus,
            );
        }
    });

    wire_history_keys(&dialog, history.clone(), rebuild.clone(), toast.clone());
    rebuild(None);
    // Strong registration: the rebuild owns its widgets, so the entry must
    // go on close (below) — that also breaks the registry → window cycle.
    let close_rebuild = rebuild.clone();
    state
        .borrow_mut()
        .history_dialogs
        .borrow_mut()
        .push(HistoryDialog {
            game_id: Some(game_id),
            rebuild,
        });

    toolbar_view.set_content(Some(&box_));
    dialog.set_child(Some(&toast_overlay));
    dialog.present(Some(&state.borrow().window));

    let refresh_state = state.clone();
    let history = history.clone();
    dialog.connect_closed(move |_| {
        // Unregister first: the registry's strong rebuild would otherwise
        // keep this window (and its widgets) alive.
        refresh_state
            .borrow_mut()
            .history_dialogs
            .borrow_mut()
            .retain(|d| !Rc::ptr_eq(&close_rebuild, &d.rebuild));
        let pending = history.borrow_mut().pending.drain().collect::<Vec<_>>();
        let mut changed = Vec::new();
        for session_id in pending {
            match delete_session_from_db(&refresh_state, session_id) {
                Ok(deleted) => changed.push((deleted.session.game_id, deleted.focus)),
                Err(error) => eprintln!("Failed to delete session: {error}"),
            }
        }
        for (changed_game_id, focus) in changed {
            notify_sessions_changed(&refresh_state, changed_game_id, Some(focus));
        }
        let still_active = refresh_state.borrow().displayed_db_id == game_id;
        let game = if still_active {
            refresh_state.borrow().find_game(game_id, variant_id)
        } else {
            None
        };
        if let Some(game) = game {
            super::game_display::display_game(&game, &refresh_state);
        }
        // The dialog allocates many small widgets; freeing them in scattered
        // order leaves holes in the glibc arena that RSS holds onto. Trim the
        // arena tail once the dialog is gone so repeated opens don't creep RSS.
        glib::idle_add_local_once(|| unsafe {
            super::state::malloc_trim(0);
        });
    });

    dialog
}

fn delete_session_from_db(state: &SharedState, session_id: i64) -> Result<DeletedSession, String> {
    let db = state.borrow().db.clone();
    let Some((session, last_played)) = ira_db::delete_session_with_metadata(&db, session_id)?
    else {
        return Err(format!("session {} not found", session_id));
    };
    let focus = session_focus(session.started_at);
    let hours = (session.duration_seconds as f64) / 3600.0;
    {
        let mut s = state.borrow_mut();
        for g in &mut s.games {
            if g.db_id == session.game_id && g.variant_id == session.variant_id {
                g.playtime = (g.playtime - hours).max(0.0);
                g.last_played = last_played;
            }
        }
    }
    Ok(DeletedSession { session, focus })
}

fn compute_game_weeks(
    sessions: &[ira_models::PlaySession],
    game_name: &str,
    pending_deletions: &HashSet<i64>,
) -> Vec<WeekData> {
    let mut by_day: HashMap<chrono::NaiveDate, Vec<&ira_models::PlaySession>> = HashMap::new();
    for s in sessions {
        let date = ts_to_date(s.started_at);
        by_day.entry(date).or_default().push(s);
    }

    let mut week_starts: Vec<chrono::NaiveDate> = by_day.keys().map(|d| week_start(*d)).collect();
    let cur_week = current_week_start();
    week_starts.push(cur_week);
    week_starts.sort();
    week_starts.dedup();

    let max_weeks = 26;
    let start = week_starts.len().saturating_sub(max_weeks);
    let mut week_starts = week_starts[start..].to_vec();
    for session in sessions
        .iter()
        .filter(|session| pending_deletions.contains(&session.id))
    {
        week_starts.push(week_start(ts_to_date(session.started_at)));
    }
    week_starts.sort();
    week_starts.dedup();

    let weeks = assemble_weeks(&week_starts, |date| {
        let sessions_for_day = by_day.get(&date);
        let total: f64 = sessions_for_day
            .map(|ss| {
                ss.iter()
                    .filter(|s| !pending_deletions.contains(&s.id))
                    .map(|s| s.duration_seconds as f64)
                    .sum()
            })
            .unwrap_or(0.0);
        let mut details: Vec<DayDetail> = sessions_for_day
            .map(|ss| {
                ss.iter()
                    .map(|s| DayDetail {
                        session_id: Some(s.id),
                        label: format_time(s.started_at),
                        value: format_duration(s.duration_seconds),
                        color_hex: None,
                        sessions: vec![],
                        deleted: pending_deletions.contains(&s.id),
                    })
                    .collect()
            })
            .unwrap_or_default();
        details.sort_by_key(|detail| detail.deleted);
        let segments = if total > 0.0 {
            vec![BarSegment {
                value: total,
                color_index: Some(0),
                label: game_name.to_string(),
            }]
        } else {
            vec![]
        };
        DayData {
            date,
            total,
            segments,
            details,
        }
    });

    weeks
}

/// Assembles one `WeekData` per week start from a per-day builder.
fn assemble_weeks(
    week_starts: &[chrono::NaiveDate],
    build_day: impl Fn(chrono::NaiveDate) -> DayData,
) -> Vec<WeekData> {
    week_starts
        .iter()
        .map(|&ws| {
            let days: Vec<DayData> = generate_week_days(ws).into_iter().map(&build_day).collect();
            let week_total = days.iter().map(|d| d.total).sum();
            WeekData {
                week_start: ws,
                days,
                week_total,
            }
        })
        .collect()
}

pub fn show_daily_history_dialog(state: &SharedState) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Daily play history"));
    dialog.set_content_width(920);
    dialog.set_content_height(500);

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);

    // Same outside-change registry as the per-game window: a game closing
    // while this is open inserts a session that must appear. None matches
    // every game; unregistered on close.
    let rebuild: RebuildFn = std::rc::Rc::new({
        let state = state.clone();
        let box_ = box_.clone();
        move |focus: Option<ChartFocus>| {
            let now = now_secs();
            let from = now - 84 * 86400;

            let db = state.borrow().db.clone();
            let all_sessions = ira_db::get_sessions_range(&db, from, now).unwrap_or_else(|e| {
                eprintln!("Failed to read play sessions: {e}");
                Vec::new()
            });

            // Straight from the database: games that left the library keep their
            // rows, and their past sessions must still read as themselves.
            let game_names: HashMap<i64, String> = {
                ira_db::load_all_games(&db)
                    .unwrap_or_else(|e| {
                        eprintln!("Failed to read game titles: {e}");
                        Vec::new()
                    })
                    .into_iter()
                    .map(|entry| (entry.id, entry.title))
                    .collect()
            };

            let empty_hint = all_sessions
                .is_empty()
                .then(|| crate::tr!("No play sessions recorded yet"));
            let weeks = compute_app_weeks(&all_sessions, &game_names);

            clear_children(&box_);
            box_.append(&build_weekly_chart(weeks, false, None, focus, empty_hint));
        }
    });

    rebuild(None);
    let close_rebuild = rebuild.clone();
    state
        .borrow_mut()
        .history_dialogs
        .borrow_mut()
        .push(HistoryDialog {
            game_id: None,
            rebuild,
        });

    let unregister = state.clone();
    dialog.connect_closed(move |_| {
        unregister
            .borrow_mut()
            .history_dialogs
            .borrow_mut()
            .retain(|d| !Rc::ptr_eq(&close_rebuild, &d.rebuild));
    });

    toolbar_view.set_content(Some(&box_));
    dialog.set_child(Some(&toolbar_view));
    dialog.present(Some(&state.borrow().window));
}

fn compute_app_weeks(
    sessions: &[ira_models::PlaySession],
    game_names: &HashMap<i64, String>,
) -> Vec<WeekData> {
    let mut by_day: HashMap<chrono::NaiveDate, HashMap<i64, Vec<&ira_models::PlaySession>>> =
        HashMap::new();
    for s in sessions {
        let date = ts_to_date(s.started_at);
        by_day
            .entry(date)
            .or_default()
            .entry(s.game_id)
            .or_default()
            .push(s);
    }

    let cur_week = current_week_start();
    let week_starts: Vec<chrono::NaiveDate> = (0..12)
        .rev()
        .map(|i| cur_week - chrono::Duration::days(i * 7))
        .collect();

    let weeks = assemble_weeks(&week_starts, |date| {
        let day_games = by_day.get(&date);
        let (segments, details) = build_day_data(day_games, game_names);
        let total: f64 = segments.iter().map(|s| s.value).sum();
        DayData {
            date,
            total,
            segments,
            details,
        }
    });

    weeks
}

fn build_day_data(
    day_games: Option<&HashMap<i64, Vec<&ira_models::PlaySession>>>,
    game_names: &HashMap<i64, String>,
) -> (Vec<BarSegment>, Vec<DayDetail>) {
    let Some(day_games) = day_games else {
        return (vec![], vec![]);
    };

    // One colored segment per game, largest playtime first. Every game has a
    // stable palette color (see color_index_for_game), so nothing collapses
    // into a shared grey "Other" bucket.
    let mut played: Vec<(i64, f64, Vec<&ira_models::PlaySession>)> = day_games
        .iter()
        .map(|(&gid, sessions)| {
            let total: f64 = sessions.iter().map(|s| s.duration_seconds as f64).sum();
            (gid, total, sessions.clone())
        })
        .filter(|&(_, total, _)| total > 0.0)
        .collect();
    played.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut segments: Vec<BarSegment> = Vec::new();
    let mut details: Vec<DayDetail> = Vec::new();
    for (gid, total, sessions) in played {
        let color_idx = color_index_for_game(gid);
        let name = game_names
            .get(&gid)
            .cloned()
            .unwrap_or_else(|| crate::tr!("Unknown game"));
        let sub_sessions: Vec<DaySession> = sessions
            .iter()
            .map(|s| DaySession {
                session_id: s.id,
                label: format_time(s.started_at),
                value: format_duration(s.duration_seconds),
                deleted: false,
            })
            .collect();
        segments.push(BarSegment {
            value: total,
            color_index: Some(color_idx),
            label: name.clone(),
        });
        details.push(DayDetail {
            session_id: None,
            label: name,
            value: format_duration(total as i64),
            color_hex: Some(color_hex(color_idx).to_string()),
            sessions: sub_sessions,
            deleted: false,
        });
    }

    (segments, details)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_session_edit_history_undo_redo() {
        let mut history = SessionEditHistory::default();

        history.toggle(1, 100);
        assert!(history.pending.contains(&1));
        assert!(history.redo.is_empty());

        let edit = history.undo().unwrap();
        assert_eq!(edit.session_id, 1);
        assert!(!history.pending.contains(&1));

        let edit = history.redo().unwrap();
        assert_eq!(edit.started_at, 100);
        assert!(history.pending.contains(&1));

        history.toggle(2, 200);
        assert!(history.redo.is_empty());
        assert!(history.pending.contains(&2));
    }

    #[test]
    fn test_session_edit_message_identifies_session_time() {
        let timestamp = 1_700_000_000;
        let time = format_time(timestamp);

        assert!(session_edit_message(true, timestamp).contains(&time));
        assert!(session_edit_message(false, timestamp).contains(&time));
    }

    #[test]
    fn test_history_shortcut_accepts_shifted_z() {
        let control = gtk4::gdk::ModifierType::CONTROL_MASK;

        assert_eq!(
            history_shortcut_redo(gtk4::gdk::Key::z, control),
            Some(false)
        );
        assert_eq!(
            history_shortcut_redo(
                gtk4::gdk::Key::Z,
                control | gtk4::gdk::ModifierType::SHIFT_MASK
            ),
            Some(true)
        );
        assert_eq!(
            history_shortcut_redo(gtk4::gdk::Key::z, gtk4::gdk::ModifierType::SHIFT_MASK),
            None
        );
    }

    #[test]
    fn test_history_dialog_matches_daily_sees_every_game() {
        assert!(history_dialog_matches(None, &[], 7));
        assert!(history_dialog_matches(None, &[7], 7));
    }

    #[test]
    fn test_history_dialog_matches_own_and_linked_games() {
        // The window's own game and its playtime-linked members refresh…
        assert!(history_dialog_matches(Some(7), &[7], 7));
        assert!(history_dialog_matches(Some(7), &[7, 9], 9));
        // …anything else does not.
        assert!(!history_dialog_matches(Some(7), &[7], 9));
        assert!(!history_dialog_matches(Some(7), &[7, 9], 12));
    }

    #[test]
    fn test_session_focus_lands_on_session_day() {
        // A Tuesday noon session focuses its own day in its Monday week.
        let noon = chrono::Local
            .with_ymd_and_hms(2026, 8, 25, 12, 0, 0)
            .single()
            .unwrap();
        let focus = session_focus(noon.timestamp());
        assert_eq!(
            focus.day,
            chrono::NaiveDate::from_ymd_opt(2026, 8, 25).unwrap()
        );
        assert_eq!(
            focus.week,
            chrono::NaiveDate::from_ymd_opt(2026, 8, 24).unwrap()
        );
    }

    #[test]
    fn test_ts_secs_passthrough_seconds() {
        assert_eq!(ts_secs(1_700_000_000), 1_700_000_000);
        assert_eq!(ts_secs(0), 0);
    }

    #[test]
    fn test_ts_secs_normalizes_milliseconds() {
        assert_eq!(ts_secs(1_700_000_000_123), 1_700_000_000);
    }

    #[test]
    fn test_format_time_uses_local_timezone() {
        let noon = chrono::Local
            .with_ymd_and_hms(2026, 3, 10, 12, 0, 0)
            .single()
            .unwrap();
        assert_eq!(format_time(noon.timestamp()), "12:00");
    }

    #[test]
    fn test_ts_to_date_uses_local_timezone() {
        let noon = chrono::Local
            .with_ymd_and_hms(2026, 3, 10, 12, 0, 0)
            .single()
            .unwrap();
        assert_eq!(
            ts_to_date(noon.timestamp()),
            chrono::NaiveDate::from_ymd_opt(2026, 3, 10).unwrap()
        );
    }

    #[test]
    fn test_format_time_invalid_returns_empty() {
        assert_eq!(format_time(i64::MAX), "");
        assert_eq!(ts_to_date(i64::MAX), chrono::NaiveDate::default());
    }

    #[test]
    fn test_week_start_always_monday() {
        use chrono::Datelike;
        // 2026-08-26 is a Wednesday.
        let wed = chrono::NaiveDate::from_ymd_opt(2026, 8, 26).unwrap();
        let ws = week_start(wed);
        assert_eq!(ws, chrono::NaiveDate::from_ymd_opt(2026, 8, 24).unwrap());
        assert_eq!(ws.weekday(), chrono::Weekday::Mon);
    }

    #[test]
    fn test_compute_game_weeks_always_includes_current_week() {
        // No sessions at all → a single (current) week of empty days.
        let weeks = compute_game_weeks(&[], "Test Game", &HashSet::new());
        assert_eq!(weeks.len(), 1);
        assert_eq!(weeks[0].week_start, current_week_start());
        assert!(weeks[0].days.iter().all(|d| d.total == 0.0));
        assert!(weeks[0].days.iter().all(|d| d.details.is_empty()));
    }

    #[test]
    fn test_compute_game_weeks_places_pending_sessions_last() {
        let sessions = [session(2, 1, 600), session(1, 1, 1200)];
        let weeks = compute_game_weeks(&sessions, "Test Game", &HashSet::from([2]));
        let day = weeks
            .iter()
            .flat_map(|week| &week.days)
            .find(|day| !day.details.is_empty())
            .unwrap();

        assert_eq!(day.total, 1200.0);
        assert_eq!(day.details[0].session_id, Some(1));
        assert!(!day.details[0].deleted);
        assert_eq!(day.details[1].session_id, Some(2));
        assert!(day.details[1].deleted);
    }

    #[test]
    fn test_compute_game_weeks_keeps_old_pending_session_visible() {
        let mut pending = session(2, 1, 600);
        pending.started_at = now_secs() - 35 * 7 * 86_400;
        pending.ended_at = pending.started_at + pending.duration_seconds;
        let pending_week = week_start(ts_to_date(pending.started_at));
        let sessions = [session(1, 1, 1200), pending];

        let weeks = compute_game_weeks(&sessions, "Test Game", &HashSet::from([2]));

        assert!(weeks.iter().any(|week| week.week_start == pending_week));
    }

    fn session(id: i64, game_id: i64, duration: i64) -> ira_models::PlaySession {
        ira_models::PlaySession {
            id,
            game_id,
            variant_id: None,
            started_at: 1_700_000_000,
            ended_at: 1_700_000_000 + duration,
            duration_seconds: duration,
        }
    }

    #[test]
    fn test_build_day_data_colors_every_game() {
        // Many games in one day: each gets its own segment and swatch —
        // nothing is lumped into a grey "Other" bucket.
        let s: Vec<ira_models::PlaySession> =
            (1..=8i64).map(|i| session(i, i, 600 * (9 - i))).collect();
        let mut day: HashMap<i64, Vec<&ira_models::PlaySession>> = HashMap::new();
        for session in &s {
            day.entry(session.game_id).or_default().push(session);
        }
        let names: HashMap<i64, String> = (1..=8i64).map(|i| (i, format!("Game {i}"))).collect();

        let (segments, details) = build_day_data(Some(&day), &names);

        assert_eq!(segments.len(), 8);
        assert_eq!(details.len(), 8);
        // Largest playtime first (game 1 has the longest sessions).
        assert_eq!(segments[0].label, "Game 1");
        assert_eq!(segments[0].value, 4800.0);
        for (segment, detail) in segments.iter().zip(&details) {
            let idx = segment.color_index.expect("every game is colored");
            assert_eq!(detail.color_hex.as_deref(), Some(color_hex(idx)));
        }
        // Colors are per game, not one shared grey.
        assert!(segments
            .iter()
            .any(|s| s.color_index != segments[0].color_index));
    }

    #[test]
    fn test_build_day_data_stable_colors_match_index() {
        let s = [session(1, 42, 1200)];
        let mut day: HashMap<i64, Vec<&ira_models::PlaySession>> = HashMap::new();
        day.insert(42, vec![&s[0]]);
        let (segments, _) = build_day_data(Some(&day), &HashMap::new());
        assert_eq!(segments.len(), 1);
        assert_eq!(
            segments[0].color_index,
            Some(color_index_for_game(42)),
            "color must be the game's stable palette index"
        );
    }
}
