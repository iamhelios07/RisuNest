#[allow(dead_code)]
pub(crate) mod shared_wire;
#[allow(dead_code)]
#[path = "../../../../crates/small-object-store/src/lib.rs"]
pub(crate) mod small_object_store;

use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
    task::{Context as TaskContext, Poll},
};
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

pub(crate) const MAX_ROLES: usize = 131_072;
const MAX_ROWS: usize = 262_144;
const MAX_RANGES: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Role {
    pub hash: String,
    pub purposes: BTreeSet<String>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Work {
    pub open_attempts: u64,
    pub opens: u64,
    pub failed_opens: u64,
    pub read_calls: u64,
    pub read_bytes: u64,
    pub failed_reads: u64,
    pub unknown_read_results: u64,
    pub outstanding_readers: u64,
    pub hash_calls: u64,
    pub hash_bytes: u64,
    pub hash_finalizations: u64,
    pub failed_hashes: u64,
}
impl Work {
    fn add(&mut self, value: &Self) -> bool {
        let mut overflow = false;
        macro_rules! add { ($($field:ident),*) => {$({
            self.$field = self.$field.checked_add(value.$field).unwrap_or_else(|| { overflow=true; u64::MAX });
        })*}; }
        add!(
            open_attempts,
            opens,
            failed_opens,
            read_calls,
            read_bytes,
            failed_reads,
            unknown_read_results,
            outstanding_readers,
            hash_calls,
            hash_bytes,
            hash_finalizations,
            failed_hashes
        );
        overflow
    }
    fn complete(&self) -> bool {
        self.failed_opens == 0
            && self.failed_reads == 0
            && self.unknown_read_results == 0
            && self.outstanding_readers == 0
            && self.failed_hashes == 0
            && self.hash_calls == self.hash_finalizations
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Range {
    pub start: u64,
    pub bytes: u64,
    pub calls: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectWork {
    pub hash: String,
    pub purposes: BTreeSet<String>,
    pub placement: String,
    pub flow: String,
    pub work: Work,
    pub read_ranges: Vec<Range>,
    pub selected_ranges: Vec<Range>,
    pub hash_domains: BTreeMap<String, Work>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub scope_id: String,
    pub generation: u64,
    pub phase: String,
    pub role_count: usize,
    pub complete: bool,
    pub objects: Vec<ObjectWork>,
    pub placements: BTreeMap<String, Work>,
    pub purposes: BTreeMap<String, Work>,
    pub flows: BTreeMap<String, Work>,
    pub hash_domains: BTreeMap<String, Work>,
    pub total: Work,
    pub unknown_work: Work,
    pub violations: BTreeMap<String, u64>,
    pub pending_workers: u64,
    pub pending_requests: u64,
    pub pending_body_work: PendingBodyWork,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingBodyWork {
    pub upload_jobs: u64,
    pub download_jobs: u64,
    pub upload_sessions: u64,
}
impl PendingBodyWork {
    pub(crate) fn settled(&self) -> bool {
        self.upload_jobs == 0 && self.download_jobs == 0 && self.upload_sessions == 0
    }
}
struct Scope {
    root: PathBuf,
    id: String,
    generation: u64,
    phase: String,
    roles: BTreeMap<String, BTreeSet<String>>,
    rows: BTreeMap<(String, String, String), ObjectWork>,
    violations: BTreeMap<String, u64>,
    workers: u64,
    requests: u64,
    closed: bool,
    pending_body_work: PendingBodyWork,
}
#[derive(Clone)]
pub(crate) struct Context {
    scope: Arc<Mutex<Scope>>,
    flow: String,
}
#[derive(Default)]
struct Registry {
    active: Option<Context>,
    generation: u64,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}
std::thread_local! { static CONTEXTS: RefCell<Vec<Context>> = const { RefCell::new(Vec::new()) }; }
tokio::task_local! { static REQUEST_CONTEXT: Option<Context>; }
fn current() -> Option<Context> {
    CONTEXTS.with(|contexts| contexts.borrow().last().cloned())
}
fn active() -> Option<Context> {
    registry().lock().unwrap().active.clone()
}
fn root_context(root: &Path) -> Option<Context> {
    let context = current().or_else(active)?;
    let root = crate::resolve_data_root(root).ok()?;
    let matches = context.scope.lock().unwrap().root == root;
    matches.then_some(context)
}
fn violation(scope: &mut Scope, name: &str) {
    let count = scope.violations.entry(name.into()).or_default();
    *count = count.saturating_add(1);
}
impl Context {
    fn update(
        &self,
        hash: &str,
        placement: &str,
        value: Work,
        domain: Option<&str>,
        range: Option<Range>,
    ) {
        let mut scope = self.scope.lock().unwrap();
        if scope.closed {
            violation(&mut scope, "late-work");
        }
        let purposes = scope.roles.get(hash).cloned().unwrap_or_default();
        if purposes.is_empty() {
            violation(&mut scope, "unknown-object");
        }
        let key = (hash.into(), placement.into(), self.flow.clone());
        if !scope.rows.contains_key(&key) && scope.rows.len() >= MAX_ROWS {
            violation(&mut scope, "row-limit");
            return;
        }
        let row = scope.rows.entry(key).or_insert_with(|| ObjectWork {
            hash: hash.into(),
            purposes,
            placement: placement.into(),
            flow: self.flow.clone(),
            work: Work::default(),
            read_ranges: Vec::new(),
            selected_ranges: Vec::new(),
            hash_domains: BTreeMap::new(),
        });
        let mut overflow = row.work.add(&value);
        if let Some(domain) = domain {
            overflow |= row
                .hash_domains
                .entry(domain.into())
                .or_default()
                .add(&value);
        }
        let mut range_limit = false;
        if let Some(range) = range {
            if let Some(last) = row
                .read_ranges
                .last_mut()
                .filter(|last| last.start.checked_add(last.bytes) == Some(range.start))
            {
                last.bytes = last.bytes.checked_add(range.bytes).unwrap_or_else(|| {
                    overflow = true;
                    u64::MAX
                });
                last.calls = last.calls.checked_add(range.calls).unwrap_or_else(|| {
                    overflow = true;
                    u64::MAX
                });
            } else if row.read_ranges.len() < MAX_RANGES {
                row.read_ranges.push(range);
            } else {
                range_limit = true;
            }
        }
        if overflow {
            violation(&mut scope, "counter-overflow");
        }
        if range_limit {
            violation(&mut scope, "range-limit");
        }
    }
    fn reader(&self, hash: &str, placement: &str, delta: bool) {
        let mut scope = self.scope.lock().unwrap();
        if scope.closed {
            violation(&mut scope, "late-reader");
        }
        let key = (hash.into(), placement.into(), self.flow.clone());
        if let Some(row) = scope.rows.get_mut(&key) {
            if delta {
                row.work.outstanding_readers = row.work.outstanding_readers.saturating_add(1);
                if row.work.outstanding_readers == u64::MAX {
                    violation(&mut scope, "reader-overflow");
                }
            } else if row.work.outstanding_readers > 0 {
                row.work.outstanding_readers -= 1;
            } else {
                violation(&mut scope, "reader-underflow");
            }
        } else {
            violation(&mut scope, "unregistered-reader");
        }
    }
}
pub(crate) struct Operation {
    pushed: bool,
}
impl Drop for Operation {
    fn drop(&mut self) {
        if self.pushed {
            CONTEXTS.with(|contexts| {
                contexts.borrow_mut().pop();
            });
        }
    }
}
pub(crate) fn operation(root: &Path) -> Operation {
    let context = root_context(root);
    let pushed = context.is_some();
    if let Some(context) = context {
        CONTEXTS.with(|contexts| contexts.borrow_mut().push(context));
    }
    Operation { pushed }
}
pub(crate) fn ingress(root: &Path) -> Operation {
    let context = root_context(root).map(|mut context| {
        context.flow = "ingress".into();
        context
    });
    let pushed = context.is_some();
    if let Some(context) = context {
        CONTEXTS.with(|contexts| contexts.borrow_mut().push(context));
    }
    Operation { pushed }
}
pub(crate) struct Worker {
    context: Option<Context>,
}
pub(crate) fn worker() -> Worker {
    let context = REQUEST_CONTEXT
        .try_with(Clone::clone)
        .ok()
        .flatten()
        .or_else(current)
        .or_else(active);
    if let Some(context) = &context {
        context.scope.lock().unwrap().workers += 1;
    }
    Worker { context }
}
impl Worker {
    pub(crate) fn enter(&self) -> Operation {
        let pushed = self.context.is_some();
        if let Some(context) = &self.context {
            CONTEXTS.with(|contexts| contexts.borrow_mut().push(context.clone()));
        }
        Operation { pushed }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(context) = &self.context {
            context.scope.lock().unwrap().workers -= 1;
        }
    }
}
pub(crate) struct Request {
    context: Option<Context>,
}
impl Drop for Request {
    fn drop(&mut self) {
        if let Some(context) = &self.context {
            context.scope.lock().unwrap().requests -= 1;
        }
    }
}
pub(crate) fn request() -> Request {
    let context = active();
    if let Some(context) = &context {
        context.scope.lock().unwrap().requests += 1;
    }
    Request { context }
}
pub(crate) async fn request_context<T>(f: impl std::future::Future<Output = T>) -> T {
    REQUEST_CONTEXT.scope(active(), f).await
}
pub(crate) fn note_violation(name: &str) {
    if let Some(context) = active() {
        violation(&mut context.scope.lock().unwrap(), name);
    }
}
pub(crate) fn body_work(value: PendingBodyWork) {
    if let Some(context) = active() {
        let mut scope = context.scope.lock().unwrap();
        if !value.settled() {
            violation(&mut scope, "pending-durable-body-work");
        }
        scope.pending_body_work = value;
    }
}
pub(crate) fn note_current_violation(name: &str) {
    if let Some(context) = current() {
        violation(&mut context.scope.lock().unwrap(), name);
    }
}
pub(crate) fn unsupported(root: &Path, name: &str) {
    if let Some(context) = root_context(root) {
        violation(&mut context.scope.lock().unwrap(), name);
    }
}

pub(crate) fn install() {
    small_object_store::source_test_observer::install(Arc::new(|event| {
        use small_object_store::source_test_observer::Event;
        let origin = match &event {
            Event::Extract { origin, .. } | Event::Hash { origin, .. } => origin,
        };
        let Some(origin) = origin else { return };
        let Some(root) = Path::new(origin).parent() else {
            return;
        };
        let Some(mut context) = root_context(root) else {
            return;
        };
        match event {
            Event::Extract {
                hash,
                flow,
                bytes,
                failed,
                ..
            } => {
                if context.flow != "ingress" {
                    context.flow = flow.into();
                }
                if bytes.is_some() || failed {
                    context.update(
                        &hash,
                        "inline",
                        Work {
                            open_attempts: 1,
                            opens: u64::from(bytes.is_some()),
                            failed_opens: u64::from(failed),
                            read_calls: 1,
                            read_bytes: bytes.unwrap_or(0) as u64,
                            unknown_read_results: u64::from(failed),
                            ..Default::default()
                        },
                        None,
                        None,
                    );
                }
            }
            Event::Hash {
                hash,
                domain,
                bytes,
                calls,
                failed,
                ..
            } => context.update(
                &hash,
                "inline",
                Work {
                    hash_calls: calls,
                    hash_bytes: bytes as u64,
                    hash_finalizations: calls,
                    failed_hashes: u64::from(failed),
                    ..Default::default()
                },
                Some(domain),
                None,
            ),
        }
    }));
    shared_wire::delta::source_test_observer::install(Arc::new(|event| {
        if let Some(context) = current() {
            context.update(
                &event.hash,
                "memory",
                Work {
                    hash_calls: event.calls,
                    hash_bytes: event.bytes,
                    hash_finalizations: event.finalizations,
                    failed_hashes: u64::from(event.failed),
                    ..Default::default()
                },
                Some(event.domain),
                None,
            );
        } else {
            note_violation("unattributed-hash");
        }
    }));
}
pub(crate) fn begin(
    root: &Path,
    id: String,
    phase: String,
    roles: Vec<Role>,
) -> Result<u64, &'static str> {
    if roles.len() > MAX_ROLES
        || id.is_empty()
        || id.len() > 128
        || phase.is_empty()
        || phase.len() > 64
    {
        return Err("scope-limit");
    }
    let root = crate::resolve_data_root(root).map_err(|_| "invalid-source-root")?;
    let mut registry = registry().lock().unwrap();
    if let Some(context) = &registry.active {
        let scope = context.scope.lock().unwrap();
        if !scope.closed
            || scope.workers != 0
            || scope.requests != 0
            || scope
                .rows
                .values()
                .any(|row| row.work.outstanding_readers != 0)
        {
            return Err("scope-not-settled");
        }
        if !scope.violations.is_empty() || scope.rows.values().any(|row| !row.work.complete()) {
            return Err("prior-scope-incomplete");
        }
    }
    let mut map = BTreeMap::new();
    for role in roles {
        risunest_sync_wire::validate_hash(&role.hash).map_err(|_| "invalid-role-hash")?;
        if role.purposes.is_empty()
            || role.purposes.iter().any(|p| p != "Control" && p != "Asset")
            || map.insert(role.hash, role.purposes).is_some()
        {
            return Err("invalid-role-registration");
        }
    }
    registry.generation = registry
        .generation
        .checked_add(1)
        .ok_or("generation-overflow")?;
    let generation = registry.generation;
    registry.active = Some(Context {
        scope: Arc::new(Mutex::new(Scope {
            root,
            id,
            generation,
            phase,
            roles: map,
            rows: BTreeMap::new(),
            violations: BTreeMap::new(),
            workers: 0,
            requests: 0,
            closed: false,
            pending_body_work: PendingBodyWork::default(),
        })),
        flow: "source".into(),
    });
    Ok(generation)
}
pub(crate) fn snapshot(close: bool) -> Option<Snapshot> {
    let context = active()?;
    let mut scope = context.scope.lock().unwrap();
    if close {
        scope.closed = true;
    }
    let mut result = Snapshot {
        scope_id: scope.id.clone(),
        generation: scope.generation,
        phase: scope.phase.clone(),
        role_count: scope.roles.len(),
        complete: false,
        objects: scope.rows.values().cloned().collect(),
        placements: BTreeMap::new(),
        purposes: BTreeMap::new(),
        flows: BTreeMap::new(),
        hash_domains: BTreeMap::new(),
        total: Work::default(),
        unknown_work: Work::default(),
        violations: scope.violations.clone(),
        pending_workers: scope.workers,
        pending_requests: scope.requests,
        pending_body_work: scope.pending_body_work.clone(),
    };
    let mut overflow = false;
    for row in &result.objects {
        overflow |= result.total.add(&row.work);
        overflow |= result
            .placements
            .entry(row.placement.clone())
            .or_default()
            .add(&row.work);
        overflow |= result
            .flows
            .entry(row.flow.clone())
            .or_default()
            .add(&row.work);
        let purpose = if row.purposes.contains("Asset") {
            "Asset"
        } else if row.purposes.contains("Control") {
            "Control"
        } else {
            "Unknown"
        };
        overflow |= result
            .purposes
            .entry(purpose.into())
            .or_default()
            .add(&row.work);
        if purpose == "Unknown" {
            overflow |= result.unknown_work.add(&row.work);
        }
        for (domain, work) in &row.hash_domains {
            overflow |= result
                .hash_domains
                .entry(domain.clone())
                .or_default()
                .add(work);
        }
    }
    if overflow {
        result.violations.insert("aggregate-overflow".into(), 1);
    }
    result.complete = result.violations.is_empty()
        && result.total.complete()
        && result.pending_workers == 0
        && result.pending_requests == 0
        && result.pending_body_work.settled()
        && result.unknown_work == Work::default();
    Some(result)
}
pub(crate) fn hashed(hash: &str, domain: &str, bytes: usize) {
    if let Some(context) = current() {
        context.update(
            hash,
            "memory",
            Work {
                hash_calls: 1,
                hash_bytes: bytes as u64,
                hash_finalizations: 1,
                ..Default::default()
            },
            Some(domain),
            None,
        );
    } else {
        note_violation("unattributed-hash");
    }
}
pub(crate) fn inline_selection(hash: &str, start: u64, bytes: u64) {
    let context = REQUEST_CONTEXT
        .try_with(Clone::clone)
        .ok()
        .flatten()
        .or_else(current)
        .or_else(active);
    if let Some(context) = context {
        let mut scope = context.scope.lock().unwrap();
        if let Some(row) = scope
            .rows
            .get_mut(&(hash.into(), "inline".into(), context.flow))
        {
            if row.selected_ranges.len() < MAX_RANGES {
                row.selected_ranges.push(Range {
                    start,
                    bytes,
                    calls: 1,
                });
            } else {
                violation(&mut scope, "range-limit");
            }
        } else {
            violation(&mut scope, "unobserved-inline-selection");
        }
    }
}
pub(crate) struct HashScope {
    context: Option<Context>,
    hash: String,
    domain: &'static str,
    complete: bool,
}
impl HashScope {
    pub(crate) fn new(hash: &str, domain: &'static str) -> Self {
        let scope = Self {
            context: current(),
            hash: hash.into(),
            domain,
            complete: false,
        };
        scope.record(Work {
            hash_calls: 1,
            ..Default::default()
        });
        scope
    }
    fn record(&self, work: Work) {
        if let Some(context) = &self.context {
            context.update(&self.hash, "memory", work, Some(self.domain), None);
        }
    }
    pub(crate) fn input(&mut self, bytes: usize) {
        self.record(Work {
            hash_bytes: bytes as u64,
            ..Default::default()
        });
    }
    pub(crate) fn finalized(&mut self) {
        self.record(Work {
            hash_finalizations: 1,
            ..Default::default()
        });
    }
    pub(crate) fn finish(&mut self) {
        self.complete = true;
    }
}
impl Drop for HashScope {
    fn drop(&mut self) {
        if !self.complete {
            self.record(Work {
                failed_hashes: 1,
                ..Default::default()
            });
        }
    }
}

struct Reader {
    context: Option<Context>,
    hash: String,
    placement: &'static str,
    position: u64,
    registered: bool,
}
impl Reader {
    fn new(root: &Path, hash: &str, placement: &'static str, result: &io::Result<File>) -> Self {
        let context = root_context(root);
        if let Some(context) = &context {
            context.update(
                hash,
                placement,
                Work {
                    open_attempts: 1,
                    opens: u64::from(result.is_ok()),
                    failed_opens: u64::from(result.is_err()),
                    ..Default::default()
                },
                None,
                None,
            );
            if result.is_ok() {
                context.reader(hash, placement, true);
            }
        }
        Self {
            context,
            hash: hash.into(),
            placement,
            position: 0,
            registered: result.is_ok(),
        }
    }
    fn read(&mut self, result: &io::Result<usize>) {
        if let Some(context) = &self.context {
            context.update(
                &self.hash,
                self.placement,
                Work {
                    read_calls: 1,
                    read_bytes: result.as_ref().copied().unwrap_or(0) as u64,
                    failed_reads: u64::from(result.is_err()),
                    unknown_read_results: u64::from(result.is_err()),
                    ..Default::default()
                },
                None,
                result.as_ref().ok().filter(|n| **n > 0).map(|n| Range {
                    start: self.position,
                    bytes: *n as u64,
                    calls: 1,
                }),
            );
        }
        if let Ok(bytes) = result {
            self.position = self.position.saturating_add(*bytes as u64);
        }
    }
    fn pending_drop(&self) {
        if let Some(context) = &self.context {
            context.update(
                &self.hash,
                self.placement,
                Work {
                    unknown_read_results: 1,
                    ..Default::default()
                },
                None,
                None,
            );
        }
    }
    fn selected(&self, start: u64, bytes: u64) {
        if let Some(context) = &self.context {
            let mut scope = context.scope.lock().unwrap();
            let key = (
                self.hash.clone(),
                self.placement.into(),
                context.flow.clone(),
            );
            if let Some(row) = scope.rows.get_mut(&key) {
                if row.selected_ranges.len() < MAX_RANGES {
                    row.selected_ranges.push(Range {
                        start,
                        bytes,
                        calls: 1,
                    });
                } else {
                    violation(&mut scope, "range-limit");
                }
            }
        }
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        if self.registered {
            if let Some(context) = &self.context {
                context.reader(&self.hash, self.placement, false);
            }
        }
    }
}
pub struct TrackedFile {
    file: File,
    reader: Reader,
}
impl TrackedFile {
    pub(crate) fn open(
        root: &Path,
        hash: &str,
        path: &Path,
        placement: &'static str,
    ) -> io::Result<Self> {
        let result = File::open(path);
        let reader = Reader::new(root, hash, placement, &result);
        if let Some(context) = &reader.context {
            if result.is_ok()
                && !std::fs::canonicalize(path)
                    .is_ok_and(|path| path.starts_with(&context.scope.lock().unwrap().root))
            {
                violation(&mut context.scope.lock().unwrap(), "source-path-escape");
            }
        }
        match result {
            Ok(file) => Ok(Self { file, reader }),
            Err(error) => Err(error),
        }
    }
    pub(crate) fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.file.metadata()
    }
    pub(crate) fn into_async(self) -> TrackedAsyncFile {
        TrackedAsyncFile {
            file: tokio::fs::File::from_std(self.file),
            reader: self.reader,
            pending: false,
            seeking: false,
        }
    }
}
impl Read for TrackedFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let result = self.file.read(bytes);
        self.reader.read(&result);
        result
    }
}
impl Seek for TrackedFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let result = self.file.seek(position);
        if let Ok(position) = result {
            self.reader.position = position;
        }
        result
    }
}
pub(crate) struct TrackedAsyncFile {
    file: tokio::fs::File,
    reader: Reader,
    pending: bool,
    seeking: bool,
}
impl TrackedAsyncFile {
    pub(crate) fn selected(&self, start: u64, bytes: u64) {
        self.reader.selected(start, bytes);
    }
}
impl AsyncRead for TrackedAsyncFile {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        bytes: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = bytes.filled().len();
        let result = Pin::new(&mut self.file).poll_read(cx, bytes);
        match &result {
            Poll::Pending => self.pending = true,
            Poll::Ready(result) => {
                self.pending = false;
                let read = result
                    .as_ref()
                    .map(|()| bytes.filled().len() - before)
                    .map_err(|error| io::Error::new(error.kind(), "source read failed"));
                self.reader.read(&read);
            }
        }
        result
    }
}
impl AsyncSeek for TrackedAsyncFile {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
        let result = Pin::new(&mut self.file).start_seek(position);
        if result.is_ok() {
            self.seeking = true;
        }
        result
    }
    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<u64>> {
        let result = Pin::new(&mut self.file).poll_complete(cx);
        if let Poll::Ready(result) = &result {
            self.seeking = false;
            match result {
                Ok(position) => self.reader.position = *position,
                Err(_) => {
                    if let Some(context) = &self.reader.context {
                        violation(&mut context.scope.lock().unwrap(), "source-seek-error");
                    }
                }
            }
        }
        result
    }
}
impl Drop for TrackedAsyncFile {
    fn drop(&mut self) {
        if self.pending {
            self.reader.pending_drop();
        }
        if self.seeking {
            if let Some(context) = &self.reader.context {
                violation(&mut context.scope.lock().unwrap(), "pending-source-seek");
            }
        }
    }
}
pub enum Body {
    File(TrackedFile),
    Bytes(std::io::Cursor<Vec<u8>>),
}
impl Body {
    pub(crate) fn bytes(bytes: Vec<u8>) -> Self {
        Self::Bytes(std::io::Cursor::new(bytes))
    }
}
impl Read for Body {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::File(file) => file.read(bytes),
            Self::Bytes(body) => body.read(bytes),
        }
    }
}
impl Seek for Body {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        match self {
            Self::File(file) => file.seek(position),
            Self::Bytes(body) => body.seek(position),
        }
    }
}
#[cfg(test)]
pub(crate) mod tests;
