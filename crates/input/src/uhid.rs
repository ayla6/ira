//! Userspace HID drivers through `/dev/uhid`: the kernel-side plumbing to
//! create virtual HID devices with a report descriptor we choose. Unlike
//! uinput's evdev nodes these get a real `/dev/hidraw` character device, so
//! SDL's hidapi drivers (DS4/DualSense/Switch) can parse authentic reports —
//! including motion — from our emulated controllers.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;

// enum uhid_event_type (linux/uhid.h)
const UHID_DESTROY: u32 = 1;
const UHID_START: u32 = 2;
const UHID_STOP: u32 = 3;
const UHID_OPEN: u32 = 4;
const UHID_CLOSE: u32 = 5;
const UHID_OUTPUT: u32 = 6;
const UHID_GET_REPORT: u32 = 9;
const UHID_GET_REPORT_REPLY: u32 = 10;
const UHID_CREATE2: u32 = 11;
const UHID_INPUT2: u32 = 12;
const UHID_SET_REPORT: u32 = 13;

/// BUS_USB from linux/input.h: hidapi treats USB devices as wired.
pub const BUS_USB: u16 = 0x03;
/// enum uhid_report_type: feature reports carry configuration both ways.
pub const FEATURE_REPORT: u8 = 0;

const NAME_LEN: usize = 128;
const PHYS_LEN: usize = 64;
const UNIQ_LEN: usize = 64;
const UHID_DATA_MAX: usize = 4096;
/// type u32 + create2 payload (name/phys/uniq/rd_size/bus/ids + descriptor).
const CREATE2_FIXED: usize = 4 + NAME_LEN + PHYS_LEN + UNIQ_LEN + 2 + 2 + 16;
const EVENT_BUF_LEN: usize = CREATE2_FIXED + UHID_DATA_MAX;

/// Kernel requests and notifications delivered on the uhid fd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UhidEvent {
    Start,
    Stop,
    /// A reader opened the hidraw node; input reports now reach someone.
    Open,
    Close,
    OutputReport {
        data: Vec<u8>,
    },
    GetReport {
        id: u32,
        number: u8,
        kind: u8,
    },
    SetReport {
        id: u32,
        number: u8,
        data: Vec<u8>,
    },
}

pub struct UhidDevice {
    file: File,
}

impl UhidDevice {
    /// Creates a kernel HID device from a report descriptor. The name also
    /// becomes the evdev twin's name; SDL picks vendor/product IDs to select
    /// its hidapi driver. `uniq` is the serial SDL's evdev backend compares
    /// to pair a gamepad with its sensor node — two devices created with
    /// the same uniq are joined by SDL, which is how flatpak-visible
    /// motion works (uinput nodes cannot carry uniq at all).
    pub fn create(
        name: &str,
        uniq: &str,
        descriptor: &[u8],
        bus: u16,
        vendor: u32,
        product: u32,
    ) -> io::Result<Self> {
        if descriptor.len() > UHID_DATA_MAX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "report descriptor exceeds UHID_DATA_MAX",
            ));
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uhid")?;
        file.write_all(&create2_event(name, uniq, descriptor, bus, vendor, product))?;
        Ok(Self { file })
    }

    /// Submits one input report; the kernel delivers it to hidraw readers.
    pub fn send_input_report(&mut self, report: &[u8]) -> io::Result<()> {
        self.file.write_all(&input2_event(report))
    }

    /// Answers a pending [`UhidEvent::GetReport`] (err 0 with data, or an
    /// errno without). Callers that never serve feature reports simply drop
    /// those events instead.
    pub fn reply_get_report(&mut self, id: u32, err: u16, data: &[u8]) -> io::Result<()> {
        let size = data.len().min(UHID_DATA_MAX);
        let mut event = Vec::with_capacity(12 + size);
        event.extend_from_slice(&UHID_GET_REPORT_REPLY.to_le_bytes());
        event.extend_from_slice(&id.to_le_bytes());
        event.extend_from_slice(&err.to_le_bytes());
        event.extend_from_slice(&(size as u16).to_le_bytes());
        event.extend_from_slice(&data[..size]);
        self.file.write_all(&event)
    }

    /// Drains every queued kernel event without blocking.
    pub fn poll(&mut self) -> io::Result<Vec<UhidEvent>> {
        let mut buf = [0u8; EVENT_BUF_LEN];
        let mut events = Vec::new();
        loop {
            match self.file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => events.push(parse_event(&buf[..n])),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(events)
    }
}

