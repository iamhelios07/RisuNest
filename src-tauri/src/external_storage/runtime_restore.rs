//! Manual external snapshot restore and durable commit recovery.
//! Remote reads finish before either the library writer fence or PDS is opened.
use super::{
    connection_commands::ConnectedRepository,
    connection_store::ConnectionStore,
    contract::{Cancellation, ErrorKind, ProviderError, Result},
    job_store::{DurableJob, JobKind, JobStore},
    runtime,
    snapshot_restore::{self, PreparedRemoteSnapshot},
};
use crate::persistent_store::{
    commands::PersistentStoreState,
    external_apply::{ExternalSnapshotApplication, ExternalSnapshotObject, ExternalSnapshotRecord},
    PersistentStore, StoreError,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path};
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, PartialEq, Eq)]
struct RestoreSelection {
    library: bool,
    sections: BTreeSet<String>,
}

const RESTORE_COMMIT_SCHEMA: &str = "risunest.external-restore-commit/v1";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RestoreCommitMarker {
    schema: String,
    job_id: String,
    connection_id: String,
    snapshot_id: String,
    expected_revision: String,
    received_revision: String,
}

fn restore_marker_key(job: &str) -> String {
    format!("external-restore-commit:{job}")
}

fn restore_marker(job: &DurableJob, expected_revision: i64) -> Result<RestoreCommitMarker> {
    let received_revision = expected_revision
        .checked_add(1)
        .ok_or_else(|| ProviderError::new(ErrorKind::PreconditionFailed))?;
    Ok(RestoreCommitMarker {
        schema: RESTORE_COMMIT_SCHEMA.into(),
        job_id: job.id.clone(),
        connection_id: job.request.connection_id.clone(),
        snapshot_id: job.request.snapshot_id.clone().ok_or_else(corrupt)?,
        expected_revision: expected_revision.to_string(),
        received_revision: received_revision.to_string(),
    })
}

fn completed_restore_in_store(store: &PersistentStore, job: &DurableJob) -> Result<Option<Value>> {
    if job.request.kind != JobKind::Restore {
        return Ok(None);
    }
    let expected_revision = job
        .request
        .target_revision
        .as_deref()
        .ok_or_else(corrupt)?
        .parse::<i64>()
        .map_err(|_| corrupt())?;
    let Some(value) = store
        .get_app_kv(&restore_marker_key(&job.id))
        .map_err(pds_error)?
    else {
        return Ok(None);
    };
    let marker: RestoreCommitMarker = serde_json::from_value(value).map_err(|_| corrupt())?;
    let expected = restore_marker(job, expected_revision)?;
    if marker != expected
        || store.revision().map_err(pds_error)?
            < marker
                .received_revision
                .parse::<i64>()
                .map_err(|_| corrupt())?
    {
        return Err(corrupt());
    }
    Ok(Some(json!({
        "snapshotId":marker.snapshot_id,
        "receivedRevision":marker.received_revision
    })))
}

/// Read-only completion recovery used before reopening a provider connection.
pub(crate) fn completed_restore(app: &AppHandle, job: &DurableJob) -> Result<Option<Value>> {
    completed_restore_in_store(&runtime::native_store(app)?, job)
}

pub(crate) fn application_started(job: &DurableJob) -> bool {
    job.request.kind == JobKind::Restore && job.summary["applicationStarted"] == true
}

fn mark_application_started(root: &Path, job: &DurableJob) -> Result<()> {
    let jobs = JobStore::open(root)?;
    let mut current = jobs.read(&job.id)?;
    current.summary["applicationStarted"] = json!(true);
    current.summary["phase"] = json!("applying-local");
    current.summary["updatedAtMs"] = json!(runtime::now_ms().to_string());
    jobs.put(&current)
}

fn corrupt() -> ProviderError {
    ProviderError::new(ErrorKind::Corrupt)
}

