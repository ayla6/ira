//! The couch screen's rails. Top: an avatar, the date, a ticking clock, and
//! the battery when the machine (or a gamepad) has one to report. Bottom:
//! the connected-gamepad dots on the left and contextual button prompts on
//! the right.

use super::css::*;
use adw::prelude::*;
use std::path::Path;

/// The clock reads the system time this often; battery drains slower still,
/// so one timer serves both.
const REFRESH_EVERY_SECS: u32 = 10;

const PSU_ROOT: &str = "/sys/class/power_supply";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BatteryKind {
    System,
    Peripheral,
}

#[derive(Clone, Copy, Debug)]
struct Battery {
    capacity: u32,
    kind: BatteryKind,
}

/// One power_supply entry that is a battery (not mains/USB), if its
/// capacity is readable.
fn read_battery(dir: &Path) -> Option<Battery> {
    let kind = match std::fs::read_to_string(dir.join("type")) {
        Ok(text) if text.trim() == "Battery" => {
            match std::fs::read_to_string(dir.join("scope")) {
                Ok(scope) if scope.trim() == "Device" => BatteryKind::Peripheral,
                _ => BatteryKind::System,
            }
        }
        _ => return None,
    };
    let capacity = std::fs::read_to_string(dir.join("capacity"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Battery { capacity, kind })
}

/// Scan every power_supply; unreadable or non-battery entries are skipped.
fn scan_batteries() -> Vec<Battery> {
    let Ok(entries) = std::fs::read_dir(PSU_ROOT) else {
        return Vec::new();
    };
    let mut batteries = Vec::new();
    for entry in entries.flatten() {
        if let Some(battery) = read_battery(&entry.path()) {
            batteries.push(battery);
        }
    }
    batteries
}

/// The battery worth showing: the fullest system battery (laptop pack)
/// wins; otherwise a peripheral pack (a gamepad) is better than nothing.
fn pick_battery(batteries: &[Battery]) -> Option<Battery> {
    let best_of = |kind| {
        batteries
            .iter()
            .filter(|b| b.kind == kind)
            .max_by_key(|b| b.capacity)
            .copied()
    };
    best_of(BatteryKind::System).or_else(|| best_of(BatteryKind::Peripheral))
}

fn battery_icon_name(capacity: u32) -> &'static str {
    match capacity {
        0..=9 => "battery-empty-symbolic",
        10..=24 => "battery-caution-symbolic",
        25..=59 => "battery-low-symbolic",
        60..=94 => "battery-good-symbolic",
        _ => "battery-full-symbolic",
    }
}

/// The top rail: avatar, then date, clock and battery pushed to the right.
pub(super) struct StatusBar {
    root: gtk4::Box,
    date: gtk4::Label,
    clock: gtk4::Label,
    battery: gtk4::Box,
    battery_icon: gtk4::Image,
    battery_label: gtk4::Label,
}

impl StatusBar {
    pub(super) fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub(super) fn build() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        root.add_css_class(CSS_BP_STATUS);

        let avatar = adw::Avatar::new(40, Some("Ira"), true);
        avatar.set_valign(gtk4::Align::Center);
        root.append(&avatar);

        let battery = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let battery_icon = gtk4::Image::from_icon_name("battery-full-symbolic");
        let battery_label = gtk4::Label::new(None);
        battery_label.add_css_class(CSS_BP_BATT);
        battery.append(&battery_icon);
        battery.append(&battery_label);

        let date = gtk4::Label::new(None);
        date.add_css_class(CSS_BP_DATE);
        let clock = gtk4::Label::new(None);
        clock.add_css_class(CSS_BP_CLOCK);

        let right = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
        right.set_hexpand(true);
        right.set_halign(gtk4::Align::End);
        right.set_valign(gtk4::Align::Center);
        right.append(&battery);
        right.append(&date);
        right.append(&clock);
        root.append(&right);

        let status = Self { root, date, clock, battery, battery_icon, battery_label };
        status.refresh();
        let date = status.date.clone();
        let clock = status.clock.clone();
        let battery_box = status.battery.clone();
        let battery_icon = status.battery_icon.clone();
        let battery_label = status.battery_label.clone();
        glib::timeout_add_seconds_local(REFRESH_EVERY_SECS, move || {
            refresh_widgets(&date, &clock, &battery_box, &battery_icon, &battery_label);
            glib::ControlFlow::Continue
        });
        status
    }

    fn refresh(&self) {
        refresh_widgets(
            &self.date,
            &self.clock,
            &self.battery,
            &self.battery_icon,
            &self.battery_label,
        );
    }
}

/// Clock, date, and battery in one pass; sysfs reads are in-memory and
/// unbounded, so this is safe on the main loop.
fn refresh_widgets(
    date: &gtk4::Label,
    clock: &gtk4::Label,
    battery_box: &gtk4::Box,
    battery_icon: &gtk4::Image,
    battery_label: &gtk4::Label,
) {
    let now = chrono::Local::now();
    clock.set_text(&now.format("%H:%M").to_string());
    date.set_text(&now.format("%a %-d %b").to_string());
    let battery = pick_battery(&scan_batteries());
    battery_box.set_visible(battery.is_some());
    if let Some(battery) = battery {
        battery_icon.set_icon_name(Some(battery_icon_name(battery.capacity)));
        battery_label.set_text(&format!("{}%", battery.capacity));
        let tip = match battery.kind {
            BatteryKind::System => crate::tr!("System battery"),
            BatteryKind::Peripheral => crate::tr!("Controller battery"),
        };
        battery_box.set_tooltip_text(Some(&tip));
    }
}

