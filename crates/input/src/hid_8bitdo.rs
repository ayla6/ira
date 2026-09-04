//! Battery reading for 8BitDo pads in DInput mode, over their hidraw node.
//!
//! XInput mode (the 2.4g dongle on xpad) exposes no battery anywhere — the
//! protocol simply has none. DInput mode does: the enhanced state packet
//! (report ids 0x01 Bluetooth, 0x03 legacy, 0x04 USB/dongle) carries the
//! power byte at offset 14 — percent in the low 7 bits, a charging bit on
//! top. This mirrors SDL's `SDL_hidapi_8bitdo` driver: reports fan out to
//! every open hidraw fd, so reading here never steals input from a game
//! using the same pad.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use crate::physical::DeviceInfo;
use crate::rumble::sibling_hidraw_nodes;
use crate::switch_hidraw::host_visible;

/// 8BitDo's USB vendor id.
const VENDOR_8BITDO: u16 = 0x2dc8;
/// The Ultimate 2 Wireless in DInput mode, via the 2.4g dongle or
/// Bluetooth. (XInput mode enumerates differently and has no battery at
/// all; wired is XInput-only.)
const PRODUCT_ULTIMATE2_WIRELESS_DINPUT: u16 = 0x6012;
/// Report ids carrying the enhanced state packet.
const STATE_REPORT_IDS: [u8; 3] = [0x01, 0x03, 0x04];
/// The first firmware revision streams 12-byte reports with no power byte;
/// from v1.03 reports are 34 bytes and the power byte is real.
const POWERSTATE_MIN_REPORT: usize = 34;
/// Offset of the power byte inside the enhanced state packet.
const POWER_BYTE: usize = 14;

/// One reading of the pad's power state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PadBattery {
    /// 0..=100.
    pub percent: u8,
    /// On the charger (or dock) and not yet full.
    pub charging: bool,
}

/// A live battery reader for one 8BitDo DInput pad. Holds the hidraw fd
/// open and drains whatever reports arrive.
pub struct EightBitDoBatteryReader {
    file: File,
    powerstate_supported: bool,
}

impl EightBitDoBatteryReader {
    /// Probe the hidraw nodes beside an evdev pad for an 8BitDo DInput
    /// device. `None` for every other pad, including the same hardware in
    /// XInput mode.
    pub fn open(pad: &DeviceInfo) -> Option<Self> {
        if pad.vendor != VENDOR_8BITDO || pad.product != PRODUCT_ULTIMATE2_WIRELESS_DINPUT {
            return None;
        }
        for node in sibling_hidraw_nodes(&pad.path) {
            let node: PathBuf = host_visible(node);
            let Ok(file) = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&node)
            else {
                continue;
            };
            return Some(Self { file, powerstate_supported: false });
        }
        None
    }

    /// Drain the reports that arrived since the last poll; the freshest
    /// power reading wins. `None` when nothing new came in.
    pub fn poll(&mut self) -> Option<PadBattery> {
        let mut latest = None;
        let mut buffer = [0u8; 64];
        loop {
            match self.file.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if let Some(battery) =
                        parse_state_report(&buffer[..n], &mut self.powerstate_supported)
                    {
                        latest = Some(battery);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        latest
    }
}

/// Parse one hidraw report against the reader's firmware detection. The
/// power byte only counts once a long (v1.03+) report has confirmed the
/// pad streams the enhanced packet shape.
fn parse_state_report(report: &[u8], powerstate_supported: &mut bool) -> Option<PadBattery> {
    let id = *report.first()?;
    if !STATE_REPORT_IDS.contains(&id) {
        return None;
    }
    if report.len() >= POWERSTATE_MIN_REPORT {
        *powerstate_supported = true;
    }
    if !*powerstate_supported || report.len() <= POWER_BYTE {
        return None;
    }
    let byte = report[POWER_BYTE];
    let percent = (byte & 0x7f).min(100);
    // The charging bit stays set once the pack is full; that state is
    // "charged", not "charging".
    let charging = byte >> 7 == 1 && percent < 100;
    Some(PadBattery { percent, charging })
}

#[cfg(test)]
mod tests {
    use super::{parse_state_report, PadBattery};

    #[test]
    fn test_parse_ignores_other_report_ids() {
        let mut supported = true;
        let report = [0x30, 0u8, 0x80, 0x02];
        assert_eq!(parse_state_report(&report, &mut supported), None);
    }

    #[test]
    fn test_parse_needs_long_report_before_reporting() {
        let mut supported = false;
        // Short (v1.02) state report: no power byte, and it must not
        // enable powerstate on its own.
        let mut report = [0u8; 12];
        report[0] = 0x01;
        assert_eq!(parse_state_report(&report, &mut supported), None);
        assert!(!supported);
    }

    #[test]
    fn test_parse_long_report_enables_powerstate() {
        let mut supported = false;
        let mut report = [0u8; 34];
        report[0] = 0x04;
        report[14] = 0x32; // 50%, on battery
        assert_eq!(
            parse_state_report(&report, &mut supported),
            Some(PadBattery { percent: 50, charging: false })
        );
        assert!(supported);
    }

    #[test]
    fn test_parse_reads_power_byte_when_supported() {
        let mut supported = true;
        let mut report = [0u8; 34];
        report[0] = 0x01;
        report[14] = 0x80 | 37;
        assert_eq!(
            parse_state_report(&report, &mut supported),
            Some(PadBattery { percent: 37, charging: true })
        );
    }

    #[test]
    fn test_parse_full_pack_on_charger_reads_charged() {
        let mut supported = true;
        let mut report = [0u8; 34];
        report[0] = 0x04;
        report[14] = 0x80 | 100;
        assert_eq!(
            parse_state_report(&report, &mut supported),
            Some(PadBattery { percent: 100, charging: false })
        );
    }

    #[test]
    fn test_parse_clamps_over_100() {
        let mut supported = true;
        let mut report = [0u8; 34];
        report[0] = 0x04;
        report[14] = 0x7f;
        assert_eq!(
            parse_state_report(&report, &mut supported),
            Some(PadBattery { percent: 100, charging: false })
        );
    }
}