fn pds_error(error: StoreError) -> ProviderError {
    match error {
        StoreError::RevisionConflict { .. } => ProviderError::new(ErrorKind::PreconditionFailed),
        StoreError::Validation { .. } => corrupt(),
        _ => ProviderError::new(ErrorKind::Transient),
    }
}

fn decode_hash(value: &str) -> Result<[u8; 32]> {
    if !crate::trust_boundary::is_lower_hex_256(value) {
        return Err(corrupt());
    }
    hex::decode(value)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(corrupt)
}

const FULL_RESTORE_AREAS: [&str;5]=["library","referencedAssets","hypa","local-plugins","local-settings"];
fn restore_selection(restore_areas:Option<&[String]>)->Result<RestoreSelection> {
    if let Some(areas)=restore_areas {
        let unique=areas.iter().map(String::as_str).collect::<BTreeSet<_>>();
        if unique.len()!=areas.len() || unique!=FULL_RESTORE_AREAS.into_iter().collect() {return Err(corrupt());}
    }
    Ok(RestoreSelection{library:true,sections:BTreeSet::from(["hypa".into(),"local-plugins".into(),"local-settings".into()])})
}
fn require_restorable_sections(_selection:&RestoreSelection,snapshot:&PreparedRemoteSnapshot,_store_id:&str)->Result<()> {
    if snapshot.captured_by_device.as_deref().is_none_or(str::is_empty) {return Err(ProviderError::new(ErrorKind::PreconditionFailed));}
    Ok(())
}
fn checked_required_bytes(
    snapshot: &PreparedRemoteSnapshot,
    selection: &RestoreSelection,
) -> Result<u64> {
    let mut total = 0u64;
    let mut add = |length: u64| -> Result<()> {
        total = total.checked_add(length).ok_or_else(corrupt)?;
        Ok(())
    };
    if selection.library {
        for record in &snapshot.records {
            add(record.byte_length)?;
        }
        for object in &snapshot.objects {
            add(object.byte_length)?;
        }
    }
    Ok(total)
}

#[cfg(windows)]
fn available_space(path: &Path) -> std::io::Result<u64> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    wide.push(0);
    let mut available = 0u64;
    let result = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(available)
    }
}

