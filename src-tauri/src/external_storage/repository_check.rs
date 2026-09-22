//! One selected published root, proved readable. The check walks the metadata
//! graph that root names, then opens every distinct pack under it and removes
//! each body as soon as it is proved. Nothing is staged into the library and
//! nothing outlives the job but the record of what it proved.
use super::{
    contract::{
        Cancellation, ErrorKind, ObjectRole, Provider, ProviderError, RepositoryHandle, Result,
    },
    package_cache,
    packaging::{cpu_permit, RemoteObject},
    reachability, snapshot_restore,
};
use risunest_external_storage_format::{pack, snapshot as wire};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

fn corrupt(_: impl std::fmt::Display) -> ProviderError {
    ProviderError::new(ErrorKind::Corrupt)
}
fn transient(_: impl std::fmt::Display) -> ProviderError {
    ProviderError::new(ErrorKind::Transient)
}

const ROOT_FILE: &str = "checked-root.json";
const LEDGER_FILE: &str = "verified-objects";
const STAGING_DIRECTORY: &str = "checked-metadata";
/// How many damaged objects the result names one by one. The count stays exact
/// past this point.
const REPORTED_DAMAGE: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Damage {
    Missing,
    Corrupt,
}
impl Damage {
    fn as_str(self) -> &'static str {
        match self {
            Damage::Missing => "missing",
            Damage::Corrupt => "corrupt",
        }
    }
}

#[derive(Debug)]
pub(crate) struct CheckOutcome {
    pub snapshot_id: String,
    pub created_at_ms: u64,
    /// The root and its catalog nodes, which the count below includes.
    pub metadata_objects: u64,
    pub verified_objects: u64,
    pub verified_bytes: u64,
    pub damaged_objects: u64,
    pub damaged: Vec<(String, Damage)>,
}
impl CheckOutcome {
    pub(crate) fn summary(&self) -> Value {
        json!({
            "snapshotId": self.snapshot_id,
            "snapshotCreatedAtMs": self.created_at_ms.to_string(),
            "metadataObjects": self.metadata_objects.to_string(),
            "verifiedObjects": self.verified_objects.to_string(),
            "verifiedBytes": self.verified_bytes.to_string(),
            "damagedObjects": self.damaged_objects.to_string(),
            "damaged": self.damaged.iter()
                .map(|(object_id, damage)| json!({"objectId":object_id,"reason":damage.as_str()}))
                .collect::<Vec<_>>(),
        })
    }
}

/// The root this job reads, fixed the first time it runs. A check that is
/// interrupted and resumed keeps reading the state it started on, so its
/// counts describe one root even when a newer one is published meanwhile.
pub(crate) fn pinned_root(
    directory: &Path,
    repository: &RepositoryHandle,
) -> Result<Option<RemoteObject>> {
    let bytes = match fs::read(directory.join(ROOT_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(transient(error)),
    };
    let stored: wire::StoredObject = serde_json::from_slice(&bytes).map_err(corrupt)?;
    Ok(Some(RemoteObject::from_stored(&stored, repository)?))
}

pub(crate) fn pin_root(
    directory: &Path,
    root: &RemoteObject,
    repository: &RepositoryHandle,
) -> Result<RemoteObject> {
    if let Some(existing) = pinned_root(directory, repository)? {
        return Ok(existing);
    }
    fs::create_dir_all(directory).map_err(transient)?;
    let encoded = serde_json::to_vec(&root.stored(repository)?).map_err(corrupt)?;
    let partial = directory.join(format!("{ROOT_FILE}.partial"));
    let mut file = fs::File::create(&partial).map_err(transient)?;
    file.write_all(&encoded).map_err(transient)?;
    file.sync_all().map_err(transient)?;
    drop(file);
    fs::rename(&partial, directory.join(ROOT_FILE)).map_err(transient)?;
    crate::trust_boundary::sync_directory(directory).map_err(transient)?;
    Ok(root.clone())
}

/// Names what this job already proved. A line reaches the disk before its
/// bodies are removed, so an interruption repeats one object at most and never
/// skips one on the strength of an unwritten line.
struct Ledger {
    path: PathBuf,
    verified: BTreeSet<String>,
}
impl Ledger {
    fn open(directory: &Path) -> Result<Self> {
        let path = directory.join(LEDGER_FILE);
        let verified = match fs::read_to_string(&path) {
            Ok(text) => text
                .lines()
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(error) => return Err(transient(error)),
        };
        Ok(Self { path, verified })
    }
    fn holds(&self, identity: &str) -> bool {
        self.verified.contains(identity)
    }
    fn record(&mut self, identity: &str) -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(transient)?;
        writeln!(file, "{identity}").map_err(transient)?;
        file.sync_all().map_err(transient)?;
        self.verified.insert(identity.to_owned());
        Ok(())
    }
}

