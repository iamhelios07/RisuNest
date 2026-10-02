use super::{
    contract::*,
    fake::{self, FakeProvider},
    lww_engine::{Admission, ExternalLwwEngine},
    lww_segment,
};
use crate::persistent_store::{
    lww::{MessageLocator, UnitMutation},
    ConversationMutation, PersistentStore, WorkingSetCommit,
};
use risunest_sync_wire::{stamp::DecimalU64, unit::UnitKey};
use std::sync::Arc;

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
pub(crate) struct CycleFixture {
    pub directory_a: tempfile::TempDir,
    pub directory_b: tempfile::TempDir,
    pub a: PersistentStore,
    pub b: PersistentStore,
    pub provider: Arc<FakeProvider>,
    pub sender: ExternalLwwEngine,
    pub receiver: ExternalLwwEngine,
}
impl CycleFixture {
    pub(crate) fn new() -> Self {
        let directory_a = tempfile::tempdir().unwrap();
        let directory_b = tempfile::tempdir().unwrap();
        let a = PersistentStore::open(directory_a.path()).unwrap();
        let b = PersistentStore::open(directory_b.path()).unwrap();
        let provider = Arc::new(FakeProvider::with_object_limit(true, 128 * 1024 * 1024));
        let engine = |id: &str| ExternalLwwEngine {
            provider: provider.clone(),
            repository: fake::repository(),
            library: "synthetic-lww-library".into(),
            root_key: zeroize::Zeroizing::new([7; 32]),
            admission: Some(Admission::synthetic(super::runtime::now_ms())),
            connection_id: id.into(),
            connection_root: std::path::PathBuf::from("synthetic-unused"),
        };
        let sender = engine("sender");
        let receiver = engine("receiver");
        Self {
            directory_a,
            directory_b,
            a,
            b,
            provider,
            sender,
            receiver,
        }
    }
    pub(crate) async fn publish_a(&mut self) -> super::lww_engine::PublicationResult {
        self.sender
            .publish(&mut self.a, DecimalU64(0), &[], &Cancellation::default())
            .await
            .unwrap()
    }
    pub(crate) async fn receive_b(&mut self) -> usize {
        self.receiver
            .receive_and_apply(&mut self.b, DecimalU64(0), &[], &Cancellation::default())
            .await
            .unwrap()
    }
}
fn set(store: &mut PersistentStore, key: &[&str], value: serde_json::Value) {
    store
        .commit(&WorkingSetCommit {
            expected_revision: store.revision().unwrap(),
            unit_mutations: Some(vec![UnitMutation::Set {
                key: UnitKey::new(key).unwrap(),
                value,
            }]),
            ..Default::default()
        })
        .unwrap();
}
fn conversation(store: &mut PersistentStore) {
    store
        .commit(&WorkingSetCommit {
            expected_revision: store.revision().unwrap(),
            unit_mutations: Some(vec![
                UnitMutation::Set {
                    key: UnitKey::new(&["exists", "character", "char"]).unwrap(),
                    value: serde_json::json!({"type":"character"}),
                },
                UnitMutation::Set {
                    key: UnitKey::new(&["exists", "conversation", "char", "conv"]).unwrap(),
                    value: serde_json::json!(true),
                },
            ]),
            ..Default::default()
        })
        .unwrap();
}
#[test]
fn two_actual_stores_publish_receive_and_quiet_listing() {
    run(async {
        let mut f = CycleFixture::new();
        assert_ne!(
            f.a.lww_clock_state().unwrap().writer_id,
            f.b.lww_clock_state().unwrap().writer_id
        );
        set(&mut f.a, &["root", "language"], serde_json::json!("ko"));
        let stamps = f.a.lww_read_outbox(DecimalU64(0), 100).unwrap().entries;
        let sent = f.publish_a().await;
        assert_eq!(sent.segments.0, 1);
        assert_eq!(f.provider.listing_count(), 0);
        assert_eq!(f.provider.read_count(), 0);
        assert_eq!(f.receive_b().await, 1);
        assert_eq!(f.b.read_root(None).unwrap().value["language"], "ko");
        let state = f.b.lww_read_unit_state(DecimalU64(0), None, 100).unwrap();
        assert!(state
            .entries
            .iter()
            .any(|entry| entry.stamp == stamps[0].stamp));
        let reads = f.provider.read_count();
        let bytes = f.provider.transferred_body_bytes();
        assert_eq!(f.receive_b().await, 0);
        assert_eq!(f.provider.read_count(), reads);
        assert_eq!(f.provider.transferred_body_bytes(), bytes);
        assert_eq!(f.publish_a().await.segments.0, 0);
    })
}

