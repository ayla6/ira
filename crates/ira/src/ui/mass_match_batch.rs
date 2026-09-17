//! The mass matcher's shared batch plumbing: one sequential worker thread
//! per source computes matches off the GTK loop, and results land on the
//! list rows through the UI loop.

use std::cell::Cell;
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
    pub(super) main: gtk4::Box,
    pub(super) ra: Option<gtk4::Box>,
    pub(super) ss: Option<gtk4::Box>,
}

/// Shared shape of every batch pass: one sequential worker thread computes
/// matches over `queue`, and results are applied on the main loop every
/// `interval_ms` until the queue drains. `worker` runs off-thread and must
/// not touch GTK; it waits `pace_ms` before every request but the first,
/// so a rate-limited service sees one request per pace, never a burst. It
/// stands down between items when `cancel` says so. `on_result` runs on
/// the main loop, with a [`BATCH_FINISHED`] hit closing the pass.
pub(super) fn run_batch<T: Send + 'static>(
    queue: Vec<BatchItem>,
    interval_ms: u64,
    pace_ms: u64,
    cancel: Option<Arc<AtomicBool>>,
    worker: impl Fn(&BatchItem) -> Option<T> + Send + 'static,
    on_result: impl Fn(BatchHit<T>) + 'static,
) {
    let total = queue.len();
    let (tx, rx) = std::sync::mpsc::channel::<BatchHit<T>>();
    std::thread::spawn(move || {
        for (index, item) in queue.iter().enumerate() {
            if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                break;
            }
            if index > 0 && pace_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(pace_ms));
            }
            let matched = worker(item);
            let _ = tx.send(BatchHit {
                row_idx: item.row_idx,
                db_id: item.db_id,
                name: item.name.clone(),
                matched,
            });
        }
        let _ = tx.send(BatchHit {
            row_idx: BATCH_FINISHED,
            db_id: 0,
            name: String::new(),
            matched: None,
        });
    });

    let rx = std::cell::RefCell::new(rx);
    // The sentinel closes the pass, so it takes a slot of its own.
    let remaining = Cell::new(total + 1);
    glib::timeout_add_local(std::time::Duration::from_millis(interval_ms), move || {
        if let Ok(hit) = rx.borrow_mut().try_recv() {
            on_result(hit);
            let left = remaining.get();
            if left <= 1 {
                return glib::ControlFlow::Break;
            }
            remaining.set(left - 1);
        }
        glib::ControlFlow::Continue
    });
}
