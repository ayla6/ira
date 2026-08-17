use std::ffi::CString;
use std::os::raw::c_int;
use std::time::{Duration, Instant};

use ira_overlay::ui::Event;
use ira_overlay_ipc::gamepad_button_mask_from_evdev;

const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const BTN_SOUTH: u16 = 0x130;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_DOWN: u16 = 0x221;
const BTN_DPAD_LEFT: u16 = 0x222;
const BTN_DPAD_RIGHT: u16 = 0x223;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;
const KEY_PRESS: i32 = 1;
const KEY_BUF_BYTES: u32 = 96;

#[repr(C)]
struct InputEvent {
    _tv_sec: i64,
    _tv_usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

const _: () = assert!(std::mem::size_of::<InputEvent>() == 24);

pub enum GamepadEvent {
    Toggle,
    Screenshot,
    Record,
    Ui(Event),
}

pub struct GamepadInput {
    fds: Vec<c_int>,
    pressed: u32,
    toggle_pending: bool,
    hat_x: i32,
    hat_y: i32,
    next_scan: Instant,
}

impl GamepadInput {
    pub fn new() -> Self {
        let mut input = Self {
            fds: Vec::new(),
            pressed: 0,
            toggle_pending: false,
            hat_x: 0,
            hat_y: 0,
            next_scan: Instant::now(),
        };
        input.rescan();
        input
    }

    pub fn poll(
        &mut self,
        toggle: u32,
        screenshot: u32,
        record: u32,
        overlay_visible: bool,
    ) -> Vec<GamepadEvent> {
        if Instant::now() >= self.next_scan {
            self.rescan();
        }
        let mut events = Vec::new();
        let mut buf = [0u8; 24 * 16];
        for fd in self.fds.clone() {
            loop {
                let read = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
                if read <= 0 {
                    break;
                }
                for chunk in buf[..read as usize].chunks_exact(24) {
                    let event = unsafe { &*chunk.as_ptr().cast::<InputEvent>() };
                    if let Some(event) =
                        self.handle_event(event, toggle, screenshot, record, overlay_visible)
                    {
                        events.push(event);
                    }
                }
            }
        }
        events
    }

    fn handle_event(
        &mut self,
        event: &InputEvent,
        toggle: u32,
        screenshot: u32,
        record: u32,
        overlay_visible: bool,
    ) -> Option<GamepadEvent> {
        match event.type_ {
            EV_KEY => self.handle_button(event, toggle, screenshot, record, overlay_visible),
            EV_ABS => self.handle_hat(event),
            _ => None,
        }
    }

    fn handle_button(
        &mut self,
        event: &InputEvent,
        toggle: u32,
        screenshot: u32,
        record: u32,
        overlay_visible: bool,
    ) -> Option<GamepadEvent> {
        let button = gamepad_button_mask_from_evdev(event.code)?;
        if event.value == 0 {
            self.pressed &= !button;
            if self.toggle_pending && self.pressed == 0 {
                self.toggle_pending = false;
                return Some(GamepadEvent::Toggle);
            }
            return None;
        }
        if event.value != KEY_PRESS {
            return None;
        }
        self.pressed |= button;
        match hotkey_action(self.pressed, toggle, screenshot, record) {
            HotkeyAction::Screenshot => {
                self.toggle_pending = false;
                Some(GamepadEvent::Screenshot)
            }
            HotkeyAction::Record => {
                self.toggle_pending = false;
                Some(GamepadEvent::Record)
            }
            HotkeyAction::Toggle => {
                self.toggle_pending = true;
                None
            }
            HotkeyAction::None if overlay_visible => match event.code {
                BTN_SOUTH => Some(GamepadEvent::Ui(Event::Activate)),
                BTN_DPAD_UP => Some(GamepadEvent::Ui(Event::NavUp)),
                BTN_DPAD_DOWN => Some(GamepadEvent::Ui(Event::NavDown)),
                BTN_DPAD_LEFT => Some(GamepadEvent::Ui(Event::NavLeft)),
                BTN_DPAD_RIGHT => Some(GamepadEvent::Ui(Event::NavRight)),
                BTN_TL => Some(GamepadEvent::Ui(Event::Scroll { delta_y: -1.0 })),
                BTN_TR => Some(GamepadEvent::Ui(Event::Scroll { delta_y: 1.0 })),
                _ => None,
            },
            HotkeyAction::None => None,
        }
    }

    fn handle_hat(&mut self, event: &InputEvent) -> Option<GamepadEvent> {
        match event.code {
            ABS_HAT0X if self.hat_x == 0 && event.value != 0 => {
                self.hat_x = event.value;
                Some(GamepadEvent::Ui(if event.value < 0 {
                    Event::NavLeft
                } else {
                    Event::NavRight
                }))
            }
            ABS_HAT0Y if self.hat_y == 0 && event.value != 0 => {
                self.hat_y = event.value;
                Some(GamepadEvent::Ui(if event.value < 0 {
                    Event::NavUp
                } else {
                    Event::NavDown
                }))
            }
            ABS_HAT0X => {
                self.hat_x = event.value;
                None
            }
            ABS_HAT0Y => {
                self.hat_y = event.value;
                None
            }
            _ => None,
        }
    }

    fn rescan(&mut self) {
        for fd in self.fds.drain(..) {
            unsafe { libc::close(fd) };
        }
        let Ok(entries) = std::fs::read_dir("/dev/input") else {
            self.next_scan = Instant::now() + Duration::from_secs(2);
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str().filter(|name| name.starts_with("event")) else {
                continue;
            };
            let path = CString::new(format!("/dev/input/{name}")).unwrap();
            let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
            if fd >= 0 && is_gamepad(fd) {
                self.fds.push(fd);
            } else if fd >= 0 {
                unsafe { libc::close(fd) };
            }
        }
        self.next_scan = Instant::now() + Duration::from_secs(2);
    }
}

impl Drop for GamepadInput {
    fn drop(&mut self) {
        for fd in self.fds.drain(..) {
            unsafe { libc::close(fd) };
        }
    }
}

fn is_gamepad(fd: c_int) -> bool {
    let mut buffer = [0u8; KEY_BUF_BYTES as usize];
    let request =
        (2u64 << 30) | ((KEY_BUF_BYTES as u64) << 16) | (0x45 << 8) | (0x20 + EV_KEY as u64);
    let result = unsafe { libc::ioctl(fd, request, buffer.as_mut_ptr()) };
    result >= 0 && (buffer[BTN_SOUTH as usize / 8] & (1 << (BTN_SOUTH % 8))) != 0
}

#[derive(Debug, PartialEq, Eq)]
enum HotkeyAction {
    None,
    Toggle,
    Screenshot,
    Record,
}

fn hotkey_action(held: u32, toggle: u32, screenshot: u32, record: u32) -> HotkeyAction {
    if held == screenshot {
        HotkeyAction::Screenshot
    } else if held == record {
        HotkeyAction::Record
    } else if held == toggle {
        HotkeyAction::Toggle
    } else {
        HotkeyAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::{hotkey_action, HotkeyAction};
    use ira_overlay_ipc::{
        DEFAULT_RECORD_GAMEPAD_HOTKEY, DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY,
        DEFAULT_TOGGLE_GAMEPAD_HOTKEY,
    };

    #[test]
    fn test_hotkey_action_prefers_screenshot_chord() {
        assert_eq!(
            hotkey_action(
                DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY,
                DEFAULT_TOGGLE_GAMEPAD_HOTKEY,
                DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY,
                DEFAULT_RECORD_GAMEPAD_HOTKEY,
            ),
            HotkeyAction::Screenshot
        );
    }
}
