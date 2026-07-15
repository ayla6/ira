use ira_models::MergedAchievement;
use std::process::Command;

pub fn notify_unlock(game_name: &str, ach: &MergedAchievement) {
    let title = format!("{} — Trophy Unlocked", game_name);
    let body = if ach.description.is_empty() {
        ach.display_name.clone()
    } else {
        format!("{}\n{}", ach.display_name, ach.description)
    };
    let icon = if ach.icon_path.is_empty() {
        "starred-symbolic".to_string()
    } else {
        ach.icon_path.clone()
    };

    std::thread::spawn(move || {
        let _ = Command::new("notify-send")
            .args([
                "--app-name=Ira",
                &format!("--icon={}", icon),
                &title,
                &body,
            ])
            .spawn()
            .and_then(|mut c| c.wait());
    });
}
