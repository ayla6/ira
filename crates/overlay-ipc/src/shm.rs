//! Shared memory helpers — create, open, and map the IPC region.
//!
//! The Ira app calls [`create_shm`] before launching a game, writes game data
//! and achievements into the mapped region, then sets `IRA_OVERLAY_SHM` to
//! the returned path. The overlay calls [`open_shm`] (or [`MappedShm::open`])
//! to read the data.

use std::ffi::CString;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::protocol::{
    achievements_offset, header_offset, notifications_offset, AchievementEntry, NotificationEntry,
    ShmHeader, SHM_MAGIC, SHM_SIZE, SHM_VERSION,
};

/// Returns the shared memory name for a given game db_id.
/// This is a POSIX shared memory name (starts with `/`, no other slashes).
/// The kernel places the file at `/dev/shm/<name>`.
pub fn shm_path(db_id: i64) -> String {
    format!("/ira_overlay_{db_id}")
}

/// Returns the full filesystem path for a given shm name.
/// Useful for checking file existence or debugging.
pub fn shm_file_path(name: &str) -> String {
    format!("/dev/shm/{name}")
}

/// RAII wrapper around a mapped shared memory region.
/// Unmaps on drop. The raw pointer is valid until drop.
pub struct MappedShm {
    ptr: *mut u8,
    size: usize,
    owned: bool,
}

unsafe impl Send for MappedShm {}

impl MappedShm {
    /// Creates a new shared memory file, truncates it to `SHM_SIZE`, and maps it.
    /// Returns the mapped region with the header zeroed.
    /// Any stale file from a previous run is unlinked first (the old mapping
    /// stays alive until the game exits — this creates a fresh inode).
    pub fn create(db_id: i64) -> Result<Self, String> {
        let path = shm_path(db_id);
        let c_path = CString::new(path.as_str()).map_err(|e| e.to_string())?;

        unsafe { libc::shm_unlink(c_path.as_ptr()) };

        let fd = unsafe { libc::shm_open(c_path.as_ptr(), libc::O_RDWR | libc::O_CREAT, 0o600) };
        if fd < 0 {
            return Err(format!("shm_open failed: {}", std::io::Error::last_os_error()));
        }
        Self::map_fd(fd, true)
    }

    /// Opens an existing shared memory file and maps it read-only.
    pub fn open(path: &str) -> Result<Self, String> {
        let c_path = CString::new(path).map_err(|e| e.to_string())?;
        let fd = unsafe { libc::shm_open(c_path.as_ptr(), libc::O_RDONLY, 0o600) };
        if fd < 0 {
            return Err(format!("shm_open failed: {}", std::io::Error::last_os_error()));
        }
        Self::map_fd(fd, false)
    }

    /// Opens an existing shared memory file and maps it read-write.
    /// Used by the Ira app to push notifications after the initial creation.
    pub fn open_rw(path: &str) -> Result<Self, String> {
        let c_path = CString::new(path).map_err(|e| e.to_string())?;
        let fd = unsafe { libc::shm_open(c_path.as_ptr(), libc::O_RDWR, 0o600) };
        if fd < 0 {
            return Err(format!("shm_open failed: {}", std::io::Error::last_os_error()));
        }
        Self::map_fd(fd, true)
    }

    fn map_fd(fd: RawFd, writable: bool) -> Result<Self, String> {
        if writable
            && unsafe { libc::ftruncate(fd, SHM_SIZE as libc::off_t) } < 0
        {
            unsafe { libc::close(fd) };
            return Err(format!("ftruncate failed: {}", std::io::Error::last_os_error()));
        }

        let prot = if writable {
            libc::PROT_READ | libc::PROT_WRITE
        } else {
            libc::PROT_READ
        };

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                SHM_SIZE,
                prot,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        unsafe { libc::close(fd) };

        if ptr == libc::MAP_FAILED {
            return Err(format!("mmap failed: {}", std::io::Error::last_os_error()));
        }

        Ok(Self {
            ptr: ptr as *mut u8,
            size: SHM_SIZE,
            owned: writable,
        })
    }

    /// Returns a pointer to the `ShmHeader` at the start of the region.
    pub fn header(&self) -> &ShmHeader {
        unsafe { &*(self.ptr.add(header_offset()) as *const ShmHeader) }
    }

    /// Returns a mutable reference to the header (panics if read-only).
    pub fn header_mut(&mut self) -> &mut ShmHeader {
        assert!(self.owned, "cannot write to read-only mapping");
        unsafe { &mut *(self.ptr.add(header_offset()) as *mut ShmHeader) }
    }

    /// Returns the achievement array slice.
    pub fn achievements(&self) -> &[AchievementEntry] {
        let base = unsafe { self.ptr.add(achievements_offset()) as *const AchievementEntry };
        unsafe { std::slice::from_raw_parts(base, crate::protocol::MAX_ACHIEVEMENTS) }
    }

    /// Returns a mutable achievement array slice (panics if read-only).
    pub fn achievements_mut(&mut self) -> &mut [AchievementEntry] {
        assert!(self.owned, "cannot write to read-only mapping");
        let base = unsafe { self.ptr.add(achievements_offset()) as *mut AchievementEntry };
        unsafe { std::slice::from_raw_parts_mut(base, crate::protocol::MAX_ACHIEVEMENTS) }
    }

