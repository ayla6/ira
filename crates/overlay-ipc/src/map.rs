//! Shared POSIX SHM open + mmap helper: `CString::new` → `shm_open` →
//! `ftruncate` → `prot` select → `mmap` → `close` → `MAP_FAILED` check.

use std::ffi::CString;
use std::os::unix::io::RawFd;

/// `/dev/shm/<name>` filesystem path for debugging.
pub(crate) fn shm_file_path(name: &str) -> String {
    format!("/dev/shm/{name}")
}

pub(crate) fn open_fd(path: &str, flags: i32, what: &str) -> Result<RawFd, String> {
    let c_path = CString::new(path).map_err(|e| e.to_string())?;
    let fd = unsafe { libc::shm_open(c_path.as_ptr(), flags, 0o600) };
    if fd < 0 {
        return Err(format!(
            "{what} shm_open failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(fd)
}

pub(crate) fn map_fd(fd: RawFd, size: usize, writable: bool, what: &str) -> Result<*mut u8, String> {
    if writable && unsafe { libc::ftruncate(fd, size as libc::off_t) } < 0 {
        unsafe { libc::close(fd) };
        return Err(format!(
            "{what} ftruncate failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let prot = if writable {
        libc::PROT_READ | libc::PROT_WRITE
    } else {
        libc::PROT_READ
    };
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            prot,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    unsafe { libc::close(fd) };
    if ptr == libc::MAP_FAILED {
        return Err(format!(
            "{what} mmap failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(ptr as *mut u8)
}
