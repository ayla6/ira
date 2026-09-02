use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

pub(crate) extern "C" fn handle_signal(_: libc::c_int) {
    STOP_REQUESTED.store(true, Ordering::Relaxed);
}

pub(crate) fn install_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGINT,
            handle_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            handle_signal as *const () as libc::sighandler_t,
        );
    }
}
