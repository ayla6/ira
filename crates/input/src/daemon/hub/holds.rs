//! Exclusive evdev holds on the physical pad's companion nodes.
//!
//! A remapping session owns the pad the game is supposed to see, so the
//! hardware must go quiet for everyone else. The pad's primary evdev node
//! is grabbed by [`PhysicalGamepad`](crate::PhysicalGamepad) itself, but a
//! controller's *other* evdev nodes — the kernel IMU companion, vendor
//! keyboard nodes, dongle side channels — would still hand raw hardware
//! input to whatever opens them. While the hub is claimed, each companion
//! node is opened and grabbed with `EVIOCGRAB`: the kernel routes events
//! only to the grabbing descriptor, so no other process (SDL's evdev
//! backend, libinput, raw `/dev/input` readers) sees a single event from
//! the real pad. Dropping the descriptors releases the holds.
//!
//! hidraw cannot be hidden this way: mainline kernels expose no exclusive
//! claim for hidraw nodes (`HIDIOCGRAB` exists only in some vendor
//! kernels). SDL-based games are covered instead by the ignore-device
//! environment the daemon injects at spawn — see `target_env_for`.

use std::path::{Path, PathBuf};

use evdev::Device;

/// How many sysfs levels above the pad's input directory to search for
/// sibling input devices. One level reaches the HID device (Sony and
/// Nintendo IMU companions live there), two reach the USB device (Xbox
/// pads register their guide button through a second interface).
const ANCESTOR_LEVELS: usize = 3;

/// One applied hold set. Recomputed only when its inputs change, so an
/// idle hub does not walk sysfs every pass.
#[derive(Default)]
pub(super) struct SiblingHolds {
    holds: Vec<Device>,
    applied: Option<Applied>,
}

#[derive(Clone, Debug, PartialEq)]
struct Applied {
    hold: bool,
    primary: Option<PathBuf>,
    imu: Option<PathBuf>,
}

impl SiblingHolds {
    /// Re-evaluates the exclusive holds. `primary` is the pad node the hub
    /// already grabbed (never grabbed twice — the kernel refuses a second
    /// `EVIOCGRAB`); `imu` is the companion node our own sensor reads and
    /// must keep receiving events on.
    pub(super) fn set(&mut self, hold: bool, primary: Option<&Path>, imu: Option<&Path>) {
        let next = Applied {
            hold,
            primary: primary.map(Path::to_path_buf),
            imu: imu.map(Path::to_path_buf),
        };
        if self.applied.as_ref() == Some(&next) {
            return;
        }
        // Dropping the old descriptors releases their grabs before new
        // ones are taken, so a changed node set never double-grabs.
        self.holds = Vec::new();
        if hold {
            if let Some(primary) = primary {
                for node in companion_event_nodes(Path::new("/sys/class/input"), primary) {
                    if next.imu.as_deref() == Some(node.as_path()) {
                        continue;
                    }
                    match Device::open(&node).map_err(|error| error.to_string()).and_then(
                        |mut device| {
                            device
                                .grab()
                                .map_err(|error| error.to_string())
                                .map(|()| device)
                        },
                    ) {
                        Ok(device) => {
                            eprintln!(
                                "hub: holding {} silent; the remapped session owns the pad",
                                node.display()
                            );
                            self.holds.push(device);
                        }
                        Err(error) => {
                            eprintln!("ira-input: could not hold {}: {error}", node.display())
                        }
                    }
                }
            }
        }
        self.applied = Some(next);
    }

    /// Releases every hold; used when the pad connection dies, since the
    /// fresh nodes it reconnects through are re-held on the next pass.
    pub(super) fn release(&mut self) {
        self.holds = Vec::new();
        self.applied = None;
    }
}

/// Every evdev node that belongs to the same physical device as `primary`:
/// the sysfs ancestors of the pad's input directory carry the HID device's
/// sibling `input/` devices, and each level is searched shallowly for
/// `input*/event*` until one yields something besides the primary node.
/// `sys_class_input` is injectable for tests.
fn companion_event_nodes(sys_class_input: &Path, primary: &Path) -> Vec<PathBuf> {
    let Some(stem) = primary.file_name() else {
        return Vec::new();
    };
    let Ok(canonical) = sys_class_input.join(stem).canonicalize() else {
        return Vec::new();
    };
    // /sys/class/input/eventN is a symlink into .../input/inputN/eventN;
    // its parent is the pad's own input directory.
    let mut ancestor = canonical.parent().and_then(Path::parent);
    for _ in 0..ANCESTOR_LEVELS {
        let Some(dir) = ancestor else {
            break;
        };
        let mut nodes = event_nodes_under(dir);
        nodes.retain(|node| node != primary);
        if !nodes.is_empty() {
            return nodes;
        }
        ancestor = dir.parent();
    }
    Vec::new()
}

