use super::{
    connection_commands::ConnectedRepository,
    connection_store::{ConnectionStore, StoredConnection},
    contract::*,
    fake,
    lww_engine::ExternalLwwEngine,
    lww_residency,
    lww_tests::{small_asset, CycleFixture},
};
use crate::{
    asset_repository::PayloadCas,
    persistent_store::{
        lww::Header,
        sync_selection::{SwitchBindingRequest, SyncTarget},
        AssetAlias, PersistentStore, WorkingSetCommit,
    },
    server_sync::{
        lww_tests::{drain_publications, LocalServerFixture},
        previous_storage_tests::{bind_server, switchable_server},
        residency::{AssetPolicy, Residency},
    },
};
use risunest_sync_wire::stamp::DecimalU64;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(future)
}

/// The connection the fixture's receiving store would hold for the repository.
fn receiver_connection(f: &CycleFixture) -> ConnectedRepository {
    ConnectedRepository {
        stored: StoredConnection {
            id: "receiver".into(),
            config: ConnectionConfig { provider: "synthetic".into(), profile: None, endpoint: "https://synthetic.invalid".into(),
                account_id: "fixture".into(), location: Default::default(), oauth_profile: None },
            descriptor: f.receiver.descriptor.clone(),
            descriptor_locator: RemoteLocator { connection_identity: f.receiver.repository.connection_identity.clone(), collection: None, object: "descriptor".into() },
            provider_repository_id: f.receiver.repository.repository_id.clone(),
            credential_ref: "credential".into(), root_key_ref: "key".into(), recovery_key_ref: "recovery".into(),
            retention_policy: None, capabilities: f.receiver.capabilities.clone(),
            created_at_ms: 1, verified_at_ms: 1, last_sync_at_ms: None, last_backup_at_ms: None,
        },
        provider: f.provider.clone(),
        handle: fake::repository(),
        dependencies: fake::loopback_dependencies(fake::MemoryVault::default(), 1).dependencies,
        root_key: zeroize::Zeroizing::new([7; 32]),
    }
}

/// Binds the store to the fixture's repository and queues its whole state, as
/// the first publication after a switch does.
fn bind_external(store: &mut PersistentStore, engine: &ExternalLwwEngine) -> DecimalU64 {
    let before = store.lww_binding_state().unwrap();
    let target = SyncTarget::External("repository".into());
    let inspection = store.register_lww_binding_inspection(before.target_authority, &target, &engine.repository.connection_identity, &engine.library).unwrap();
    let authority = store.switch_lww_binding(&SwitchBindingRequest {
        header: Header { binding_authority: before.target_authority, request_id: uuid::Uuid::new_v4().to_string() },
        expected_selection_epoch: before.selection_epoch, target, inspection_id: Some(inspection),
    }).unwrap().target_authority;
    let mut after = None;
    loop {
        let header = Header { binding_authority: authority, request_id: uuid::Uuid::new_v4().to_string() };
        let page = store.lww_queue_unit_state_page(&header, after.as_ref(), 4096).unwrap();
        if !page.has_more {
            break;
        }
        after = page.after_key;
    }
    authority
}

fn segments(f: &CycleFixture) -> usize {
    f.provider.uploaded_ids().iter().filter(|id| parse_segment_object_id(id).is_ok()).count()
}

#[test]
fn a_previous_server_body_that_cannot_be_fetched_publishes_no_segment_that_references_it() {
    let offline = Arc::new(AtomicBool::new(false));
    let server = switchable_server(offline.clone());
    let mut f = CycleFixture::new();
    let core = server.client(&f.a);
    let missing = small_asset(&mut f.a, "z-previous-only", &[71; 96 * 1024]);
    drain_publications(&core, &mut f.a, &[]).unwrap();
    f.a.asset_residency_set_policy(AssetPolicy::Remote, || Ok(())).unwrap();
    f.a.asset_residency_evict(|| Ok(())).unwrap();
    assert!(!f.a.external_lww_object_is_local(&missing).unwrap());
    let shared_body = b"synthetic shared local body";
    let shared = small_asset(&mut f.a, "a-shared", shared_body);
    let template = AssetAlias { key: String::new(), object_hash: Some(shared), kind: "asset".into(), size: shared_body.len() as i64,
        mime: "application/octet-stream".into(), name: "synthetic".into(), ext: "bin".into(),
        inlay_type: None, width: None, height: None, metadata: serde_json::json!({}) };
    let fillers = (0..4100).map(|index| AssetAlias { key: format!("a-{index:04}"), ..template.clone() }).collect::<Vec<_>>();
    f.a.commit_with_asset_aliases(&WorkingSetCommit { expected_revision: f.a.revision().unwrap(), ..Default::default() }, &fillers).unwrap();
    let authority = bind_external(&mut f.a, &f.sender);
    let missing_key = risunest_sync_wire::unit::UnitKey::new(&["asset", "z-previous-only"]).unwrap();
    assert!(f.a.lww_read_outbox(authority, 4096).unwrap().entries.iter().all(|entry| entry.key != missing_key));
    offline.store(true, Ordering::SeqCst);
    let error = run(f.sender.publish(&mut f.a, authority, &[], &Cancellation::default())).err().unwrap();
    assert_eq!(error.kind, ErrorKind::PreviousStorageUnavailable);
    assert_eq!(segments(&f), 1, "the segment before the failing one stays published");
    assert!(f.a.lww_read_outbox(authority, 4096).unwrap().entries.iter().any(|entry| entry.key == missing_key));
    run(f.receive_b());
    assert!(f.b.read_asset_alias("asset", "a-0000", None).unwrap().is_some());
    assert!(f.b.read_asset_alias("asset", "z-previous-only", None).unwrap().is_none());
    offline.store(false, Ordering::SeqCst);
    run(f.sender.publish(&mut f.a, authority, &[], &Cancellation::default())).unwrap();
    assert_eq!(segments(&f), 2);
    assert!(f.a.lww_read_outbox(authority, 16).unwrap().entries.is_empty());
    run(f.receive_b());
    assert!(f.b.read_asset_alias("asset", "z-previous-only", None).unwrap().is_some());
    assert!(!f.a.external_lww_object_is_local(&missing).unwrap());
}

