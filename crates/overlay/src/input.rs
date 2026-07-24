use std::fs::File;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use crate::types::OVERLAY_VISIBLE;
use crate::ui::Event;
use crate::wayland::HAS_FOCUS;

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const EVIOCGBIT_KEY: libc::c_ulong = 0x80604521;
const EVIOCGBIT_REL: libc::c_ulong = 0x80084522;
const ABS_HAT0X: u16 = 16;
const ABS_HAT0Y: u16 = 17;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const BTN_TOUCH: u16 = 330;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_TAB: u16 = 15;
const KEY_F11: u16 = 87;
const KEY_F12: u16 = 88;
const BTN_LEFT: u16 = 272;
const BTN_RIGHT: u16 = 273;
const BTN_MODE: u16 = 316;

#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    _tv_sec: u64,
    _tv_usec: u64,
    typ: u16,
    code: u16,
    value: i32,
}

const ZERO: InputEvent = InputEvent { _tv_sec: 0, _tv_usec: 0, typ: 0, code: 0, value: 0 };
const _: () = assert!(std::mem::size_of::<InputEvent>() == 24);

pub fn start_input_thread() {
    std::thread::spawn(input_loop);
}

fn is_input_device(fd: libc::c_int) -> bool {
    let mut keybit = [0u8; 96];
    if unsafe { libc::ioctl(fd, EVIOCGBIT_KEY, keybit.as_mut_ptr()) } >= 0 {
        let check = |code: u16| keybit[(code / 8) as usize] & (1 << (code % 8)) != 0;
        if check(KEY_TAB) || check(BTN_MODE) || check(BTN_LEFT) {
            return true;
        }
    }
    let mut relbit = [0u8; 8];
    if unsafe { libc::ioctl(fd, EVIOCGBIT_REL, relbit.as_mut_ptr()) } >= 0 {
        let check = |code: u16| relbit[(code / 8) as usize] & (1 << (code % 8)) != 0;
        if check(REL_X) && check(REL_Y) {
            return true;
        }
    }
    false
}

fn scan_devices(files: &mut Vec<(PathBuf, File)>) {
    let Ok(entries) = std::fs::read_dir("/dev/input") else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("event")) {
            continue;
        }
        if files.iter().any(|(p, _)| p == &path) {
            continue;
        }
        if let Ok(file) = File::options()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
        {
            if is_input_device(file.as_raw_fd()) {
                files.push((path, file));
            }
        }
    }
}