impl Drop for UhidDevice {
    fn drop(&mut self) {
        // Removing the device keeps stale /dev/hidraw nodes from piling up
        // across daemon restarts.
        let destroy = UHID_DESTROY.to_le_bytes();
        let _ = self.file.write_all(&destroy);
    }
}

fn create2_event(
    name: &str,
    uniq: &str,
    descriptor: &[u8],
    bus: u16,
    vendor: u32,
    product: u32,
) -> Vec<u8> {
    let mut event = vec![0u8; CREATE2_FIXED];
    event[0..4].copy_from_slice(&UHID_CREATE2.to_le_bytes());
    let bytes = name.as_bytes();
    let copied = bytes.len().min(NAME_LEN - 1);
    event[4..4 + copied].copy_from_slice(&bytes[..copied]);
    let uniq_offset = 4 + NAME_LEN + PHYS_LEN;
    let uniq_bytes = uniq.as_bytes();
    let uniq_copied = uniq_bytes.len().min(UNIQ_LEN - 1);
    event[uniq_offset..uniq_offset + uniq_copied].copy_from_slice(&uniq_bytes[..uniq_copied]);
    let rd_size_offset = 4 + NAME_LEN + PHYS_LEN + UNIQ_LEN;
    event[rd_size_offset..rd_size_offset + 2]
        .copy_from_slice(&(descriptor.len() as u16).to_le_bytes());
    event[rd_size_offset + 2..rd_size_offset + 4].copy_from_slice(&bus.to_le_bytes());
    event[rd_size_offset + 4..rd_size_offset + 8].copy_from_slice(&vendor.to_le_bytes());
    event[rd_size_offset + 8..rd_size_offset + 12].copy_from_slice(&product.to_le_bytes());
    // version/country stay zero; the descriptor carries everything else.
    event.resize(CREATE2_FIXED + descriptor.len(), 0);
    event[CREATE2_FIXED..].copy_from_slice(descriptor);
    event
}

fn input2_event(report: &[u8]) -> Vec<u8> {
    let mut event = Vec::with_capacity(6 + report.len());
    event.extend_from_slice(&UHID_INPUT2.to_le_bytes());
    event.extend_from_slice(&(report.len() as u16).to_le_bytes());
    event.extend_from_slice(report);
    event
}

fn parse_event(raw: &[u8]) -> UhidEvent {
    if raw.len() < 4 {
        return UhidEvent::Stop;
    }
    let kind = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    match kind {
        UHID_START => UhidEvent::Start,
        UHID_STOP => UhidEvent::Stop,
        UHID_OPEN => UhidEvent::Open,
        UHID_CLOSE => UhidEvent::Close,
        UHID_OUTPUT => {
            // struct uhid_output_req: data[4096], size u16 at 4096, rtype u8.
            let size_at = 4 + UHID_DATA_MAX;
            let len = raw
                .get(size_at..size_at + 2)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]) as usize)
                .unwrap_or(0);
            let end = (4 + len).min(raw.len());
            UhidEvent::OutputReport {
                data: raw[4..end].to_vec(),
            }
        }
        UHID_GET_REPORT => UhidEvent::GetReport {
            id: read_u32(raw, 4),
            number: raw.get(8).copied().unwrap_or(0),
            kind: raw.get(9).copied().unwrap_or(0),
        },
        UHID_SET_REPORT => {
            // struct uhid_set_report_req: id u32, rnum u8, rtype u8,
            // size u16, data[]. A short kernel write just truncates data.
            let size = raw
                .get(10..12)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]) as usize)
                .unwrap_or(0);
            let start = 12.min(raw.len());
            let end = (12 + size).min(raw.len());
            UhidEvent::SetReport {
                id: read_u32(raw, 4),
                number: raw.get(8).copied().unwrap_or(0),
                data: if start <= end {
                    raw[start..end].to_vec()
                } else {
                    Vec::new()
                },
            }
        }
        _ => UhidEvent::Stop,
    }
}