/// One pack and every distinct placement the catalogs read out of it. Two
/// entries that share a chunk share its proof.
struct PackPlan {
    object: RemoteObject,
    chunks: BTreeMap<u64, wire::StoredChunk>,
}

struct Dependencies {
    snapshot_id: String,
    created_at_ms: u64,
    /// The root and every catalog node under it. Reading one proves it, so
    /// these are counted where they are read.
    metadata_objects: u64,
    metadata_bytes: u64,
    packs: BTreeMap<String, PackPlan>,
}

fn library_entry_kind(kind: wire::CatalogEntryKind) -> bool {
    matches!(
        kind,
        wire::CatalogEntryKind::Record | wire::CatalogEntryKind::Object
    )
}

async fn dependencies(
    root: &RemoteObject,
    root_key: &[u8; 32],
    staging: &Path,
    provider: &dyn Provider,
    repository: &RepositoryHandle,
    cancel: &Cancellation,
) -> Result<Dependencies> {
    let role = match root.role {
        ObjectRole::SyncState => wire::ObjectRole::SyncState,
        ObjectRole::BackupBundle => wire::ObjectRole::BackupBundle,
        _ => return Err(corrupt("checked root role")),
    };
    let path =
        snapshot_restore::open_object(root, root_key, staging, provider, repository, cancel).await?;
    let bytes = snapshot_restore::read_bytes(&path, wire::MAX_METADATA_BYTES)?;
    let document = super::control::SnapshotView::read(&bytes, role, &root.repository_id)?;
    if root.object_id != format!("snapshot-{}", document.snapshot_id) {
        return Err(corrupt("snapshot identity differs"));
    }
    let mut catalogs = vec![
        (
            wire::CatalogKind::Records,
            RemoteObject::from_stored(&document.library.record_catalog, repository)?,
        ),
        (
            wire::CatalogKind::Assets,
            RemoteObject::from_stored(&document.library.asset_catalog, repository)?,
        ),
    ];
    for reference in document.sections.values() {
        catalogs.push((
            wire::CatalogKind::Section,
            RemoteObject::from_stored(&reference.entries_root, repository)?,
        ));
    }
    let mut packs: BTreeMap<String, PackPlan> = BTreeMap::new();
    let mut metadata_objects = 1;
    let mut metadata_bytes = root.receipt.byte_length;
    for (kind, catalog) in catalogs {
        if catalog.repository_id != root.repository_id {
            return Err(corrupt("catalog repository differs"));
        }
        let (entries, referenced, nodes) = snapshot_restore::read_catalog(
            &catalog, kind, root_key, staging, provider, repository, cancel,
        )
        .await?;
        metadata_objects += nodes.len() as u64;
        metadata_bytes += nodes
            .iter()
            .map(|(node, _)| node.receipt.byte_length)
            .sum::<u64>();
        for entry in &entries {
            cancel.check()?;
            if library_entry_kind(entry.kind) != (kind != wire::CatalogKind::Section) {
                return Err(corrupt("entry kind outside its catalog"));
            }
            let mut length = 0u64;
            for chunk in &entry.chunks {
                let object = referenced
                    .get(&chunk.pack_id)
                    .ok_or_else(|| corrupt("catalog pack is missing"))?;
                if chunk
                    .offset
                    .checked_add(chunk.stored_length)
                    .is_none_or(|end| end > object.plaintext_length)
                {
                    return Err(corrupt("chunk escaped pack"));
                }
                length = length
                    .checked_add(chunk.plaintext_length)
                    .ok_or_else(|| corrupt("entry length overflow"))?;
                let plan = packs
                    .entry(chunk.pack_id.clone())
                    .or_insert_with(|| PackPlan {
                        object: object.clone(),
                        chunks: BTreeMap::new(),
                    });
                if plan.object != *object {
                    return Err(corrupt("conflicting catalog pack"));
                }
                if plan
                    .chunks
                    .insert(chunk.offset, chunk.clone())
                    .is_some_and(|previous| previous != *chunk)
                {
                    return Err(corrupt("conflicting chunk placement"));
                }
            }
            if length != entry.byte_length {
                return Err(corrupt("entry length differs"));
            }
        }
    }
    Ok(Dependencies {
        snapshot_id: document.snapshot_id,
        created_at_ms: document.created_at_ms,
        metadata_objects,
        metadata_bytes,
        packs,
    })
}

