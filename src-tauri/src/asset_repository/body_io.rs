//! Current-thread CAS read-I/O observations for verification entries only.
//!
//! Reset/run/take must execute on the same thread. This sample covers CAS read
//! opens and reads inside payload_cas, not every filesystem read in the app.
//! Returned raw File handles and paths make subsequent coverage incomplete.
//! Worker-thread samples must be taken there and explicitly returned to the
//! caller; this module does not propagate or aggregate worker scopes.
#![cfg(test)]

use std::{cell::RefCell, collections::BTreeMap, io, thread::ThreadId};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct BodyWork {
    pub open_attempts: u64,
    pub opens: u64,
    pub failed_opens: u64,
    pub unknown_open_results: u64,
    pub read_operations: u64,
    pub read_bytes: u64,
    pub incomplete_reads: u64,
    pub escaped_handles: u64,
    pub escaped_paths: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BodyIo {
    pub thread: ThreadId,
    // managed: canonical assets/objects bodies. owned: CAS staging/import files.
    pub domains: BTreeMap<&'static str, BodyWork>,
    // API requests/queries, not OS metadata syscall counts.
    pub stat_requests: u64,
    pub batch_stat_requests: u64,
    pub presence_queries: u64,
}

impl Default for BodyIo {
    fn default() -> Self {
        Self {
            thread: std::thread::current().id(), domains: BTreeMap::new(),
            stat_requests: 0, batch_stat_requests: 0, presence_queries: 0,
        }
    }
}

impl BodyIo {
    pub(crate) fn complete(&self) -> bool {
        self.domains.values().all(|work| work.unknown_open_results == 0
            && work.incomplete_reads == 0 && work.escaped_handles == 0 && work.escaped_paths == 0)
    }
}

thread_local! { static WORK: RefCell<BodyIo> = RefCell::new(BodyIo::default()); }

pub(crate) fn reset_body_io() {
    WORK.with(|work| *work.borrow_mut() = BodyIo::default());
}

pub(crate) fn take_body_io() -> BodyIo {
    WORK.with(|work| std::mem::take(&mut *work.borrow_mut()))
}

fn domain(domain: &'static str, record: impl FnOnce(&mut BodyWork)) {
    WORK.with(|work| record(work.borrow_mut().domains.entry(domain).or_default()));
}

pub(super) fn stat_request() { WORK.with(|work| work.borrow_mut().stat_requests += 1); }
pub(super) fn batch_stat_request() { WORK.with(|work| work.borrow_mut().batch_stat_requests += 1); }
pub(super) fn presence_query() { WORK.with(|work| work.borrow_mut().presence_queries += 1); }

pub(super) fn open_result<T>(kind: &'static str, result: &io::Result<T>) {
    domain(kind, |work| {
        work.open_attempts += 1;
        if result.is_ok() { work.opens += 1; } else { work.failed_opens += 1; }
    });
}

pub(super) fn escaped_handle(kind: &'static str) {
    domain(kind, |work| work.escaped_handles += 1);
}

pub(super) fn escaped_path(kind: &'static str) {
    domain(kind, |work| work.escaped_paths += 1);
}

// fs::read performs one read-open and returns all successfully read bytes.
// Its error does not expose whether opening or a later read failed.
pub(super) fn read_file_result(result: &io::Result<Vec<u8>>) {
    domain("managed", |work| {
        work.open_attempts += 1;
        work.read_operations += 1;
        match result {
            Ok(bytes) => { work.opens += 1; work.read_bytes += bytes.len() as u64; }
            Err(_) => { work.unknown_open_results += 1; work.incomplete_reads += 1; }
        }
    });
}

// Operations count observed Read/read_exact/fs::read boundaries, not syscalls.
pub(super) fn read_result(kind: &'static str, result: &io::Result<usize>) {
    domain(kind, |work| {
        work.read_operations += 1;
        match result {
            Ok(bytes) => work.read_bytes += *bytes as u64,
            Err(_) => work.incomplete_reads += 1,
        }
    });
}

pub(super) fn read_exact_result(kind: &'static str, result: &io::Result<()>, bytes: usize) {
    domain(kind, |work| {
        work.read_operations += 1;
        if result.is_ok() { work.read_bytes += bytes as u64; } else { work.incomplete_reads += 1; }
    });
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delegated_file_read_error_preserves_unknown_outcome() {
        let directory = tempfile::tempdir().unwrap();
        reset_body_io();
        let result = std::fs::read(directory.path().join("missing"));
        assert!(result.is_err());
        read_file_result(&result);
        let work = take_body_io();
        assert_eq!(work.domains["managed"], BodyWork {
            open_attempts: 1, read_operations: 1, unknown_open_results: 1, incomplete_reads: 1,
            ..Default::default()
        });
        assert!(!work.complete());
    }
}
