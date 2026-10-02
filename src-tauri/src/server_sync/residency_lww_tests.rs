use super::{
    lww_tests::{local, put_asset, LocalServerFixture},
    residency::Residency,
};
use crate::{
    asset_repository::{
        job_pins::{CasJobKind, CasObjectRole, DurableCasJob},
        PayloadCas,
    },
    persistent_store::{PersistentStore, PluginStorageMutation, WorkingSetCommit},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[test]
fn cancelled_full_hydration_keeps_remote_proof_and_retries_without_a_partial_body() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let armed = Arc::new(AtomicBool::new(false));
    let request_cancel = cancelled.clone();
    let request_armed = armed.clone();
    let server = LocalServerFixture::with_router(move |router| {
        router.layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let cancelled = request_cancel.clone();
                let armed = request_armed.clone();
                async move {
                    if (request.uri().path() == "/objects/transfer"
                        || request.uri().path().starts_with("/objects/"))
                        && armed.swap(false, Ordering::SeqCst)
                    {
                        cancelled.store(true, Ordering::SeqCst);
                        return axum::response::Response::builder()
                            .status(503)
                            .body(axum::body::Body::from("{\"error\":\"server-unavailable\"}"))
                            .unwrap();
                    }
                    next.run(request).await
                }
            },
        ))
    });
    let (_root, mut store) = local();
    let core = server.client(&store);
    let bytes = vec![43; 128 * 1024 + 1];
    let hash = put_asset(&mut store, "assets/recover.png", &bytes)
        .object_hash
        .unwrap();
    let request = super::lww_tests::header(&store);
    core.push(&mut store, &request, &[]).unwrap();
    store
        .asset_residency_set_policy(super::residency::AssetPolicy::Remote, || Ok(()))
        .unwrap();
    store.asset_residency_evict(|| Ok(())).unwrap();
    armed.store(true, Ordering::SeqCst);
    let result = store.asset_residency_set_policy_cancelled(
        super::residency::AssetPolicy::Full,
        Some(cancelled.clone()),
        || {
            if cancelled.load(Ordering::SeqCst) {
                Err(super::SyncError::new("cancelled", 409))
            } else {
                Ok(())
            }
        },
    );
    assert_eq!(result.err().unwrap().code, "cancelled");
    let cas = PayloadCas::new(store.repository_root()).unwrap();
    assert!(cas.stat_object(&hash).unwrap().is_none());
    assert!(Residency::open(store.repository_root())
        .unwrap()
        .object(&hash, None)
        .unwrap()
        .is_some());
    cancelled.store(false, Ordering::SeqCst);
    store
        .asset_residency_set_policy(super::residency::AssetPolicy::Full, || Ok(()))
        .unwrap();
    assert_eq!(cas.read_object(&hash).unwrap().unwrap(), bytes);
}

#[test]
fn transient_hydration_keeps_the_body_remote_and_reclaims_its_spool() {
    let server = LocalServerFixture::new();
    let (_root, mut store) = local();
    let core = server.client(&store);
    let bytes = vec![89; 128 * 1024 + 1];
    let hash = put_asset(&mut store, "assets/transient.png", &bytes)
        .object_hash
        .unwrap();
    let request = super::lww_tests::header(&store);
    core.push(&mut store, &request, &[]).unwrap();
    store
        .asset_residency_set_policy(super::residency::AssetPolicy::Remote, || Ok(()))
        .unwrap();
    store.asset_residency_evict(|| Ok(())).unwrap();
    let cas = PayloadCas::new(store.repository_root()).unwrap();
    let mut body = super::residency::open_transient_with_check(
        store.repository_root(),
        store.repository_root(),
        &hash,
        &|| Ok(()),
    )
    .unwrap()
    .unwrap();
    let mut actual = Vec::new();
    std::io::Read::read_to_end(&mut body, &mut actual).unwrap();
    assert_eq!(actual, bytes);
    assert!(cas.stat_object(&hash).unwrap().is_none());
    drop(body);
    assert!(!std::fs::read_dir(store.repository_root())
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("asset-transient-")));
}

#[test]
fn residency_classification_preserves_live_plugin_snapshot_and_durable_job_roots() {
    let (_directory, mut store) = local();
    let root = store.repository_root().to_path_buf();
    let cas = PayloadCas::new(&root).unwrap();
    let live = put_asset(&mut store, "assets/classification.png", b"synthetic live")
        .object_hash
        .unwrap();
    let plugin = cas.prepare_bytes(b"synthetic plugin").unwrap().content_hash;
    store
        .commit(&WorkingSetCommit {
            expected_revision: store.revision().unwrap(),
            plugin_storage: Some(vec![PluginStorageMutation::Set {
                owner: "synthetic".into(),
                key: "payload".into(),
                value: serde_json::json!(crate::asset_repository::object_physical_key(&plugin)),
            }]),
            ..Default::default()
        })
        .unwrap();
    store.snapshot_create("synthetic-classification").unwrap();
    let mut job = DurableCasJob::begin(
        &root,
        "synthetic-classification-job",
        CasJobKind::OfficialPublicationOrExportPreparation,
        1,
    )
    .unwrap();
    let pinned = job
        .prepare_bytes(&cas, b"synthetic job-only", CasObjectRole::DirectObject)
        .unwrap()
        .content_hash;
    job.seal(&mut store, 1).unwrap();
    let baseline = store
        .residency_inventory_classification_test(false)
        .unwrap();
    let _guard = crate::asset_repository::coordinator::lock_repository_mutation().unwrap();
    assert_eq!(
        baseline,
        store.residency_inventory_classification_test(true).unwrap()
    );
    assert!(baseline.2);
    for hash in [&live, &plugin, &pinned] {
        assert!(baseline.0.contains(hash));
    }
    assert!(baseline.1.contains(&pinned));
}

