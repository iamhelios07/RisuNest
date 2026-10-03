use super::{contract::*, lww_tests::CycleFixture};
use crate::persistent_store::{
    external_capture::{published_json_asset_roots, BackupBodyRole},
    lww::UnitMutation,
    PersistentStore, WorkingSetCommit,
};
use risunest_sync_wire::unit::{UnitKey, UnitValue};
use serde_json::{json, Value};

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(future)
}
fn set(store: &mut PersistentStore, key: &[&str], value: Value) {
    store.commit(&WorkingSetCommit {
        expected_revision: store.revision().unwrap(),
        unit_mutations: Some(vec![UnitMutation::Set { key: UnitKey::new(key).unwrap(), value }]),
        ..Default::default()
    }).unwrap();
}
// One body fits the segment and one exceeds the segment body bound, so both
// control carriages are exercised.
fn large_fixture() -> (CycleFixture, Value, Value) {
    let mut f = CycleFixture::new();
    let segment = json!("s".repeat(2 * 1024 * 1024));
    let catalog = json!("c".repeat(super::lww_segment::SMALL_BODY_BYTES + 1));
    set(&mut f.a, &["root", "additionalPrompt"], segment.clone());
    set(&mut f.a, &["root", "NAIImgUrl"], catalog.clone());
    assert!(f.a.lww_read_outbox(0.into(), 100).unwrap().entries.iter().all(|entry| matches!(entry.value, UnitValue::Object { .. })));
    (f, segment, catalog)
}
fn assert_received(store: &PersistentStore, segment: &Value, catalog: &Value) {
    let root = store.read_root(None).unwrap().value;
    assert!(root["additionalPrompt"] == *segment);
    assert!(root["NAIImgUrl"] == *catalog);
}

#[test]
fn large_units_publish_and_receive_through_segments_and_data_catalogs() {
    run(async {
        let (mut f, segment, catalog) = large_fixture();
        f.publish_a().await;
        assert!(f.receive_b().await > 0);
        assert_received(&f.b, &segment, &catalog);
    });
}

#[test]
fn compaction_captures_large_unit_bodies_before_segments_retire() {
    run(async {
        let (mut f, segment, catalog) = large_fixture();
        f.publish_a().await;
        let job = tempfile::tempdir().unwrap();
        f.sender.compact_published(job.path(), "00000000-0000-4000-8000-000000000099",
            &f.a.lww_clock_state().unwrap().writer_id, &super::fake::capabilities(true), &Cancellation::default(), None)
            .await.unwrap();
        for object in f.sender.listing(&Cancellation::default()).await.unwrap() {
            f.provider.delete_object(&f.sender.repository, &object.locator, &Cancellation::default()).await.unwrap();
        }
        assert!(f.receive_b().await > 0);
        assert_received(&f.b, &segment, &catalog);
    });
}

#[test]
fn oversized_large_unit_controls_are_spooled_and_scanned() {
    let asset = "a".repeat(64);
    let body = risunest_sync_wire::payload_value::encode(&json!([asset, "x".repeat(risunest_sync_wire::MAX_METADATA_BYTES)])).unwrap();
    let hash = risunest_sync_wire::hash(&body);
    assert!(published_json_asset_roots(&body).unwrap().object_hashes.contains(&asset));
    let padded = [b"[".as_slice(), &body[1..body.len() - 1], b" ]"].concat();
    assert!(published_json_asset_roots(&padded).is_err());

    let directory = tempfile::tempdir().unwrap();
    let mut spool = super::capture::BackupDependencySpool::new(directory.path()).unwrap();
    spool.push(&hash, &body, BackupBodyRole::Control).unwrap();
    assert!(spool.push(&risunest_sync_wire::hash(&padded), &padded, BackupBodyRole::Control).is_err());
    spool.seal().unwrap();
    assert_eq!(spool.control(&hash).unwrap(), Some(body.clone()));
    let mut visited = Vec::new();
    spool.visit(&mut |hash, bytes, role| { visited.push((hash.to_owned(), bytes.len(), role)); Ok(()) }).unwrap();
    assert_eq!(visited, vec![(hash, body.len(), BackupBodyRole::Control)]);
}