    /// Returns the notification ring buffer slice.
    pub fn notifications(&self) -> &[NotificationEntry] {
        let base = unsafe { self.ptr.add(notifications_offset()) as *const NotificationEntry };
        unsafe { std::slice::from_raw_parts(base, crate::protocol::MAX_NOTIFICATIONS) }
    }

    /// Returns a mutable notification ring buffer slice (panics if read-only).
    pub fn notifications_mut(&mut self) -> &mut [NotificationEntry] {
        assert!(self.owned, "cannot write to read-only mapping");
        let base = unsafe { self.ptr.add(notifications_offset()) as *mut NotificationEntry };
        unsafe { std::slice::from_raw_parts_mut(base, crate::protocol::MAX_NOTIFICATIONS) }
    }

    /// Initializes the header with magic/version and zeros the rest.
    /// Call this right after `create()`.
    pub fn init_header(&mut self, db_id: i64) {
        let hdr = self.header_mut();
        hdr.magic = SHM_MAGIC;
        hdr.version = SHM_VERSION;
        hdr.game_db_id = db_id;
        hdr.notification_write_index = AtomicU32::new(0);
    }

    /// Pushes a notification to the ring buffer (producer side).
    /// Call from the Ira app when an achievement is unlocked.
    pub fn push_notification(&mut self, entry: NotificationEntry) {
        let hdr_ptr = unsafe { self.ptr.add(header_offset()) as *const ShmHeader };
        let idx = unsafe { (*hdr_ptr).notification_write_index.load(Ordering::SeqCst) };
        let slot = idx as usize % crate::protocol::MAX_NOTIFICATIONS;
        let notif_ptr = unsafe { self.ptr.add(notifications_offset()) as *mut NotificationEntry };
        unsafe { *notif_ptr.add(slot) = entry; }
        // SeqCst fence ensures the write above is visible before the index update.
        std::sync::atomic::fence(Ordering::SeqCst);
        unsafe { (*hdr_ptr).notification_write_index.store(idx + 1, Ordering::SeqCst); }
    }
}

impl Drop for MappedShm {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.size) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{NotificationType, SHM_MAGIC, SHM_VERSION};

    #[test]
    fn test_create_open_roundtrip() {
        let db_id = 999_999_999;
        let mut shm = MappedShm::create(db_id).unwrap();
        shm.init_header(db_id);
        shm.header_mut().total_achievements = 5;
        shm.header_mut().unlocked_achievements = 2;

        let path = shm_path(db_id);
        let reader = MappedShm::open(&path).unwrap();
        assert_eq!(reader.header().magic, SHM_MAGIC);
        assert_eq!(reader.header().version, SHM_VERSION);
        assert_eq!(reader.header().game_db_id, db_id);
        assert_eq!(reader.header().total_achievements, 5);
        assert_eq!(reader.header().unlocked_achievements, 2);

        // Clean up.
        let c_path = CString::new(path).unwrap();
        unsafe { libc::shm_unlink(c_path.as_ptr()) };
    }

    #[test]
    fn test_push_notification() {
        let db_id = 888_888_888;
        let mut shm = MappedShm::create(db_id).unwrap();
        shm.init_header(db_id);

        shm.push_notification(NotificationEntry {
            notification_type: NotificationType::AchievementUnlocked as u32,
            achievement_index: 3,
            timestamp: 12345,
        });

        let reader = MappedShm::open(&shm_path(db_id)).unwrap();
        let write_idx = reader.header().notification_write_index.load(Ordering::SeqCst);
        assert_eq!(write_idx, 1);
        assert_eq!(reader.notifications()[0].achievement_index, 3);

        let c_path = CString::new(shm_path(db_id)).unwrap();
        unsafe { libc::shm_unlink(c_path.as_ptr()) };
    }

    #[test]
    fn test_write_read_achievements() {
        let db_id = 777_777_777;
        let mut shm = MappedShm::create(db_id).unwrap();
        shm.init_header(db_id);

        let ach = &mut shm.achievements_mut()[0];
        ach.earned = 1;
        ach.earned_time = 99;
        write_bytes(&mut ach.display_name, b"Test Achievement");

        let reader = MappedShm::open(&shm_path(db_id)).unwrap();
        assert_eq!(reader.achievements()[0].earned, 1);
        assert_eq!(reader.achievements()[0].earned_time, 99);
        assert_eq!(
            cstr_to_string(&reader.achievements()[0].display_name),
            "Test Achievement"
        );

        let c_path = CString::new(shm_path(db_id)).unwrap();
        unsafe { libc::shm_unlink(c_path.as_ptr()) };
    }

    fn write_bytes(dst: &mut [u8], src: &[u8]) {
        let len = src.len().min(dst.len() - 1);
        dst[..len].copy_from_slice(&src[..len]);
        dst[len] = 0;
    }

    fn cstr_to_string(bytes: &[u8]) -> String {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).to_string()
    }
}
