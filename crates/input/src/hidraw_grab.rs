//! Exclusive hold on a physical pad's `/dev/hidraw` node.
//!
//! When a session exposes a native controller twin (the virtual Switch Pro,
//! DS4 or DualSense), the game must end up talking to *the twin*, not to the
//! physical pad it mirrors. SDL's hidapi stack claims the physical pad by
//! opening its hidraw — and with two identical identities in play, SDL's
//! older builds drop ours instead of the hardware's. Holding the physical
//! pad's hidraw with `HIDIOCGRAB` for the session's lifetime makes that
//! claim fail cleanly (the kernel returns EPERM to later openers), so SDL
//! falls back to the twin's evdev nodes and the twin becomes the pad the
//! game sees. The grab is released when the file is closed, which happens
//! when the session ends.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

/// HIDIOCGRAB: `_IO('H', 0x11)` from linux/hidraw.h. The held argument is
/// a plain int flag; closing the descriptor releases the grab.
const HIDIOCGRAB: libc::c_ulong = 0x4811;

/// One held hidraw grab. Dropping this value closes the descriptor and
/// releases the exclusive hold.
pub(crate) struct PadHidrawGrab {
    _file: File,
    path: PathBuf,
}

impl PadHidrawGrab {
    /// The grabbed hidraw node, for diagnostics.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

/// Grabs the hidraw that belongs to the physical pad at `evdev_path`
/// (a `/dev/input/eventN` node), when one exists. Pads without a hidraw
/// (pure uinput devices) and unopenable nodes yield `None` with a log line.
pub(crate) fn grab_pad_hidraw(evdev_path: &Path) -> Option<PadHidrawGrab> {
    let hidraw = hidraw_node_for_event(evdev_path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&hidraw)
        .inspect_err(|error| {
            eprintln!(
                "ira-input: could not open {} to hold the physical pad's claim: {error}",
                hidraw.display()
            )
        })
        .ok()?;
    // Two ira sessions may race for the same pad; the first grab wins and
    // the loser keeps running without one (the twin still works, the real
    // pad just stays visible to SDL).
    if unsafe { libc::ioctl(file.as_raw_fd(), HIDIOCGRAB, 1i32) } != 0 {
        let error = io::Error::last_os_error();
        eprintln!(
            "ira-input: could not grab {}: {error}; the physical pad stays visible to SDL",
            hidraw.display()
        );
        return None;
    }
    Some(PadHidrawGrab { _file: file, path: hidraw })
}

/// Resolves the `/dev/hidrawN` node behind an evdev device by walking the
/// sysfs tree: `/sys/class/input/eventN` → the hid device → its `hidraw`
/// child. `sys_class_input` is injectable for tests.
fn hidraw_node_for_event(evdev_path: &Path) -> Option<PathBuf> {
    hidraw_node_in(evdev_path, Path::new("/sys/class/input"))
}

fn hidraw_node_in(evdev_path: &Path, sys_class_input: &Path) -> Option<PathBuf> {
    let event_name = evdev_path.file_name()?.to_string_lossy().into_owned();
    let device_dir = sys_class_input.join(&event_name).canonicalize().ok()?;
    let hidraw_dir = device_dir.join("hidraw");
    let hidraw_name = std::fs::read_dir(&hidraw_dir)
        .ok()?
        .flatten()
        .next()?
        .file_name()
        .into_string()
        .ok()?;
    Some(Path::new("/dev").join(hidraw_name))
}

#[cfg(test)]
mod tests {
    use super::hidraw_node_in;
    use std::path::Path;

    #[test]
    fn test_hidraw_node_resolves_through_sysfs_layout() {
        let base = tempfile::tempdir().unwrap();
        let input = base.path().join("input");
        let event_device = input.join("event17");
        let hidraw_dir = event_device.join("hidraw");
        std::fs::create_dir_all(&hidraw_dir).unwrap();
        std::fs::create_dir(hidraw_dir.join("hidraw4")).unwrap();

        let resolved = hidraw_node_in(Path::new("/dev/input/event17"), &input);
        assert_eq!(resolved, Some(Path::new("/dev").join("hidraw4")));
    }

    #[test]
    fn test_hidraw_node_missing_when_no_hidraw_child() {
        let base = tempfile::tempdir().unwrap();
        let input = base.path().join("input");
        std::fs::create_dir_all(input.join("event5")).unwrap();

        let resolved = hidraw_node_in(Path::new("/dev/input/event5"), &input);
        assert_eq!(resolved, None);
    }
}
