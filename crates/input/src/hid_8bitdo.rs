//! The 8BitDo DInput protocol, spoken over hidraw: who these pads are
//! (vendor, product ids, identity helpers) and what they say (the enhanced
//! state packet with the battery byte, the rumble output report, and the
//! enable-reports handshake older firmware needs). The consumers live with
//! their backends — rumble.rs replays effects, physical.rs picks button
//! layouts and controller families, the couch view reads the battery — so
//! every magic number on the wire exists exactly once, here.
//!
//! XInput mode (the 2.4g dongle claimed by xpad) and wired expose none of
//! this: no battery, no hidraw protocol. Switch-mode pads speak Nintendo's
//! protocol instead (see switch_hidraw).

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use crate::physical::DeviceInfo;
use crate::rumble::sibling_hidraw_nodes;
use crate::switch_hidraw::host_visible;

/// 8BitDo's USB vendor id.
pub const VENDOR_8BITDO: u16 = 0x2dc8;
/// The Ultimate 2 Wireless in DInput mode, via the 2.4g dongle or
/// Bluetooth (wired is XInput-only).
pub const ULTIMATE_2_WIRELESS: u16 = 0x6012;
/// The Ultimate 3, likewise handshake-free (its feature report 0x30
/// carries capabilities instead).
pub const ULTIMATE_3: u16 = 0x202f;

/// True for the Ultimate 2 Wireless in DInput mode, whose button map
/// follows SDL's HIDAPI driver rather than evdev's positional standard.
pub fn is_ultimate_2(vendor: u16, product: u16) -> bool {
    vendor == VENDOR_8BITDO && product == ULTIMATE_2_WIRELESS
}

/// _IOC(_IOC_READ|_IOC_WRITE, 'H', 0x07, 64): HIDIOCGFEATURE(64).
const HIDIOCGFEATURE_64: libc::c_ulong = 0xC040_4807;
/// SDL's "enable SDL reports" feature id: older 8BitDo firmware (SF30/SN30
/// Pro, Pro 2, Pro 3) keeps rumble and the enhanced reports off until
/// userspace reads this feature.
const ENABLE_REPORTS_FEATURE_ID: u8 = 0x06;

/// The rumble report SDL's hidapi 8BitDo driver writes to these pads:
/// report id 0x05 followed by the strong (low-frequency) and weak
/// (high-frequency) motor strengths, one byte each. The trailing pair would
/// be trigger rumble, which only Ultimate 3 firmware understands.
pub fn rumble_report_8bitdo(strong: u16, weak: u16) -> [u8; 5] {
    [0x05, (strong >> 8) as u8, (weak >> 8) as u8, 0, 0]
}

/// Whether this 8BitDo product needs the feature-report handshake before
/// its motors listen: the Ultimate 2 and 3 dongles rumble out of the box,
/// everything older follows SDL's read-feature-0x06 first.
pub(crate) fn needs_enable_handshake(product: u16) -> bool {
    product != ULTIMATE_2_WIRELESS && product != ULTIMATE_3
}

/// Reads the "enable SDL reports" feature, the setup older 8BitDo firmware
/// requires. One GET_FEATURE is exactly what SDL's driver does at init;
/// failure is logged and survived — some clones rumble regardless.
pub(crate) fn enable_8bitdo_reports(file: &mut File) {
    use std::os::fd::AsRawFd;
    let mut report = [0u8; 64];
    report[0] = ENABLE_REPORTS_FEATURE_ID;
    let result = unsafe { libc::ioctl(file.as_raw_fd(), HIDIOCGFEATURE_64, report.as_mut_ptr()) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        eprintln!(
            "ira-input: 8BitDo enable-reports handshake failed (rumble may still work): {error}"
        );
    }
}

/// Report ids carrying the enhanced state packet.
const STATE_REPORT_IDS: [u8; 3] = [0x01, 0x03, 0x04];
/// The first Ultimate 2 firmware revision streams 12-byte reports with no
/// power byte; from v1.03 reports are 34 bytes and the power byte is real.
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
/// open and drains whatever reports arrive. Reports fan out to every open
/// hidraw fd, so reading here never steals input from a game using the
/// same pad.
pub struct EightBitDoBatteryReader {
    file: File,
    powerstate_supported: bool,
}

impl EightBitDoBatteryReader {
    /// Probe the hidraw nodes beside an evdev pad for an 8BitDo DInput
    /// device. `None` for every other pad, including the same hardware in
    /// XInput mode.
    pub fn open(pad: &DeviceInfo) -> Option<Self> {
        if !is_ultimate_2(pad.vendor, pad.product) {
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
    use super::{is_ultimate_2, needs_enable_handshake, parse_state_report, rumble_report_8bitdo, PadBattery};

    #[test]
    fn test_is_ultimate_2_matches_dinput_identity_only() {
        assert!(is_ultimate_2(0x2dc8, 0x6012));
        assert!(!is_ultimate_2(0x2dc8, 0x310b), "XInput mode is a different product");
        assert!(!is_ultimate_2(0x057e, 0x2009), "Switch-mode twins are not this protocol");
    }

    #[test]
    fn test_rumble_report_matches_sdl_layout() {
        // SDL's hidapi 8BitDo driver sends report 0x05 with one magnitude
        // byte per motor, taken from the top half of the 16-bit scale.
        assert_eq!(
            rumble_report_8bitdo(u16::MAX, u16::MAX),
            [0x05, 0xFF, 0xFF, 0x00, 0x00]
        );
        assert_eq!(
            rumble_report_8bitdo(0, 0),
            [0x05, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            rumble_report_8bitdo(0x1234, 0xABCD),
            [0x05, 0x12, 0xAB, 0x00, 0x00]
        );
    }

    #[test]
    fn test_rumble_report_recovers_hid_motor_bytes() {
        // The DS4/DualSense twins scale one HID motor byte up to the evdev
        // range with byte * 257; shifting back down must return the byte.
        for byte in [0u16, 1, 64, 128, 200, 255] {
            let scaled = byte * 257;
            assert_eq!(rumble_report_8bitdo(scaled, scaled)[1], byte as u8);
        }
    }

    #[test]
    fn test_only_older_models_need_the_enable_handshake() {
        // Ultimate 2 and Ultimate 3 rumble without setup; SF30/SN30 Pro,
        // Pro 2, Pro 3 and their Bluetooth twins need SDL's feature-0x06
        // read first.
        assert!(!needs_enable_handshake(0x6012));
        assert!(!needs_enable_handshake(0x202f));
        assert!(needs_enable_handshake(0x6000));
        assert!(needs_enable_handshake(0x6001));
        assert!(needs_enable_handshake(0x6003));
        assert!(needs_enable_handshake(0x6009));
        assert!(needs_enable_handshake(0x6101));
    }

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
