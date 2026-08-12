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
}
