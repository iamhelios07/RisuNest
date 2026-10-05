//! Counters the settings screen reads to show what server sync is doing.
//!
//! Every counter only grows, so a reader subtracts what it saw when its
//! attempt started. Lanes that can run at the same time never share counters.
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering::Relaxed};
use std::sync::{Arc, LazyLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Step {
    Idle = 0,
    /// Reading local changes and the objects they reference.
    Preparing = 1,
    /// Asking the server which objects it already holds.
    Checking = 2,
    Uploading = 3,
    /// Waiting for the server to accept a publication or a receipt.
    Confirming = 4,
    /// Reading change and state pages.
    Listing = 5,
    Downloading = 6,
    /// Writing received units into this device's staging area.
    Staging = 7,
}
impl Step {
    fn name(value: u8) -> &'static str {
        match value {
            1 => "preparing",
            2 => "checking",
            3 => "uploading",
            4 => "confirming",
            5 => "listing",
            6 => "downloading",
            7 => "staging",
            _ => "idle",
        }
    }
}

#[derive(Default)]
pub(crate) struct ProgressLane {
    active: AtomicU32,
    step: AtomicU8,
    /// Units read from the server, before their bodies are fetched.
    listed: AtomicU64,
    items_done: AtomicU64,
    items_total: AtomicU64,
    files_done: AtomicU64,
    files_total: AtomicU64,
    bytes_done: AtomicU64,
    /// Payload bytes planned where sizes are known before the transfer.
    bytes_total: AtomicU64,
    sent: AtomicU64,
    received: AtomicU64,
}
impl ProgressLane {
    pub(crate) fn step(&self, step: Step) {
        self.step.store(step as u8, Relaxed);
    }
    pub(crate) fn listed(&self, count: usize) {
        self.listed.fetch_add(count as u64, Relaxed);
    }
    pub(crate) fn plan_items(&self, count: usize) {
        self.items_total.fetch_add(count as u64, Relaxed);
    }
    pub(crate) fn items_done(&self, count: usize) {
        self.items_done.fetch_add(count as u64, Relaxed);
    }
    pub(crate) fn plan_files(&self, count: usize, bytes: u64) {
        self.files_total.fetch_add(count as u64, Relaxed);
        self.bytes_total.fetch_add(bytes, Relaxed);
    }
    pub(crate) fn file_done(&self, bytes: u64) {
        self.files_done.fetch_add(1, Relaxed);
        self.bytes_done.fetch_add(bytes, Relaxed);
    }
    pub(crate) fn wire(&self, sent: u64, received: u64) {
        self.sent.fetch_add(sent, Relaxed);
        self.received.fetch_add(received, Relaxed);
    }
    pub(crate) fn snapshot(&self, lane: &'static str) -> LaneSnapshot {
        let active = self.active.load(Relaxed) > 0;
        LaneSnapshot {
            lane,
            active,
            step: if active { Step::name(self.step.load(Relaxed)) } else { "idle" },
            listed: self.listed.load(Relaxed),
            items_done: self.items_done.load(Relaxed),
            items_total: self.items_total.load(Relaxed),
            files_done: self.files_done.load(Relaxed),
            files_total: self.files_total.load(Relaxed),
            bytes_done: self.bytes_done.load(Relaxed),
            bytes_total: self.bytes_total.load(Relaxed),
            sent_bytes: self.sent.load(Relaxed),
            received_bytes: self.received.load(Relaxed),
        }
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LaneSnapshot {
    pub lane: &'static str,
    pub active: bool,
    pub step: &'static str,
    pub listed: u64,
    pub items_done: u64,
    pub items_total: u64,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub sent_bytes: u64,
    pub received_bytes: u64,
}

#[derive(Default)]
pub(crate) struct Lanes {
    pub send: Arc<ProgressLane>,
    pub receive: Arc<ProgressLane>,
    pub hydrate: Arc<ProgressLane>,
    /// First binding: inspecting, staging and activating a target.
    pub binding: Arc<ProgressLane>,
    /// Asset storage changes: downloading every file or clearing local copies.
    pub assets: Arc<ProgressLane>,
}
impl Lanes {
    pub(crate) fn snapshot(&self) -> Vec<LaneSnapshot> {
        vec![
            self.send.snapshot("send"),
            self.receive.snapshot("receive"),
            self.hydrate.snapshot("hydrate"),
            self.binding.snapshot("binding"),
            self.assets.snapshot("assets"),
        ]
    }
}
pub(crate) static LANES: LazyLock<Lanes> = LazyLock::new(Lanes::default);

thread_local! {
    static CURRENT: RefCell<Option<Arc<ProgressLane>>> = const { RefCell::new(None) };
}

/// Runs an operation with `lane` active. Server clients created on this thread
/// meanwhile report to it, including the workers that borrow them.
pub(crate) fn within<T>(lane: &Arc<ProgressLane>, operation: impl FnOnce() -> T) -> T {
    struct Scope<'a> {
        lane: &'a ProgressLane,
        previous: Option<Arc<ProgressLane>>,
    }
    impl Drop for Scope<'_> {
        fn drop(&mut self) {
            if self.lane.active.fetch_sub(1, Relaxed) == 1 {
                self.lane.step(Step::Idle);
            }
            let previous = self.previous.take();
            CURRENT.with(|current| *current.borrow_mut() = previous);
        }
    }
    lane.active.fetch_add(1, Relaxed);
    let previous = CURRENT.with(|current| current.borrow_mut().replace(lane.clone()));
    let _scope = Scope { lane, previous };
    operation()
}

/// The lane of the operation running on this thread, if any.
pub(crate) fn current() -> Option<Arc<ProgressLane>> {
    CURRENT.with(|current| current.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lane_is_active_only_inside_its_scope_and_keeps_its_counts() {
        let lane = Arc::new(ProgressLane::default());
        assert!(current().is_none());
        within(&lane, || {
            let reporting = current().expect("lane");
            reporting.step(Step::Uploading);
            reporting.plan_files(2, 300);
            reporting.file_done(100);
            reporting.wire(120, 40);
            let inside = lane.snapshot("send");
            assert!(inside.active);
            assert_eq!(inside.step, "uploading");
            assert_eq!((inside.files_done, inside.files_total, inside.bytes_done, inside.bytes_total), (1, 2, 100, 300));
        });
        assert!(current().is_none());
        let after = lane.snapshot("send");
        assert!(!after.active);
        assert_eq!(after.step, "idle");
        assert_eq!((after.sent_bytes, after.received_bytes), (120, 40));
    }

    #[test]
    fn a_nested_scope_restores_the_outer_lane() {
        let outer = Arc::new(ProgressLane::default());
        let inner = Arc::new(ProgressLane::default());
        within(&outer, || {
            outer.step(Step::Listing);
            within(&inner, || assert!(Arc::ptr_eq(&current().unwrap(), &inner)));
            assert!(Arc::ptr_eq(&current().unwrap(), &outer));
            assert_eq!(outer.snapshot("receive").step, "listing");
        });
    }

    #[test]
    fn the_same_lane_stays_active_until_its_last_scope_ends() {
        let lane = Arc::new(ProgressLane::default());
        within(&lane, || {
            lane.step(Step::Confirming);
            within(&lane, || ());
            assert!(lane.snapshot("send").active);
            assert_eq!(lane.snapshot("send").step, "confirming");
        });
        assert!(!lane.snapshot("send").active);
    }
}
