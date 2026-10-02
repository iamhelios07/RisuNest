//! Scoped CAS read-I/O observations for verification entries only.
//!
//! Reset/run/take must execute on the same thread. This sample covers CAS read
//! opens and reads inside payload_cas, not every filesystem read in the app.
//! Returned raw File handles and paths make subsequent coverage incomplete.
//! Register purposes from actual object provenance after reset. Domains are
//! totals; object rows are a classification of those totals, not extra work.
//! Tracked readers retain their owner scope. Crossing threads or reset/take
//! boundaries invalidates that scope. Other worker I/O is not propagated.
#![cfg(test)]

use std::{cell::RefCell, collections::{BTreeMap, BTreeSet}, fs::File, io::{self, Read, Seek, SeekFrom}, sync::{Arc, Mutex}, thread::ThreadId};

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
    pub outstanding_readers: u64,
}

impl BodyWork {
    fn add(&mut self, other: &Self) {
        self.open_attempts += other.open_attempts; self.opens += other.opens;
        self.failed_opens += other.failed_opens; self.unknown_open_results += other.unknown_open_results;
        self.read_operations += other.read_operations; self.read_bytes += other.read_bytes;
        self.incomplete_reads += other.incomplete_reads; self.escaped_handles += other.escaped_handles;
        self.escaped_paths += other.escaped_paths; self.outstanding_readers += other.outstanding_readers;
    }
    fn observed_completely(&self) -> bool {
        self.unknown_open_results == 0 && self.incomplete_reads == 0
            && self.escaped_handles == 0 && self.escaped_paths == 0 && self.outstanding_readers == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum BodyPurpose { Control, Asset }

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ObjectBodyWork {
    pub purposes: BTreeSet<BodyPurpose>,
    pub work: BodyWork,
    pub owned_work: BodyWork,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BodyIo {
    pub thread: ThreadId,
    // managed: canonical assets/objects bodies. owned: CAS staging/import files.
    pub domains: BTreeMap<&'static str, BodyWork>,
    pub objects: BTreeMap<String, ObjectBodyWork>,
    pub unattributed_managed: BodyWork,
    pub unattributed_owned: BodyWork,
    pub pending_owned_identities: u64,
    pub scope_violations: u64,
    // API requests/queries, not OS metadata syscall counts.
    pub stat_requests: u64,
    pub batch_stat_requests: u64,
    pub presence_queries: u64,
}

impl Default for BodyIo {
    fn default() -> Self {
        Self {
            thread: std::thread::current().id(), domains: BTreeMap::new(), objects: BTreeMap::new(),
            unattributed_managed: BodyWork::default(), unattributed_owned: BodyWork::default(),
            pending_owned_identities: 0, scope_violations: 0,
            stat_requests: 0, batch_stat_requests: 0, presence_queries: 0,
        }
    }
}

impl BodyIo {
    pub(crate) fn complete(&self) -> bool {
        self.scope_violations == 0 && self.pending_owned_identities == 0 && self.domains.values().all(BodyWork::observed_completely)
            && self.unknown_work() == BodyWork::default()
    }
    pub(crate) fn asset_work(&self) -> BodyWork {
        self.classified_work(|row| row.purposes.contains(&BodyPurpose::Asset))
    }
    pub(crate) fn control_work(&self) -> BodyWork {
        self.classified_work(|row| row.purposes.contains(&BodyPurpose::Control) && !row.purposes.contains(&BodyPurpose::Asset))
    }
    pub(crate) fn unknown_work(&self) -> BodyWork {
        let mut work = self.classified_work(|row| row.purposes.is_empty());
        work.add(&self.unattributed_managed); work.add(&self.unattributed_owned); work
    }
    fn classified_work(&self, include: impl Fn(&ObjectBodyWork) -> bool) -> BodyWork {
        let mut work = BodyWork::default();
        for row in self.objects.values().filter(|row| include(row)) { work.add(&row.work); work.add(&row.owned_work); }
        work
    }
}

thread_local! {
    static WORK: RefCell<Arc<Mutex<BodyIo>>> = RefCell::new(Arc::new(Mutex::new(BodyIo::default())));
    static OBJECT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PENDING_OWNED: RefCell<Vec<(Arc<Mutex<BodyIo>>, Arc<Mutex<BodyWork>>)>> = const { RefCell::new(Vec::new()) };
}

fn current_scope() -> Arc<Mutex<BodyIo>> { WORK.with(|work| work.borrow().clone()) }

pub(crate) fn reset_body_io() {
    WORK.with(|work| *work.borrow_mut() = Arc::new(Mutex::new(BodyIo::default())));
}

pub(crate) fn take_body_io() -> BodyIo {
    let scope = current_scope();
    let work = scope.lock().unwrap().clone();
    reset_body_io(); work
}

pub(crate) fn register_object_purpose(hash: &str, purpose: BodyPurpose) {
    current_scope().lock().unwrap().objects.entry(hash.to_owned()).or_default().purposes.insert(purpose);
}

pub(super) struct ObjectScope;
pub(super) fn object_scope(hash: &str) -> ObjectScope {
    OBJECT.with(|object| object.borrow_mut().push(hash.to_owned())); ObjectScope
}
impl Drop for ObjectScope {
    fn drop(&mut self) { OBJECT.with(|object| { object.borrow_mut().pop(); }); }
}

fn record(work: &mut BodyIo, kind: &'static str, hash: Option<&str>, update: impl Fn(&mut BodyWork)) {
    update(work.domains.entry(kind).or_default());
    if kind == "managed" || kind == "owned" {
        match hash {
            Some(hash) => {
                let row = work.objects.entry(hash.to_owned()).or_default();
                update(if kind == "managed" { &mut row.work } else { &mut row.owned_work });
            }
            None => update(if kind == "managed" { &mut work.unattributed_managed } else { &mut work.unattributed_owned }),
        }
    }
}
fn domain(kind: &'static str, update: impl Fn(&mut BodyWork)) {
    if kind == "owned" {
        if let Some((scope, pending)) = PENDING_OWNED.with(|pending| pending.borrow().last().cloned()) {
            let current = current_scope();
            if !Arc::ptr_eq(&scope, &current) {
                scope.lock().unwrap().scope_violations += 1;
                current.lock().unwrap().scope_violations += 1;
            }
            update(scope.lock().unwrap().domains.entry(kind).or_default());
            update(&mut pending.lock().unwrap());
            return;
        }
    }
    let hash = OBJECT.with(|object| object.borrow().last().cloned());
    record(&mut current_scope().lock().unwrap(), kind, hash.as_deref(), update);
}

// Staging opens precede final stream validation. Associate their observed work
// only after that validation supplies the actual immutable content identity.
pub(super) struct PendingOwnedIdentity {
    scope: Arc<Mutex<BodyIo>>,
    work: Arc<Mutex<BodyWork>>,
    hash: Option<String>,
    owner: ThreadId,
}
impl PendingOwnedIdentity {
    pub(super) fn new() -> Self {
        let scope = current_scope();
        scope.lock().unwrap().pending_owned_identities += 1;
        let work = Arc::new(Mutex::new(BodyWork::default()));
        PENDING_OWNED.with(|pending| pending.borrow_mut().push((scope.clone(), work.clone())));
        Self { scope, work, hash: None, owner: std::thread::current().id() }
    }
    pub(super) fn verified(&mut self, hash: &str) { self.hash = Some(hash.to_owned()); }
}
impl Drop for PendingOwnedIdentity {
    fn drop(&mut self) {
        PENDING_OWNED.with(|pending| { pending.borrow_mut().pop(); });
        let current = current_scope();
        let changed = self.owner != std::thread::current().id() || !Arc::ptr_eq(&self.scope, &current);
        let mut scope = self.scope.lock().unwrap();
        scope.pending_owned_identities -= 1;
        if changed { scope.scope_violations += 1; }
        let work = self.work.lock().unwrap();
        match &self.hash {
            Some(hash) => scope.objects.entry(hash.clone()).or_default().owned_work.add(&work),
            None => scope.unattributed_owned.add(&work),
        }
        drop(scope);
        if changed && !Arc::ptr_eq(&self.scope, &current) { current.lock().unwrap().scope_violations += 1; }
    }
}

pub(super) struct OwnedIdentity {
    scope: Arc<Mutex<BodyIo>>,
    hash: String,
    owner: ThreadId,
}
impl OwnedIdentity {
    pub(super) fn new(hash: &str) -> Self {
        let scope = current_scope();
        record(&mut scope.lock().unwrap(), "owned", Some(hash), |work| work.outstanding_readers += 1);
        Self { scope, hash: hash.to_owned(), owner: std::thread::current().id() }
    }
    pub(super) fn check_scope(&self) {
        let current = current_scope();
        if self.owner != std::thread::current().id() || !Arc::ptr_eq(&self.scope, &current) {
            self.scope.lock().unwrap().scope_violations += 1;
            if !Arc::ptr_eq(&self.scope, &current) { current.lock().unwrap().scope_violations += 1; }
        }
    }
}
impl Drop for OwnedIdentity {
    fn drop(&mut self) {
        self.check_scope();
        record(&mut self.scope.lock().unwrap(), "owned", Some(&self.hash), |work| work.outstanding_readers -= 1);
    }
}

pub(super) fn stat_request() { current_scope().lock().unwrap().stat_requests += 1; }
pub(super) fn batch_stat_request() { current_scope().lock().unwrap().batch_stat_requests += 1; }
pub(super) fn presence_query() { current_scope().lock().unwrap().presence_queries += 1; }

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

pub(crate) struct TrackedBodyFile {
    file: File,
    hash: String,
    scope: Arc<Mutex<BodyIo>>,
    owner: ThreadId,
}

impl TrackedBodyFile {
    pub(super) fn new(file: File, hash: &str) -> Self {
        let scope = current_scope();
        record(&mut scope.lock().unwrap(), "managed", Some(hash), |work| work.outstanding_readers += 1);
        Self { file, hash: hash.to_owned(), scope, owner: std::thread::current().id() }
    }
    fn check_scope(&self) {
        let current = current_scope();
        if self.owner != std::thread::current().id() || !Arc::ptr_eq(&self.scope, &current) {
            self.scope.lock().unwrap().scope_violations += 1;
            if !Arc::ptr_eq(&self.scope, &current) { current.lock().unwrap().scope_violations += 1; }
        }
    }
    pub(crate) fn len(&self) -> io::Result<u64> {
        self.check_scope(); self.file.metadata().map(|metadata| metadata.len())
    }
}

impl Read for TrackedBodyFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.check_scope();
        let result = self.file.read(bytes);
        record(&mut self.scope.lock().unwrap(), "managed", Some(&self.hash), |work| {
            work.read_operations += 1;
            match &result {
                Ok(bytes) => work.read_bytes += *bytes as u64,
                Err(_) => work.incomplete_reads += 1,
            }
        });
        result
    }
}

impl Seek for TrackedBodyFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.check_scope(); self.file.seek(position)
    }
}