/// Reads every placement the catalogs named out of one opened pack. This is
/// what a restore does with the same bytes, without keeping the result.
fn verify_chunks(path: &Path, chunks: &[wire::StoredChunk]) -> Result<()> {
    let mut input = crate::trust_boundary::open_regular_source(path).map_err(corrupt)?;
    let length = input.metadata().map_err(corrupt)?.len();
    for chunk in chunks {
        if chunk
            .offset
            .checked_add(chunk.stored_length)
            .is_none_or(|end| end > length)
        {
            return Err(corrupt("chunk escaped pack"));
        }
        input.seek(SeekFrom::Start(chunk.offset)).map_err(corrupt)?;
        let mut limited = (&mut input).take(chunk.stored_length);
        let decoded = pack::read_entry(&mut limited, pack::MAX_CHUNK_BYTES).map_err(corrupt)?;
        if limited.limit() != 0
            || decoded.hash != chunk.plaintext_sha256
            || decoded.bytes.len() as u64 != chunk.plaintext_length
        {
            return Err(corrupt("pack chunk differs"));
        }
    }
    Ok(())
}

async fn verify_pack(
    plan: &PackPlan,
    root_key: &[u8; 32],
    staging: &Path,
    provider: &dyn Provider,
    repository: &RepositoryHandle,
    cancel: &Cancellation,
) -> Result<()> {
    if plan.object.role != ObjectRole::Pack {
        return Err(corrupt("catalog pack role"));
    }
    let path = snapshot_restore::open_object(
        &plan.object,
        root_key,
        staging,
        provider,
        repository,
        cancel,
    )
    .await?;
    let chunks = plan.chunks.values().cloned().collect::<Vec<_>>();
    let cpu = cpu_permit().await?;
    let proved = tokio::task::spawn_blocking(move || verify_chunks(&path, &chunks))
        .await
        .map_err(transient)?;
    drop(cpu);
    proved
}

