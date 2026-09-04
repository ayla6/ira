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
    /// The machine's own pack (a laptop battery).
    System,
    /// A peripheral's pack (a gamepad, headset…).
    Peripheral,
}

#[derive(Clone, Copy, Debug)]
struct Battery {
    capacity: u32,
    kind: BatteryKind,
    charging: bool,
}

/// One power_supply entry that is a battery (not mains/USB), with its
/// charge state when the kernel says.
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
    let charging = std::fs::read_to_string(dir.join("status"))
        .map(|status| status.trim() == "Charging")
        .unwrap_or(false);
    Some(Battery { capacity, kind, charging })
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

/// The machine's own battery, for the status rail. A gamepad's pack never
/// stands in for the computer's — pads report in the pad area instead.
fn system_battery(batteries: &[Battery]) -> Option<Battery> {
    batteries
        .iter()
        .filter(|b| b.kind == BatteryKind::System)
        .max_by_key(|b| b.capacity)
        .copied()
}

/// The fullest connected gamepad battery, for the pad area. Peripheral
/// packs surface here (kernel HID drivers expose many pads), and the
/// 8BitDo DInput reader fills the gap for hardware the kernel can't see.
pub(super) fn pad_battery() -> Option<(u8, bool)> {
    pad_battery_of(&scan_batteries())
}

fn pad_battery_of(batteries: &[Battery]) -> Option<(u8, bool)> {
    batteries
        .iter()
        .filter(|b| b.kind == BatteryKind::Peripheral)
        .max_by_key(|b| b.capacity)
        .map(|b| (b.capacity.min(u32::from(u8::MAX)) as u8, b.charging))
}

fn battery_icon_name(capacity: u32, charging: bool) -> String {
    let level = match capacity {
        0..=9 => "empty",
        10..=24 => "caution",
        25..=59 => "low",
        60..=94 => "good",
        _ => "full",
    };
    let suffix = if charging { "-charging" } else { "" };
    format!("battery-{level}{suffix}-symbolic")
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
    let battery = system_battery(&scan_batteries());
    battery_box.set_visible(battery.is_some());
    if let Some(battery) = battery {
        battery_icon.set_icon_name(Some(&battery_icon_name(battery.capacity, battery.charging)));
        battery_label.set_text(&format!("{}%", battery.capacity));
        battery_box.set_tooltip_text(Some(&crate::tr!("System battery")));
    }
}

/// The bottom rail: gamepad dots (and the pads' battery) on the left,
/// button prompts on the right.
pub(super) struct BottomBar {
    root: gtk4::Box,
    pads: gtk4::Box,
    dots: Vec<gtk4::Image>,
    pad_battery: gtk4::Box,
    pad_battery_icon: gtk4::Image,
    pad_battery_label: gtk4::Label,
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

        let pad_battery = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        pad_battery.set_valign(gtk4::Align::Center);
        let pad_battery_icon = gtk4::Image::from_icon_name("battery-full-symbolic");
        let pad_battery_label = gtk4::Label::new(None);
        pad_battery_label.add_css_class(CSS_BP_BATT);
        pad_battery.append(&pad_battery_icon);
        pad_battery.append(&pad_battery_label);
        pads.append(&pad_battery);

        root.append(&pads);

        let prompts = gtk4::Box::new(gtk4::Orientation::Horizontal, 18);
        prompts.set_valign(gtk4::Align::Center);
        prompts.set_halign(gtk4::Align::End);
        root.append(&prompts);

        let bar = Self {
            root,
            pads,
            dots,
            pad_battery,
            pad_battery_icon,
            pad_battery_label,
            prompts,
        };
        bar.set_pad_status(0, None);
        bar
    }

    /// Light one dot per connected gamepad (four shown at most); unlit dots
    /// stay visible as slots, Switch-style. The pads' battery reading (any
    /// connected pad) sits beside the dots when one is known.
    pub(super) fn set_pad_status(&self, count: usize, battery: Option<(u8, bool)>) {
        for (index, dot) in self.dots.iter().enumerate() {
            dot.set_opacity(if index < count { 1.0 } else { 0.25 });
        }
        self.pad_battery.set_visible(battery.is_some());
        if let Some((percent, charging)) = battery {
            self.pad_battery_icon
                .set_icon_name(Some(&battery_icon_name(u32::from(percent), charging)));
            self.pad_battery_label.set_text(&format!("{percent}%"));
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
    fn test_read_battery_type_scope_and_status() {
        let tmp = TempDir::new().unwrap();
        let bat = tmp.path().join("BAT0");
        fs::create_dir(&bat).unwrap();
        write(&bat, "type", "Battery\n");
        write(&bat, "capacity", "87\n");
        write(&bat, "status", "Discharging\n");
        let battery = read_battery(&bat).unwrap();
        assert_eq!(battery.capacity, 87);
        assert_eq!(battery.kind, BatteryKind::System);
        assert!(!battery.charging);

        let pad = tmp.path().join("ps-controller-battery-aa");
        fs::create_dir(&pad).unwrap();
        write(&pad, "type", "Battery\n");
        write(&pad, "scope", "Device\n");
        write(&pad, "capacity", "45\n");
        write(&pad, "status", "Charging\n");
        let battery = read_battery(&pad).unwrap();
        assert_eq!(battery.kind, BatteryKind::Peripheral);
        assert!(battery.charging);
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
    fn test_system_battery_ignores_peripheral_packs() {
        let system = Battery { capacity: 40, kind: BatteryKind::System, charging: false };
        let peripheral = Battery { capacity: 99, kind: BatteryKind::Peripheral, charging: false };
        assert_eq!(system_battery(&[peripheral, system]).unwrap().capacity, 40);
        assert!(system_battery(&[peripheral]).is_none());
        assert!(system_battery(&[]).is_none());
    }

    #[test]
    fn test_pad_battery_picks_peripheral_pack() {
        let system = Battery { capacity: 40, kind: BatteryKind::System, charging: false };
        let peripheral = Battery { capacity: 99, kind: BatteryKind::Peripheral, charging: true };
        let (capacity, charging) = pad_battery_of(&[system, peripheral]).unwrap();
        assert_eq!(capacity, 99);
        assert!(charging);
        assert!(pad_battery_of(&[system]).is_none());
    }

    #[test]
    fn test_battery_icon_thresholds_and_charging() {
        assert_eq!(battery_icon_name(100, false), "battery-full-symbolic");
        assert_eq!(battery_icon_name(70, false), "battery-good-symbolic");
        assert_eq!(battery_icon_name(40, false), "battery-low-symbolic");
        assert_eq!(battery_icon_name(15, false), "battery-caution-symbolic");
        assert_eq!(battery_icon_name(5, false), "battery-empty-symbolic");
        assert_eq!(battery_icon_name(70, true), "battery-good-charging-symbolic");
    }
}