/// The `/dev` event nodes of every `input*/event*` pair below `dir`.
fn event_nodes_under(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("input") {
            continue;
        }
        let Ok(devices) = std::fs::read_dir(entry.path()) else {
            continue;
        };
        for device in devices.flatten() {
            let device_name = device.file_name().to_string_lossy().into_owned();
            if device_name.starts_with("event") {
                nodes.push(Path::new("/dev/input").join(device_name));
            }
        }
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::{companion_event_nodes, event_nodes_under, SiblingHolds};
    use std::path::Path;

    /// Mirrors the kernel layout: /sys/class/input/eventN is a symlink
    /// into the HID device's input tree, where the pad and its IMU
    /// companion are sibling input devices.
    fn layout_with_imu(base: &Path) -> std::path::PathBuf {
        let class = base.join("class");
        let hid = base.join("devices").join("0003:054c:0ce6.000a");
        std::fs::create_dir_all(hid.join("input4")).unwrap();
        std::fs::create_dir_all(hid.join("input5")).unwrap();
        let event_node = hid.join("input4").join("event4");
        std::fs::create_dir_all(&event_node).unwrap();
        std::fs::create_dir_all(hid.join("input5").join("event5")).unwrap();
        std::fs::create_dir_all(&class).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(event_node, class.join("event4")).unwrap();
        class
    }

    #[test]
    fn test_companion_nodes_find_the_imu_sibling() {
        let base = tempfile::tempdir().unwrap();
        let class = layout_with_imu(base.path());
        let primary = Path::new("/dev/input/event4");
        let nodes = companion_event_nodes(&class, primary);
        assert_eq!(nodes, vec![Path::new("/dev/input").join("event5")]);
    }

    #[test]
    fn test_companion_nodes_empty_without_siblings() {
        let base = tempfile::tempdir().unwrap();
        let class = base.path().join("class");
        std::fs::create_dir_all(class.join("event4")).unwrap();
        let nodes = companion_event_nodes(&class, Path::new("/dev/input/event4"));
        assert!(nodes.is_empty(), "a single-node pad has no companions");
    }

    #[test]
    fn test_companion_nodes_ignore_unrelated_devices() {
        let base = tempfile::tempdir().unwrap();
        let class = base.path().join("class");
        let hid = base.path().join("devices").join("0003:054c:0ce6.000a");
        std::fs::create_dir_all(hid.join("input4").join("event4")).unwrap();
        // A keyboard plugged into another controller's tree must not be
        // mistaken for a companion of this pad.
        std::fs::create_dir_all(
            base.path()
                .join("devices")
                .join("other")
                .join("input9")
                .join("event9"),
        )
        .unwrap();
        std::fs::create_dir_all(&class).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            hid.join("input4").join("event4"),
            class.join("event4"),
        )
        .unwrap();
        let nodes = companion_event_nodes(&class, Path::new("/dev/input/event4"));
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_event_nodes_under_collects_every_input_device() {
        let base = tempfile::tempdir().unwrap();
        let hid = base.path().join("0003:054c:0ce6.000a");
        std::fs::create_dir_all(hid.join("input4").join("event4")).unwrap();
        std::fs::create_dir_all(hid.join("input5").join("event5")).unwrap();
        std::fs::create_dir_all(hid.join("hidraw")).unwrap();
        let nodes = event_nodes_under(&hid);
        assert!(nodes.contains(&Path::new("/dev/input").join("event4")));
        assert!(nodes.contains(&Path::new("/dev/input").join("event5")));
    }

    #[test]
    fn test_sibling_holds_recompute_only_on_input_change() {
        let mut holds = SiblingHolds::default();
        // The first application records the state; identical repeats must
        // not release and re-grab (visible only as no panic, but the cheap
        // path is what keeps the hub pass idle).
        holds.set(true, Some(Path::new("/dev/input/event4")), None);
        let applied = holds.applied.clone();
        holds.set(true, Some(Path::new("/dev/input/event4")), None);
        assert_eq!(holds.applied, applied);
        holds.set(false, Some(Path::new("/dev/input/event4")), None);
        assert!(!holds.applied.as_ref().unwrap().hold);
    }
}
