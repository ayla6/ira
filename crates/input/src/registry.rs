use std::ffi::CString;
use std::mem::size_of;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{discover_gamepads, DeviceInfo};

const INPUT_DIR: &str = "/dev/input";
const DEBOUNCE: Duration = Duration::from_millis(200);
const POLL_LIMIT: Duration = Duration::from_millis(100);
const WATCH_RETRY: Duration = Duration::from_secs(1);
const EVENT_MASK: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_ATTRIB
    | libc::IN_CLOSE_WRITE
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF
    | libc::IN_IGNORED
    | libc::IN_Q_OVERFLOW;

pub struct ControllerRegistry {
    snapshot: Mutex<Vec<DeviceInfo>>,
    generation: AtomicU64,
    stop: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ControllerRegistry {
    pub fn snapshot_only() -> Arc<Self> {
        Arc::new(Self {
            snapshot: Mutex::new(discover_gamepads()),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            worker: Mutex::new(None),
        })
    }

    pub fn start() -> Result<Arc<Self>, String> {
        let inotify = Inotify::new(Path::new(INPUT_DIR))?;
        let initial = discover_gamepads();
        let registry = Arc::new(Self {
            snapshot: Mutex::new(initial),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            worker: Mutex::new(None),
        });
        let worker_registry = Arc::downgrade(&registry);
        let worker = thread::Builder::new()
            .name("ira-controller-registry".to_string())
            .spawn(move || worker_loop(worker_registry, inotify))
            .map_err(|error| format!("failed to start controller registry: {error}"))?;
        *registry
            .worker
            .lock()
            .expect("registry worker mutex poisoned") = Some(worker);
        Ok(registry)
    }

    pub fn snapshot(&self) -> Vec<DeviceInfo> {
        self.snapshot
            .lock()
            .expect("registry snapshot mutex poisoned")
            .clone()
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
}

impl Drop for ControllerRegistry {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let worker = self
            .worker
            .get_mut()
            .expect("registry worker mutex poisoned")
            .take();
        if let Some(worker) = worker {
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

fn worker_loop(registry: Weak<ControllerRegistry>, mut inotify: Inotify) {
    let mut pending: Option<Instant> = None;
    let mut buffer = [0_u8; 4096];
    while registry
        .upgrade()
        .is_some_and(|registry| !registry.stop.load(Ordering::Acquire))
    {
        let retry = inotify.next_watch_retry();
        let timeout = pending
            .map(|deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(POLL_LIMIT)
            })
            .or_else(|| {
                retry.map(|deadline| {
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(POLL_LIMIT)
                })
            })
            .unwrap_or(POLL_LIMIT);
        let _ = inotify.wait(timeout);
        let events = has_relevant_event(&inotify, &mut buffer);
        if events.watch_lost {
            inotify.mark_watch_lost();
            pending = Some(Instant::now());
        } else if events.overflow {
            pending = Some(Instant::now());
        } else if events.relevant {
            pending = Some(Instant::now() + DEBOUNCE);
        }
        if inotify.try_readd_watch(Instant::now()) {
            pending = Some(Instant::now());
        }
        if pending.is_some_and(|deadline| deadline <= Instant::now()) {
            if let Some(registry) = registry.upgrade() {
                refresh(&registry);
            }
            pending = None;
        }
    }
}

fn refresh(registry: &ControllerRegistry) {
    let discovered = discover_gamepads();
    replace_snapshot(&registry.snapshot, &registry.generation, discovered);
}

fn replace_snapshot(
    snapshot: &Mutex<Vec<DeviceInfo>>,
    generation: &AtomicU64,
    discovered: Vec<DeviceInfo>,
) {
    let mut snapshot = snapshot.lock().expect("registry snapshot mutex poisoned");
    if *snapshot != discovered {
        *snapshot = discovered;
        generation.fetch_add(1, Ordering::AcqRel);
    }
}

fn has_relevant_event(inotify: &Inotify, buffer: &mut [u8]) -> EventState {
    let count = match read_events(inotify.fd, buffer) {
        Ok(count) => count,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            return EventState::default();
        }
        Err(_) => {
            return EventState {
                relevant: true,
                ..EventState::default()
            };
        }
    };
    parse_events(&buffer[..count])
}

#[derive(Clone, Copy, Default)]
struct EventState {
    relevant: bool,
    watch_lost: bool,
    overflow: bool,
}

fn parse_events(mut bytes: &[u8]) -> EventState {
    let mut state = EventState::default();
    while bytes.len() >= size_of::<libc::inotify_event>() {
        let (header, rest) = bytes.split_at(size_of::<libc::inotify_event>());
        let mask = u32::from_ne_bytes(header[4..8].try_into().unwrap());
        let name_len = u32::from_ne_bytes(header[12..16].try_into().unwrap()) as usize;
        let record_len = size_of::<libc::inotify_event>().saturating_add(name_len);
        if record_len > bytes.len() || record_len < size_of::<libc::inotify_event>() {
            break;
        }
        let name = &rest[..name_len];
        if mask & libc::IN_Q_OVERFLOW != 0 {
            state.relevant = true;
            state.overflow = true;
        }
        if mask & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF | libc::IN_IGNORED) != 0 {
            state.relevant = true;
            state.watch_lost = true;
        }
        if mask & EVENT_MASK != 0 && event_name_is_input(name) {
            state.relevant = true;
        }
        bytes = &bytes[record_len..];
    }
    state
}

fn event_name_is_input(name: &[u8]) -> bool {
    let name = name.split(|byte| *byte == 0).next().unwrap_or_default();
    name.starts_with(b"event")
}

fn read_events(fd: RawFd, buffer: &mut [u8]) -> std::io::Result<usize> {
    let result = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
    if result < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(result as usize)
    }
}

