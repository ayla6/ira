//! Verifies the 8BitDo DInput-mode rumble protocol against a live pad:
//! writes the same 5-byte output report SDL's hidapi 8BitDo driver sends
//! (report id 0x05, strong motor byte, weak motor byte) and silences it
//! again — first both motors, then the weak one alone so the two are
//! distinguishable by feel.
//!
//! ```sh
//! distrobox enter rust-dev -- cargo build -p ira-input --example dinput_rumble_probe
//! target/debug/examples/dinput_rumble_probe          # host user, uaccess rules apply
//! ```

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

use ira_input::rumble_report_8bitdo;

const HIDRAW_DIR: &str = "/dev/hidraw";
/// _IOR('H', 0x03, struct hidraw_devinfo{u32 bus; s16 vendor; s16 product})
const HIDIOCGRAW_INFO: u64 = 0x8008_4803;

fn raw_info(path: &str) -> Option<(u16, u16)> {
    let file = File::open(path).ok()?;
    let mut buf = [0u8; 8];
    let rc = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            HIDIOCGRAW_INFO as libc::c_ulong,
            buf.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return None;
    }
    let vendor = u16::from_le_bytes([buf[4], buf[5]]);
    let product = u16::from_le_bytes([buf[6], buf[7]]);
    Some((vendor, product))
}

/// Finds every hidraw node of the 8BitDo dongle (0x2dc8:0x6012).
fn find_pads() -> Vec<PathBuf> {
    let mut pads = Vec::new();
    for index in 0..32 {
        let path = format!("{HIDRAW_DIR}{index}");
        if let Some((vendor, product)) = raw_info(&path) {
            println!("{path}: {vendor:04x}:{product:04x}");
            if vendor == 0x2dc8 && product == 0x6012 {
                pads.push(PathBuf::from(path));
            }
        }
    }
    pads
}

fn burst(file: &mut File, strong: u16, weak: u16, ms: u64) -> std::io::Result<()> {
    file.write_all(&rumble_report_8bitdo(strong, weak))?;
    std::thread::sleep(Duration::from_millis(ms));
    file.write_all(&rumble_report_8bitdo(0, 0))
}

fn main() {
    let pads = find_pads();
    let Some(pad) = pads.first() else {
        eprintln!("no 2dc8:6012 hidraw node found; connect the pad first");
        return;
    };
    println!("rumbling {} — both motors, then the weak one alone\n", pad.display());
    let Ok(mut file) = OpenOptions::new().write(true).open(pad) else {
        eprintln!("cannot open {} for writing; check the hidraw uaccess rule", pad.display());
        return;
    };
    for (strong, weak) in [(u16::MAX, u16::MAX), (0, u16::MAX)] {
        if let Err(error) = burst(&mut file, strong, weak, 1200) {
            eprintln!("write failed: {error}");
            return;
        }
        std::thread::sleep(Duration::from_millis(600));
    }
    println!("done — both bursts should have vibrated, the second lighter");
}
