use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct ParsedEvent {
    mask: u32,
    name: String,
}

pub(crate) struct ProfileMonitor {
    fd: libc::c_int,
    watch: libc::c_int,
    path: PathBuf,
    parent: PathBuf,
    filename: String,
    reload: bool,
    last_watch_error: Instant,
    last_read_error: Instant,
}

impl ProfileMonitor {
    pub(crate) fn new(path: PathBuf) -> Self {
        let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let parent = if parent.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            parent
        };
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK) };
        let fd = if fd < 0 {
            eprintln!(
                "ira-input: inotify_init1 failed: {}",
                std::io::Error::last_os_error()
            );
            -1
        } else {
            fd
        };
        let mut monitor = Self {
            fd,
            watch: -1,
            path,
            parent,
            filename,
            reload: false,
            last_watch_error: Instant::now(),
            last_read_error: Instant::now(),
        };
        monitor.ensure_watch();
        monitor.reload = false;
        monitor
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The inotify descriptor to park in an external poll set, so profile
    /// writes wake the session loop immediately. `None` when inotify could
    /// not be created and the caller must fall back to periodic drains.
    pub(crate) fn fd(&self) -> Option<libc::c_int> {
        (self.fd >= 0).then_some(self.fd)
    }

    pub(crate) fn changed(&mut self) -> bool {
        if self.fd < 0 {
            return false;
        }
        self.drain_events();
        self.ensure_watch();
        std::mem::take(&mut self.reload)
    }

    fn drain_events(&mut self) {
        let mut buffer = [0u8; 4096];
        loop {
            let read = unsafe { libc::read(self.fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if read < 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EAGAIN) {
                    break;
                }
                if self.last_read_error.elapsed() >= Duration::from_secs(5) {
                    eprintln!("ira-input: inotify read failed: {error}");
                    self.last_read_error = Instant::now();
                }
                break;
            }
            if read == 0 {
                break;
            }
            for event in parse_inotify_buffer(&buffer[..read as usize]) {
                self.handle_parsed(&event);
            }
        }
    }

    fn handle_parsed(&mut self, event: &ParsedEvent) {
        let mask = event.mask;
        if mask & libc::IN_IGNORED != 0 {
            self.watch = -1;
            return;
        }
        if mask & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF) != 0 {
            self.watch = -1;
            return;
        }
        if mask & libc::IN_Q_OVERFLOW != 0 {
            eprintln!("ira-input: inotify queue overflow; forcing profile reload");
            self.reload = true;
            return;
        }
        if event.name == self.filename {
            self.reload = true;
        }
    }

    fn ensure_watch(&mut self) {
        if self.watch != -1 || self.fd < 0 {
            return;
        }
        let Some(c_path) = std::ffi::CString::new(self.parent.as_os_str().as_bytes()).ok() else {
            return;
        };
        let mask = libc::IN_CLOSE_WRITE
            | libc::IN_MOVED_TO
            | libc::IN_CREATE
            | libc::IN_DELETE
            | libc::IN_DELETE_SELF
            | libc::IN_MOVE_SELF;
        let watch = unsafe { libc::inotify_add_watch(self.fd, c_path.as_ptr(), mask) };
        if watch < 0 {
            if self.last_watch_error.elapsed() >= Duration::from_secs(5) {
                eprintln!(
                    "ira-input: inotify_add_watch failed for {}: {}",
                    self.parent.display(),
                    std::io::Error::last_os_error()
                );
                self.last_watch_error = Instant::now();
            }
            return;
        }
        self.watch = watch;
        self.reload = true;
    }
}

impl Drop for ProfileMonitor {
    fn drop(&mut self) {
        if self.fd >= 0 {
            unsafe {
                libc::close(self.fd);
            }
        }
    }
}