fn input_loop() {
    let mut files: Vec<(PathBuf, File)> = Vec::new();
    scan_devices(&mut files);

    if files.is_empty() {
        eprintln!("ira-overlay: no input devices accessible (are you in the 'input' group?)");
        return;
    }

    let mut inotify_fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK) };
    if inotify_fd >= 0 {
        let wd = unsafe {
            libc::inotify_add_watch(inotify_fd, c"/dev/input".as_ptr(), libc::IN_CREATE)
        };
        if wd < 0 {
            unsafe { libc::close(inotify_fd) };
            inotify_fd = -1;
        }
    }

    let mut left_shift = false;
    let mut right_shift = false;
    let mut hat_x: i32 = 0;
    let mut hat_y: i32 = 0;
    let mut last_abs_x: Option<i32> = None;
    let mut last_abs_y: Option<i32> = None;
    let mut buf = [ZERO; 64];
    let mut notify_buf = [0u8; 256];

    loop {
        let mut pollfds: Vec<libc::pollfd> = Vec::with_capacity(files.len() + 1);
        if inotify_fd >= 0 {
            pollfds.push(libc::pollfd { fd: inotify_fd, events: libc::POLLIN, revents: 0 });
        }
        for (_, f) in &files {
            pollfds.push(libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }

        let ret = unsafe {
            libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, 2000)
        };

        let offset = usize::from(inotify_fd >= 0);

        if inotify_fd >= 0 && pollfds[0].revents & libc::POLLIN != 0 {
            unsafe {
                libc::read(inotify_fd, notify_buf.as_mut_ptr() as *mut libc::c_void, notify_buf.len());
            }
            scan_devices(&mut files);
        }

        if ret == 0 {
            files.retain(|(path, _)| path.exists());
            scan_devices(&mut files);
            continue;
        }
        if ret < 0 {
            continue;
        }

        let mut dead = Vec::new();
        for i in offset..pollfds.len() {
            let pf = &pollfds[i];
            if pf.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                dead.push(i - offset);
                continue;
            }
            if pf.revents & libc::POLLIN == 0 {
                continue;
            }

            let n = unsafe {
                libc::read(
                    files[i - offset].1.as_raw_fd(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    std::mem::size_of_val(&buf),
                )
            };
            if n <= 0 {
                continue;
            }

            let count = n as usize / 24;
            for ev in buf.iter().take(count) {
                if ev.typ == EV_ABS {
                    match ev.code {
                        ABS_HAT0X | ABS_HAT0Y => {
                            let visible = OVERLAY_VISIBLE.load(Ordering::Relaxed)
                                && HAS_FOCUS.load(Ordering::Relaxed);
                            if !visible {
                                continue;
                            }
                            match ev.code {
                                ABS_HAT0X => {
                                    let old = hat_x;
                                    hat_x = ev.value;
                                    if old == 0 && ev.value != 0 {
                                        let event = if ev.value < 0 { Event::NavLeft } else { Event::NavRight };
                                        crate::ui::push_event(event);
                                    }
                                }
                                ABS_HAT0Y => {
                                    let old = hat_y;
                                    hat_y = ev.value;
                                    if old == 0 && ev.value != 0 {
                                        let event = if ev.value < 0 { Event::NavUp } else { Event::NavDown };
                                        crate::ui::push_event(event);
                                    }
                                }
                                _ => {}
                            }
                        }
                        ABS_X | ABS_Y => {
                            if !OVERLAY_VISIBLE.load(Ordering::Relaxed) {
                                continue;
                            }
                            match ev.code {
                                ABS_X => {
                                    if let Some(prev) = last_abs_x {
                                        let dx = ev.value - prev;
                                        if dx != 0 {
                                            let (mx, my) = crate::ui::update_mouse(dx, 0);
                                            crate::ui::push_event(Event::MouseMove { x: mx, y: my });
                                        }
                                    }
                                    last_abs_x = Some(ev.value);
                                }
                                ABS_Y => {
                                    if let Some(prev) = last_abs_y {
                                        let dy = ev.value - prev;
                                        if dy != 0 {
                                            let (mx, my) = crate::ui::update_mouse(0, dy);
                                            crate::ui::push_event(Event::MouseMove { x: mx, y: my });
                                        }
                                    }
                                    last_abs_y = Some(ev.value);
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                if ev.typ == EV_REL {
                    if !OVERLAY_VISIBLE.load(Ordering::Relaxed) {
                        continue;
                    }
                    match ev.code {
                        REL_X => {
                            let (mx, my) = crate::ui::update_mouse(ev.value, 0);
                            crate::ui::push_event(Event::MouseMove { x: mx, y: my });
                        }
                        REL_Y => {
                            let (mx, my) = crate::ui::update_mouse(0, ev.value);
                            crate::ui::push_event(Event::MouseMove { x: mx, y: my });
                        }
                        _ => {}
                    }
                    continue;
                }
                if ev.typ != EV_KEY {
                    continue;
                }
                let pressed = ev.value != 0;
                let is_press = ev.value == 1;
                match ev.code {
                    KEY_LEFTSHIFT => left_shift = pressed,
                    KEY_RIGHTSHIFT => right_shift = pressed,
                    KEY_TAB => {
                        if is_press && (left_shift || right_shift) && HAS_FOCUS.load(Ordering::Relaxed) {
                            OVERLAY_VISIBLE.fetch_xor(true, Ordering::Relaxed);
                            let visible = OVERLAY_VISIBLE.load(Ordering::Relaxed);
                            eprintln!(
                                "ira-overlay: toggle visible={} focus={}",
                                visible, HAS_FOCUS.load(Ordering::Relaxed)
                            );
                        }
                    }
                    BTN_MODE => {
                        if is_press {
                            OVERLAY_VISIBLE.fetch_xor(true, Ordering::Relaxed);
                            let visible = OVERLAY_VISIBLE.load(Ordering::Relaxed);
                            eprintln!(
                                "ira-overlay: toggle visible={} focus={}",
                                visible, HAS_FOCUS.load(Ordering::Relaxed)
                            );
                        }
                    }
                    BTN_LEFT | BTN_RIGHT => {
                        if OVERLAY_VISIBLE.load(Ordering::Relaxed)
                        {
                            let (mx, my) = crate::ui::mouse_pos();
                            if is_press {
                                crate::ui::push_event(Event::MouseDown { x: mx, y: my });
                            } else {
                                crate::ui::push_event(Event::MouseUp { x: mx, y: my });
                            }
                        }
                    }
                    BTN_TOUCH => {
                        if !pressed {
                            last_abs_x = None;
                            last_abs_y = None;
                        }
                    }
                    _ => {
                        if ev.code == KEY_F12 && is_press {
                            crate::ui::capture::request_screenshot();
                        }
                        if ev.code == KEY_F11 && is_press {
                            crate::ui::capture::toggle_recording();
                        }
                        if OVERLAY_VISIBLE.load(Ordering::Relaxed)
                            && HAS_FOCUS.load(Ordering::Relaxed)
                            && is_press
                        {
                            let event = match ev.code {
                                103 => Some(Event::NavUp),
                                108 => Some(Event::NavDown),
                                105 => Some(Event::NavLeft),
                                106 => Some(Event::NavRight),
                                28 | 304 => Some(Event::Activate),
                                _ => None,
                            };
                            if let Some(e) = event {
                                crate::ui::push_event(e);
                            }
                        }
                    }
                }
            }
        }

        for i in dead.iter().rev() {
            files.remove(*i);
        }
    }
}
