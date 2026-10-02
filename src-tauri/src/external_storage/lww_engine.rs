use super::{
    contract::*,
    http::HttpClockSample,
    lww_segment::{self as segment, LargeBody, Segment},
};
#[cfg(test)]
use crate::persistent_store::lww::ApplyReceive;
use crate::persistent_store::{
    external_lww::{SealedBody, SealedPublication, UploadState},
    lww::{Change, Header, MessageLocator, Progress, StageReceive},
    PersistentStore,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use risunest_sync_wire::{stamp::DecimalU64, unit::UnitValue};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncWrite};

fn store_error(_: crate::persistent_store::StoreError) -> ProviderError {
    segment::corrupt()
}
fn transient(_: impl std::fmt::Display) -> ProviderError {
    ProviderError::new(ErrorKind::Transient)
}
pub(crate) struct BytesSource(pub Vec<u8>);
impl TransferSource for BytesSource {
    fn byte_length(&self) -> u64 {
        self.0.len() as u64
    }
    fn open<'a>(
        &'a self,
        offset: u64,
        length: u64,
        cancel: &'a Cancellation,
    ) -> ProviderFuture<'a, Pin<Box<dyn AsyncRead + Send>>> {
        Box::pin(async move {
            cancel.check()?;
            let start = usize::try_from(offset).map_err(|_| segment::corrupt())?;
            let end = usize::try_from(offset.checked_add(length).ok_or_else(segment::corrupt)?)
                .map_err(|_| segment::corrupt())?;
            let bytes = self
                .0
                .get(start..end)
                .ok_or_else(segment::corrupt)?
                .to_vec();
            Ok(Box::pin(Cursor::new(bytes)) as Pin<Box<dyn AsyncRead + Send>>)
        })
    }
}
struct BytesSink {
    bytes: Arc<std::sync::Mutex<Vec<u8>>>,
    verified: bool,
}
struct BytesWriter(Arc<std::sync::Mutex<Vec<u8>>>);
impl AsyncWrite for BytesWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.0.lock() {
            Ok(mut out) => {
                out.extend_from_slice(bytes);
                std::task::Poll::Ready(Ok(bytes.len()))
            }
            Err(_) => std::task::Poll::Ready(Err(std::io::Error::other("receive lock"))),
        }
    }
    fn poll_flush(
        self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}