#[test]
fn checkpoint_uses_only_published_units_and_restores_after_segment_retirement() {
    run(async {
        let mut f=CycleFixture::new();
        set(&mut f.a,&["root","language"],serde_json::json!("ko"));
        f.publish_a().await;
        set(&mut f.a,&["root","language"],serde_json::json!("en"));
        let job=tempfile::tempdir().unwrap();
        let completed=f.sender.compact_published(job.path(),"00000000-0000-4000-8000-000000000099",&f.a.lww_clock_state().unwrap().writer_id,&fake::capabilities(true),&Cancellation::default(),None).await.unwrap();
        assert_eq!(completed.reference.role,ObjectRole::Snapshot);
        let snapshots=f.sender.snapshot_listing(&Cancellation::default()).await.unwrap();
        assert_eq!(snapshots.len(),1);
        let (_,snapshot)=f.sender.checkpoint(&snapshots[0],&Cancellation::default()).await.unwrap();
        assert_eq!(snapshot.covered_prefixes.values().next().unwrap().0,1);
        let mut identical=snapshot.clone(); identical.snapshot_id="00000000-0000-4000-8000-000000000098".into();
        assert_eq!(super::lww_checkpoint::retained(&[snapshot.clone(),identical.clone()]).unwrap(),std::collections::BTreeSet::from([identical.snapshot_id.clone()]));
        identical.state_identity="a".repeat(64);
        assert!(super::lww_checkpoint::retained(&[snapshot.clone(),identical]).is_err());
        for object in f.sender.listing(&Cancellation::default()).await.unwrap() {
            f.provider.delete_object(&f.sender.repository,&object.locator,&Cancellation::default()).await.unwrap();
        }
        assert_eq!(f.receive_b().await,1);
        assert_eq!(f.b.read_root(None).unwrap().value["language"],"ko");
        assert_eq!(f.a.read_root(None).unwrap().value["language"],"en");
        assert_eq!(f.receive_b().await,0);
    });
}
#[test]
fn cycle_key_sets_match_real_emission_and_durable_apply_and_noop_is_empty() {
    use super::lww_engine::cycle_keys;
    run(async {
        let mut f = CycleFixture::new();
        cycle_keys::reset();
        set(&mut f.a, &["root", "language"], serde_json::json!("ko"));
        let published = f.publish_a().await;
        assert_eq!(f.receive_b().await, 1);
        let keys = cycle_keys::take();
        let expected = std::collections::BTreeSet::from([
            UnitKey::new(&["root", "language"]).unwrap(),
        ]);
        assert!(keys.complete());
        assert_eq!(keys.selected_keys, expected);
        assert_eq!(keys.attempted_keys, expected);
        assert_eq!(keys.emitted_keys, expected);
        assert_eq!(keys.affected_keys, expected);
        assert!(keys.held_keys.is_empty());
        assert!(keys.deferred_keys.is_empty());
        assert_eq!(keys.segment_attempts, 1);
        assert_eq!(keys.accepted_publications as u64, published.segments.0);
        assert_eq!(keys.receive_applies, 1);

        cycle_keys::reset();
        set(&mut f.a, &["root", "language"], serde_json::json!("ko"));
        assert_eq!(f.publish_a().await.segments.0, 0);
        assert_eq!(f.receive_b().await, 0);
        let keys = cycle_keys::take();
        assert!(keys.complete());
        assert!(keys.selected_keys.is_empty());
        assert!(keys.attempted_keys.is_empty());
        assert!(keys.emitted_keys.is_empty());
        assert!(keys.affected_keys.is_empty());
        assert_eq!(keys.segment_attempts, 0);
        assert_eq!(keys.accepted_publications, 0);
        assert_eq!(keys.receive_applies, 0);
    });
}
#[test]
fn cycle_key_sample_stays_failed_when_uncertain_publication_is_later_reconciled() {
    use super::lww_engine::cycle_keys;
    run(async {
        let mut f = CycleFixture::new();
        cycle_keys::reset();
        set(&mut f.a, &["root", "language"], serde_json::json!("ko"));
        f.provider.state.lock().unwrap().lose_response = true;
        assert!(f.sender.publish(&mut f.a, 0.into(), &[], &Cancellation::default()).await.is_err());
        assert_eq!(f.publish_a().await.segments.0, 1);
        assert_eq!(f.receive_b().await, 1);
        let keys = cycle_keys::take();
        assert!(!keys.complete());
        assert_eq!(keys.failed_operations, 1);
        assert_eq!(keys.pending_operations, 0);
        assert_eq!(keys.segment_attempts, 1);
        assert_eq!(keys.accepted_publications, 1);
        assert_eq!(keys.receive_applies, 1);
        assert_eq!(keys.selected_keys, keys.emitted_keys);
        assert_eq!(keys.attempted_keys, keys.emitted_keys);
        assert_eq!(keys.emitted_keys, keys.affected_keys);
        assert_eq!(keys.emitted_keys.len(), 1);
        assert_eq!(f.provider.upload_count(), 1);
    });
}
#[test]
fn cycle_key_sets_exclude_generating_publication_and_report_actual_deferred_apply() {
    use super::lww_engine::cycle_keys;
    run(async {
        let mut f = CycleFixture::new();
        conversation(&mut f.a);
        f.publish_a().await;
        f.receive_b().await;
        let generating = vec![MessageLocator {
            character_id: "char".into(), conversation_id: "conv".into(), start: None,
        }];
        cycle_keys::reset();
        f.a.commit(&WorkingSetCommit {
            expected_revision: f.a.revision().unwrap(),
            conversations: Some(vec![ConversationMutation::ReplaceRange {
                character_id: "char".into(), conversation_id: "conv".into(),
                start: 0, delete_count: 0,
                messages: vec![serde_json::json!({"data":"synthetic-message","chatId":"synthetic-id"})],
                conversation: None, configured_index: None,
            }]),
            ..Default::default()
        }).unwrap();
        assert_eq!(f.sender.publish(&mut f.a, 0.into(), &generating, &Cancellation::default())
            .await.unwrap().segments.0, 0);
        let keys = cycle_keys::take();
        assert!(keys.complete());
        assert!(keys.selected_keys.is_empty());
        assert!(keys.attempted_keys.is_empty());
        assert!(keys.emitted_keys.is_empty());
        assert!(keys.affected_keys.is_empty());
        assert_eq!(keys.accepted_publications, 0);
        assert!(!f.a.lww_read_outbox(0.into(), 4096).unwrap().entries.is_empty());

        cycle_keys::reset();
        assert_eq!(f.publish_a().await.segments.0, 1);
        assert_eq!(f.receiver.receive_and_apply(&mut f.b, 0.into(), &generating,
            &Cancellation::default()).await.unwrap(), 1);
        let keys = cycle_keys::take();
        let expected = std::collections::BTreeSet::from([
            UnitKey::new(&["messages", "char", "conv"]).unwrap(),
        ]);
        assert!(keys.complete());
        assert_eq!(keys.selected_keys, expected);
        assert_eq!(keys.emitted_keys, expected);
        assert!(keys.affected_keys.is_empty());
        assert!(keys.held_keys.is_empty());
        assert_eq!(keys.deferred_keys, expected);
        assert_eq!(keys.receive_applies, 1);
    });
}
#[test]
fn crash_retry_uses_exact_ciphertext_nonce_sequence_and_capture() {
    run(async {
        let mut f = CycleFixture::new();
        set(&mut f.a, &["root", "language"], serde_json::json!("old"));
        f.provider.state.lock().unwrap().lose_response = true;
        assert!(f
            .sender
            .publish(&mut f.a, DecimalU64(0), &[], &Cancellation::default())
            .await
            .is_err());
        let writer = f.a.lww_clock_state().unwrap().writer_id;
        let (pending, sealed) =
            f.a.external_lww_pending(&f.sender.target_scope(), &writer)
                .unwrap()
                .unwrap();
        assert!(pending.dispatched);
        assert_eq!(f.provider.contents(&pending.object_id).unwrap(), sealed);
        f.a = PersistentStore::open(f.directory_a.path()).unwrap();
        let header = crate::persistent_store::lww::Header {
            binding_authority: 0.into(),
            request_id: "initial-requeue-after-response-loss".into(),
        };
        f.a.lww_queue_unit_state_page(&header, None, 4096).unwrap();
        assert_eq!(f.publish_a().await.segments.0, 1);
        assert_eq!(f.provider.upload_attempts(&pending.object_id), 1);
        assert_eq!(f.provider.uploaded_ids(), vec![pending.object_id.clone()]);
        assert_eq!(f.provider.contents(&pending.object_id).unwrap(), sealed);
        assert!(f
            .a
            .lww_read_outbox(0.into(), 4096)
            .unwrap()
            .entries
            .is_empty());
        assert_eq!(
            f.a.external_lww_next_sequence(&f.sender.target_scope(), &writer)
                .unwrap(),
            2
        );
        f.a.lww_queue_unit_state_page(&header, None, 4096).unwrap();
        assert!(f
            .a
            .lww_read_outbox(0.into(), 4096)
            .unwrap()
            .entries
            .is_empty());
        assert_eq!(f.publish_a().await.segments.0, 0);
        assert_eq!(f.provider.uploaded_ids(), vec![pending.object_id.clone()]);
        set(&mut f.a, &["root", "language"], serde_json::json!("new"));
        f.provider.state.lock().unwrap().lose_response = true;
        assert!(f
            .sender
            .publish(&mut f.a, 0.into(), &[], &Cancellation::default())
            .await
            .is_err());
        let (later_pending, later_sealed) =
            f.a.external_lww_pending(&f.sender.target_scope(), &writer)
                .unwrap()
                .unwrap();
        set(&mut f.a, &["root", "language"], serde_json::json!("newer"));
        f.a = PersistentStore::open(f.directory_a.path()).unwrap();
        f.a.lww_queue_unit_state_page(&header, None, 4096).unwrap();
        let newer = f.a.lww_read_outbox(0.into(), 4096).unwrap().entries;
        f.provider
            .fail_upload_number(f.provider.upload_count() + 1, ErrorKind::Transient);
        assert!(f
            .sender
            .publish(&mut f.a, 0.into(), &[], &Cancellation::default())
            .await
            .is_err());
        assert_eq!(f.a.lww_read_outbox(0.into(), 4096).unwrap().entries, newer);
        assert_eq!(f.provider.upload_attempts(&later_pending.object_id), 1);
        assert_eq!(
            f.provider.contents(&later_pending.object_id).unwrap(),
            later_sealed
        );
        assert_eq!(f.publish_a().await.segments.0, 1);
        assert_eq!(f.provider.upload_attempts(&pending.object_id), 1);
        assert_eq!(f.provider.contents(&pending.object_id).unwrap(), sealed);
        assert_eq!(
            f.a.external_lww_next_sequence(&f.sender.target_scope(), &writer)
                .unwrap(),
            4
        );
        f.receive_b().await;
        assert_eq!(f.b.read_root(None).unwrap().value["language"], "newer");
    })
}
#[test]
fn publisher_is_distinct_from_forwarded_unit_issuer() {
    run(async {
        let mut f = CycleFixture::new();
        set(&mut f.a, &["root", "language"], serde_json::json!("ko"));
        f.publish_a().await;
        f.receive_b().await;
        let issuer = f.a.lww_clock_state().unwrap().writer_id;
        let publisher = f.b.lww_clock_state().unwrap().writer_id;
        let before = f.b.lww_binding_state().unwrap();
        let target =
            crate::persistent_store::sync_selection::SyncTarget::External("forward".into());
        let inspection =
            f.b.register_lww_binding_inspection(
                before.target_authority,
                &target,
                "forward-target",
                "forward-library",
            )
            .unwrap();
        let state =
            f.b.switch_lww_binding(
                &crate::persistent_store::sync_selection::SwitchBindingRequest {
                    header: crate::persistent_store::lww::Header {
                        binding_authority: before.target_authority,
                        request_id: "forward-bind".into(),
                    },
                    expected_selection_epoch: before.selection_epoch,
                    target,
                    inspection_id: Some(inspection),
                },
            )
            .unwrap();
        let header = crate::persistent_store::lww::Header {
            binding_authority: state.target_authority,
            request_id: "forward-existing".into(),
        };
        f.b.lww_queue_unit_state_page(&header, None, 100).unwrap();
        f.receiver
            .publish(
                &mut f.b,
                state.target_authority,
                &[],
                &Cancellation::default(),
            )
            .await
            .unwrap();
        let id = f
            .provider
            .uploaded_ids()
            .into_iter()
            .find(|id| parse_segment_object_id(id).unwrap().0 == publisher)
            .unwrap();
        let (_, seq, _) = parse_segment_object_id(&id).unwrap();
        let payload = lww_segment::open(
            &f.provider.contents(&id).unwrap(),
            &f.sender.library,
            &publisher,
            seq,
            &[7; 32],
        )
        .unwrap();
        assert!(payload
            .changes
            .iter()
            .any(|change| change.stamp.writer_id == issuer));
    })
}
#[test]
fn gaps_do_not_advance_and_authenticated_variants_stop_even_after_consumption() {
    run(async {
        let mut f = CycleFixture::new();
        set(&mut f.a, &["root", "language"], serde_json::json!("one"));
        f.publish_a().await;
        let first = f.provider.uploaded_ids()[0].clone();
        let bytes = f.provider.contents(&first).unwrap();
        f.provider.forget(&first);
        set(&mut f.a, &["root", "language"], serde_json::json!("two"));
        f.publish_a().await;
        assert_eq!(f.receive_b().await, 0);
        assert!(f.b.lww_receive_progress(DecimalU64(0)).unwrap().is_empty());
        f.provider.seed(&first, ObjectRole::Segment, bytes.clone());
        assert_eq!(f.receive_b().await, 2);
        let (writer, seq, _) = parse_segment_object_id(&first).unwrap();
        let mut payload =
            lww_segment::open(&bytes, &f.sender.library, writer, seq, &[7; 32]).unwrap();
        payload.changes.clear();
        let (variant, _) = lww_segment::seal(&payload, &[7; 32]).unwrap();
        let id = segment_object_id(writer, seq, &lww_segment::digest(&variant)).unwrap();
        f.provider.seed(&id, ObjectRole::Segment, variant);
        assert_eq!(
            f.receiver
                .receive_requests(&mut f.b, DecimalU64(0), &Cancellation::default())
                .await
                .err()
                .unwrap()
                .kind,
            ErrorKind::Corrupt
        );
        assert_eq!(f.provider.read_attempts(&id), 1);
    })
}
#[test]
fn generating_whole_messages_are_held_and_later_applied() {
    run(async {
        let mut f = CycleFixture::new();
        conversation(&mut f.a);
        f.publish_a().await;
        f.receive_b().await;
        f.a.commit(&WorkingSetCommit {
            expected_revision: f.a.revision().unwrap(),
            conversations: Some(vec![ConversationMutation::ReplaceRange {
                character_id: "char".into(),
                conversation_id: "conv".into(),
                start: 0,
                delete_count: 0,
                messages: vec![
                    serde_json::json!({"data":"synthetic-message","chatId":"synthetic-id"}),
                ],
                conversation: None,
                configured_index: None,
            }]),
            ..Default::default()
        })
        .unwrap();
        f.publish_a().await;
        let generating = vec![MessageLocator {
            character_id: "char".into(),
            conversation_id: "conv".into(),
            start: None,
        }];
        f.receiver
            .receive_and_apply(
                &mut f.b,
                DecimalU64(0),
                &generating,
                &Cancellation::default(),
            )
            .await
            .unwrap();
        let header = crate::persistent_store::lww::Header {
            binding_authority: DecimalU64(0),
            request_id: "drain-generation".into(),
        };
        let result =
            f.b.lww_drain_deferred(&crate::persistent_store::lww::ApplyReceive {
                header,
                generating: vec![],
            })
            .unwrap();
        assert!(!result.affected_keys.is_empty());
    })
}
#[test]
fn large_bodies_stay_remote_then_only_missing_body_hydrates() {
    run(async {
        let mut f = CycleFixture::new();
        let body = vec![31; 5 * 1024 * 1024];
        let hash = risunest_sync_wire::hash(&body);
        f.a.lww_put_managed_object(&hash, &body).unwrap();
        let alias = crate::persistent_store::AssetAlias {
            key: "synthetic-asset".into(),
            object_hash: Some(hash.clone()),
            kind: "asset".into(),
            size: body.len() as i64,
            mime: "application/octet-stream".into(),
            name: "synthetic".into(),
            ext: "bin".into(),
            inlay_type: None,
            width: None,
            height: None,
            metadata: serde_json::json!({}),
        };
        f.a.commit_asset_alias(&alias, f.a.revision().unwrap())
            .unwrap();
        f.publish_a().await;
        let before = f.provider.transferred_body_bytes();
        f.receive_b().await;
        assert!(!f.b.external_lww_object_is_local(&hash).unwrap());
        let source = super::lww_residency::source(f.directory_b.path(), &hash)
            .unwrap()
            .unwrap();
        assert!(parse_segment_object_id(&source.protected_segment).is_ok());
        assert_eq!(
            f.provider
                .read_attempts(&source.body.locator.as_ref().unwrap().object),
            0
        );
        assert_eq!(
            super::lww_residency::stat(f.directory_b.path(), &hash).unwrap(),
            Some(body.len() as u64)
        );
        let mut file = super::lww_residency::fulfill(
            f.directory_b.path(),
            &hash,
            f.provider.as_ref(),
            &f.receiver.repository,
            &[7; 32],
            &Cancellation::default(),
        )
        .await
        .unwrap()
        .unwrap();
        let mut restored = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut restored).unwrap();
        assert_eq!(restored, body);
        let hydrated = f.provider.transferred_body_bytes();
        assert!(hydrated.1 > before.1 + body.len() as u64);
        lww_segment::reset_hash_bytes();
        assert!(super::lww_residency::fulfill(
            f.directory_b.path(),
            &hash,
            f.provider.as_ref(),
            &f.receiver.repository,
            &[7; 32],
            &Cancellation::default()
        )
        .await
        .unwrap()
        .is_some());
        assert_eq!(f.provider.transferred_body_bytes(), hydrated);
        assert_eq!(lww_segment::take_hash_bytes(), 0);
    })
}

