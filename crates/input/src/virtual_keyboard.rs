use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode};

use crate::OutputEvent;

const VIRTUAL_VENDOR: u16 = 0x1a1a;
const VIRTUAL_PRODUCT: u16 = 0x0001;
const VIRTUAL_VERSION: u16 = 0x0001;

pub struct VirtualKeyboard {
    device: VirtualDevice,
}

impl VirtualKeyboard {
    pub fn create(keycodes: impl IntoIterator<Item = u16>) -> io::Result<Self> {
        let keys: AttributeSet<KeyCode> = keycodes.into_iter().map(KeyCode).collect();
        let device = VirtualDevice::builder()?
            .name("Ira Virtual Keyboard")
            .input_id(InputId::new(
                BusType::BUS_VIRTUAL,
                VIRTUAL_VENDOR,
                VIRTUAL_PRODUCT,
                VIRTUAL_VERSION,
            ))
            .with_keys(&keys)?
            .build()?;
        Ok(Self { device })
    }

    pub fn emit(&mut self, event: &OutputEvent) -> io::Result<()> {
        let OutputEvent::Key { keycode, pressed } = event else {
            return Ok(());
        };
        let input = InputEvent::new(EventType::KEY.0, *keycode, i32::from(*pressed));
        self.device.emit(&[input])
    }

    /// A re-emission keyboard declaring every Linux keycode: for
    /// intercept-style forwarding, where any key may need a 1:1 copy.
    /// Bounded by `KEY_MAX` (0x2FF) — evdev's key set holds exactly 768
    /// bits and panics on anything past index 767.
    pub fn create_all_keys() -> io::Result<Self> {
        Self::create(1..=0x2FF)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_all_keys_stays_within_evdev_bound() {
        // Regression: an unbounded range panicked inside evdev's 768-bit
        // key set and took the whole session down with it.
        assert!(VirtualKeyboard::create_all_keys().is_ok());
    }
}
