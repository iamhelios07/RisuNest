use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often the counters reach the renderer. A publication places tens of
/// thousands of sources, and every report rewrites the job row.
const REPORT_INTERVAL: Duration = Duration::from_millis(250);

/// What a publication has already placed, against what it has found to place.
/// A large library is read, compressed and packed for a long time before the
/// first object is sealed, so the transfer journal holds no row yet and the
/// job would otherwise report nothing at all.
///
/// Every domain the publication carries plans itself here as it starts, so the
/// planned totals grow as work is discovered and never shrink.
pub(crate) struct PreparationProgress {
    placed_items: AtomicU64,
    placed_bytes: AtomicU64,
    planned_items: AtomicU64,
    planned_bytes: AtomicU64,
    reported: Mutex<Instant>,
    sink: Box<dyn Fn(Prepared) + Send + Sync>,
}

/// One reading of the counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Prepared {
    pub items: u64,
    pub total_items: u64,
    pub bytes: u64,
    pub total_bytes: u64,
}

impl PreparationProgress {
    pub(crate) fn new(sink: impl Fn(Prepared) + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            placed_items: AtomicU64::new(0),
            placed_bytes: AtomicU64::new(0),
            planned_items: AtomicU64::new(0),
            planned_bytes: AtomicU64::new(0),
            // Far enough back that the first placement reports.
            reported: Mutex::new(Instant::now() - REPORT_INTERVAL),
            sink: Box::new(sink),
        })
    }

    /// Nothing is reported. For the paths that package without a job to tell.
    pub(crate) fn silent() -> Arc<Self> {
        Self::new(|_| {})
    }

    /// What one domain has to place, added to the totals before it places any
    /// of it.
    pub(crate) fn plan(&self, items: u64, bytes: u64) {
        self.planned_items.fetch_add(items, Ordering::Relaxed);
        self.planned_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.report();
    }

    /// One source has reached its placement, whether it was packed here or
    /// reused from what the parent already carries.
    pub(crate) fn placed(&self, bytes: u64) {
        self.placed_items.fetch_add(1, Ordering::Relaxed);
        self.placed_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.report();
    }

    pub(crate) fn read(&self) -> Prepared {
        Prepared {
            items: self.placed_items.load(Ordering::Relaxed),
            total_items: self.planned_items.load(Ordering::Relaxed),
            bytes: self.placed_bytes.load(Ordering::Relaxed),
            total_bytes: self.planned_bytes.load(Ordering::Relaxed),
        }
    }

    fn report(&self) {
        let Ok(mut reported) = self.reported.lock() else {
            return;
        };
        let now = Instant::now();
        if now.duration_since(*reported) < REPORT_INTERVAL {
            return;
        }
        *reported = now;
        drop(reported);
        (self.sink)(self.read());
    }
}