fn binding_context(
    store: &PersistentStore,
    engine: &ExternalLwwEngine,
    request: &str,
) -> (crate::persistent_store::lww::Header, String) {
    let state = store.lww_binding_state().unwrap();
    let inspection = store
        .register_lww_binding_inspection(
            state.target_authority,
            &crate::persistent_store::sync_selection::SyncTarget::External(
                engine.connection_id.clone(),
            ),
            &engine.repository.connection_identity,
            &engine.library,
        )
        .unwrap();
    (
        crate::persistent_store::lww::Header {
            binding_authority: state.target_authority,
            request_id: request.into(),
        },
        inspection,
    )
}
#[test]
fn bootstrap_gap_blocks_activation_and_writer_authorization() {
    run(async {
        let mut f = CycleFixture::new();
        set(&mut f.a, &["root", "language"], serde_json::json!("one"));
        f.publish_a().await;
        let first = f.provider.uploaded_ids()[0].clone();
        f.provider.forget(&first);
        set(&mut f.a, &["root", "language"], serde_json::json!("two"));
        f.publish_a().await;
        let before = f.b.lww_clock_state().unwrap();
        let (header, inspection) = binding_context(&f.b, &f.receiver, "gap-binding");
        assert_eq!(
            f.receiver
                .stage_binding(&mut f.b, &header, &inspection, &Cancellation::default())
                .await
                .err()
                .unwrap()
                .kind,
            ErrorKind::PreconditionFailed
        );
        assert_eq!(f.b.lww_clock_state().unwrap().writer_id, before.writer_id);
        assert_eq!(
            f.b.device_store()
                .unwrap()
                .connection()
                .query_row(
                    "SELECT count(*) FROM lww_new_device_authorizations",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    })
}
#[test]
fn coherent_new_device_uses_native_settlement_and_collided_target_never_authorizes() {
    run(async {
        let mut f = CycleFixture::new();
        set(&mut f.a, &["root", "language"], serde_json::json!("remote"));
        f.publish_a().await;
        set(
            &mut f.b,
            &["root", "language"],
            serde_json::json!("unsent-local"),
        );
        let before = f.b.lww_clock_state().unwrap();
        let (header, inspection) = binding_context(&f.b, &f.receiver, "new-device");
        let stage = f
            .receiver
            .stage_binding(&mut f.b, &header, &inspection, &Cancellation::default())
            .await
            .unwrap();
        let first = f.provider.uploaded_ids()[0].clone();
        let (writer, seq, _) = parse_segment_object_id(&first).unwrap();
        let mut payload = lww_segment::open(
            &f.provider.contents(&first).unwrap(),
            &f.sender.library,
            writer,
            seq,
            &[7; 32],
        )
        .unwrap();
        payload.changes.clear();
        let (variant, _) = lww_segment::seal(&payload, &[7; 32]).unwrap();
        let id = segment_object_id(writer, seq, &lww_segment::digest(&variant)).unwrap();
        f.provider.seed(&id, ObjectRole::Segment, variant);
        assert_eq!(
            f.receiver
                .prepare_new_device(
                    &mut f.b,
                    &header,
                    &stage.staging_id,
                    &Cancellation::default()
                )
                .await
                .err()
                .unwrap()
                .kind,
            ErrorKind::Corrupt
        );
        assert_eq!(f.b.lww_clock_state().unwrap().writer_id, before.writer_id);
        assert_eq!(
            f.b.lww_binding_authority().unwrap(),
            before.binding_authority
        );
        assert_eq!(
            f.b.read_root(None).unwrap().value["language"],
            "unsent-local"
        );
        assert_eq!(
            f.b.device_store()
                .unwrap()
                .connection()
                .query_row(
                    "SELECT count(*) FROM lww_new_device_authorizations",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        f.provider.forget(&id);
        let prepared = f
            .receiver
            .prepare_new_device(
                &mut f.b,
                &header,
                &stage.staging_id,
                &Cancellation::default(),
            )
            .await
            .unwrap();
        let result =
            f.b.lww_replace_target_as_new_device(
                &header,
                &stage.staging_id,
                &prepared.authorization_id,
            )
            .unwrap();
        assert_ne!(result.writer_id, before.writer_id);
        assert_eq!(f.b.read_root(None).unwrap().value["language"], "remote");
        assert!(f
            .b
            .lww_read_outbox(result.binding_authority, 100)
            .unwrap()
            .entries
            .is_empty());
    })
}
#[test]
fn authenticated_future_stream_does_not_change_hlc_or_progress() {
    run(async {
        let mut f = CycleFixture::new();
        set(&mut f.a, &["root", "language"], serde_json::json!("future"));
        f.publish_a().await;
        let id = f.provider.uploaded_ids()[0].clone();
        let bytes = f.provider.contents(&id).unwrap();
        f.provider.forget(&id);
        let (writer, seq, _) = parse_segment_object_id(&id).unwrap();
        let mut payload =
            lww_segment::open(&bytes, &f.sender.library, writer, seq, &[7; 32]).unwrap();
        payload.changes[0].stamp.physical_ms = DecimalU64(super::runtime::now_ms() + 600_000);
        let (sealed, _) = lww_segment::seal(&payload, &[7; 32]).unwrap();
        let id = segment_object_id(writer, seq, &lww_segment::digest(&sealed)).unwrap();
        f.provider.seed(&id, ObjectRole::Segment, sealed);
        let before = f.b.lww_clock_state().unwrap();
        assert_eq!(
            f.receiver
                .receive_requests(&mut f.b, DecimalU64(0), &Cancellation::default())
                .await
                .err()
                .unwrap()
                .kind,
            ErrorKind::ClockSkew
        );
        let after = f.b.lww_clock_state().unwrap();
        assert_eq!(after.accepted, before.accepted);
        assert_eq!(after.issued, before.issued);
        assert!(f.b.lww_receive_progress(DecimalU64(0)).unwrap().is_empty());
    })
}
#[test]
fn dependency_response_loss_reuses_exact_frozen_ciphertext_before_segment_seal() {
    run(async {
        let mut f = CycleFixture::new();
        let body = vec![33; 5 * 1024 * 1024];
        let hash = risunest_sync_wire::hash(&body);
        f.a.lww_put_managed_object(&hash, &body).unwrap();
        let alias = crate::persistent_store::AssetAlias {
            key: "synthetic".into(),
            object_hash: Some(hash),
            kind: "asset".into(),
            size: body.len() as i64,
            mime: "application/octet-stream".into(),
            name: "synthetic".into(),
            ext: "bin".into(),
            inlay_type: None,
            width: None,
            height: None,
            metadata: serde_json::json!({}),
        };
        f.a.commit_asset_alias(&alias, f.a.revision().unwrap())
            .unwrap();
        f.provider.state.lock().unwrap().lose_response = true;
        assert!(f
            .sender
            .publish(&mut f.a, DecimalU64(0), &[], &Cancellation::default())
            .await
            .is_err());
        let writer = f.a.lww_clock_state().unwrap().writer_id;
        let (pending, bytes) =
            f.a.external_lww_pending(&f.sender.target_scope(), &writer)
                .unwrap()
                .unwrap();
        assert!(!pending.sealed);
        assert!(bytes.is_empty());
        assert_eq!(pending.seq.0, 1);
        assert_eq!(pending.bodies.len(), 1);
        let object = pending.bodies[0].object_id.clone();
        let sealed = f.provider.contents(&object).unwrap();
        f.a = PersistentStore::open(f.directory_a.path()).unwrap();
        f.publish_a().await;
        assert_eq!(f.provider.contents(&object).unwrap(), sealed);
        assert_eq!(
            f.a.external_lww_next_sequence(&f.sender.target_scope(), &writer)
                .unwrap(),
            2
        );
        assert_eq!(
            f.provider
                .uploaded_ids()
                .iter()
                .filter(|id| parse_segment_object_id(id).is_ok())
                .count(),
            1
        );
    })
}

#[test]
fn switching_targets_uses_independent_sequences_and_ack_receipts_without_restamping() {
    run(async {
        let mut f = CycleFixture::new();
        set(
            &mut f.a,
            &["root", "language"],
            serde_json::json!("preserved"),
        );
        f.publish_a().await;
        let original =
            f.a.lww_read_unit_state(DecimalU64(0), None, 100)
                .unwrap()
                .entries;
        let before = f.a.lww_binding_state().unwrap();
        let target = crate::persistent_store::sync_selection::SyncTarget::External("next".into());
        f.sender.library = "new-library".into();
        let inspection =
            f.a.register_lww_binding_inspection(
                before.target_authority,
                &target,
                &f.sender.repository.connection_identity,
                &f.sender.library,
            )
            .unwrap();
        let bound =
            f.a.switch_lww_binding(
                &crate::persistent_store::sync_selection::SwitchBindingRequest {
                    header: crate::persistent_store::lww::Header {
                        binding_authority: before.target_authority,
                        request_id: "different-target".into(),
                    },
                    expected_selection_epoch: before.selection_epoch,
                    target,
                    inspection_id: Some(inspection),
                },
            )
            .unwrap();
        let header = crate::persistent_store::lww::Header {
            binding_authority: bound.target_authority,
            request_id: "queue-target-initial".into(),
        };
        f.a.lww_queue_unit_state_page(&header, None, 100).unwrap();
        assert_eq!(
            f.sender
                .publish(
                    &mut f.a,
                    bound.target_authority,
                    &[],
                    &Cancellation::default()
                )
                .await
                .unwrap()
                .segments
                .0,
            1
        );
        let writer = f.a.lww_clock_state().unwrap().writer_id;
        assert_eq!(
            f.a.external_lww_next_sequence(&f.sender.target_scope(), &writer)
                .unwrap(),
            2
        );
        let after =
            f.a.lww_read_unit_state(bound.target_authority, None, 100)
                .unwrap()
                .entries;
        for prior in original {
            assert!(after
                .iter()
                .any(|entry| entry.key == prior.key && entry.stamp == prior.stamp));
        }
    })
}
#[test]
fn delegated_hash_observations_count_real_wire_invocations_without_rehashing() {
    use risunest_sync_wire::unit::UnitValue;
    let descriptor = risunest_sync_wire::descriptor::RecordDescriptor::content("a".repeat(64));
    let value = UnitValue::object(descriptor.clone()).unwrap();
    let bytes = descriptor.bytes().unwrap();
    let mut payload =
        lww_segment::Segment::new("library", "00000000-0000-4000-8000-000000000001", 1);
    payload.changes.push(crate::persistent_store::lww::Change {
        key: UnitKey::new(&["archive", "synthetic"]).unwrap(),
        stamp: risunest_sync_wire::stamp::Stamp {
            physical_ms: 1.into(),
            logical: 0,
            writer_id: payload.writer_id.clone(),
        },
        value: value.clone(),
    });
    lww_segment::reset_hash_bytes();
    lww_segment::reset_delegated_hash_work();
    payload.validate().unwrap();
    assert_eq!(lww_segment::take_hash_bytes(), 0);
    assert_eq!(lww_segment::take_hash_calls(), 0);
    assert_eq!(
        lww_segment::take_delegated_hash_work().get("segment-validate.descriptor"),
        Some(&(1, bytes.len() as u64))
    );
    let mut winners = std::collections::BTreeMap::new();
    lww_segment::merge(&mut winners, payload.changes[0].clone()).unwrap();
    lww_segment::merge(&mut winners, payload.changes[0].clone()).unwrap();
    let work = lww_segment::take_delegated_hash_work();
    assert_eq!(
        work["segment-merge.descriptor"],
        (2, 2 * bytes.len() as u64)
    );
    assert_eq!(
        work["segment-merge.identity"],
        (
            2,
            2 * risunest_sync_wire::canonical::encode(&value).unwrap().len() as u64
        )
    );
    assert_eq!(lww_segment::take_hash_bytes(), 0);
}
#[test]
fn production_cycle_reports_disjoint_native_and_transport_hash_work() {
    run(async {
        let mut fixture = CycleFixture::new();
        crate::persistent_store::hash_work::reset_hash_work();
        lww_segment::reset_hash_bytes();
        lww_segment::reset_delegated_hash_work();
        set(
            &mut fixture.a,
            &["root", "language"],
            serde_json::json!("counter"),
        );
        fixture.publish_a().await;
        fixture.receive_b().await;
        let native = crate::persistent_store::hash_work::take_hash_work();
        assert!(native.incomplete.is_empty());
        assert!(native.domains["native_unit_envelope"].calls > 0);
        assert!(lww_segment::take_hash_bytes() > 0);
        assert!(lww_segment::take_hash_calls() > 0);
        let delegated = lww_segment::take_delegated_hash_work();
        assert!(delegated["publish-ack.identity"].0 > 0);
        assert!(delegated["receive-history.identity"].0 > 0);
        assert!(!native.domains.contains_key("publish-ack.identity"));
        assert_eq!(
            fixture.b.read_root(None).unwrap().value["language"],
            "counter"
        );
    });
}

#[test]
fn superseded_published_values_retain_equal_stamp_integrity_history() {
    run(async {
        let mut f = CycleFixture::new();
        set(&mut f.a, &["root", "language"], serde_json::json!("first"));
        f.publish_a().await;
        let first = f.provider.uploaded_ids()[0].clone();
        let (writer, seq, _) = parse_segment_object_id(&first).unwrap();
        let mut payload = lww_segment::open(
            &f.provider.contents(&first).unwrap(),
            &f.sender.library,
            writer,
            seq,
            &[7; 32],
        )
        .unwrap();
        set(&mut f.a, &["root", "language"], serde_json::json!("later"));
        f.publish_a().await;
        payload.writer_id = f.b.lww_clock_state().unwrap().writer_id;
        payload.seq = 1.into();
        payload.changes[0].value =
            risunest_sync_wire::unit::UnitValue::inline(br#""changed""#).unwrap();
        let (bytes, _) = lww_segment::seal(&payload, &[7; 32]).unwrap();
        let id = segment_object_id(&payload.writer_id, 1, &lww_segment::digest(&bytes)).unwrap();
        f.provider.seed(&id, ObjectRole::Segment, bytes);
        let before = f.a.lww_clock_state().unwrap();
        assert_eq!(
            f.sender
                .receive_requests(&mut f.a, DecimalU64(0), &Cancellation::default())
                .await
                .err()
                .unwrap()
                .kind,
            ErrorKind::Corrupt
        );
        assert_eq!(f.a.lww_clock_state().unwrap().accepted, before.accepted);
        assert_eq!(f.a.read_root(None).unwrap().value["language"], "later");
    })
}
#[test]
fn native_unpublished_clock_repair_preserves_values_and_never_repairs_dispatched_bytes() {
    run(async {
        let mut f = CycleFixture::new();
        let future = risunest_sync_wire::stamp::Stamp {
            physical_ms: DecimalU64(super::runtime::now_ms() + 1_000_000),
            logical: 0,
            writer_id: f.a.lww_clock_state().unwrap().writer_id,
        };
        f.a.device_store()
            .unwrap()
            .connection()
            .execute(
                "UPDATE lww_clock SET issued=?1",
                [serde_json::to_string(&future).unwrap()],
            )
            .unwrap();
        set(
            &mut f.a,
            &["root", "language"],
            serde_json::json!("same-value"),
        );
        let original = f.a.lww_read_outbox(0.into(), 100).unwrap().entries;
        f.a.device_store_mut().unwrap().write_plugin_device_values("inactive", &[crate::persistent_store::device_store::plugin_values::PluginDeviceMutation::Set {space:"string".into(),key:"synthetic".into(),value:"kept".into()}]).unwrap();
        f.publish_a().await;
        assert_eq!(f.a.read_root(None).unwrap().value["language"], "same-value");
        assert!(
            f.a.lww_clock_state().unwrap().issued.unwrap().physical_ms
                < original[0].stamp.physical_ms
        );
        set(
            &mut f.a,
            &["root", "language"],
            serde_json::json!("dispatched"),
        );
        f.provider.state.lock().unwrap().lose_response = true;
        assert!(f
            .sender
            .publish(&mut f.a, 0.into(), &[], &Cancellation::default())
            .await
            .is_err());
        let writer = f.a.lww_clock_state().unwrap().writer_id;
        let (pending, bytes) =
            f.a.external_lww_pending(&f.sender.target_scope(), &writer)
                .unwrap()
                .unwrap();
        // A trusted time discontinuity cannot authorize rewriting an already dispatched intent.
        f.sender.invalidate_clock();
        assert_eq!(
            f.sender
                .publish(&mut f.a, 0.into(), &[], &Cancellation::default())
                .await
                .err()
                .unwrap()
                .kind,
            ErrorKind::ClockSkew
        );
        let (after, after_bytes) =
            f.a.external_lww_pending(&f.sender.target_scope(), &writer)
                .unwrap()
                .unwrap();
        assert_eq!(after.object_id, pending.object_id);
        assert_eq!(after_bytes, bytes);
    })
}