fn read_u32(raw: &[u8], offset: usize) -> u32 {
    raw.get(offset..offset + 4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{create2_event, input2_event, parse_event, UhidEvent, BUS_USB};

    const DESCRIPTOR: &[u8] = &[0x05, 0x01, 0x09, 0x05];

    #[test]
    fn test_create2_event_carries_identity_and_descriptor() {
        let event = create2_event(
            "Ira Virtual DS4",
            "ira-uniq-1",
            DESCRIPTOR,
            BUS_USB,
            0x054c,
            0x09cc,
        );
        assert_eq!(u32::from_le_bytes(event[0..4].try_into().unwrap()), 11);
        let name = &event[4..4 + "Ira Virtual DS4".len()];
        assert_eq!(std::str::from_utf8(name).unwrap(), "Ira Virtual DS4");
        // Name is NUL padded inside its fixed 128-byte field.
        assert_eq!(event[4 + "Ira Virtual DS4".len()], 0);
        // Uniq sits between the 64-byte phys field and rd_size, NUL padded.
        let uniq_offset = 4 + 128 + 64;
        assert_eq!(&event[uniq_offset..uniq_offset + 10], b"ira-uniq-1");
        assert_eq!(event[uniq_offset + 10], 0);
        let base = 4 + 128 + 64 + 64;
        let rd_size = u16::from_le_bytes(event[base..base + 2].try_into().unwrap());
        assert_eq!(usize::from(rd_size), DESCRIPTOR.len());
        let bus = u16::from_le_bytes(event[base + 2..base + 4].try_into().unwrap());
        assert_eq!(bus, BUS_USB);
        let vendor = u32::from_le_bytes(event[base + 4..base + 8].try_into().unwrap());
        assert_eq!(vendor, 0x054c);
        let product = u32::from_le_bytes(event[base + 8..base + 12].try_into().unwrap());
        assert_eq!(product, 0x09cc);
        assert_eq!(&event[base + 20..base + 20 + DESCRIPTOR.len()], DESCRIPTOR);
    }

    #[test]
    fn test_input2_event_prefixes_length_and_data() {
        let report = [0x01u8, 0x7f, 0x80];
        let event = input2_event(&report);
        assert_eq!(event.len(), 6 + report.len());
        assert_eq!(u32::from_le_bytes(event[0..4].try_into().unwrap()), 12);
        assert_eq!(u16::from_le_bytes(event[4..6].try_into().unwrap()), 3);
        assert_eq!(&event[6..], &report);
    }

    #[test]
    fn test_parse_kernel_events() {
        assert_eq!(parse_event(&[]), UhidEvent::Stop);
        assert_eq!(parse_event(&[2, 0, 0, 0]), UhidEvent::Start);
        assert_eq!(parse_event(&[4, 0, 0, 0]), UhidEvent::Open);
        assert_eq!(parse_event(&[5, 0, 0, 0]), UhidEvent::Close);

        // GET_REPORT: id, rnum, rtype packed after the type word.
        let request = [9u8, 0, 0, 0, 0x2a, 0, 0, 0, 0x03, 0x01];
        assert_eq!(
            parse_event(&request),
            UhidEvent::GetReport {
                id: 42,
                number: 3,
                kind: 1,
            }
        );

        // OUTPUT: data then size then rtype, all behind a 4096-byte field.
        let mut output = vec![0u8; 4 + 4096 + 3];
        output[0] = 6;
        output[4] = 0xAA;
        // The u16 length lives right after the data field.
        output[4 + 4096] = 1;
        output[4 + 4096 + 1] = 0;
        assert_eq!(
            parse_event(&output),
            UhidEvent::OutputReport { data: vec![0xAA] }
        );
    }

    #[test]
    fn test_parse_truncated_events_degrade_without_panicking() {
        let truncated_output = [6u8, 0, 0, 0, 0xFF];
        matches!(
            parse_event(&truncated_output),
            UhidEvent::OutputReport { .. }
        );
    }
}
