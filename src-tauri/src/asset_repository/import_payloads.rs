use super::{job_pins::DurableCasJob, PayloadCas, PreparedPayload, StagedPayload};
use std::{
    io,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

const WORKERS: usize = 4;
const PUBLICATION_BATCH: usize = 32;

#[derive(Clone, Copy)]
pub(crate) enum ImportPinning {
    DuringPreparation,
    AfterContentMapping,
}

pub(crate) struct ImportPayload {
    pub path: PathBuf,
    pub hash: String,
    pub size: u64,
    pub sync_required: bool,
    pub progress_items: usize,
}

/// Workers only prepare private files. The caller publishes bounded batches and
/// owns progress, cancellation, and the durable job throughout the operation.
pub(crate) fn prepare_import_payloads(
    cas: &PayloadCas,
    durable: &mut DurableCasJob,
    inputs: &[ImportPayload],
    pinning: ImportPinning,
    cancelled: &dyn Fn() -> bool,
    progress: impl FnMut(u64, usize),
) -> io::Result<Vec<PreparedPayload>> {
    prepare_with(
        cas,
        durable,
        inputs,
        pinning,
        cancelled,
        progress,
        &|input, stopped, bytes| {
            cas.stage_import_payload(
                &input.path,
                &input.hash,
                input.size,
                input.sync_required,
                &|| stopped.load(Ordering::Acquire),
                &|count| {
                    bytes.fetch_add(count, Ordering::Relaxed);
                },
            )
        },
    )
}

fn prepare_with(
    cas: &PayloadCas,
    durable: &mut DurableCasJob,
    inputs: &[ImportPayload],
    pinning: ImportPinning,
    cancelled: &dyn Fn() -> bool,
    mut progress: impl FnMut(u64, usize),
    stage: &(impl Fn(&ImportPayload, &AtomicBool, &AtomicU64) -> io::Result<StagedPayload> + Sync),
) -> io::Result<Vec<PreparedPayload>> {
    if cancelled() {
        return Err(io::Error::other("import cancelled"));
    }
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let stopped = AtomicBool::new(false);
    let next = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let mut reported_bytes = 0;
    let mut completed = 0;
    let mut output: Vec<Option<PreparedPayload>> = inputs.iter().map(|_| None).collect();
    let mut failure = None;
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel(WORKERS);
        let mut workers = Vec::new();
        #[cfg(test)]
        let owner = std::thread::current().id();
        for _ in 0..WORKERS.min(inputs.len()) {
            let sender = sender.clone();
            let (stopped, next, bytes) = (&stopped, &next, &bytes);
            #[cfg(test)]
            let body_scope = super::body_io::capture_body_io_scope();
            workers.push(scope.spawn(move || {
                let run = || {
                    while !stopped.load(Ordering::Acquire) {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(input) = inputs.get(index) else {
                            break;
                        };
                        let result = stage(input, stopped, bytes);
                        #[cfg(test)]
                        let result = result.map(|mut payload| {
                            payload.transfer_observation_to(owner);
                            payload
                        });
                        let failed = result.is_err();
                        if sender.send((index, result)).is_err() {
                            break;
                        }
                        if failed {
                            stopped.store(true, Ordering::Release);
                            break;
                        }
                    }
                };
                #[cfg(test)]
                super::body_io::with_body_io_scope(body_scope, run);
                #[cfg(not(test))]
                run();
            }));
        }
        drop(sender);
        let mut pending: Vec<(usize, StagedPayload)> = Vec::with_capacity(PUBLICATION_BATCH);
        loop {
            if failure.is_none() && cancelled() {
                stopped.store(true, Ordering::Release);
                failure = Some(io::Error::other("import cancelled"));
            }
            let mut disconnected = false;
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok((index, Ok(payload))) if failure.is_none() => pending.push((index, payload)),
                Ok((_, Ok(_))) => {}
                Ok((_, Err(error))) => {
                    stopped.store(true, Ordering::Release);
                    if failure.is_none() {
                        failure = Some(error);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
            }
            if failure.is_none() && cancelled() {
                stopped.store(true, Ordering::Release);
                failure = Some(io::Error::other("import cancelled"));
            }
            if failure.is_none()
                && !pending.is_empty()
                && (pending.len() == PUBLICATION_BATCH || disconnected)
            {
                let (indices, staged): (Vec<_>, Vec<_>) = pending.drain(..).unzip();
                let published = match pinning {
                    ImportPinning::DuringPreparation => durable.publish_import_batch(cas, staged),
                    ImportPinning::AfterContentMapping => {
                        durable.publish_unmapped_content_batch(cas, staged)
                    }
                };
                match published {
                    Ok(payloads) => {
                        for (index, payload) in indices.into_iter().zip(payloads) {
                            output[index] = Some(payload);
                            completed += inputs[index].progress_items;
                        }
                    }
                    Err(error) => {
                        stopped.store(true, Ordering::Release);
                        failure = Some(error);
                    }
                }
            }
            let current_bytes = bytes.load(Ordering::Relaxed);
            progress(current_bytes - reported_bytes, completed);
            reported_bytes = current_bytes;
            if disconnected {
                break;
            }
        }
        // Drain and join even after cancellation; no worker may publish after abort.
        for worker in workers {
            if worker.join().is_err() && failure.is_none() {
                failure = Some(io::Error::other("import worker failed"));
            }
        }
    });
    if let Some(error) = failure {
        return Err(error);
    }
    if cancelled() {
        return Err(io::Error::other("import cancelled"));
    }
    output
        .into_iter()
        .map(|payload| payload.ok_or_else(|| io::Error::other("import preparation incomplete")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asset_repository::{
            body_io,
            job_pins::{CasJobKind, CasJobOwner, CasReleaseOutcome},
        },
        persistent_store::PersistentStore,
    };
    use sha2::{Digest, Sha256};
    use std::{fs, sync::Barrier};

    fn fixture(
        count: usize,
    ) -> (
        tempfile::TempDir,
        PayloadCas,
        DurableCasJob,
        Vec<ImportPayload>,
        String,
    ) {
        let root = tempfile::tempdir().unwrap();
        drop(PersistentStore::open(root.path()).unwrap());
        let directory = root.path().join("native-file-jobs/jobs/synthetic");
        fs::create_dir_all(&directory).unwrap();
        let inputs = (0..count)
            .map(|index| {
                let bytes = format!("synthetic asset {index}").into_bytes();
                let path = directory.join(format!("{index}.entry"));
                fs::write(&path, &bytes).unwrap();
                ImportPayload {
                    path,
                    hash: hex::encode(Sha256::digest(&bytes)),
                    size: bytes.len() as u64,
                    sync_required: true,
                    progress_items: 1,
                }
            })
            .collect();
        let cas = PayloadCas::new(root.path()).unwrap();
        let job_id = uuid::Uuid::new_v4().to_string();
        let durable = DurableCasJob::begin(
            root.path(),
            &job_id,
            CasJobKind::LocalBackupRestore,
            CasJobOwner::for_test(),
            0,
        )
        .unwrap();
        (root, cas, durable, inputs, job_id)
    }

    #[test]
    fn four_workers_reuse_files_preserve_order_and_publish_complete_pinned_batches() {
        let (root, cas, mut durable, inputs, job_id) = fixture(73);
        body_io::reset_body_io();
        for input in &inputs {
            body_io::register_object_purpose(&input.hash, body_io::BodyPurpose::Asset);
        }
        let active = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);
        let started = AtomicUsize::new(0);
        let barrier = Barrier::new(WORKERS);
        let mut reported = (0_u64, 0_usize);
        let output = prepare_with(
            &cas,
            &mut durable,
            &inputs,
            ImportPinning::DuringPreparation,
            &|| false,
            |bytes, count| {
                assert!(count >= reported.1 && count <= inputs.len());
                reported.0 += bytes;
                reported.1 = count;
            },
            &|input, stopped, bytes| {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(current, Ordering::SeqCst);
                if started.fetch_add(1, Ordering::SeqCst) < WORKERS {
                    barrier.wait();
                }
                let result = cas.stage_import_payload(
                    &input.path,
                    &input.hash,
                    input.size,
                    input.sync_required,
                    &|| stopped.load(Ordering::Acquire),
                    &|count| {
                        bytes.fetch_add(count, Ordering::Relaxed);
                    },
                );
                active.fetch_sub(1, Ordering::SeqCst);
                result
            },
        )
        .unwrap();
        assert_eq!(maximum.load(Ordering::SeqCst), WORKERS);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(
            reported,
            (inputs.iter().map(|input| input.size).sum(), inputs.len())
        );
        let work = body_io::take_body_io();
        assert!(work.complete(), "{work:?}");
        assert_eq!(work.worker_scopes_started, WORKERS as u64);
        assert_eq!(work.asset_work().staging_written_bytes, 0);
        for (index, (input, actual)) in inputs.iter().zip(output).enumerate() {
            assert_eq!(actual.content_hash, input.hash);
            assert_eq!(actual.byte_size, input.size);
            assert!(!input.path.exists());
            assert_eq!(
                cas.read_object(&input.hash).unwrap().unwrap(),
                format!("synthetic asset {index}").as_bytes()
            );
        }
        assert_eq!(durable.pin_count(), inputs.len());
        let reopened = DurableCasJob::open(root.path(), &job_id).unwrap();
        assert_eq!(reopened.pin_count(), inputs.len());
        drop(reopened);
        durable.release(CasReleaseOutcome::Aborted).unwrap();
    }

    #[test]
    fn cancellation_joins_workers_and_keeps_published_objects_in_the_recoverable_job() {
        let (_root, cas, mut durable, inputs, _) = fixture(100);
        let cancelled = AtomicBool::new(false);
        let result = prepare_import_payloads(
            &cas,
            &mut durable,
            &inputs,
            ImportPinning::DuringPreparation,
            &|| cancelled.load(Ordering::Acquire),
            |_, count| {
                if count >= PUBLICATION_BATCH {
                    cancelled.store(true, Ordering::Release);
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(durable.pin_count(), PUBLICATION_BATCH);
        let present = inputs
            .iter()
            .filter(|input| cas.stat_object(&input.hash).unwrap().is_some())
            .count();
        assert_eq!(present, PUBLICATION_BATCH);
        durable.release(CasReleaseOutcome::Aborted).unwrap();
    }

    #[test]
    fn bad_input_hash_prevents_publication_and_preserves_the_original_file() {
        let (_root, cas, mut durable, mut inputs, _) = fixture(8);
        inputs[0].hash = "0".repeat(64);
        let error = prepare_import_payloads(
            &cas,
            &mut durable,
            &inputs,
            ImportPinning::DuringPreparation,
            &|| false,
            |_, _| {}
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(inputs[0].path.exists());
        assert_eq!(durable.pin_count(), 0);
        assert!(inputs
            .iter()
            .all(|input| cas.stat_object(&input.hash).unwrap().is_none()));
        durable.release(CasReleaseOutcome::Aborted).unwrap();
    }

    #[test]
    fn duplicate_bodies_are_verified_and_keep_every_logical_result() {
        let (_root, cas, mut durable, mut inputs, _) = fixture(5);
        let bytes = fs::read(&inputs[0].path).unwrap();
        for index in 1..inputs.len() {
            fs::write(&inputs[index].path, &bytes).unwrap();
            inputs[index].hash = inputs[0].hash.clone();
            inputs[index].size = inputs[0].size;
        }
        let output = prepare_import_payloads(
            &cas,
            &mut durable,
            &inputs,
            ImportPinning::DuringPreparation,
            &|| false,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(output.len(), inputs.len());
        assert_eq!(
            output
                .iter()
                .filter(|payload| !payload.deduplicated)
                .count(),
            1
        );
        assert_eq!(durable.pin_count(), 1);
        assert!(inputs.iter().all(|input| !input.path.exists()));
        durable.release(CasReleaseOutcome::Aborted).unwrap();
    }

    #[test]
    fn existing_same_size_corruption_is_rejected_without_overwriting_it() {
        let (_root, cas, mut durable, inputs, _) = fixture(1);
        let bytes = fs::read(&inputs[0].path).unwrap();
        cas.prepare_bytes(&bytes).unwrap();
        let object = cas.object_path(&inputs[0].hash).unwrap().unwrap();
        let corrupt = vec![0x55; bytes.len()];
        fs::write(&object, &corrupt).unwrap();
        assert!(prepare_import_payloads(
            &cas,
            &mut durable,
            &inputs,
            ImportPinning::DuringPreparation,
            &|| false,
            |_, _| {}
        )
        .is_err());
        assert_eq!(fs::read(&object).unwrap(), corrupt);
        durable.release(CasReleaseOutcome::Aborted).unwrap();
    }

    #[test]
    fn import_custody_rejects_outside_paths_and_hard_linked_sources() {
        let (root, cas, mut durable, inputs, _) = fixture(1);
        let input = &inputs[0];
        let outside = root.path().join("outside.entry");
        fs::copy(&input.path, &outside).unwrap();
        assert!(cas
            .stage_import_payload(&outside, &input.hash, input.size, true, &|| false, &|_| {})
            .is_err());
        let linked = input.path.with_file_name("linked.entry");
        fs::hard_link(&input.path, &linked).unwrap();
        assert!(cas
            .stage_import_payload(
                &input.path,
                &input.hash,
                input.size,
                true,
                &|| false,
                &|_| {}
            )
            .is_err());
        assert!(outside.exists() && linked.exists() && input.path.exists());
        assert_eq!(durable.pin_count(), 0);
        durable.release(CasReleaseOutcome::Aborted).unwrap();
    }

    #[test]
    fn a_partial_publication_remains_pinned_and_recoverable() {
        let (root, cas, mut durable, inputs, job_id) = fixture(2);
        let original = fs::read(&inputs[1].path).unwrap();
        cas.prepare_bytes(&original).unwrap();
        let object = cas.object_path(&inputs[1].hash).unwrap().unwrap();
        fs::write(&object, vec![0x66; original.len()]).unwrap();
        let staged = inputs
            .iter()
            .map(|input| {
                cas.stage_import_payload(
                    &input.path,
                    &input.hash,
                    input.size,
                    true,
                    &|| false,
                    &|_| {},
                )
                .unwrap()
            })
            .collect();
        assert!(durable.publish_import_batch(&cas, staged).is_err());
        assert_eq!(
            cas.read_object(&inputs[0].hash).unwrap().unwrap(),
            b"synthetic asset 0"
        );
        let reopened = DurableCasJob::open(root.path(), &job_id).unwrap();
        assert_eq!(reopened.pin_count(), 2);
        drop(reopened);
        durable.release(CasReleaseOutcome::Aborted).unwrap();
        assert_eq!(fs::read(&object).unwrap(), vec![0x66; original.len()]);
    }

    #[test]
    fn duplicate_references_advance_by_their_actual_published_occurrence_counts() {
        let (_root, cas, mut durable, mut inputs, _) = fixture(3);
        inputs[0].progress_items = 4;
        inputs[1].progress_items = 2;
        let mut completed = 0;
        let output = prepare_import_payloads(
            &cas,
            &mut durable,
            &inputs,
            ImportPinning::DuringPreparation,
            &|| false,
            |_, count| completed = count,
        )
        .unwrap();
        assert_eq!(completed, 7);
        assert_eq!(output.len(), 3);
        durable.release(CasReleaseOutcome::Aborted).unwrap();
    }

    #[test]
    fn card_preparation_keeps_roots_unmapped_and_collection_blocked() {
        let (root, cas, mut durable, inputs, _) = fixture(4);
        durable.release(CasReleaseOutcome::Aborted).unwrap();
        let job_id = uuid::Uuid::new_v4().to_string();
        let mut durable = DurableCasJob::begin(
            root.path(),
            &job_id,
            CasJobKind::CardOrModuleContentImport,
            CasJobOwner::for_test(),
            0,
        )
        .unwrap();
        let output = prepare_import_payloads(
            &cas,
            &mut durable,
            &inputs,
            ImportPinning::AfterContentMapping,
            &|| false,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(output.len(), inputs.len());
        assert_eq!(durable.pin_count(), 0);
        assert!(!durable.is_sealed());
        assert!(
            crate::asset_repository::job_pins::collect_durable_cas_job_roots(root.path())
                .blockers
                .contains(&format!("job-pin-unsealed:{job_id}"))
        );
        for input in &inputs {
            durable
                .pin_existing(
                    &cas,
                    &input.hash,
                    input.size,
                    crate::asset_repository::job_pins::CasObjectRole::DirectObject,
                )
                .unwrap();
        }
        let mut store = PersistentStore::open(root.path()).unwrap();
        durable.seal(&mut store, 1).unwrap();
        assert!(durable.is_sealed());
        assert_eq!(durable.pin_count(), inputs.len());
        durable.release(CasReleaseOutcome::Aborted).unwrap();
    }

    #[test]
    #[ignore = "explicit filesystem performance measurement"]
    fn measure_backup_and_content_preparation() {
        use crate::asset_repository::job_pins::CasObjectRole;
        use std::io::Write;
        for repetition in 0..2 {
            for mode in [
                "backup-serial",
                "backup-parallel",
                "card-serial",
                "card-parallel",
                "module-serial",
                "module-parallel",
            ] {
                let (root, cas, mut durable, mut inputs, _) = fixture(1000);
                if !mode.starts_with("backup") {
                    durable.release(CasReleaseOutcome::Aborted).unwrap();
                    durable = DurableCasJob::begin(
                        root.path(),
                        &uuid::Uuid::new_v4().to_string(),
                        CasJobKind::CardOrModuleContentImport,
                        CasJobOwner::for_test(),
                        0,
                    )
                    .unwrap();
                }
                let pinning = if mode.starts_with("card") {
                    ImportPinning::AfterContentMapping
                } else {
                    ImportPinning::DuringPreparation
                };
                for (index, input) in inputs.iter_mut().enumerate() {
                    let mut data = vec![0x5a; 80 * 1024];
                    data[..8].copy_from_slice(&(index as u64).to_le_bytes());
                    let path = input.path.with_extension("payload");
                    fs::rename(&input.path, &path).unwrap();
                    let mut file = fs::File::create(&path).unwrap();
                    file.write_all(&data).unwrap();
                    if !mode.starts_with("backup") {
                        file.sync_all().unwrap();
                    }
                    input.path = path;
                    input.hash = hex::encode(Sha256::digest(&data));
                    input.size = data.len() as u64;
                    input.sync_required = mode.starts_with("backup");
                }
                let started = std::time::Instant::now();
                if mode.ends_with("parallel") {
                    prepare_import_payloads(
                        &cas,
                        &mut durable,
                        &inputs,
                        pinning,
                        &|| false,
                        |_, _| {},
                    )
                    .unwrap();
                } else {
                    for input in &inputs {
                        if mode.starts_with("backup") {
                            durable
                                .prepare_reader(
                                    &cas,
                                    &mut fs::File::open(&input.path).unwrap(),
                                    CasObjectRole::DirectObject,
                                )
                                .unwrap();
                        } else if mode.starts_with("card") {
                            cas.adopt_import_payload(&input.path, &input.hash, input.size, &|| {
                                false
                            })
                            .unwrap();
                        } else {
                            durable
                                .adopt_import_payload(
                                    &cas,
                                    &input.path,
                                    &input.hash,
                                    input.size,
                                    &|| false,
                                )
                                .unwrap();
                        }
                    }
                }
                let elapsed = started.elapsed();
                for input in &inputs {
                    let data = cas.read_object(&input.hash).unwrap().unwrap();
                    assert_eq!(hex::encode(Sha256::digest(&data)), input.hash);
                    assert_eq!(data.len() as u64, input.size);
                }
                println!(
                    "{}",
                    serde_json::json!({"mode":mode,"repetition":repetition,"objects":inputs.len(),"bytesEach":80 * 1024,"elapsedMs":elapsed.as_secs_f64() * 1000.0})
                );
                durable.release(CasReleaseOutcome::Aborted).unwrap();
            }
        }
    }
}