fn parse_inotify_buffer(buffer: &[u8]) -> Vec<ParsedEvent> {
    let header = std::mem::size_of::<libc::inotify_event>();
    let mut events = Vec::new();
    let mut offset = 0;
    while offset + header <= buffer.len() {
        let event = unsafe {
            std::ptr::read_unaligned(buffer.as_ptr().add(offset) as *const libc::inotify_event)
        };
        let event_size = header + event.len as usize;
        if offset + event_size > buffer.len() {
            break;
        }
        let name = if event.len > 0 {
            let bytes = &buffer[offset + header..offset + event_size];
            String::from_utf8_lossy(bytes)
                .trim_end_matches('\0')
                .to_string()
        } else {
            String::new()
        };
        events.push(ParsedEvent {
            mask: event.mask,
            name,
        });
        offset += event_size;
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn make_event(mask: u32, name: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1i32.to_ne_bytes());
        bytes.extend_from_slice(&mask.to_ne_bytes());
        bytes.extend_from_slice(&0u32.to_ne_bytes());
        bytes.extend_from_slice(&(name.len() as u32).to_ne_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes
    }

    #[test]
    fn test_parse_inotify_buffer_extracts_names() {
        let mut buffer = make_event(libc::IN_CLOSE_WRITE, "profile.json");
        buffer.extend(make_event(libc::IN_MOVED_TO, "other.txt"));
        let events = parse_inotify_buffer(&buffer);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].mask, libc::IN_CLOSE_WRITE);
        assert_eq!(events[0].name, "profile.json");
        assert_eq!(events[1].mask, libc::IN_MOVED_TO);
        assert_eq!(events[1].name, "other.txt");
    }

    #[test]
    fn test_parse_inotify_buffer_handles_self_events() {
        let buffer = make_event(libc::IN_MOVE_SELF, "");
        let events = parse_inotify_buffer(&buffer);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].mask, libc::IN_MOVE_SELF);
        assert_eq!(events[0].name, "");
    }

    #[test]
    fn test_parse_inotify_buffer_ignores_truncated_tail() {
        let mut buffer = make_event(libc::IN_CREATE, "profile.json");
        buffer.extend_from_slice(&[0xff; 3]);
        let events = parse_inotify_buffer(&buffer);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "profile.json");
    }

    fn temp_profile_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ira-input-test-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait_for_change(monitor: &mut ProfileMonitor, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if monitor.changed() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn test_profile_monitor_detects_in_place_write() {
        let dir = temp_profile_dir("inplace");
        let path = dir.join("profile.json");
        std::fs::write(&path, "one").unwrap();
        let mut monitor = ProfileMonitor::new(path.clone());
        thread::sleep(Duration::from_millis(20));
        assert!(!monitor.changed());
        std::fs::write(&path, "two").unwrap();
        assert!(wait_for_change(&mut monitor, Duration::from_secs(2)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_profile_monitor_detects_atomic_rename() {
        let dir = temp_profile_dir("rename");
        let path = dir.join("profile.json");
        std::fs::write(&path, "one").unwrap();
        let mut monitor = ProfileMonitor::new(path.clone());
        thread::sleep(Duration::from_millis(20));
        assert!(!monitor.changed());
        let tmp = dir.join(".profile.json.tmp");
        std::fs::write(&tmp, "two").unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        assert!(wait_for_change(&mut monitor, Duration::from_secs(2)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_profile_monitor_detects_delete_and_recreate() {
        let dir = temp_profile_dir("recreate");
        let path = dir.join("profile.json");
        std::fs::write(&path, "one").unwrap();
        let mut monitor = ProfileMonitor::new(path.clone());
        thread::sleep(Duration::from_millis(20));
        assert!(!monitor.changed());
        std::fs::remove_file(&path).unwrap();
        assert!(wait_for_change(&mut monitor, Duration::from_secs(2)));
        let tmp = dir.join(".profile.json.tmp");
        std::fs::write(&tmp, "two").unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        assert!(wait_for_change(&mut monitor, Duration::from_secs(2)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_profile_monitor_ignores_unrelated_files() {
        let dir = temp_profile_dir("unrelated");
        let path = dir.join("profile.json");
        std::fs::write(&path, "one").unwrap();
        let mut monitor = ProfileMonitor::new(path.clone());
        thread::sleep(Duration::from_millis(20));
        assert!(!monitor.changed());
        std::fs::write(dir.join("other.txt"), "x").unwrap();
        thread::sleep(Duration::from_millis(50));
        assert!(!monitor.changed());
        std::fs::remove_dir_all(&dir).ok();
    }
}