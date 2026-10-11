use super::{lww_tests::{local, put_asset, drain_publications, LocalServerFixture}, residency::AssetPolicy};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use super::asset_cleanup_progress::{Progress, Stage};
use crate::asset_repository::PayloadCas;

#[test]
fn retained_cleanup_uses_one_page_and_no_payload_hashing() {
    let requests = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
    let observed = requests.clone();
    let server = LocalServerFixture::with_router(move |router| router.layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let observed = observed.clone();
            async move {
                if request.uri().path().ends_with("/objects/retention") { observed.0.fetch_add(1, Ordering::Relaxed); }
                if request.uri().path().ends_with("/objects/missing") { observed.1.fetch_add(1, Ordering::Relaxed); }
                next.run(request).await
            }
        },
    )));
    let (_root, mut store) = local();
    let core = server.client(&store);
    let count = 257;
    let size = 4096;
    for index in 0..count {
        let mut body = vec![42; size];
        body[..8].copy_from_slice(&(index as u64).to_le_bytes());
        put_asset(&mut store, &format!("assets/cleanup-{index}.png"), &body);
    }
    drain_publications(&core, &mut store, &[]).unwrap();
    store.asset_residency_set_policy(AssetPolicy::Remote, || Ok(())).unwrap();
    requests.0.store(0, Ordering::Relaxed);
    requests.1.store(0, Ordering::Relaxed);
    crate::persistent_store::hash_work::reset_hash_work();
    let started = std::time::Instant::now();
    let result = store.asset_residency_evict(|| Ok(())).unwrap();
    let elapsed = started.elapsed().as_millis();
    let work = crate::persistent_store::hash_work::take_hash_work();
    assert!(work.incomplete.is_empty());
    let hashed = work.domains.get("cas_existing_verify").map_or(0, |work| work.bytes);
    let retained = requests.0.load(Ordering::Relaxed);
    let missing = requests.1.load(Ordering::Relaxed);
    eprintln!("cleanup sample: objects={count} bytes={} retention_requests={retained} missing_requests={missing} payload_hash_bytes={hashed} elapsed_ms={elapsed}", count * size);
    assert_eq!(result.evicted_bytes, (count * size) as u64);
    assert_eq!((retained, missing, hashed), (1, 0, 0));
}

#[test]
fn cleanup_uploads_unpublished_objects_before_offloading_and_reports_each_stage() {
    let server = LocalServerFixture::new();
    let (_root, mut store) = local();
    let _core = server.client(&store);
    let body = b"synthetic unpublished retained content";
    let object = put_asset(&mut store, "assets/unpublished.png", body);
    store.asset_residency_set_policy(AssetPolicy::Remote, || Ok(())).unwrap();
    let readings = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = readings.clone();
    let mut progress = Progress::new(move |snapshot| observed.lock().unwrap().push(snapshot));
    assert_eq!(store.asset_residency_evict_observed(None, || Ok(()), &mut progress).unwrap().evicted_bytes, body.len() as u64);
    drop(progress);
    let readings = readings.lock().unwrap();
    for stage in [Stage::Preparing, Stage::Checking, Stage::Uploading, Stage::Deleting, Stage::Finalizing] {
        assert!(readings.iter().any(|snapshot| snapshot.stage == stage), "missing {stage:?}");
    }
    let last = readings.last().unwrap();
    assert_eq!((last.evicted_items, last.evicted_bytes), (1, body.len() as u64));
    assert!(readings.windows(2).all(|pair| pair[0].sequence < pair[1].sequence));
    let cas = PayloadCas::new(store.repository_root()).unwrap();
    assert!(cas.stat_object(object.object_hash.as_ref().unwrap()).unwrap().is_none());
    store.asset_residency_set_policy(AssetPolicy::Full, || Ok(())).unwrap();
    assert_eq!(cas.read_object(object.object_hash.as_ref().unwrap()).unwrap().unwrap(), body);
}

#[test]
fn a_damaged_server_body_blocks_every_local_deletion_even_with_previous_custody() {
    let server = LocalServerFixture::new();
    let (_root, mut store) = local();
    let core = server.client(&store);
    let body = vec![42; 128 * 1024];
    let object = put_asset(&mut store, "assets/damaged-server.png", &body);
    let hash = object.object_hash.unwrap();
    drain_publications(&core, &mut store, &[]).unwrap();
    store.asset_residency_set_policy(AssetPolicy::Remote, || Ok(())).unwrap();
    let remote = server.root().join("objects").join(&hash[..2]).join(&hash);
    assert!(remote.is_file());
    std::fs::write(&remote, b"synthetic truncated server object").unwrap();
    let readings = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = readings.clone();
    let mut progress = Progress::new(move |snapshot| observed.lock().unwrap().push(snapshot));
    let error = store.asset_residency_evict_observed(None, || Ok(()), &mut progress).err().unwrap();
    assert_eq!(error.code, "corrupt-object");
    drop(progress);
    assert!(readings.lock().unwrap().iter().all(|snapshot| snapshot.stage != Stage::Deleting && snapshot.evicted_bytes == 0));
    assert_eq!(PayloadCas::new(store.repository_root()).unwrap().read_object(&hash).unwrap().unwrap(), body);
}

#[test]
fn cancellation_at_the_delete_boundary_preserves_local_bytes_and_retry_is_safe() {
    let server = LocalServerFixture::new();
    let (_root, mut store) = local();
    let core = server.client(&store);
    let body = b"synthetic cancelled retained content";
    let object = put_asset(&mut store, "assets/cancel-cleanup.png", body);
    drain_publications(&core, &mut store, &[]).unwrap();
    store.asset_residency_set_policy(AssetPolicy::Remote, || Ok(())).unwrap();
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = cancelled.clone();
    let mut progress = Progress::new(move |snapshot| { if snapshot.stage == Stage::Deleting { observed.store(true, Ordering::Release); } });
    let error = store.asset_residency_evict_observed(Some(cancelled.clone()), || {
        if cancelled.load(Ordering::Acquire) { Err(super::SyncError::new("cancelled", 409)) } else { Ok(()) }
    }, &mut progress).err().unwrap();
    assert_eq!(error.code, "cancelled");
    let cas = PayloadCas::new(store.repository_root()).unwrap();
    assert_eq!(cas.read_object(object.object_hash.as_ref().unwrap()).unwrap().unwrap(), body);
    assert_eq!(store.asset_residency_evict(|| Ok(())).unwrap().evicted_bytes, body.len() as u64);
}