struct Inotify {
    fd: RawFd,
    path: PathBuf,
    watch_descriptor: libc::c_int,
    next_watch_retry: Option<Instant>,
}

impl Inotify {
    fn new(path: &Path) -> Result<Self, String> {
        let display_path = path.display().to_string();
        let watch_path = path.to_path_buf();
        let path_bytes = path.as_os_str().as_encoded_bytes();
        let path =
            CString::new(path_bytes).map_err(|_| format!("invalid inotify path {display_path}"))?;
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return Err(format!(
                "failed to initialize inotify: {}",
                std::io::Error::last_os_error()
            ));
        }
        let wd = unsafe { libc::inotify_add_watch(fd, path.as_ptr(), EVENT_MASK) };
        if wd < 0 {
            let error = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(format!("failed to watch {display_path}: {error}"));
        }
        Ok(Self {
            fd,
            path: watch_path,
            watch_descriptor: wd,
            next_watch_retry: None,
        })
    }

    fn next_watch_retry(&self) -> Option<Instant> {
        self.next_watch_retry
    }

    fn mark_watch_lost(&mut self) {
        if self.watch_descriptor < 0 {
            return;
        }
        self.watch_descriptor = -1;
        if self.next_watch_retry.is_none() {
            self.next_watch_retry = Some(Instant::now());
        }
    }

    fn try_readd_watch(&mut self, now: Instant) -> bool {
        let Some(retry) = self.next_watch_retry else {
            return false;
        };
        if retry > now {
            return false;
        }
        let path = match CString::new(self.path.as_os_str().as_encoded_bytes()) {
            Ok(path) => path,
            Err(_) => return false,
        };
        let wd = unsafe { libc::inotify_add_watch(self.fd, path.as_ptr(), EVENT_MASK) };
        if wd < 0 {
            self.next_watch_retry = Some(now + WATCH_RETRY);
            return false;
        }
        self.watch_descriptor = wd;
        self.next_watch_retry = None;
        true
    }

    fn wait(&self, timeout: Duration) -> std::io::Result<()> {
        let mut pollfd = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
        let result = unsafe { libc::poll(&mut pollfd, 1, millis) };
        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for Inotify {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

#[cfg(test)]
mod tests {
    use super::{event_name_is_input, parse_events, replace_snapshot, EVENT_MASK};
    use crate::DeviceInfo;
    use std::mem::size_of;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn event(name: &[u8], mask: u32) -> Vec<u8> {
        event_with_cookie(name, mask, 0)
    }

    fn event_with_cookie(name: &[u8], mask: u32, cookie: u32) -> Vec<u8> {
        let mut bytes = vec![0; size_of::<libc::inotify_event>() + name.len() + 1];
        bytes[0..4].copy_from_slice(&1_u32.to_ne_bytes());
        bytes[4..8].copy_from_slice(&mask.to_ne_bytes());
        bytes[8..12].copy_from_slice(&cookie.to_ne_bytes());
        bytes[12..16].copy_from_slice(&((name.len() + 1) as u32).to_ne_bytes());
        bytes[size_of::<libc::inotify_event>()..][..name.len()].copy_from_slice(name);
        bytes
    }

    #[test]
    fn test_unrelated_filenames_are_ignored() {
        assert!(!parse_events(&event(b"mouse0", libc::IN_CREATE)).relevant);
        assert!(parse_events(&event(b"event3", libc::IN_CREATE)).relevant);
        assert!(event_name_is_input(b"event3\0"));
        assert!(!event_name_is_input(b"js0\0"));
    }

    #[test]
    fn test_parent_loss_and_overflow_trigger_discovery() {
        assert!(parse_events(&event(b"", libc::IN_IGNORED)).watch_lost);
        assert!(parse_events(&event(b"", libc::IN_Q_OVERFLOW)).relevant);
        assert_eq!(EVENT_MASK & libc::IN_CLOSE_WRITE, libc::IN_CLOSE_WRITE);
    }

    #[test]
    fn test_cookie_does_not_masquerade_as_mask() {
        assert!(!parse_events(&event_with_cookie(b"event3", 0, libc::IN_CREATE,)).relevant);
        assert!(parse_events(&event_with_cookie(b"event3", libc::IN_CREATE, 0,)).relevant);
    }

    #[test]
    fn test_generation_changes_only_when_snapshot_changes() {
        let snapshot = Mutex::new(Vec::new());
        let generation = AtomicU64::new(0);
        let device = DeviceInfo {
            path: PathBuf::from("/dev/input/event0"),
            name: "Test controller".to_string(),
            vendor: 1,
            product: 2,
            version: 3,
            has_evdev_gyro: false,
            supported_buttons: Vec::new(),
        };
        replace_snapshot(&snapshot, &generation, vec![device.clone()]);
        replace_snapshot(&snapshot, &generation, vec![device]);
        assert_eq!(generation.load(std::sync::atomic::Ordering::Acquire), 1);
    }

    #[test]
    fn test_registry_shutdown_is_bounded() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("ira-input-registry-{suffix}"));
        std::fs::create_dir(&directory).unwrap();
        let inotify = super::Inotify::new(&directory).unwrap();
        let registry = Arc::new(super::ControllerRegistry {
            snapshot: Mutex::new(Vec::new()),
            generation: AtomicU64::new(0),
            stop: std::sync::atomic::AtomicBool::new(false),
            worker: Mutex::new(None),
        });
        let worker = std::thread::spawn({
            let weak = Arc::downgrade(&registry);
            move || super::worker_loop(weak, inotify)
        });
        *registry.worker.lock().unwrap() = Some(worker);
        let started = std::time::Instant::now();
        drop(registry);
        assert!(started.elapsed() < Duration::from_secs(1));
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn test_snapshot_only_has_no_worker() {
        let registry = super::ControllerRegistry::snapshot_only();

        assert_eq!(registry.generation(), 0);
        assert!(registry.worker.lock().unwrap().is_none());
    }

    #[test]
    fn test_watch_loss_can_be_readded() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("ira-input-watch-{suffix}"));
        std::fs::create_dir(&directory).unwrap();
        let mut inotify = super::Inotify::new(&directory).unwrap();
        inotify.mark_watch_lost();
        assert!(inotify.try_readd_watch(std::time::Instant::now()));
        assert!(inotify.next_watch_retry().is_none());
        drop(inotify);
        std::fs::remove_dir(directory).unwrap();
    }
}
