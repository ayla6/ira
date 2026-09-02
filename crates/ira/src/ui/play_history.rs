use super::helpers::{clear_children, format_duration};
use super::play_history_chart::{
    build_weekly_chart, color_hex, color_index_for_game, BarSegment, ChartFocus, DayData,
    DayDetail, DaySession, WeekData,
};
use super::state::SharedState;
use adw::prelude::*;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

type RebuildFn = std::rc::Rc<dyn Fn(Option<ChartFocus>)>;
type RebuildHandle = std::rc::Rc<std::cell::RefCell<Option<RebuildFn>>>;

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

pub fn show_play_history_dialog(
    state: &SharedState,
    game_id: i64,
    variant_id: Option<i64>,
) -> adw::Dialog {
    let game_name = state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == game_id && g.variant_id == variant_id)
        .map(|g| g.name.clone())
        .unwrap_or_default();

    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("{} {}", crate::tr!("Play history for"), game_name));
    dialog.set_content_width(820);
    dialog.set_content_height(500);

    let ctrl_held = Rc::new(Cell::new(false));
    {
        let kc = gtk4::EventControllerKey::new();
        kc.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let kp = ctrl_held.clone();
        let kr = ctrl_held.clone();
        kc.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Control_L || key == gtk4::gdk::Key::Control_R {
                kp.set(true);
            }
            glib::Propagation::Proceed
        });
        kc.connect_key_released(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Control_L || key == gtk4::gdk::Key::Control_R {
                kr.set(false);
            }
        });
        dialog.add_controller(kc);
    }

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);

    // Rebuild handle stored in a RefCell so the delete callback can reference
    // the rebuild closure that creates it (self-referential).
    let rebuild_handle: RebuildHandle = std::rc::Rc::new(std::cell::RefCell::new(None));

    {
        let state = state.clone();
        let box_ = box_.clone();
        let dialog = dialog.clone();
        let rebuild_handle_c = rebuild_handle.clone();
        let rebuild: RebuildFn = std::rc::Rc::new(move |focus: Option<ChartFocus>| {
            let sessions = ira_db::get_sessions_for_game(&state.borrow().db, game_id, variant_id)
                .unwrap_or_default();
            clear_children(&box_);
            let on_delete: super::play_history_chart::DeleteSessionFn = {
                let state = state.clone();
                let dialog_weak = dialog.downgrade();
                let rebuild_handle = rebuild_handle_c.clone();
                std::rc::Rc::new(move |session_id: i64, ctrl: bool| {
                    let Some(dialog) = dialog_weak.upgrade() else {
                        return;
                    };
                    let rebuild = rebuild_handle.borrow().clone();
                    if let Some(rebuild) = rebuild {
                        delete_session_with_confirm(&state, &dialog, session_id, ctrl, rebuild);
                    }
                })
            };
            // Always render the full chart (axes, day labels, nav, sidebar);
            // when nothing was ever recorded it doubles as the empty state.
            let empty_hint = sessions
                .is_empty()
                .then(|| crate::tr!("No play sessions recorded yet"));
            let weeks = compute_game_weeks(&sessions, &game_name);
            box_.append(&build_weekly_chart(
                weeks,
                true,
                Some(on_delete),
                focus,
                ctrl_held.clone(),
                empty_hint,
            ));
        });
        *rebuild_handle.borrow_mut() = Some(rebuild);
    }

    if let Some(rebuild) = rebuild_handle.borrow().clone() {
        rebuild(None);
    }

    toolbar_view.set_content(Some(&box_));
    dialog.set_child(Some(&toolbar_view));
    dialog.present(Some(&state.borrow().window));

    let refresh_state = state.clone();
    let rebuild_handle_close = rebuild_handle.clone();
    dialog.connect_closed(move |_| {
        // The rebuild closure is stored inside rebuild_handle and itself holds
        // a strong clone of it — an Rc cycle. Clear it here so the dialog and
        // its widget tree are freed on close instead of leaking.
        *rebuild_handle_close.borrow_mut() = None;
        let still_active = refresh_state.borrow().displayed_db_id == game_id;
        let game = if still_active {
            refresh_state
                .borrow()
                .games
                .iter()
                .find(|g| g.db_id == game_id && g.variant_id == variant_id)
                .cloned()
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

fn delete_session_with_confirm(
    state: &SharedState,
    parent: &adw::Dialog,
    session_id: i64,
    ctrl: bool,
    rebuild: RebuildFn,
) {
    let do_delete = {
        let state = state.clone();
        let rebuild = rebuild.clone();
        move || {
            let focus = delete_session_from_db(&state, session_id)
                .map_err(|e| eprintln!("Failed to delete session: {}", e))
                .ok()
                .flatten();
            rebuild(focus);
        }
    };
    if ctrl {
        do_delete();
    } else {
        let dialog = adw::AlertDialog::new(
            Some(&crate::tr!("Delete session")),
            Some(&crate::tr!(
                "Delete this play session and subtract its playtime?"
            )),
        );
        dialog.add_response("cancel", &crate::tr!("Cancel"));
        dialog.add_response("delete", &crate::tr!("Delete"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        dialog.connect_response(None, move |_, resp| {
            if resp == "delete" {
                do_delete();
            }
        });
        dialog.present(Some(parent));
    }
}

fn delete_session_from_db(
    state: &SharedState,
    session_id: i64,
) -> Result<Option<ChartFocus>, String> {
    let session = ira_db::delete_session(&state.borrow().db, session_id)?
        .ok_or_else(|| format!("session {} not found", session_id))?;
    let deleted_day = ts_to_date(session.started_at);
    let focus = ChartFocus {
        week: week_start(deleted_day),
        day: deleted_day,
    };
    let hours = (session.duration_seconds as f64) / 3600.0;
    let (db, new_base_playtime, new_variant_playtime) = {
        let mut s = state.borrow_mut();
        let db = s.db.clone();
        let mut base_pt = 0.0;
        let mut var_pt: Option<(i64, f64)> = None;
        for g in &mut s.games {
            if g.db_id == session.game_id && g.variant_id.is_none() {
                g.playtime = (g.playtime - hours).max(0.0);
                base_pt = g.playtime;
            } else if g.db_id == session.game_id
                && g.variant_id == session.variant_id
                && session.variant_id.is_some()
            {
                g.playtime = (g.playtime - hours).max(0.0);
                var_pt = Some((session.variant_id.unwrap(), g.playtime));
            }
        }
        (db, base_pt, var_pt)
    };
    if session.variant_id.is_none() {
        ira_db::update_field(&db, session.game_id, "playtime", &new_base_playtime)?;
    }
    if let Some((vid, vpt)) = new_variant_playtime {
        ira_db::update_variant_playtime(&db, vid, vpt)?;
    }
    Ok(Some(focus))
}

fn compute_game_weeks(sessions: &[ira_models::PlaySession], game_name: &str) -> Vec<WeekData> {
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
    let week_starts = &week_starts[start..];

    let mut weeks: Vec<WeekData> = Vec::new();
    for &ws in week_starts {
        let days: Vec<DayData> = generate_week_days(ws)
            .into_iter()
            .map(|date| {
                let sessions_for_day = by_day.get(&date);
                let total: f64 = sessions_for_day
                    .map(|ss| ss.iter().map(|s| s.duration_seconds as f64).sum())
                    .unwrap_or(0.0);
                let details: Vec<DayDetail> = sessions_for_day
                    .map(|ss| {
                        ss.iter()
                            .map(|s| DayDetail {
                                session_id: Some(s.id),
                                label: format_time(s.started_at),
                                value: format_duration(s.duration_seconds),
                                color_hex: None,
                                sessions: vec![],
                            })
                            .collect()
                    })
                    .unwrap_or_default();
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
            })
            .collect();
        let week_total = days.iter().map(|d| d.total).sum();
        weeks.push(WeekData {
            week_start: ws,
            days,
            week_total,
        });
    }

    weeks
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

    let now = now_secs();
    let from = now - 84 * 86400;

    let all_sessions =
        ira_db::get_sessions_range(&state.borrow().db, from, now).unwrap_or_default();

    let game_names: HashMap<i64, String> = state
        .borrow()
        .games
        .iter()
        .map(|g| (g.db_id, g.name.clone()))
        .collect();

    let empty_hint = all_sessions
        .is_empty()
        .then(|| crate::tr!("No play sessions recorded yet"));
    let weeks = compute_app_weeks(&all_sessions, &game_names);

    box_.append(&build_weekly_chart(
        weeks,
        false,
        None,
        None,
        Rc::new(Cell::new(false)),
        empty_hint,
    ));

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

    let mut weeks: Vec<WeekData> = Vec::new();
    for &ws in &week_starts {
        let days: Vec<DayData> = generate_week_days(ws)
            .into_iter()
            .map(|date| {
                let day_games = by_day.get(&date);
                let (segments, details) = build_day_data(day_games, game_names);
                let total: f64 = segments.iter().map(|s| s.value).sum();
                DayData {
                    date,
                    total,
                    segments,
                    details,
                }
            })
            .collect();
        let week_total = days.iter().map(|d| d.total).sum();
        weeks.push(WeekData {
            week_start: ws,
            days,
            week_total,
        });
    }

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
        let name = game_names.get(&gid).cloned().unwrap_or_default();
        let sub_sessions: Vec<DaySession> = sessions
            .iter()
            .map(|s| DaySession {
                session_id: s.id,
                label: format_time(s.started_at),
                value: format_duration(s.duration_seconds),
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
        });
    }

    (segments, details)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
        let weeks = compute_game_weeks(&[], "Test Game");
        assert_eq!(weeks.len(), 1);
        assert_eq!(weeks[0].week_start, current_week_start());
        assert!(weeks[0].days.iter().all(|d| d.total == 0.0));
        assert!(weeks[0].days.iter().all(|d| d.details.is_empty()));
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
        let s: Vec<ira_models::PlaySession> = (1..=8i64)
            .map(|i| session(i, i, 600 * (9 - i)))
            .collect();
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
        assert!(segments.iter().any(|s| s.color_index != segments[0].color_index));
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