impl TransferSink for BytesSink {
    fn open<'a>(
        &'a mut self,
        offset: u64,
        _length: u64,
        cancel: &'a Cancellation,
    ) -> ProviderFuture<'a, Pin<Box<dyn AsyncWrite + Send>>> {
        Box::pin(async move {
            cancel.check()?;
            if offset != 0 {
                return Err(segment::corrupt());
            }
            self.bytes.lock().map_err(transient)?.clear();
            Ok(Box::pin(BytesWriter(self.bytes.clone())) as Pin<Box<dyn AsyncWrite + Send>>)
        })
    }
    fn finish<'a>(&'a mut self, length: u64, hash: &'a str) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let bytes = self.bytes.lock().map_err(transient)?;
            if bytes.len() as u64 != length || segment::digest(&bytes) != hash {
                return Err(segment::corrupt());
            }
            self.verified = true;
            Ok(())
        })
    }
}
pub(crate) async fn read_bytes(
    provider: &dyn Provider,
    repository: &RepositoryHandle,
    locator: &RemoteLocator,
    cancel: &Cancellation,
) -> Result<Vec<u8>> {
    let mut sink = BytesSink {
        bytes: Arc::new(std::sync::Mutex::new(Vec::new())),
        verified: false,
    };
    if !matches!(
        provider
            .read_object(repository, locator, None, &mut sink, cancel)
            .await?,
        ReadReceipt::Body(_)
    ) || !sink.verified
    {
        return Err(segment::corrupt());
    }
    let bytes = sink.bytes.lock().map_err(transient)?.clone();
    Ok(bytes)
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Admission {
    pub upper_ms: u64,
    at: Instant,
    wall_ms: u64,
}
impl Admission {
    pub(crate) fn from_sample(sample: &HttpClockSample) -> Result<Self> {
        if !sample.usable() || sample.observed_at.elapsed() > Duration::from_secs(15 * 60) {
            return Err(ProviderError::new(ErrorKind::ClockSkew));
        }
        let upper = sample
            .date_ms
            .ok_or_else(segment::corrupt)?
            .checked_add(1000)
            .and_then(|v| v.checked_add(sample.round_trip_ms.div_ceil(2)))
            .ok_or_else(segment::corrupt)?;
        Ok(Self {
            upper_ms: upper,
            at: sample.observed_at,
            wall_ms: sample.local_after_ms,
        })
    }
    #[cfg(test)]
    pub(crate) fn synthetic(now: u64) -> Self {
        Self {
            upper_ms: now,
            at: Instant::now(),
            wall_ms: now,
        }
    }
    fn upper(self) -> Result<u64> {
        let elapsed = u64::try_from(self.at.elapsed().as_millis()).map_err(transient)?;
        let now = super::runtime::now_ms();
        if elapsed > 15 * 60_000 || now.abs_diff(self.wall_ms.saturating_add(elapsed)) > 1000 {
            return Err(ProviderError::new(ErrorKind::ClockSkew));
        }
        self.upper_ms
            .checked_add(elapsed)
            .ok_or_else(segment::corrupt)
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublicationResult {
    pub segments: DecimalU64,
    pub units: DecimalU64,
    pub bytes: DecimalU64,
}
impl Default for PublicationResult {
    fn default() -> Self {
        Self {
            segments: DecimalU64(0),
            units: DecimalU64(0),
            bytes: DecimalU64(0),
        }
    }
}
#[cfg(test)]
pub(crate) mod cycle_keys {
    use risunest_sync_wire::unit::UnitKey;
    use std::{cell::RefCell, collections::BTreeSet};

    #[derive(Debug, Default)]
    pub(crate) struct CycleKeys {
        pub selected_keys: BTreeSet<UnitKey>,
        pub attempted_keys: BTreeSet<UnitKey>,
        pub emitted_keys: BTreeSet<UnitKey>,
        pub affected_keys: BTreeSet<UnitKey>,
        pub held_keys: BTreeSet<UnitKey>,
        pub deferred_keys: BTreeSet<UnitKey>,
        pub segment_attempts: usize,
        pub accepted_publications: usize,
        pub receive_applies: usize,
        pub failed_operations: usize,
        pub pending_operations: usize,
    }
    impl CycleKeys {
        pub(crate) fn complete(&self) -> bool {
            self.failed_operations == 0 && self.pending_operations == 0
        }
    }
    thread_local! {
        static WORK: RefCell<CycleKeys> = RefCell::new(CycleKeys::default());
    }
    pub(crate) fn reset() {
        WORK.with(|work| {
            let mut work = work.borrow_mut();
            assert_eq!(work.pending_operations, 0, "cycle-key operation is still running");
            *work = CycleKeys::default();
        });
    }
    pub(crate) fn take() -> CycleKeys {
        WORK.with(|work| {
            let mut work = work.borrow_mut();
            let result = std::mem::take(&mut *work);
            work.pending_operations = result.pending_operations;
            result
        })
    }
    pub(super) fn observe(observe: impl FnOnce(&mut CycleKeys)) {
        WORK.with(|work| observe(&mut work.borrow_mut()));
    }
    pub(super) struct Operation {
        finished: bool,
    }
    impl Operation {
        pub(super) fn start() -> Self {
            observe(|work| work.pending_operations += 1);
            Self { finished: false }
        }
        pub(super) fn finish(mut self) {
            self.finished = true;
        }
    }
    impl Drop for Operation {
        fn drop(&mut self) {
            observe(|work| {
                work.pending_operations -= 1;
                if !self.finished {
                    work.failed_operations += 1;
                }
            });
        }
    }
}
pub(crate) struct ExternalLwwEngine {
    pub provider: Arc<dyn Provider>,
    pub repository: RepositoryHandle,
    pub library: String,
    pub root_key: zeroize::Zeroizing<[u8; 32]>,
    pub admission: Option<Admission>,
    pub connection_id: String,
    pub connection_root: std::path::PathBuf,
}
impl ExternalLwwEngine {
    pub(crate) fn target_scope(&self) -> String {
        format!("{}:{}", self.library, self.repository.connection_identity)
    }
    pub(crate) fn admit(&mut self, sample: &HttpClockSample) -> Result<()> {
        self.admission = Some(Admission::from_sample(sample)?);
        Ok(())
    }
    pub(crate) fn invalidate_clock(&mut self) {
        self.admission = None;
    }
    pub(crate) fn admitted_upper(&self) -> Result<u64> {
        self.admission
            .ok_or_else(|| ProviderError::new(ErrorKind::ClockSkew))?
            .upper()
    }
    fn intent(&self, id: &str, role: ObjectRole, bytes: &[u8]) -> ObjectIntent {
        ObjectIntent {
            job_id: self.library.clone(),
            repository_id: self.repository.repository_id.clone(),
            object_id: id.into(),
            role,
            byte_length: bytes.len() as u64,
            sha256: segment::digest(bytes),
        }
    }
    fn resume(state: &UploadState) -> ResumeState {
        ResumeState {
            sealed_state: SecretRef(state.sealed_state.clone()),
            confirmed_offset: state.confirmed_offset.0,
            expires_at_ms: state.expires_at_ms.map(|v| v.0),
        }
    }
    fn saved(state: ResumeState) -> UploadState {
        UploadState {
            sealed_state: state.sealed_state.0,
            confirmed_offset: DecimalU64(state.confirmed_offset),
            expires_at_ms: state.expires_at_ms.map(DecimalU64),
        }
    }
    pub(crate) async fn publish(
        &mut self,
        store: &mut PersistentStore,
        authority: DecimalU64,
        generating: &[MessageLocator],
        cancel: &Cancellation,
    ) -> Result<PublicationResult> {
        #[cfg(test)]
        let key_operation = cycle_keys::Operation::start();
        let mut result = PublicationResult::default();
        let writer = store.lww_clock_state().map_err(store_error)?.writer_id;
        loop {
            cancel.check()?;
            let upper = self
                .admitted_upper()?
                .checked_add(300_000)
                .ok_or_else(segment::corrupt)?;
            let pending = store
                .external_lww_pending(&self.target_scope(), &writer)
                .map_err(store_error)?;
            let (mut publication, mut sealed) = match pending {
                Some(value) => value,
                None => {
                    let entries = store
                        .lww_read_outbox_generating(authority, 4096, generating)
                        .map_err(store_error)?
                        .entries;
                    if entries.is_empty() {
                        break;
                    }
                    if entries.iter().any(|e| e.stamp.physical_ms.0 > upper) {
                        self.repair_unsent(store, authority, &entries)?;
                        continue;
                    }
                    let seq = store
                        .external_lww_next_sequence(&self.target_scope(), &writer)
                        .map_err(store_error)?;
                    let mut payload = Segment::new(&self.library, &writer, seq);
                    let mut selected = Vec::new();
                    let mut bodies = Vec::new();
                    for entry in entries {
                        let mut candidate = payload.clone();
                        let mut added = Vec::new();
                        self.include_objects(
                            store,
                            &entry.value,
                            &mut candidate,
                            &mut added,
                            cancel,
                        )
                        .await?;
                        candidate.changes.push(Change {
                            key: entry.key.clone(),
                            stamp: entry.stamp.clone(),
                            value: entry.value.clone(),
                        });
                        if candidate.encode()?.len() > segment::SEGMENT_BYTES {
                            if selected.is_empty() {
                                return Err(ProviderError::new(ErrorKind::FileTooLarge));
                            }
                            break;
                        }
                        payload = candidate;
                        bodies.extend(added);
                        selected.push(entry);
                    }
                    let plaintext = payload.encode()?;
                    let payload_sha256 = segment::digest(&plaintext);
                    let bytes = Vec::new();
                    let sha256 = String::new();
                    let object_id = String::new();
                    let publication = SealedPublication {
                        target: self.target_scope(),
                        writer: writer.clone(),
                        seq: DecimalU64(seq),
                        authority,
                        object_id,
                        sha256,
                        payload_sha256,
                        payload: URL_SAFE_NO_PAD.encode(plaintext),
                        sealed: false,
                        entries: selected,
                        bodies,
                        resume: None,
                        dispatched: false,
                        complete: false,
                    };
                    store
                        .external_lww_persist(&publication, &bytes)
                        .map_err(store_error)?;
                    (publication, bytes)
                }
            };
            if publication.authority != authority {
                return Err(segment::corrupt());
            }
            if publication
                .entries
                .iter()
                .any(|e| e.stamp.physical_ms.0 > upper)
            {
                if publication.dispatched {
                    return Err(ProviderError::new(ErrorKind::ClockSkew));
                }
                self.repair_unsent(store, authority, &publication.entries)?;
                store
                    .external_lww_abandon_unsent(&self.target_scope(), &writer)
                    .map_err(store_error)?;
                continue;
            }
            #[cfg(test)]
            cycle_keys::observe(|work| {
                work.selected_keys.extend(publication.entries.iter().map(|entry| entry.key.clone()));
            });
            for index in 0..publication.bodies.len() {
                if publication.bodies[index].complete {
                    continue;
                }
                let body = publication.bodies[index].clone();
                let bytes = URL_SAFE_NO_PAD
                    .decode(&body.bytes)
                    .map_err(|_| segment::corrupt())?;
                let intent = self.intent(&body.object_id, ObjectRole::Pack, &bytes);
                if publication.bodies[index].resume.is_none() {
                    publication.bodies[index].resume = self
                        .provider
                        .begin_upload(&self.repository, &intent, cancel)
                        .await?
                        .map(Self::saved);
                    store
                        .external_lww_persist(&publication, &sealed)
                        .map_err(store_error)?;
                }
                let resume = publication.bodies[index].resume.as_ref().map(Self::resume);
                let receipt = self
                    .provider
                    .create_object(
                        &self.repository,
                        &intent,
                        &BytesSource(bytes),
                        resume.as_ref(),
                        cancel,
                    )
                    .await?;
                Self::validate_receipt(&intent, &receipt)?;
                publication.bodies[index].locator = Some(receipt.locator);
                publication.bodies[index].complete = true;
                store
                    .external_lww_persist(&publication, &sealed)
                    .map_err(store_error)?;
            }
            if !publication.sealed {
                let plain = URL_SAFE_NO_PAD
                    .decode(&publication.payload)
                    .map_err(|_| segment::corrupt())?;
                let mut payload = Segment::decode(&plain)?;
                for body in payload.large_bodies.values_mut() {
                    body.locator = publication
                        .bodies
                        .iter()
                        .find(|item| item.object_id == body.object_id)
                        .and_then(|item| item.locator.clone());
                    if body.locator.is_none() {
                        return Err(segment::corrupt());
                    }
                }
                let (bytes, payload_hash) = segment::seal(&payload, &self.root_key)?;
                sealed = bytes;
                publication.payload_sha256 = payload_hash;
                publication.sha256 = segment::digest(&sealed);
                publication.object_id =
                    segment_object_id(&writer, publication.seq.0, &publication.sha256)?;
                publication.sealed = true;
                publication.payload = URL_SAFE_NO_PAD.encode(payload.encode()?);
                store
                    .external_lww_persist(&publication, &sealed)
                    .map_err(store_error)?;
            }
            let intent = ObjectIntent {
                job_id: self.library.clone(),
                repository_id: self.repository.repository_id.clone(),
                object_id: publication.object_id.clone(),
                role: ObjectRole::Segment,
                byte_length: sealed.len() as u64,
                sha256: publication.sha256.clone(),
            };
            if publication.dispatched && !publication.complete {
                let resume = publication.resume.as_ref().map(Self::resume);
                match self
                    .provider
                    .reconcile_upload(&self.repository, &intent, resume.as_ref(), cancel)
                    .await?
                {
                    UploadResolution::Complete(receipt) => {
                        Self::validate_receipt(&intent, &receipt)?;
                        publication.complete = true
                    }
                    UploadResolution::Resumable(state) => {
                        publication.resume = Some(Self::saved(state))
                    }
                    UploadResolution::Conflict => return Err(segment::corrupt()),
                    UploadResolution::RestartRequired => {}
                }
            }
            if !publication.complete {
                if publication.resume.is_none() {
                    publication.resume = self
                        .provider
                        .begin_upload(&self.repository, &intent, cancel)
                        .await?
                        .map(Self::saved);
                    store
                        .external_lww_persist(&publication, &sealed)
                        .map_err(store_error)?;
                }
                publication.dispatched = true;
                store
                    .external_lww_persist(&publication, &sealed)
                    .map_err(store_error)?;
                let resume = publication.resume.as_ref().map(Self::resume);
                #[cfg(test)]
                cycle_keys::observe(|work| {
                    work.segment_attempts += 1;
                    work.attempted_keys.extend(publication.entries.iter().map(|entry| entry.key.clone()));
                });
                let receipt = self
                    .provider
                    .create_object(
                        &self.repository,
                        &intent,
                        &BytesSource(sealed.clone()),
                        resume.as_ref(),
                        cancel,
                    )
                    .await?;
                Self::validate_receipt(&intent, &receipt)?;
                publication.complete = true;
            }
            let payload = segment::open(
                &sealed,
                &self.library,
                &writer,
                publication.seq.0,
                &self.root_key,
            )?;
            store
                .external_lww_verify_versions(&self.target_scope(), &payload.changes)
                .map_err(store_error)?;
            self.register_sources(store, &payload, &publication.object_id)?;
            for hash in payload
                .message_pages
                .keys()
                .chain(payload.small_bodies.keys())
                .chain(payload.large_bodies.keys())
            {
                store
                    .external_lww_witness_object(&self.target_scope(), hash)
                    .map_err(store_error)?;
            }
            store
                .external_lww_record_seen(
                    &self.target_scope(),
                    &writer,
                    publication.seq.0,
                    &publication.sha256,
                )
                .map_err(store_error)?;
            store
                .external_lww_finish_publication(&publication, &sealed)
                .map_err(store_error)?;
            #[cfg(test)]
            cycle_keys::observe(|work| {
                work.accepted_publications += 1;
                work.emitted_keys.extend(payload.changes.iter().map(|change| change.key.clone()));
            });
            result.segments.0 += 1;
            result.units.0 += publication.entries.len() as u64;
            result.bytes.0 += sealed.len() as u64;
        }
        #[cfg(test)]
        key_operation.finish();
        Ok(result)
    }
    fn validate_receipt(intent: &ObjectIntent, receipt: &ObjectReceipt) -> Result<()> {
        if !receipt.complete
            || receipt.byte_length != intent.byte_length
            || receipt.checksum.as_ref().is_some_and(|checksum| {
                checksum.algorithm != "sha256" || checksum.value != intent.sha256
            })
        {
            return Err(segment::corrupt());
        }
        Ok(())
    }
    fn repair_unsent(
        &self,
        store: &mut PersistentStore,
        authority: DecimalU64,
        _entries: &[crate::persistent_store::lww::OutboxEntry],
    ) -> Result<()> {
        let header = Header {
            binding_authority: authority,
            request_id: format!("external-clock-repair-{}", uuid::Uuid::new_v4()),
        };
        let proof = format!("external-unsent-{}", header.request_id);
        let entries = store
            .external_lww_unpublished_entries(authority)
            .map_err(store_error)?;
        let acks = entries
            .iter()
            .map(|entry| {
                let identity = entry.value.identity();
                #[cfg(test)]
                segment::observe_value_identity(
                    "publish-repair.descriptor",
                    "publish-repair.identity",
                    &entry.value,
                    identity.is_ok(),
                );
                Ok(crate::persistent_store::lww::AckEntry {
                    key: entry.key.clone(),
                    stamp: entry.stamp.clone(),
                    version: entry.version.clone(),
                    value_identity: identity.map_err(|_| segment::corrupt())?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        store
            .lww_record_unpublished_proof(&header, &proof, &acks)
            .map_err(store_error)?;
        store
            .lww_retry_unpublished(&header, &proof, DecimalU64(self.admitted_upper()?))
            .map_err(|_| ProviderError::new(ErrorKind::ClockSkew))?;
        Ok(())
    }
    pub(crate) async fn settle_publication(
        &self,
        store: &mut PersistentStore,
        cancel: &Cancellation,
    ) -> Result<()> {
        let writer = store.lww_clock_state().map_err(store_error)?.writer_id;
        let Some((publication, bytes)) = store
            .external_lww_pending(&self.target_scope(), &writer)
            .map_err(store_error)?
        else {
            return Ok(());
        };
        if !publication.dispatched || publication.complete {
            return Ok(());
        }
        let intent = self.intent(&publication.object_id, ObjectRole::Segment, &bytes);
        let resume = publication.resume.as_ref().map(Self::resume);
        if let UploadResolution::Complete(receipt) = self
            .provider
            .reconcile_upload(&self.repository, &intent, resume.as_ref(), cancel)
            .await?
        {
            Self::validate_receipt(&intent, &receipt)?;
            let received = read_bytes(
                self.provider.as_ref(),
                &self.repository,
                &receipt.locator,
                cancel,
            )
            .await?;
            if received != bytes {
                return Err(segment::corrupt());
            }
            segment::open(
                &received,
                &self.library,
                &writer,
                publication.seq.0,
                &self.root_key,
            )?;
            return Ok(());
        }
        Err(ProviderError::new(ErrorKind::PreconditionFailed))
    }
    async fn include_objects(
        &self,
        store: &mut PersistentStore,
        value: &UnitValue,
        payload: &mut Segment,
        bodies: &mut Vec<SealedBody>,
        cancel: &Cancellation,
    ) -> Result<()> {
        let mut hashes = BTreeSet::new();
        if let UnitValue::Object { descriptor, .. } = value {
            hashes.insert(descriptor.object_hash.clone());
            hashes.extend(descriptor.dependencies.iter().cloned());
            if let Some(root) = &descriptor.dependency_root {
                risunest_sync_wire::descriptor::visit_reference_tree(
                    root,
                    false,
                    |hash| {
                        let bytes = store
                            .lww_object_body(hash)
                            .map_err(|_| risunest_sync_wire::WireError("object-read"))?
                            .ok_or(risunest_sync_wire::WireError("object-missing"))?;
                        #[cfg(test)]
                        segment::observe_hash_input("publish-reference-tree.page", bytes.len());
                        Ok(bytes)
                    },
                    |hash, _| {
                        hashes.insert(hash.into());
                        Ok(())
                    },
                )
                .map_err(|_| segment::corrupt())?;
            }
        } else if let UnitValue::Inline { bytes } = value {
            let bytes = URL_SAFE_NO_PAD
                .decode(bytes)
                .map_err(|_| segment::corrupt())?;
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| segment::corrupt())?;
            if let Some(hash) = value.get("objectHash").and_then(serde_json::Value::as_str) {
                hashes.insert(hash.into());
            }
        }
        for hash in hashes {
            if payload.message_pages.contains_key(&hash)
                || payload.small_bodies.contains_key(&hash)
                || payload.large_bodies.contains_key(&hash)
                || store
                    .external_lww_has_object(&self.target_scope(), &hash)
                    .map_err(store_error)?
            {
                continue;
            }
            let bytes = if let Some(bytes) = store.lww_object_body(&hash).map_err(store_error)? {
                bytes
            } else {
                let root = store.external_lww_root();
                super::lww_residency::hydrate_missing(&root, &hash, cancel)
                    .await?
                    .ok_or_else(segment::corrupt)?;
                store
                    .lww_object_body(&hash)
                    .map_err(store_error)?
                    .ok_or_else(segment::corrupt)?
            };
            if store
                .external_lww_object_is_control(&hash)
                .map_err(store_error)?
            {
                payload
                    .message_pages
                    .insert(hash, URL_SAFE_NO_PAD.encode(bytes));
            } else if bytes.len() <= segment::SMALL_BODY_BYTES {
                payload
                    .small_bodies
                    .insert(hash, URL_SAFE_NO_PAD.encode(bytes));
            } else {
                let id = uuid::Uuid::new_v4().to_string();
                let sealed = segment::seal_body(&bytes, &self.library, &id, &self.root_key)?;
                let sha = segment::digest(&sealed);
                payload.large_bodies.insert(
                    hash,
                    LargeBody {
                        object_id: id.clone(),
                        sha256: sha.clone(),
                        byte_length: DecimalU64(sealed.len() as u64),
                        plaintext_byte_length: DecimalU64(bytes.len() as u64),
                        locator: None,
                    },
                );
                bodies.push(SealedBody {
                    object_id: id,
                    sha256: sha,
                    bytes: URL_SAFE_NO_PAD.encode(sealed),
                    resume: None,
                    complete: false,
                    locator: None,
                });
            }
        }
        Ok(())
    }
    pub(crate) async fn listing(&self, cancel: &Cancellation) -> Result<Vec<ObjectReceipt>> {
        let mut objects = Vec::new();
        let mut cursor = None;
        let mut cursors = BTreeSet::new();
        loop {
            let page = self
                .provider
                .list_objects(
                    &self.repository,
                    Collection::Segments,
                    cursor.as_deref(),
                    100,
                    cancel,
                )
                .await?;
            objects.extend(page.objects);
            match page.next_cursor {
                Some(next) => {
                    if !cursors.insert(next.clone()) {
                        return Err(segment::corrupt());
                    }
                    cursor = Some(next)
                }
                None => break,
            }
        }
        Ok(objects)
    }
    pub(crate) async fn verified_segments(
        &self,
        store: &mut PersistentStore,
        all: bool,
        cancel: &Cancellation,
    ) -> Result<Vec<Segment>> {
        let upper = self
            .admitted_upper()?
            .checked_add(300_000)
            .ok_or_else(segment::corrupt)?;
        let objects = self.listing(cancel).await?;
        let mut groups: BTreeMap<(String, u64), Vec<ObjectReceipt>> = BTreeMap::new();
        for object in objects {
            let name = object
                .locator
                .object
                .rsplit('/')
                .next()
                .ok_or_else(segment::corrupt)?;
            let (writer, seq, _) = parse_segment_object_id(name)?;
            groups.entry((writer.into(), seq)).or_default().push(object);
        }
        let progress = store
            .lww_receive_progress(store.lww_binding_authority().map_err(store_error)?)
            .map_err(store_error)?;
        let mut prefixes: BTreeMap<String, u64> = if all {
            BTreeMap::new()
        } else {
            progress
                .into_iter()
                .filter(|p| p.kind == "external")
                .filter_map(|p| p.writer_id.map(|w| (w, p.cursor.0)))
                .collect()
        };
        let mut segments = Vec::new();
        for ((writer, seq), mut variants) in groups {
            variants.sort_by(|a, b| a.locator.object.cmp(&b.locator.object));
            let prefix = *prefixes.get(&writer).unwrap_or(&0);
            let mut winner = None;
            let mut identity = None;
            for receipt in variants {
                let name = receipt
                    .locator
                    .object
                    .rsplit('/')
                    .next()
                    .ok_or_else(segment::corrupt)?;
                let (_, _, hash) = parse_segment_object_id(name)?;
                let seen = store
                    .external_lww_matches_seen(&self.target_scope(), &writer, seq, hash)
                    .map_err(store_error)?;
                if !all && seen && seq <= prefix {
                    continue;
                }
                let bytes = read_bytes(
                    self.provider.as_ref(),
                    &self.repository,
                    &receipt.locator,
                    cancel,
                )
                .await?;
                if segment::digest(&bytes) != hash {
                    return Err(segment::corrupt());
                }
                let payload = segment::open(&bytes, &self.library, &writer, seq, &self.root_key)?;
                store
                    .external_lww_verify_seen(&self.target_scope(), &writer, seq, hash)
                    .map_err(store_error)?;
                if payload
                    .changes
                    .iter()
                    .any(|change| change.stamp.physical_ms.0 > upper)
                {
                    return Err(ProviderError::new(ErrorKind::ClockSkew));
                }
                store
                    .external_lww_verify_versions(&self.target_scope(), &payload.changes)
                    .map_err(store_error)?;
                if identity.as_deref().is_some_and(|value| value != hash) {
                    return Err(segment::corrupt());
                }
                identity = Some(hash.to_owned());
                winner = Some(payload);
            }
            if seq != prefix.checked_add(1).ok_or_else(segment::corrupt)? {
                if all {
                    return Err(ProviderError::new(ErrorKind::PreconditionFailed));
                }
                continue;
            }
            if let Some(payload) = winner {
                self.admit_objects(
                    store,
                    &payload,
                    &segment_object_id(
                        &writer,
                        seq,
                        identity.as_deref().ok_or_else(segment::corrupt)?,
                    )?,
                    cancel,
                )
                .await?;
                store
                    .external_lww_record_seen(
                        &self.target_scope(),
                        &writer,
                        seq,
                        identity.as_deref().ok_or_else(segment::corrupt)?,
                    )
                    .map_err(store_error)?;
                prefixes.insert(writer, seq);
                segments.push(payload);
            }
        }
        Ok(segments)
    }
    async fn admit_objects(
        &self,
        store: &mut PersistentStore,
        payload: &Segment,
        physical_segment: &str,
        _cancel: &Cancellation,
    ) -> Result<()> {
        for (hash, encoded) in &payload.message_pages {
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded)
                .map_err(|_| segment::corrupt())?;
            store.lww_put_object(hash, &bytes).map_err(store_error)?;
            store
                .external_lww_witness_object(&self.target_scope(), hash)
                .map_err(store_error)?;
        }
        for (hash, encoded) in &payload.small_bodies {
            if !store
                .external_lww_object_is_local(hash)
                .map_err(store_error)?
            {
                let bytes = URL_SAFE_NO_PAD
                    .decode(encoded)
                    .map_err(|_| segment::corrupt())?;
                store
                    .lww_put_managed_object(hash, &bytes)
                    .map_err(store_error)?;
            }
            store
                .external_lww_witness_object(&self.target_scope(), hash)
                .map_err(store_error)?;
        }
        self.register_sources(store, payload, physical_segment)?;
        Ok(())
    }
    fn register_sources(
        &self,
        store: &PersistentStore,
        payload: &Segment,
        physical_segment: &str,
    ) -> Result<()> {
        for (hash, body) in &payload.large_bodies {
            let locator = body.locator.as_ref().ok_or_else(segment::corrupt)?;
            locator.validate_for(&self.repository)?;
            store
                .external_lww_register_source(&super::lww_residency::Source {
                    hash: hash.clone(),
                    library_id: self.library.clone(),
                    connection_id: self.connection_id.clone(),
                    connection_root: self.connection_root.clone(),
                    protected_segment: physical_segment.into(),
                    body: body.clone(),
                })
                .map_err(store_error)?;
            store
                .external_lww_witness_object(&self.target_scope(), hash)
                .map_err(store_error)?;
        }
        Ok(())
    }
    pub(crate) async fn stage_binding(
        &self,
        store: &mut PersistentStore,
        header: &Header,
        inspection: &str,
        cancel: &Cancellation,
    ) -> Result<crate::persistent_store::lww::BindingUnitStage> {
        let segments = self.verified_segments(store, true, cancel).await?;
        let mut winners = BTreeMap::new();
        for segment in segments {
            for change in segment.changes {
                segment::merge(&mut winners, change)?;
            }
        }
        store
            .lww_stage_binding_units(
                header,
                inspection,
                &winners.into_values().collect::<Vec<_>>(),
                DecimalU64(
                    self.admitted_upper()?
                        .checked_add(300_000)
                        .ok_or_else(segment::corrupt)?,
                ),
            )
            .map_err(store_error)
    }
    pub(crate) async fn prepare_new_device(
        &self,
        store: &mut PersistentStore,
        header: &Header,
        staging: &str,
        cancel: &Cancellation,
    ) -> Result<crate::persistent_store::lww::NewDevicePreparation> {
        self.settle_publication(store, cancel).await?;
        self.verified_segments(store, true, cancel).await?;
        let preparation = store
            .prepare_lww_new_device(header, staging)
            .map_err(store_error)?;
        store
            .authorize_lww_new_device(&preparation.authorization_id)
            .map_err(store_error)
    }
    pub(crate) async fn receive_requests(
        &self,
        store: &mut PersistentStore,
        authority: DecimalU64,
        cancel: &Cancellation,
    ) -> Result<Vec<StageReceive>> {
        let segments = self.verified_segments(store, false, cancel).await?;
        segments
            .into_iter()
            .map(|segment| {
                store
                    .external_lww_stable_receive(StageReceive {
                        header: Header {
                            binding_authority: authority,
                            request_id: format!(
                                "external-receive-{}-{}-{}-{}",
                                self.library, authority.0, segment.writer_id, segment.seq.0
                            ),
                        },
                        changes: segment.changes,
                        progress: Progress {
                            kind: "external".into(),
                            cursor: segment.seq,
                            writer_id: Some(segment.writer_id),
                        },
                        admitted_time_upper_ms: DecimalU64(
                            self.admitted_upper()?
                                .checked_add(300_000)
                                .ok_or_else(segment::corrupt)?,
                        ),
                    })
                    .map_err(store_error)
            })
            .collect()
    }
    #[cfg(test)]
    pub(crate) async fn receive_and_apply(
        &self,
        store: &mut PersistentStore,
        authority: DecimalU64,
        generating: &[MessageLocator],
        cancel: &Cancellation,
    ) -> Result<usize> {
        let key_operation = cycle_keys::Operation::start();
        let requests = self.receive_requests(store, authority, cancel).await?;
        let mut count = 0;
        for request in requests {
            store.lww_stage_receive(&request).map_err(store_error)?;
            let applied = store
                .lww_apply_receive(&ApplyReceive {
                    header: request.header.clone(),
                    generating: generating.to_vec(),
                })
                .map_err(store_error)?;
            store
                .lww_finish_receive(&request.header)
                .map_err(store_error)?;
            cycle_keys::observe(|work| {
                work.receive_applies += 1;
                work.affected_keys.extend(applied.affected_keys);
                work.held_keys.extend(applied.held_keys);
                work.deferred_keys.extend(applied.deferred_keys);
            });
            count += 1;
        }
        key_operation.finish();
        Ok(count)
    }
}
