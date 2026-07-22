use gtk4::prelude::*;
use adw::prelude::*;
use crate::strings as S;
use super::state::SharedState;
use super::helpers::format_duration;
use super::play_history_chart::{
    assign_game_colors, build_weekly_chart, color_hex, other_hex, BarSegment, DayData,
    DayDetail, DaySession, GameColorAssignment, WeekData,
};
use std::collections::HashMap;

fn format_time(timestamp: i64) -> String {
    let secs = if timestamp > 1_000_000_000_000 { timestamp / 1000 } else { timestamp };
    chrono::DateTime::from_timestamp(secs, 0)
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
    let secs = if ts > 1_000_000_000_000 { ts / 1000 } else { ts };
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.date_naive())
        .unwrap_or_default()
}

fn week_start(date: chrono::NaiveDate) -> chrono::NaiveDate {
    use chrono::Datelike;
    let weekday = date.weekday().num_days_from_monday() as i64;
    date - chrono::Duration::days(weekday)
}

fn current_week_start() -> chrono::NaiveDate {
    week_start(ts_to_date(now_secs()))
}

fn generate_week_days(ws: chrono::NaiveDate) -> Vec<chrono::NaiveDate> {
    (0..7).map(|i| ws + chrono::Duration::days(i)).collect()
}

pub fn show_play_history_dialog(state: &SharedState, game_id: i64, variant_id: Option<i64>) -> adw::Dialog {
    let game_name = state.borrow().games.iter()
        .find(|g| g.db_id == game_id && g.variant_id == variant_id)
        .map(|g| g.name.clone())
        .unwrap_or_default();

    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("{} {}", S::SESSION_HISTORY_FOR, game_name));
    dialog.set_content_width(820);
    dialog.set_content_height(500);

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);

    let sessions = ira_db::get_sessions_for_game(&state.borrow().db, game_id, variant_id)
        .unwrap_or_default();

    if sessions.is_empty() {
        let empty_label = gtk4::Label::new(Some(S::NO_SESSIONS));
        empty_label.set_xalign(0.0);
        empty_label.set_opacity(0.6);
        box_.append(&empty_label);
    } else {
        let weeks = compute_game_weeks(&sessions, &game_name);
        box_.append(&build_weekly_chart(weeks, true));
    }

    toolbar_view.set_content(Some(&box_));
    dialog.set_child(Some(&toolbar_view));
    dialog.present(Some(&state.borrow().window));
    dialog
}

fn compute_game_weeks(
    sessions: &[ira_models::PlaySession],
    game_name: &str,
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
                DayData { date, total, segments, details }
            })
            .collect();
        let week_total = days.iter().map(|d| d.total).sum();
        weeks.push(WeekData { week_start: ws, days, week_total });
    }

    weeks
}

pub fn show_daily_history_dialog(state: &SharedState) {
    let dialog = adw::Dialog::new();
    dialog.set_title(S::DAILY_HISTORY);
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

    let all_sessions = ira_db::get_sessions_range(&state.borrow().db, from, now)
        .unwrap_or_default();

    if all_sessions.is_empty() {
        let empty_label = gtk4::Label::new(Some(S::NO_SESSIONS));
        empty_label.set_xalign(0.0);
        empty_label.set_opacity(0.6);
        box_.append(&empty_label);
    } else {
        let game_names: HashMap<i64, String> = state.borrow().games.iter()
            .map(|g| (g.db_id, g.name.clone()))
            .collect();

        let assignment = assign_game_colors(&all_sessions);
        let weeks = compute_app_weeks(&all_sessions, &assignment, &game_names);

        box_.append(&build_weekly_chart(weeks, false));
    }

    toolbar_view.set_content(Some(&box_));
    dialog.set_child(Some(&toolbar_view));
    dialog.present(Some(&state.borrow().window));
}

fn compute_app_weeks(
    sessions: &[ira_models::PlaySession],
    assignment: &GameColorAssignment,
    game_names: &HashMap<i64, String>,
) -> Vec<WeekData> {
    let mut by_day: HashMap<chrono::NaiveDate, HashMap<i64, Vec<&ira_models::PlaySession>>> = HashMap::new();
    for s in sessions {
        let date = ts_to_date(s.started_at);
        by_day.entry(date).or_default()
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
                let (segments, details) = build_day_data(day_games, assignment, game_names);
                let total: f64 = segments.iter().map(|s| s.value).sum();
                DayData { date, total, segments, details }
            })
            .collect();
        let week_total = days.iter().map(|d| d.total).sum();
        weeks.push(WeekData { week_start: ws, days, week_total });
    }

    weeks
}

fn build_day_data(
    day_games: Option<&HashMap<i64, Vec<&ira_models::PlaySession>>>,
    assignment: &GameColorAssignment,
    game_names: &HashMap<i64, String>,
) -> (Vec<BarSegment>, Vec<DayDetail>) {
    let Some(day_games) = day_games else {
        return (vec![], vec![]);
    };

    let mut segments: Vec<BarSegment> = Vec::new();
    let mut details: Vec<DayDetail> = Vec::new();

    for &gid in &assignment.top_games {
        if let Some(sessions) = day_games.get(&gid) {
            let total: f64 = sessions.iter().map(|s| s.duration_seconds as f64).sum();
            if total > 0.0 {
                let color_idx = assignment.color_map.get(&gid).copied();
                let name = game_names.get(&gid).cloned().unwrap_or_default();
                let hex = color_hex(color_idx.unwrap_or(0)).to_string();
                let sub_sessions: Vec<DaySession> = sessions.iter()
                    .map(|s| DaySession {
                        label: format_time(s.started_at),
                        value: format_duration(s.duration_seconds),
                    })
                    .collect();
                segments.push(BarSegment {
                    value: total,
                    color_index: color_idx,
                    label: name.clone(),
                });
                details.push(DayDetail {
                    label: name,
                    value: format_duration(total as i64),
                    color_hex: Some(hex),
                    sessions: sub_sessions,
                });
            }
        }
    }

    // Non-top-5 games: single "Other" bar segment, but individual sidebar details
    let mut other_total: f64 = 0.0;
    let mut other_details: Vec<(i64, f64, Vec<&ira_models::PlaySession>)> = Vec::new();
    for (&gid, sessions) in day_games {
        if !assignment.color_map.contains_key(&gid) {
            let total: f64 = sessions.iter().map(|s| s.duration_seconds as f64).sum();
            if total > 0.0 {
                other_total += total;
                other_details.push((gid, total, sessions.clone()));
            }
        }
    }
    if other_total > 0.0 {
        segments.push(BarSegment {
            value: other_total,
            color_index: None,
            label: "Other".to_string(),
        });
        // Sort by playtime descending
        other_details.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (gid, total, sessions) in other_details {
            let name = game_names.get(&gid).cloned().unwrap_or_default();
            let sub_sessions: Vec<DaySession> = sessions.iter()
                .map(|s| DaySession {
                    label: format_time(s.started_at),
                    value: format_duration(s.duration_seconds),
                })
                .collect();
            details.push(DayDetail {
                label: name,
                value: format_duration(total as i64),
                color_hex: Some(other_hex().to_string()),
                sessions: sub_sessions,
            });
        }
    }

    (segments, details)
}
