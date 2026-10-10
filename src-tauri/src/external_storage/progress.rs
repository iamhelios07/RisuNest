use super::{capabilities::Capabilities, contract::*, phase_progress::{PhaseCounters, PhaseProgress}};
use serde::Serialize;
use std::{collections::{BTreeMap, BTreeSet}, path::{Path, PathBuf}, sync::{Arc, Mutex, OnceLock}, time::{Duration, Instant}};
use tauri::ipc::Channel;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    sequence: u64,
    stage: &'static str,
    prepared_bytes: String,
    uploaded_bytes: String,
    downloaded_bytes: String,
    uploaded_objects: String,
    downloaded_objects: String,
}
struct Reading {
    snapshot: Snapshot,
    prepared: u64,
    uploaded: u64,
    downloaded: u64,
    uploads: BTreeSet<String>,
    downloads: BTreeSet<String>,
    sent: Instant,
}
pub(crate) struct Run {
    sink: Box<dyn Fn(Snapshot) + Send + Sync>,
    reading: Mutex<Reading>,
}
impl Run {
    fn update(&self, stage: &'static str, change: impl FnOnce(&mut Reading), force: bool) {
        if let Ok(mut reading) = self.reading.lock() {
            let changed = reading.snapshot.stage != stage;
            reading.snapshot.stage = stage;
            change(&mut reading);
            if force || changed || reading.sent.elapsed() >= Duration::from_millis(250) {
                reading.sent = Instant::now();
                reading.snapshot.sequence += 1;
                reading.snapshot.prepared_bytes = reading.prepared.to_string();
                reading.snapshot.uploaded_bytes = reading.uploaded.to_string();
                reading.snapshot.downloaded_bytes = reading.downloaded.to_string();
                reading.snapshot.uploaded_objects = reading.uploads.len().to_string();
                reading.snapshot.downloaded_objects = reading.downloads.len().to_string();
                (self.sink)(reading.snapshot.clone());
            }
        }
    }
    fn accepted(&self, receipt: &ObjectReceipt, upload: bool) {
        if !receipt.complete { return; }
        // Counts verified complete objects, including already accepted objects found on retry.
        self.update(if upload { "uploading" } else { "downloading" }, |r| {
            let key = serde_json::to_string(&receipt.locator).unwrap_or_default();
            if upload {
                if r.uploads.insert(key) { r.uploaded = r.uploaded.saturating_add(receipt.byte_length); }
            } else if r.downloads.insert(key) { r.downloaded = r.downloaded.saturating_add(receipt.byte_length); }
        }, false);
    }
}

#[derive(Clone, Default)]
pub(crate) struct Observer(Arc<Mutex<Option<Arc<Run>>>>);
impl Observer {
    pub(crate) fn begin(&self, channel: Option<Channel<Snapshot>>) -> Guard {
        self.begin_sink(channel.map(|channel| Box::new(move |snapshot| { let _ = channel.send(snapshot); }) as Box<dyn Fn(Snapshot) + Send + Sync>))
    }
    fn begin_sink(&self, sink: Option<Box<dyn Fn(Snapshot) + Send + Sync>>) -> Guard {
        let run = sink.map(|sink| Arc::new(Run { sink, reading: Mutex::new(Reading {
            snapshot: Snapshot::default(), prepared: 0, uploaded: 0, downloaded: 0,
            uploads: BTreeSet::new(), downloads: BTreeSet::new(), sent: Instant::now(),
        }) }));
        if let Ok(mut current) = self.0.lock() { *current = run.clone(); }
        if let Some(run) = &run { run.update("checking", |_| {}, true); }
        Guard { observer: self.clone(), run }
    }
    fn current(&self) -> Option<Arc<Run>> { self.0.lock().ok()?.clone() }
    pub(crate) fn wrap(&self, provider: Arc<dyn Provider>) -> Arc<dyn Provider> {
        Arc::new(ObservedProvider { provider, observer: self.clone() })
    }
}