/// The bottom rail: gamepad dots on the left, button prompts on the right.
pub(super) struct BottomBar {
    root: gtk4::Box,
    pads: gtk4::Box,
    dots: Vec<gtk4::Image>,
    prompts: gtk4::Box,
}

impl BottomBar {
    pub(super) fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub(super) fn build() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        root.add_css_class(CSS_BP_BOTTOM);
        root.set_hexpand(true);

        let pads = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        pads.set_valign(gtk4::Align::Center);
        pads.set_halign(gtk4::Align::Start);
        let pad_icon = gtk4::Image::from_icon_name("input-gaming-symbolic");
        pad_icon.set_pixel_size(22);
        pads.append(&pad_icon);
        let dots_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        dots_row.set_valign(gtk4::Align::Center);
        let dots = (0..4)
            .map(|_| {
                let dot = gtk4::Image::from_icon_name("media-record-symbolic");
                dot.set_pixel_size(8);
                dot.add_css_class(CSS_BP_PAD_DOT);
                dots_row.append(&dot);
                dot
            })
            .collect();
        pads.append(&dots_row);
        root.append(&pads);

        let prompts = gtk4::Box::new(gtk4::Orientation::Horizontal, 18);
        prompts.set_valign(gtk4::Align::Center);
        prompts.set_halign(gtk4::Align::End);
        root.append(&prompts);

        let bar = Self { root, pads, dots, prompts };
        bar.set_pad_count(0);
        bar
    }

    /// Light one dot per connected gamepad (four shown at most); unlit dots
    /// stay visible as slots, Switch-style. The exact count is in the
    /// tooltip.
    pub(super) fn set_pad_count(&self, count: usize) {
        for (index, dot) in self.dots.iter().enumerate() {
            dot.set_opacity(if index < count { 1.0 } else { 0.25 });
        }
        self.pads.set_tooltip_text(Some(
            &crate::tr!("Controllers connected: {}").replacen("{}", &count.to_string(), 1),
        ));
    }

    /// Replace the prompt row, e.g. A/Play on the home screen, B/Back plus
    /// A/Play on the All Software grid.
    pub(super) fn set_prompts(&self, items: &[(&str, &str)]) {
        super::helpers::clear_children(&self.prompts);
        for (glyph, label) in items {
            let item = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let key = gtk4::Label::new(Some(glyph));
            key.add_css_class(CSS_BP_PROMPT_KEY);
            let text = gtk4::Label::new(Some(label));
            text.add_css_class(CSS_BP_PROMPT);
            item.append(&key);
            item.append(&text);
            self.prompts.append(&item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn test_read_battery_type_and_scope() {
        let tmp = TempDir::new().unwrap();
        let bat = tmp.path().join("BAT0");
        fs::create_dir(&bat).unwrap();
        write(&bat, "type", "Battery\n");
        write(&bat, "capacity", "87\n");
        let battery = read_battery(&bat).unwrap();
        assert_eq!(battery.capacity, 87);
        assert_eq!(battery.kind, BatteryKind::System);

        let pad = tmp.path().join("ps-controller-battery-aa");
        fs::create_dir(&pad).unwrap();
        write(&pad, "type", "Battery\n");
        write(&pad, "scope", "Device\n");
        write(&pad, "capacity", "45\n");
        let battery = read_battery(&pad).unwrap();
        assert_eq!(battery.kind, BatteryKind::Peripheral);
    }

    #[test]
    fn test_read_battery_skips_mains_and_unreadable() {
        let tmp = TempDir::new().unwrap();
        let ac = tmp.path().join("AC");
        fs::create_dir(&ac).unwrap();
        write(&ac, "type", "Mains\n");
        assert!(read_battery(&ac).is_none());
        assert!(read_battery(&tmp.path().join("missing")).is_none());
    }

    #[test]
    fn test_pick_battery_prefers_system_pack() {
        let system = Battery { capacity: 40, kind: BatteryKind::System };
        let peripheral = Battery { capacity: 99, kind: BatteryKind::Peripheral };
        assert_eq!(pick_battery(&[peripheral, system]).unwrap().capacity, 40);
        assert_eq!(pick_battery(&[peripheral]).unwrap().kind, BatteryKind::Peripheral);
        assert!(pick_battery(&[]).is_none());
    }

    #[test]
    fn test_battery_icon_thresholds() {
        assert_eq!(battery_icon_name(100), "battery-full-symbolic");
        assert_eq!(battery_icon_name(70), "battery-good-symbolic");
        assert_eq!(battery_icon_name(40), "battery-low-symbolic");
        assert_eq!(battery_icon_name(15), "battery-caution-symbolic");
        assert_eq!(battery_icon_name(5), "battery-empty-symbolic");
    }
}