#[cfg(unix)]
fn available_space(path: &Path) -> std::io::Result<u64> {
    use std::{ffi::CString, mem::MaybeUninit, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid path"))?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let stats = unsafe { stats.assume_init() };
    Ok((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

fn validate_download(
    connected: &ConnectedRepository,
    requested_snapshot: &str,
    snapshot: &PreparedRemoteSnapshot,
) -> Result<()> {
    if snapshot.snapshot_id != requested_snapshot
        || snapshot.repository_id != connected.stored.descriptor.repository_id
    {
        return Err(corrupt());
    }
    decode_hash(&snapshot.fingerprint)?;
    decode_hash(&snapshot.library_fingerprint)?;
    Ok(())
}

fn update_phase(root: &Path, job: &DurableJob, phase: &str) -> Result<()> {
    let store = JobStore::open(root)?;
    let mut current = store.read(&job.id)?;
    if current.request.connection_id != job.request.connection_id || current.terminal() {
        return Err(ProviderError::new(ErrorKind::PreconditionFailed));
    }
    current.summary["state"] = json!("running");
    current.summary["phase"] = json!(phase);
    current.summary["updatedAtMs"] = json!(runtime::now_ms().to_string());
    store.put(&current)
}

pub(crate) async fn run_restore(
    app: &AppHandle,
    connected: &ConnectedRepository,
    job: &DurableJob,
    cancel: &Cancellation,
) -> Result<Value> {
    if job.request.kind != JobKind::Restore {
        return Err(corrupt());
    }
    if let Some(result) = completed_restore(app, job)? {
        return Ok(result);
    }
    let snapshot_id = job.request.snapshot_id.as_deref().ok_or_else(corrupt)?;
    let expected_revision = job
        .request
        .target_revision
        .as_deref()
        .ok_or_else(corrupt)?
        .parse::<i64>()
        .map_err(|_| corrupt())?;
    let root = runtime::root(app)?;
    update_phase(&root, job, "downloading")?;
    let known = match ConnectionStore::open(&root)?
        .discovery_snapshot(&job.request.connection_id, snapshot_id)
    {
        Ok(value) => Some(value),
        Err(error) if error.kind == ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let cache_root = root.clone();
    let cache_connection = job.request.connection_id.clone();
    let cache_id = snapshot_id.to_owned();
    let remote = super::control::find_snapshot_with_locator_invalidation(
        connected,
        snapshot_id,
        known.as_ref(),
        move || {
            ConnectionStore::open(&cache_root)?.forget_discovery(&cache_connection, &cache_id)
        },
        cancel,
    )
    .await?;
    ConnectionStore::open(&root)?.remember_discovery(
        &job.request.connection_id,
        snapshot_id,
        &remote,
    )?;
    let staging_root = staging_directory(&root, job);
    let transferred = runtime::transfer_progress(&root, &job.id);
    let snapshot = snapshot_restore::download_snapshot(
        &remote,
        &staging_root,
        &connected.root_key,
        None,
        snapshot_restore::SourceTrust::ProvenLibrary(&root),
        connected.provider.as_ref(),
        &connected.handle,
        &transferred,
        cancel,
    )
    .await?;
    cancel.check()?;
    validate_download(connected, snapshot_id, &snapshot)?;
    let selection = restore_selection(job.request.restore_areas.as_deref())?;
    require_restorable_sections(&selection, &snapshot, &job.admission_identity.store_id)?;
    let sections = snapshot_restore::download_sections(
        &remote,
        &selection.sections,
        &staging_root,
        &connected.root_key,
        None,
        connected.provider.as_ref(),
        &connected.handle,
        &transferred,
        cancel,
    )
    .await?;
    transferred.flush();
    if sections.len() != selection.sections.len() {
        return Err(ProviderError::new(ErrorKind::NotFound));
    }
    let required = checked_required_bytes(&snapshot, &selection)?;
    if available_space(&staging_root).map_err(runtime::local_error)? < required {
        return Err(ProviderError::new(ErrorKind::StorageFull));
    }
    update_phase(&root, job, "preparing-local")?;

    let worker_app = app.clone();
    let worker_job = job.clone();
    let worker_cancel = cancel.clone();
    let result = tokio::task::spawn_blocking(move || {
        prepare_local_restore(
            &worker_app,
            &worker_job,
            expected_revision,
            snapshot,
            selection,
            sections,
            worker_cancel,
        )
    })
    .await
    .map_err(runtime::local_error)??;
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn prepare_local_restore(
    app: &AppHandle,
    job: &DurableJob,
    expected_revision: i64,
    snapshot: PreparedRemoteSnapshot,
    selection: RestoreSelection,
    sections: Vec<super::sections::CapturedSection>,
    cancel: Cancellation,
) -> Result<Value> {
    cancel.check()?;
    let admission = app
        .state::<crate::native_file_jobs::NativeFileJobState>()
        .admission
        .clone();
    let permit = admission.file(true).map_err(runtime::local_error)?;
    // Open a dedicated native connection while renderer admission is still
    // available. It remains owned across the maintenance WebView reload.
    let mut store = runtime::native_store(app)?;
    let maintenance = app
        .state::<PersistentStoreState>()
        .acquire_device_maintenance()
        .map_err(pds_error)?;
    cancel.check()?;
    if store.revision().map_err(pds_error)? != expected_revision {
        return Err(ProviderError::new(ErrorKind::PreconditionFailed));
    }
    let snapshot_id = snapshot.snapshot_id.clone();
    let staging_root = snapshot.staging_root.clone();
    let prepared = if selection.library {
        let scope_id = risunest_external_storage_format::format::library_fingerprint_domain();
        let fingerprint = decode_hash(&snapshot.library_fingerprint)?;
        let records = snapshot.records.into_iter().map(|record| {
            Ok(ExternalSnapshotRecord {
                key: record.key,
                content_hash: record.content_hash,
                byte_length: record.byte_length,
                source: record.source,
            })
        });
        let objects = snapshot.objects.into_iter().map(|object| {
            Ok(ExternalSnapshotObject {
                content_hash: object.content_hash,
                byte_length: object.byte_length,
                source: object.source,
            })
        });
        let probe = runtime::CancelProbe(cancel.clone());
        Some(
            store
                .prepare_external_snapshot_application(
                    &ExternalSnapshotApplication {
                        expected_revision,
                        staging_root: &staging_root,
                        scope_id: &scope_id,
                        fingerprint: &fingerprint,
                        probe: &probe,
                    },
                    records,
                    objects,
                )
                .map_err(|error| {
                    if cancel.check().is_err() {
                        ProviderError::new(ErrorKind::Cancelled)
                    } else {
                        pds_error(error)
                    }
                })?,
        )
    } else {
        None
    };

    let prepared = prepared.ok_or_else(corrupt)?;
    // Preparing every selected section before touching the device file keeps a
    // bundle with a missing object from installing half of itself.
    let prepared_sections =
        super::sections::prepare_received_backup_sections(&sections, &cancel)?;
    let marker = restore_marker(job, expected_revision)?;
    let marker_value = serde_json::to_value(&marker).map_err(runtime::local_error)?;
    cancel.check()?;
    mark_application_started(&runtime::root(app)?, job)?;
    let device = store.device_store_mut().map_err(pds_error)?;
    for rows in &prepared_sections {
        device.restore_prepared_backup_section(rows).map_err(pds_error)?;
    }
    let revision = store
        .finish_prepared_replace_with_app_kv(
            prepared,
            &restore_marker_key(&job.id),
            &marker_value,
        )
        .map_err(pds_error)?;
    if revision.revision.to_string() != marker.received_revision {
        return Err(corrupt());
    }
    drop(maintenance);
    drop(permit);
    cleanup_staging(&staging_root);
    Ok(json!({
        "snapshotId": snapshot_id,
        "receivedRevision": revision.revision.to_string()
    }))
}

fn staging_directory(root: &Path, job: &DurableJob) -> std::path::PathBuf {
    runtime::job_directory(root, &job.request.connection_id, &job.id).join("restore-snapshot")
}

/// A finished restore never reads its download again, whether it succeeded,
/// failed or was cancelled.
pub(crate) fn discard_finished_staging(root: &Path, job: &DurableJob) {
    if job.request.kind == JobKind::Restore && job.terminal() {
        cleanup_staging(&staging_directory(root, job));
    }
}

fn cleanup_staging(path: &Path) {
    let safe = std::fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| {
            metadata.is_dir() && !crate::trust_boundary::is_link_like(&metadata)
        });
    if safe {
        let _ = std::fs::remove_dir_all(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistent_store::sync_selection::CaptureIdentity;

    fn identity() -> CaptureIdentity {
        CaptureIdentity {
            store_id: "store".into(),
            library_epoch: "library".into(),
            generation: "generation".into(),
            selection_epoch: "selection".into(),
            revision: 0,
        }
    }

    #[test]
    fn local_application_evidence_survives_reopen_and_later_phase_updates() {
        let root = tempfile::tempdir().unwrap();
        let input = serde_json::from_value(json!({
            "connectionId":"synthetic-connection", "kind":"restore",
            "snapshotId":"synthetic-snapshot", "targetRevision":"0"
        })).unwrap();
        let job = DurableJob::new(input, false, 1, identity());
        JobStore::open(root.path()).unwrap().put(&job).unwrap();
        mark_application_started(root.path(), &job).unwrap();
        update_phase(root.path(), &job, "downloading").unwrap();
        let reopened = JobStore::open(root.path()).unwrap().read(&job.id).unwrap();
        assert!(application_started(&reopened));
        assert_eq!(reopened.summary["phase"], "downloading");
        let mut store = PersistentStore::open(root.path()).unwrap();
        assert!(completed_restore_in_store(&store, &reopened).unwrap().is_none());
        let stage = store.replace_begin().unwrap();
        store.replace_put_root(&stage.staging_id, &json!({"marker":"restored"})).unwrap();
        let prepared = store.prepare_replace_commit(&stage.staging_id, Some(0)).unwrap();
        let marker = serde_json::to_value(restore_marker(&job, 0).unwrap()).unwrap();
        store.finish_prepared_replace_with_app_kv(prepared, &restore_marker_key(&job.id), &marker).unwrap();
        for _ in 0..2 {
            assert_eq!(completed_restore_in_store(&store, &reopened).unwrap().unwrap()["receivedRevision"], "1");
        }
        assert_eq!(store.revision().unwrap(), 1);
    }

    #[test]
    fn restore_area_selection_matches_the_full_renderer_contract() {
        let areas=FULL_RESTORE_AREAS.map(str::to_owned);
        assert_eq!(restore_selection(None).unwrap(),restore_selection(Some(&areas)).unwrap());
        assert_eq!(restore_selection(None).unwrap().sections.len(),3);
    }
    #[test]
    fn restore_area_selection_rejects_partial_duplicate_and_undeclared_areas() {
        for areas in [vec!["library".into()],vec!["hypa".into()],vec!["devicePlugins".into()],vec!["library".into(),"library".into()],vec![]] {assert!(restore_selection(Some(&areas)).is_err());}
    }
    fn prepared(captured_by_device: Option<&str>) -> PreparedRemoteSnapshot {
        PreparedRemoteSnapshot {
            snapshot_id: "synthetic-snapshot".into(),
            repository_id: "synthetic-repository".into(),
            fingerprint: "00".repeat(32),
            library_fingerprint: "00".repeat(32),
            logical_revision: 1,
            staging_root: std::path::PathBuf::from("staging"),
            records: Vec::new(),
            objects: Vec::new(),
            captured_by_device: captured_by_device.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn complete_backup_restores_device_sections_across_devices_but_periodic_state_does_not() {
        let selection=restore_selection(None).unwrap();
        assert!(require_restorable_sections(&selection,&prepared(Some("other-device")),"this-device").is_ok());
        assert!(require_restorable_sections(&selection,&prepared(None),"this-device").is_err());
        assert!(require_restorable_sections(&selection,&prepared(Some("")),"this-device").is_err());
    }
    #[test]
    fn library_restore_marker_recovers_only_the_exact_durable_request() {
        let root = tempfile::tempdir().unwrap();
        let store = PersistentStore::open(root.path()).unwrap();
        let request = serde_json::from_value(json!({
            "connectionId":"synthetic-connection",
            "kind":"restore",
            "snapshotId":"synthetic-snapshot",
            "targetRevision":"0"
        }))
        .unwrap();
        let job = DurableJob::new(request, false, 1, identity());
        assert!(completed_restore_in_store(&store, &job).unwrap().is_none());
        let marker = restore_marker(&job, 0).unwrap();
        store
            .set_app_kv(
                &restore_marker_key(&job.id),
                &serde_json::to_value(marker).unwrap(),
            )
            .unwrap();
        let database =
            rusqlite::Connection::open(root.path().join("persistent/persistent.sqlite")).unwrap();
        database
            .execute("UPDATE meta SET value='1' WHERE key='currentRevision'", [])
            .unwrap();

        assert_eq!(
            completed_restore_in_store(&store, &job).unwrap().unwrap(),
            json!({"snapshotId":"synthetic-snapshot","receivedRevision":"1"})
        );
        let mut different = job.clone();
        different.request.snapshot_id = Some("different-snapshot".into());
        assert!(completed_restore_in_store(&store, &different).is_err());
    }
}