type Jobs = BTreeMap<PathBuf, Arc<Mutex<Snapshot>>>;
static JOBS: OnceLock<Mutex<Jobs>> = OnceLock::new();
pub(crate) struct JobProgress { key: PathBuf, _guard: Guard, observer: Observer }
impl JobProgress {
    pub(crate) fn begin(key: PathBuf) -> Self {
        let value = Arc::new(Mutex::new(Snapshot::default()));
        if let Ok(mut jobs) = JOBS.get_or_init(Default::default).lock() { jobs.insert(key.clone(), value.clone()); }
        let observer = Observer::default();
        let guard = observer.begin_sink(Some(Box::new(move |snapshot| { if let Ok(mut value) = value.lock() { *value = snapshot; } })));
        Self { key, _guard: guard, observer }
    }
    pub(crate) fn wrap(&self, provider: Arc<dyn Provider>) -> Arc<dyn Provider> { self.observer.wrap(provider) }
}
impl Drop for JobProgress {
    fn drop(&mut self) { if let Some(jobs) = JOBS.get() { if let Ok(mut jobs) = jobs.lock() { jobs.remove(&self.key); } } }
}
pub(crate) fn job_snapshot(key: &Path) -> Option<Snapshot> {
    let value = JOBS.get()?.lock().ok()?.get(key)?.clone();
    let snapshot = value.lock().ok()?.clone();
    Some(snapshot)
}
pub(crate) struct Guard { observer: Observer, run: Option<Arc<Run>> }
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(run) = &self.run {
            let stage = run.reading.lock().map(|r| r.snapshot.stage).unwrap_or("checking");
            run.update(stage, |_| {}, true);
        }
        if let Ok(mut current) = self.observer.0.lock() {
            if current.as_ref().zip(self.run.as_ref()).is_some_and(|(a,b)| Arc::ptr_eq(a,b)) { *current = None; }
        }
    }
}
struct ObservedProvider { provider: Arc<dyn Provider>, observer: Observer }
impl Provider for ObservedProvider {
    fn progress_stage(&self, stage: &'static str) {
        if let Some(run) = self.observer.current() { run.update(stage, |_| {}, true); }
    }
    fn preparation_progress(&self) -> Arc<PhaseProgress> {
        let run = self.observer.current();
        let previous = Mutex::new(0u64);
        PhaseProgress::new(move |reading: PhaseCounters| {
            if let (Some(run), Ok(mut previous)) = (&run, previous.lock()) {
                let delta = reading.bytes.saturating_sub(*previous);
                *previous = reading.bytes;
                run.update("preparing", |r| r.prepared = r.prepared.saturating_add(delta), false);
            }
        })
    }
    fn open_repository<'a>(&'a self, c: &'a ConnectionConfig, s: &'a SecretRef, m: OpenMode, x: &'a Cancellation) -> ProviderFuture<'a, (RepositoryHandle, Capabilities)> { self.provider.open_repository(c,s,m,x) }
    fn read_object<'a>(&'a self, r: &'a RepositoryHandle, l: &'a RemoteLocator, v: Option<&'a VersionToken>, s: &'a mut dyn TransferSink, x: &'a Cancellation) -> ProviderFuture<'a, ReadReceipt> {
        let run = self.observer.current();
        Box::pin(async move {
            if let Some(run) = &run { run.update("downloading", |_| {}, false); }
            let result = self.provider.read_object(r,l,v,s,x).await?;
            if let (Some(run), ReadReceipt::Body(receipt)) = (&run, &result) { run.accepted(receipt, false); }
            Ok(result)
        })
    }
    fn begin_upload<'a>(&'a self, r: &'a RepositoryHandle, i: &'a ObjectIntent, x: &'a Cancellation) -> ProviderFuture<'a, Option<ResumeState>> { self.provider.begin_upload(r,i,x) }
    fn create_object<'a>(&'a self, r: &'a RepositoryHandle, i: &'a ObjectIntent, s: &'a dyn TransferSource, resume: Option<&'a ResumeState>, x: &'a Cancellation) -> ProviderFuture<'a, ObjectReceipt> {
        let run = self.observer.current();
        Box::pin(async move {
            if let Some(run) = &run { run.update("uploading", |_| {}, false); }
            let result = self.provider.create_object(r,i,s,resume,x).await?;
            if let Some(run) = &run { run.accepted(&result, true); }
            Ok(result)
        })
    }
    fn compare_exchange_head<'a>(&'a self, r: &'a RepositoryHandle, l: &'a RemoteLocator, e: &'a ExpectedHead, h: &'a HeadBytes, x: &'a Cancellation) -> ProviderFuture<'a, HeadReceipt> { self.provider.compare_exchange_head(r,l,e,h,x) }
    fn replace_head<'a>(&'a self, r: &'a RepositoryHandle, l: &'a RemoteLocator, h: &'a HeadBytes, x: &'a Cancellation) -> ProviderFuture<'a, HeadReceipt> { self.provider.replace_head(r,l,h,x) }
    fn list_objects<'a>(&'a self, r: &'a RepositoryHandle, c: Collection, cursor: Option<&'a str>, limit: u16, x: &'a Cancellation) -> ProviderFuture<'a, ObjectPage> {
        if let Some(run) = self.observer.current() { run.update("checking", |_| {}, false); }
        self.provider.list_objects(r,c,cursor,limit,x)
    }
    fn delete_object<'a>(&'a self, r: &'a RepositoryHandle, l: &'a RemoteLocator, x: &'a Cancellation) -> ProviderFuture<'a, ()> { self.provider.delete_object(r,l,x) }
    fn delete_empty_container<'a>(&'a self, r: &'a RepositoryHandle, l: &'a RemoteLocator, j: &'a [String], x: &'a Cancellation) -> ProviderFuture<'a, ()> { self.provider.delete_empty_container(r,l,j,x) }
    fn reconcile_upload<'a>(&'a self, r: &'a RepositoryHandle, i: &'a ObjectIntent, s: Option<&'a ResumeState>, x: &'a Cancellation) -> ProviderFuture<'a, UploadResolution> {
        let run = self.observer.current();
        Box::pin(async move {
            let result = self.provider.reconcile_upload(r,i,s,x).await?;
            if let (Some(run), UploadResolution::Complete(receipt)) = (&run, &result) { run.accepted(receipt, true); }
            Ok(result)
        })
    }
    fn lookup_metadata<'a>(&'a self, r: &'a RepositoryHandle, i: &'a ObjectIntent, l: Option<&'a RemoteLocator>, x: &'a Cancellation) -> ProviderFuture<'a, Option<ObjectReceipt>> { self.provider.lookup_metadata(r,i,l,x) }
    fn head_locator(&self, r: &RepositoryHandle) -> Result<RemoteLocator> { self.provider.head_locator(r) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_storage::{fake::{FakeProvider, repository}, transfer::{SpoolSource, SpoolSink}};

    #[test]
    fn confirmed_receipts_are_deduplicated_and_lost_responses_wait_for_reconciliation() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let input = directory.path().join("synthetic");
            std::fs::write(&input, b"test").unwrap();
            let digest = risunest_sync_wire::hash(b"test");
            let source = SpoolSource::verified(&input, 4, &digest).unwrap();
            let provider = Arc::new(FakeProvider::new(true));
            let observer = Observer::default();
            let snapshots = Arc::new(Mutex::new(Vec::new()));
            let sink = snapshots.clone();
            let guard = observer.begin_sink(Some(Box::new(move |value| sink.lock().unwrap().push(value))));
            let observed = observer.wrap(provider.clone());
            let repository = repository();
            let cancel = Cancellation::default();
            let intent = ObjectIntent { repository_id: repository.repository_id.clone(), job_id: "job".into(), object_id: "synthetic-pack".into(), role: ObjectRole::Pack, byte_length: 4, sha256: digest };
            provider.state.lock().unwrap().lose_response = true;
            assert!(observed.create_object(&repository, &intent, &source, None, &cancel).await.is_err());
            assert_eq!(observer.current().unwrap().reading.lock().unwrap().uploaded, 0);
            assert!(matches!(observed.reconcile_upload(&repository, &intent, None, &cancel).await.unwrap(), UploadResolution::Complete(_)));
            let receipt = observed.create_object(&repository, &intent, &source, None, &cancel).await.unwrap();
            let mut sink = SpoolSink::create(&directory.path().join("received"), 4).unwrap();
            observed.read_object(&repository, &receipt.locator, None, &mut sink, &cancel).await.unwrap();
            let prepared = observed.preparation_progress();
            prepared.plan(10, 100); prepared.completed(20); prepared.flush();
            observed.progress_stage("applying");
            drop(guard);
            let readings = snapshots.lock().unwrap();
            let last = readings.last().unwrap();
            assert_eq!((&last.uploaded_bytes, &last.uploaded_objects, &last.downloaded_bytes, &last.prepared_bytes), (&"4".into(), &"1".into(), &"4".into(), &"20".into()));
            assert!(readings.windows(2).all(|v| v[0].sequence < v[1].sequence));
            assert_eq!(provider.upload_attempts("synthetic-pack"), 2);
            assert_eq!(provider.read_attempts("synthetic-pack"), 1);
            assert_eq!(provider.listing_count(), 0);
        });
    }

    #[test]
    fn separate_operations_and_jobs_do_not_inherit_counters_or_clear_new_owners() {
        let observer = Observer::default();
        let first = observer.begin_sink(Some(Box::new(|_| {})));
        let old_preparation = observer.wrap(Arc::new(FakeProvider::new(true))).preparation_progress();
        let second = observer.begin_sink(Some(Box::new(|_| {})));
        drop(first);
        old_preparation.plan(1, 9); old_preparation.completed(9); old_preparation.flush();
        assert_eq!(observer.current().unwrap().reading.lock().unwrap().prepared, 0);
        drop(second);
        assert!(observer.current().is_none());
        let root = tempfile::tempdir().unwrap();
        let job = JobProgress::begin(root.path().join("one"));
        assert!(job_snapshot(&root.path().join("one")).is_some());
        assert!(job_snapshot(&root.path().join("two")).is_none());
        drop(job);
        assert!(job_snapshot(&root.path().join("one")).is_none());
    }
}
