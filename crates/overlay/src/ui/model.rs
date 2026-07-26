//! Reads game data from shared memory and builds the overlay widget tree.
//!
//! The Ira app writes game info + achievements to `/dev/shm/ira_overlay_<db_id>`
//! before launching the game. This module reads it each time the overlay
//! becomes visible and rebuilds the UI tree.

use ira_overlay_ipc::{MappedShm, SHM_MAGIC};

use super::widget::{Padding, Widget};
use super::widgets::{Button, Column, Label, Panel, ProgressBar, Row, ScrollView};

/// Reads the shared memory and builds a widget tree for the overlay.
/// Returns `None` if the SHM is not available or invalid.
pub fn build_ui() -> Option<Box<dyn Widget>> {
    let shm_path = std::env::var("IRA_OVERLAY_SHM").ok()?;
    let shm = MappedShm::open(&shm_path).ok()?;
    let hdr = shm.header();
    if hdr.magic != SHM_MAGIC {
        return None;
    }

    let game_name = cstr_to_string(&hdr.game_name);
    let total = hdr.total_achievements;
    let unlocked = hdr.unlocked_achievements;
    let playtime = hdr.playtime_seconds;

    let achievements: Vec<AchievementView> = shm
        .achievements()
        .iter()
        .take(total as usize)
        .map(|a| AchievementView {
            display_name: cstr_to_string(&a.display_name),
            description: cstr_to_string(&a.description),
            earned: a.earned != 0,
            _earned_time: a.earned_time,
            hidden: a.hidden != 0,
            _rarity: a.global_percent,
            trophy_type: a.trophy_type,
        })
        .collect();

    Some(build_tree(&game_name, total, unlocked, playtime, &achievements))
}

fn build_tree(
    game_name: &str,
    total: u32,
    unlocked: u32,
    playtime_secs: u64,
    achievements: &[AchievementView],
) -> Box<dyn Widget> {
    let mut children: Vec<Box<dyn Widget>> = Vec::new();

    // Game name header
    children.push(Box::new(Label::new(
        if game_name.is_empty() { "Ira Overlay" } else { game_name },
        20.0,
        [255, 255, 255, 255],
    )));

    // Progress bar + completion text
    let progress = if total > 0 {
        unlocked as f32 / total as f32
    } else {
        0.0
    };
    children.push(Box::new(ProgressBar::new(progress, 320.0)));

    let completion_text = if total > 0 {
        format!("{}/{} ({}%)", unlocked, total, (progress * 100.0) as u32)
    } else {
        "No achievements".to_string()
    };
    children.push(Box::new(Label::new(&completion_text, 13.0, [180, 180, 180, 255])));

    // Playtime
    if playtime_secs > 0 {
        let hours = playtime_secs as f64 / 3600.0;
        let playtime_str = if hours >= 1.0 {
            format!("Playtime: {:.1}h", hours)
        } else {
            format!("Playtime: {}m", playtime_secs / 60)
        };
        children.push(Box::new(Label::new(&playtime_str, 12.0, [150, 150, 150, 255])));
    }

    // Achievement list (scrollable)
    if !achievements.is_empty() {
        let mut ach_children: Vec<Box<dyn Widget>> = Vec::new();
        for ach in achievements {
            let name = if ach.earned {
                ach.display_name.clone()
            } else if ach.hidden {
                "???".to_string()
            } else {
                ach.display_name.clone()
            };
            let desc = if ach.earned || !ach.hidden {
                if ach.description.is_empty() {
                    continue;
                }
                ach.description.clone()
            } else {
                "Hidden achievement".to_string()
            };

            let name_color = if ach.earned {
                trophy_color(ach.trophy_type)
            } else {
                [120, 120, 120, 255]
            };
            let desc_color = if ach.earned {
                [160, 160, 160, 255]
            } else {
                [100, 100, 100, 255]
            };

            let trophy = trophy_glyph(ach.trophy_type, ach.earned);

            ach_children.push(Box::new(Column::new(2.0, vec![
                Box::new(Row::new(4.0, vec![
                    Box::new(Label::new(trophy, 14.0, name_color)),
                    Box::new(Label::new(&name, 14.0, name_color)),
                ])),
                Box::new(Label::new(&desc, 12.0, desc_color)),
            ])));
        }
        children.push(Box::new(ScrollView::new(ach_children, 350.0)));
    }

    // Capture controls
    children.push(Box::new(Row::new(5.0, vec![
        Box::new(Button::new("Screenshot", 14.0, || {
            super::capture::request_screenshot();
        })),
        Box::new(Button::new("Record", 14.0, || {
            super::capture::toggle_recording();
        })),
    ])));

    // Hints
    children.push(Box::new(Label::new("Shift+Tab to toggle | Scroll to browse", 11.0, [130, 130, 130, 255])));

    Box::new(Panel::new(
        Padding::all(10.0),
        [30, 30, 30, 210],
        8.0,
        Box::new(Column::new(6.0, children)),
    ))
}

struct AchievementView {
    display_name: String,
    description: String,
    earned: bool,
    _earned_time: u64,
    hidden: bool,
    _rarity: f32,
    trophy_type: u8,
}

fn cstr_to_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).to_string()
}

fn trophy_color(trophy_type: u8) -> [u8; 4] {
    match trophy_type {
        b'p' => [200, 170, 255, 255], // platinum
        b'g' => [255, 215, 0, 255],   // gold
        b's' => [192, 192, 192, 255], // silver
        b'b' => [205, 127, 50, 255],  // bronze
        _ => [255, 255, 255, 255],     // default white
    }
}

fn trophy_glyph(trophy_type: u8, earned: bool) -> &'static str {
    if !earned {
        return "  ";
    }
    match trophy_type {
        b'p' => "P",
        b'g' => "G",
        b's' => "S",
        b'b' => "B",
        _ => "*",
    }
}