/// Proves the selected root can still be read. Damage is reported and costs
/// the object its local reuse evidence; everything else is the caller's error
/// to classify, so a slow provider or an exhausted budget stops the job
/// instead of being counted as a repository fault.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn check(
    root: &RemoteObject,
    root_key: &[u8; 32],
    directory: &Path,
    cache_root: &Path,
    provider: &dyn Provider,
    repository: &RepositoryHandle,
    progress: &(dyn Fn(u64, u64, u64, u64) + Sync),
    cancel: &Cancellation,
) -> Result<CheckOutcome> {
    let staging = directory.join(STAGING_DIRECTORY);
    let plan = dependencies(root, root_key, &staging, provider, repository, cancel).await?;
    let total_objects = plan.metadata_objects + plan.packs.len() as u64;
    let total_bytes = plan.metadata_bytes
        + plan
            .packs
            .values()
            .map(|pack| pack.object.receipt.byte_length)
            .sum::<u64>();
    let mut ledger = Ledger::open(directory)?;
    let mut outcome = CheckOutcome {
        snapshot_id: plan.snapshot_id,
        created_at_ms: plan.created_at_ms,
        metadata_objects: plan.metadata_objects,
        verified_objects: plan.metadata_objects,
        verified_bytes: plan.metadata_bytes,
        damaged_objects: 0,
        damaged: Vec::new(),
    };
    progress(
        outcome.verified_objects,
        total_objects,
        outcome.verified_bytes,
        total_bytes,
    );
    for pack in plan.packs.values() {
        cancel.check()?;
        let identity = reachability::object_identity(&pack.object, repository)?;
        if !ledger.holds(&identity) {
            let proved = verify_pack(pack, root_key, &staging, provider, repository, cancel).await;
            snapshot_restore::discard_object(&staging, &pack.object);
            match proved {
                Ok(()) => ledger.record(&identity)?,
                Err(error)
                    if matches!(error.kind, ErrorKind::Corrupt | ErrorKind::NotFound) =>
                {
                    package_cache::forget_remote_object(cache_root, repository, &pack.object)?;
                    outcome.damaged_objects += 1;
                    if outcome.damaged.len() < REPORTED_DAMAGE {
                        let damage = if error.kind == ErrorKind::NotFound {
                            Damage::Missing
                        } else {
                            Damage::Corrupt
                        };
                        outcome.damaged.push((pack.object.object_id.clone(), damage));
                    }
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        outcome.verified_objects += 1;
        outcome.verified_bytes += pack.object.receipt.byte_length;
        progress(
            outcome.verified_objects,
            total_objects,
            outcome.verified_bytes,
            total_bytes,
        );
    }
    let _ = fs::remove_dir_all(&staging);
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_storage::{
        capture::CaptureCatalog,
        fake::{self, FakeProvider},
        journal::{JobIdentity, TransferJournal},
        packaging::{
            package_and_upload, CatalogRoot, CompletedSnapshot, PackageLimits, ParentGraph,
            SnapshotMetadata, SnapshotPurpose,
        },
    };
    use crate::{
        asset_repository::PayloadCas,
        persistent_store::{
            content_capture::ContentCaptureSink, external_capture::CapturedSnapshot,
            sync_selection::CaptureIdentity,
        },
    };
    use risunest_external_storage_format::format::library_fingerprint_domain;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn identity(revision: i64) -> CaptureIdentity {
        CaptureIdentity {
            store_id: "store".into(),
            library_epoch: "epoch".into(),
            generation: "generation".into(),
            selection_epoch: "selection".into(),
            revision,
        }
    }

    /// Records that do not compress, so a publication under a small pack limit
    /// has to spread them over several packs.
    fn captured(root: &Path, records: usize, revision: i64) -> CapturedSnapshot {
        let cas = PayloadCas::new(root).unwrap();
        let asset = cas.prepare_bytes(b"synthetic referenced asset").unwrap();
        let external = root.join("external-storage");
        let mut catalog = CaptureCatalog::create(
            &external.join("captures").join(format!("check-{revision}")),
            &external,
            None,
        )
        .unwrap();
        catalog.begin(&identity(revision), None).unwrap();
        for index in 0..records {
            let mut body = String::new();
            let mut value = index as u64 + 1;
            while body.len() < 4096 {
                value = value.wrapping_mul(6364136223846793005).wrapping_add(1);
                body.push_str(&format!("{value:016x}"));
            }
            catalog
                .record(&format!("record/{index:06}"), body.as_bytes())
                .unwrap();
        }
        catalog
            .reference("record/000000", &asset.content_hash, asset.byte_size)
            .unwrap();
        catalog.finish().unwrap();
        CapturedSnapshot {
            id: format!("check-{revision}"),
            identity: identity(revision),
            catalog,
            projected_records: records,
            shared: false,
        }
    }

    async fn published(
        root: &Path,
        provider: &FakeProvider,
        repository: &RepositoryHandle,
        records: usize,
        revision: i64,
        parent: Option<&ParentGraph>,
    ) -> CompletedSnapshot {
        let capture = captured(root, records, revision);
        let fingerprint = capture
            .catalog
            .content_fingerprint(&library_fingerprint_domain())
            .unwrap();
        let mut journal = TransferJournal::open(
            &root.join(format!("journal-{revision}")),
            JobIdentity {
                job_id: format!("check-publication-{revision}"),
                connection_id: "check-connection".into(),
                repository_id: repository.repository_id.clone(),
                capture_id: capture.id.clone(),
                capture: identity(revision),
            },
        )
        .unwrap();
        package_and_upload(
            capture,
            Vec::new(),
            root,
            &root.join("package-cache"),
            SnapshotMetadata {
                snapshot_id: format!("checked-{revision}"),
                repository_id: "format-repository".into(),
                library_id: identity(revision).library_epoch.clone(),
                author_device_id: identity(revision).store_id.clone(),
                created_at_ms: 7,
                logical_revision: revision as u64,
                purpose: SnapshotPurpose::SyncState {
                    epoch: "epoch".into(),
                    generation: risunest_sync_wire::head::Sequence::from(revision as u64),
                    parent_sections: BTreeMap::new(),
                },
                parent_snapshot_id: None,
                content_fingerprint: fingerprint,
            },
            &[9; 32],
            PackageLimits {
                max_stored_bytes: 64 * 1024,
                sdk_overhead_bytes: 0,
                target_plaintext_bytes: 64 * 1024,
            },
            parent,
            &mut journal,
            provider,
            repository,
            &Cancellation::default(),
        )
        .await
        .unwrap()
    }

    fn parent_graph(
        previous: &CompletedSnapshot,
        repository: &RepositoryHandle,
    ) -> ParentGraph {
        ParentGraph::new(
            vec![
                (
                    CatalogRoot::Records,
                    previous.record_catalog.stored(repository).unwrap(),
                ),
                (
                    CatalogRoot::Assets,
                    previous.asset_catalog.stored(repository).unwrap(),
                ),
            ],
            repository,
        )
        .unwrap()
    }

    /// Every pack the record catalog of a published state points at.
    async fn record_packs(
        snapshot: &CompletedSnapshot,
        staging: &Path,
        provider: &FakeProvider,
        repository: &RepositoryHandle,
    ) -> BTreeSet<String> {
        snapshot_restore::read_catalog(
            &snapshot.record_catalog,
            wire::CatalogKind::Records,
            &[9; 32],
            staging,
            provider,
            repository,
            &Cancellation::default(),
        )
        .await
        .unwrap()
        .1
        .into_keys()
        .collect()
    }

    fn stored_pack(provider: &FakeProvider) -> String {
        provider
            .uploaded_ids()
            .into_iter()
            .find(|id| id.starts_with("pack-"))
            .expect("a publication of this size stores packs")
    }

    fn silent() -> impl Fn(u64, u64, u64, u64) + Sync {
        |_, _, _, _| {}
    }

    #[test]
    fn c_a_published_root_proves_every_pack_it_names_and_keeps_none_of_them() {
        runtime().block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let provider = FakeProvider::new(false);
            let repository = fake::repository();
            let root = published(directory.path(), &provider, &repository, 24, 1, None)
                .await
                .reference;
            let job = directory.path().join("check-job");
            let packs = provider
                .uploaded_ids()
                .into_iter()
                .filter(|id| id.starts_with("pack-"))
                .collect::<BTreeSet<_>>();
            let outcome = check(
                &root,
                &[9; 32],
                &job,
                &directory.path().join("package-cache"),
                &provider,
                &repository,
                &silent(),
                &Cancellation::default(),
            )
            .await
            .unwrap();
            assert!(packs.len() > 1, "one pack would not prove deduplication");
            assert_eq!(
                outcome.verified_objects - outcome.metadata_objects,
                packs.len() as u64
            );
            assert!(outcome.metadata_objects > 2, "the root and its catalogs");
            assert_eq!(outcome.damaged_objects, 0);
            assert_eq!(outcome.snapshot_id, "checked-1");
            assert_eq!(outcome.created_at_ms, 7);
            for pack in &packs {
                assert_eq!(provider.read_attempts(pack), 1, "{pack} was read twice");
            }
            assert!(!job.join(STAGING_DIRECTORY).exists(), "bodies were kept");
        });
    }

    #[test]
    fn c_a_resumed_check_reads_no_body_it_already_proved() {
        runtime().block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let provider = FakeProvider::new(false);
            let repository = fake::repository();
            let root = published(directory.path(), &provider, &repository, 24, 1, None)
                .await
                .reference;
            let job = directory.path().join("check-job");
            let cache = directory.path().join("package-cache");
            let pack = stored_pack(&provider);
            let first = check(
                &root, &[9; 32], &job, &cache, &provider, &repository, &silent(),
                &Cancellation::default(),
            )
            .await
            .unwrap();
            assert_eq!(provider.read_attempts(&pack), 1);
            let second = check(
                &root, &[9; 32], &job, &cache, &provider, &repository, &silent(),
                &Cancellation::default(),
            )
            .await
            .unwrap();
            assert_eq!(second.verified_objects, first.verified_objects);
            assert_eq!(second.verified_bytes, first.verified_bytes);
            assert_eq!(
                provider.read_attempts(&pack),
                1,
                "a resumed check downloaded a body it had already proved"
            );
            fs::remove_file(job.join(LEDGER_FILE)).unwrap();
            check(
                &root, &[9; 32], &job, &cache, &provider, &repository, &silent(),
                &Cancellation::default(),
            )
            .await
            .unwrap();
            assert_eq!(
                provider.read_attempts(&pack),
                2,
                "without its record the check has to read the body again"
            );
        });
    }

    #[test]
    fn c_a_body_that_changed_under_the_catalog_is_reported_and_loses_its_reuse_evidence() {
        runtime().block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let provider = FakeProvider::new(false);
            let repository = fake::repository();
            let root = published(directory.path(), &provider, &repository, 24, 1, None)
                .await
                .reference;
            let cache = directory.path().join("package-cache");
            let pack = stored_pack(&provider);
            let known =
                package_cache::known_remote_objects(&cache, &root.repository_id, &repository)
                    .unwrap();
            assert!(
                known.iter().any(|object| object.object_id == pack),
                "the publication recorded the pack it uploaded"
            );
            let mut damaged = provider.contents(&pack).expect("an uploaded body");
            let last = damaged.len() - 1;
            damaged[last] ^= 0x01;
            provider.seed(&pack, ObjectRole::Pack, damaged);
            let outcome = check(
                &root,
                &[9; 32],
                &directory.path().join("check-job"),
                &cache,
                &provider,
                &repository,
                &silent(),
                &Cancellation::default(),
            )
            .await
            .unwrap();
            assert_eq!(outcome.damaged_objects, 1);
            assert_eq!(outcome.damaged[0].0, pack);
            assert_eq!(outcome.damaged[0].1, Damage::Corrupt);
            let remaining =
                package_cache::known_remote_objects(&cache, &root.repository_id, &repository)
                    .unwrap();
            assert!(
                !remaining.iter().any(|object| object.object_id == pack),
                "a damaged pack kept the evidence a publication reuses it by"
            );
        });
    }

    #[test]
    fn c_a_body_the_repository_no_longer_holds_is_reported_as_missing() {
        runtime().block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let provider = FakeProvider::new(false);
            let repository = fake::repository();
            let root = published(directory.path(), &provider, &repository, 24, 1, None)
                .await
                .reference;
            let pack = stored_pack(&provider);
            provider.forget(&pack);
            let outcome = check(
                &root,
                &[9; 32],
                &directory.path().join("check-job"),
                &directory.path().join("package-cache"),
                &provider,
                &repository,
                &silent(),
                &Cancellation::default(),
            )
            .await
            .unwrap();
            assert_eq!(outcome.damaged_objects, 1);
            assert_eq!(outcome.damaged[0].0, pack);
            assert_eq!(outcome.damaged[0].1, Damage::Missing);
        });
    }

    #[test]
    fn c_a_provider_that_cannot_answer_stops_the_check_instead_of_reporting_damage() {
        runtime().block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let provider = FakeProvider::new(false);
            let repository = fake::repository();
            let root = published(directory.path(), &provider, &repository, 24, 1, None)
                .await
                .reference;
            let pack = stored_pack(&provider);
            provider.fail_read(&pack, ErrorKind::Transient);
            let error = check(
                &root,
                &[9; 32],
                &directory.path().join("check-job"),
                &directory.path().join("package-cache"),
                &provider,
                &repository,
                &silent(),
                &Cancellation::default(),
            )
            .await
            .expect_err("a transient read is not a repository fault");
            assert_eq!(error.kind, ErrorKind::Transient);
        });
    }

    #[test]
    fn c_the_root_a_check_started_on_stays_its_root() {
        runtime().block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let provider = FakeProvider::new(false);
            let repository = fake::repository();
            let root = published(directory.path(), &provider, &repository, 4, 1, None)
                .await
                .reference;
            let job = directory.path().join("check-job");
            let mut newer = root.clone();
            newer.object_id = "snapshot-newer".into();
            assert!(pinned_root(&job, &repository).unwrap().is_none());
            assert_eq!(
                pin_root(&job, &root, &repository).unwrap().object_id,
                root.object_id
            );
            assert_eq!(
                pin_root(&job, &newer, &repository).unwrap().object_id,
                root.object_id,
                "a resumed check followed a newer publication"
            );
            let read_back = pinned_root(&job, &repository).unwrap().unwrap();
            assert_eq!(
                reachability::object_identity(&read_back, &repository).unwrap(),
                reachability::object_identity(&root, &repository).unwrap(),
                "the root read back is not the one the check started on"
            );
        });
    }

    #[test]
    fn c_a_publication_after_a_check_does_not_inherit_the_damaged_body() {
        runtime().block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let provider = FakeProvider::new(false);
            let repository = fake::repository();
            let first = published(directory.path(), &provider, &repository, 24, 1, None).await;
            let read = directory.path().join("read");
            let before = record_packs(&first, &read, &provider, &repository).await;
            let damaged = before.iter().next().expect("a record pack").clone();
            let mut bytes = provider.contents(&damaged).expect("an uploaded body");
            let last = bytes.len() - 1;
            bytes[last] ^= 0x01;
            provider.seed(&damaged, ObjectRole::Pack, bytes);
            let outcome = check(
                &first.reference,
                &[9; 32],
                &directory.path().join("check-job"),
                &directory.path().join("package-cache"),
                &provider,
                &repository,
                &silent(),
                &Cancellation::default(),
            )
            .await
            .unwrap();
            assert_eq!(outcome.damaged_objects, 1);
            let parent = parent_graph(&first, &repository);
            let second =
                published(directory.path(), &provider, &repository, 24, 2, Some(&parent)).await;
            let after = record_packs(&second, &read.join("second"), &provider, &repository).await;
            assert!(
                !after.contains(&damaged),
                "a publication inherited a body the check found damaged"
            );
            assert!(
                after.len() >= before.len(),
                "the entries of the damaged pack were dropped instead of stored again"
            );
        });
    }

    #[test]
    fn c_a_publication_without_a_check_keeps_reusing_the_damaged_body() {
        runtime().block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let provider = FakeProvider::new(false);
            let repository = fake::repository();
            let first = published(directory.path(), &provider, &repository, 24, 1, None).await;
            let read = directory.path().join("read");
            let before = record_packs(&first, &read, &provider, &repository).await;
            let damaged = before.iter().next().expect("a record pack").clone();
            let mut bytes = provider.contents(&damaged).expect("an uploaded body");
            let last = bytes.len() - 1;
            bytes[last] ^= 0x01;
            provider.seed(&damaged, ObjectRole::Pack, bytes);
            let parent = parent_graph(&first, &repository);
            let second =
                published(directory.path(), &provider, &repository, 24, 2, Some(&parent)).await;
            let after = record_packs(&second, &read.join("second"), &provider, &repository).await;
            assert!(
                after.contains(&damaged),
                "nothing but the check tells a publication the body changed"
            );
        });
    }
}