#[test]
fn corrupt_durable_job_proof_blocks_residency_cleanup_before_any_release() {
    let (_directory, store) = local();
    let root = store.repository_root();
    let job = DurableCasJob::begin(
        root,
        "synthetic-broken-job",
        CasJobKind::OfficialPublicationOrExportPreparation,
        1,
    )
    .unwrap();
    drop(job);
    std::fs::write(
        root.join("assets/job-pins/job-synthetic-broken-job.journal"),
        b"synthetic corruption",
    )
    .unwrap();
    for guarded in [false, true] {
        let _guard = guarded
            .then(|| crate::asset_repository::coordinator::lock_repository_mutation().unwrap());
        let error = store
            .residency_inventory_classification_test(guarded)
            .unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.status, error.retryable),
            ("asset-jobs-unresolved", 409, false)
        );
    }
}

#[derive(Default)]
struct ReleaseTrace {
    armed: AtomicBool,
    target: Mutex<Option<(std::path::PathBuf, String)>>,
    requests: Mutex<Vec<Vec<String>>>,
}
#[test]
fn a_plugin_root_added_after_first_release_page_keeps_every_later_custody_proof() {
    let trace = Arc::new(ReleaseTrace::default());
    let observed = trace.clone();
    let server = LocalServerFixture::with_router(move |router| {
        router.layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let trace = observed.clone();
                async move {
                    if request.method() == axum::http::Method::POST
                        && request.uri().path() == "/objects/retention/release"
                    {
                        let (parts, body) = request.into_parts();
                        let bytes =
                            axum::body::to_bytes(body, risunest_sync_wire::MAX_METADATA_BYTES)
                                .await
                                .unwrap();
                        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                        let hashes = value["objects"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|item| item["hash"].as_str().unwrap().to_owned())
                            .collect::<Vec<_>>();
                        trace.requests.lock().unwrap().push(hashes.clone());
                        if trace.armed.swap(false, Ordering::SeqCst) {
                            let (root, target) = trace.target.lock().unwrap().clone().unwrap();
                            assert!(!hashes.contains(&target));
                            let mut store = PersistentStore::open(&root).unwrap();
                            store
                                .commit(&WorkingSetCommit {
                                    expected_revision: store.revision().unwrap(),
                                    plugin_storage: Some(vec![PluginStorageMutation::Set {
                                        owner: "synthetic-release".into(),
                                        key: "later-page".into(),
                                        value: serde_json::json!(
                                            crate::asset_repository::object_physical_key(&target)
                                        ),
                                    }]),
                                    ..Default::default()
                                })
                                .unwrap();
                        }
                        return next
                            .run(axum::extract::Request::from_parts(
                                parts,
                                axum::body::Body::from(bytes),
                            ))
                            .await;
                    }
                    next.run(request).await
                }
            },
        ))
    });
    let (_root, store) = local();
    let core = server.client(&store);
    let config = core.client.config();
    let stored = store.server_stored_config().unwrap().unwrap();
    let device = server
        .server
        .authenticate(&config.library_id, &config.token)
        .unwrap();
    let head = server.server.head().unwrap();
    let mut objects = Vec::new();
    for index in 0..300 {
        let body = format!("synthetic release {index}").into_bytes();
        let hash = risunest_sync_wire::hash(&body);
        server.server.put_object(&device, &hash, &body).unwrap();
        objects.push((hash, Some(body.len() as u64)));
    }
    objects.sort_by(|left, right| left.0.cmp(&right.0));
    let target = objects.last().unwrap().0.clone();
    let context = Residency::context_id(&stored, &head.epoch);
    Residency::open(store.repository_root())
        .unwrap()
        .retain(&core.client, &stored, &head, &objects)
        .unwrap();
    *trace.target.lock().unwrap() = Some((store.repository_root().to_path_buf(), target));
    trace.armed.store(true, Ordering::SeqCst);
    store.asset_residency_release_unused(|| Ok(())).unwrap();
    assert!(!trace.armed.load(Ordering::SeqCst));
    let requests = trace.requests.lock().unwrap();
    assert_eq!(requests.iter().map(Vec::len).collect::<Vec<_>>(), vec![128]);
    let residency = Residency::open(store.repository_root()).unwrap();
    let retained = server
        .server
        .retained_objects(&device, &head.epoch, None)
        .unwrap();
    for (hash, _) in &objects[128..] {
        assert!(residency.object(hash, Some(&context)).unwrap().is_some());
        assert!(retained.objects.iter().any(|item| item.hash == *hash));
    }
}