impl Drop for TrackedBodyFile {
    fn drop(&mut self) {
        self.check_scope();
        record(&mut self.scope.lock().unwrap(), "managed", Some(&self.hash), |work| work.outstanding_readers -= 1);
    }
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

    #[test]
    fn actual_tracked_read_error_records_failure_without_guessing_partial_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let hash = "a".repeat(64);
        reset_body_io(); register_object_purpose(&hash, BodyPurpose::Control);
        let scope = object_scope(&hash);
        let file = File::create(directory.path().join("write-only")).unwrap();
        open_result("managed", &Ok::<_, io::Error>(&file));
        let mut reader = TrackedBodyFile::new(file, &hash);
        drop(scope);
        assert!(reader.read(&mut [0; 4]).is_err()); drop(reader);
        let work = take_body_io();
        assert!(!work.complete());
        assert_eq!(work.control_work(), BodyWork { open_attempts: 1, opens: 1, read_operations: 1, incomplete_reads: 1, ..Default::default() });
        assert_eq!(work.unknown_work(), BodyWork::default());
    }

    #[test]
    fn unattributed_managed_work_never_disappears_into_a_control_default() {
        reset_body_io(); register_object_purpose(&"a".repeat(64), BodyPurpose::Control);
        open_result("managed", &Err::<File, _>(io::Error::other("synthetic failed open")));
        let work = take_body_io();
        assert!(!work.complete());
        assert_eq!(work.unknown_work(), BodyWork { open_attempts: 1, failed_opens: 1, ..Default::default() });
        assert_eq!(work.control_work(), BodyWork::default());
    }
}