#[test]
fn first_publication_to_a_server_copies_bodies_only_an_external_storage_holds() {
    let mut f = CycleFixture::new();
    let body = vec![73; 96 * 1024];
    let hash = small_asset(&mut f.a, "external-only", &body);
    run(f.publish_a());
    run(f.receive_b());
    assert!(!f.b.external_lww_object_is_local(&hash).unwrap());
    let connection = receiver_connection(&f);
    ConnectionStore::open(f.directory_b.path()).unwrap().insert(&connection.stored).unwrap();
    let resolver = lww_residency::install_test_source_connection(f.directory_b.path(), Arc::new(connection)).unwrap();
    let packs = lww_residency::packed_source(f.directory_b.path(), &hash).unwrap().unwrap().packs;
    let server = LocalServerFixture::new();
    let client = bind_server(&mut f.b, &server);
    drain_publications(&client, &mut f.b, &[]).unwrap();
    assert!(PayloadCas::new(f.directory_b.path()).unwrap().stat_object(&hash).unwrap().is_none());
    assert!(packs.iter().all(|pack| f.provider.holds(&pack.header.object_id)), "the external storage keeps its copy");
    let routed = Residency::open(f.directory_b.path()).unwrap().object(&hash, None).unwrap().unwrap();
    assert_eq!(routed.config.library_id, client.client.config().library_id);
    drop(resolver);
    f.b.asset_residency_set_policy(AssetPolicy::Full, || Ok(())).unwrap();
    assert_eq!(PayloadCas::new(f.directory_b.path()).unwrap().read_object(&hash).unwrap().unwrap(), body, "later downloads come from the server");
}

#[test]
fn a_previous_external_body_that_cannot_be_fetched_publishes_no_segment_to_the_next_storage() {
    let mut f = CycleFixture::new();
    let body = vec![79; 96 * 1024];
    let hash = small_asset(&mut f.a, "external-previous", &body);
    run(f.publish_a());
    run(f.receive_b());
    assert!(!f.b.external_lww_object_is_local(&hash).unwrap());
    let next = CycleFixture::new();
    let mut engine = next.sender;
    engine.connection_id = "next".into();
    engine.connection_root = f.directory_b.path().into();
    engine.library = "synthetic-next-library".into();
    let authority = bind_external(&mut f.b, &engine);
    let error = run(engine.publish(&mut f.b, authority, &[], &Cancellation::default())).err().unwrap();
    assert_eq!(error.kind, ErrorKind::PreviousStorageUnavailable);
    assert_eq!(next.provider.uploaded_ids().iter().filter(|id| parse_segment_object_id(id).is_ok()).count(), 0);
    let connection = receiver_connection(&f);
    ConnectionStore::open(f.directory_b.path()).unwrap().insert(&connection.stored).unwrap();
    let _resolver = lww_residency::install_test_source_connection(f.directory_b.path(), Arc::new(connection)).unwrap();
    run(engine.publish(&mut f.b, authority, &[], &Cancellation::default())).unwrap();
    assert_eq!(next.provider.uploaded_ids().iter().filter(|id| parse_segment_object_id(id).is_ok()).count(), 1);
    assert!(f.b.lww_read_outbox(authority, 16).unwrap().entries.is_empty());
    assert!(!f.b.external_lww_object_is_local(&hash).unwrap());
}
