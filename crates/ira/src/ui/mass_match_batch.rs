//! The mass matcher's shared batch plumbing: one sequential worker thread
//! per source computes matches off the GTK loop, and results land on the
//! list rows through the UI loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One queued batch candidate: the game to match plus which list row its
/// result belongs to.
pub(super) struct BatchItem {
    pub(super) name: String,
    pub(super) db_id: i64,
    pub(super) row_idx: usize,
}

/// A finished candidate handed from the worker thread back to the UI loop.
/// `T` is the worker's match payload: a `(id, title)` pair for Steam/SGDB/RA
/// and a full `ScrapedGame` for ScreenScraper. The last hit of a pass —
/// sent once the queue drains or is cancelled — carries `row_idx` of
/// [`BATCH_FINISHED`] and nothing else.
pub(super) struct BatchHit<T> {
    pub(super) row_idx: usize,
    pub(super) db_id: i64,
    pub(super) name: String,
    pub(super) matched: Option<T>,
}

/// The `row_idx` of a pass's completion sentinel.
pub(super) const BATCH_FINISHED: usize = usize::MAX;

/// The action boxes of one match-list row: `main` takes the Steam or SGDB
/// result, `ra` (retro games on RA-covered consoles only) the
/// RetroAchievements one, and `ss` (console games ScreenScraper covers)
/// the ScreenScraper one, so the sources never overwrite each other.
#[derive(Clone)]
pub(super) struct RowActions {
    pub(super) row: gtk4::ListBoxRow,
    pub(super) main: gtk4::Box,
    pub(super) ra: Option<gtk4::Box>,
    pub(super) ss: Option<gtk4::Box>,
    /// Console games whose metadata can be garnished from Steam by an
    /// exact title match — additive to the SS/RA boxes, never a match.
    pub(super) steam: Option<gtk4::Box>,
}

/// Shared shape of every batch pass: one sequential worker thread computes
/// matches over `queue`, and each result is applied on the main loop as
/// it arrives. `worker` runs off-thread and must not touch GTK; it waits
/// `pace_ms` before every request but the first, so a rate-limited
/// service sees one request per pace, never a burst. It stands down
/// between items when `cancel` says so. `on_result` runs on the main
/// loop, with a [`BATCH_FINISHED`] hit closing the pass.
pub(super) fn run_batch<T: Send + 'static>(
    queue: Vec<BatchItem>,
    pace_ms: u64,
    cancel: Option<Arc<AtomicBool>>,
    worker: impl Fn(&BatchItem) -> Option<T> + Send + 'static,
    on_result: impl Fn(BatchHit<T>) + 'static,
) {
    let (tx, rx) = super::helpers::ui_channel();
    std::thread::spawn(move || {
        for (index, item) in queue.iter().enumerate() {
            if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                break;
            }
            if index > 0 && pace_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(pace_ms));
            }
            // A panicking worker must not skip the sentinel below — the
            // pass's completion bookkeeping depends on it.
            let matched = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                worker(item)
            }))
            .unwrap_or_else(|panic| {
                let message = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "worker panicked".to_string());
                eprintln!("batch worker panicked on '{}': {message}", item.name);
                None
            });
            let _ = tx.try_send(BatchHit {
                row_idx: item.row_idx,
                db_id: item.db_id,
                name: item.name.clone(),
                matched,
            });
        }
        let _ = tx.try_send(BatchHit {
            row_idx: BATCH_FINISHED,
            db_id: 0,
            name: String::new(),
            matched: None,
        });
    });

    // The sentinel hit closes the pass; afterwards the senders are gone
    // and the watch loop ends on its own.
    super::helpers::watch_channel(rx, move |hit| {
        on_result(hit);
        glib::ControlFlow::Continue
    });
}
