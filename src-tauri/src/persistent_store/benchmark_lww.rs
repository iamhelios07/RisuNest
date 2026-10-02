#![cfg(test)]

#[path = "../../../benchmarks/lww-native/src/fixture.rs"]
mod fixture;
#[path = "../../../benchmarks/lww-native/src/native_observation.rs"]
mod native_observation;
#[path = "../../../benchmarks/lww-native/src/invalid_diagnostics.rs"]
mod invalid_diagnostics;

use super::{lww::{self, MessageLocator, UnitMutation}, sync_selection::{SwitchBindingRequest, SyncTarget},
    ConversationMutation, PersistentStore, WorkingSetCommit};
use crate::{external_storage::{contract::Cancellation, lww_tests::CycleFixture, lww_segment},
    server_sync::{self, lww_client::LwwClient, lww_tests::LocalServerFixture}};
use fixture::{FixtureReceipt, FixtureScale};
use native_observation::{BodyDomain, NativeObservation, ObjectBodyDomain};
use risunest_sync_wire::unit::UnitKey;
use serde::Serialize;
use serde_json::{json, Value};
use std::{fs::File, path::Path, sync::Arc, time::Instant};

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) enum SmallChange { Setting, Preset, Append }

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) enum Direction { AtoB, BtoA }

#[derive(Debug, Serialize)]
struct ExternalKeyScope {
    attempted_keys: Vec<String>,
    held_keys: Vec<String>,
    deferred_keys: Vec<String>,
    segment_attempts: usize,
    accepted_publications: usize,
    receive_applies: usize,
    failed_operations: usize,
    pending_operations: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct NativeSample {
    transport: &'static str,
    direction: Direction,
    change: SmallChange,
    iteration: u64,
    fixture_scale: FixtureScale,
    generated_json_bytes: u64,
    source_database_bytes_before_change: u64,
    selected_keys: Option<Vec<String>>,
    emitted_keys: Option<Vec<String>>,
    accepted_keys: Option<Vec<String>>,
    affected_keys: Option<Vec<String>>,
    external_key_scope: Option<ExternalKeyScope>,
    observation: NativeObservation,
    durable_save_ms: f64,
    publication_complete_ms: f64,
    receiver_durable_complete_ms: f64,
}

pub(crate) fn frozen_small_scales() -> [FixtureScale; 2] {
    [FixtureScale::small(), FixtureScale { minimum_database_bytes: 16 * 1024 * 1024,
        ..FixtureScale::small() }]
}

fn seed(store: &mut PersistentStore, directory: &Path, scale: FixtureScale) -> FixtureReceipt {
    assert!(frozen_small_scales().contains(&scale), "small driver accepts only frozen A/B fixtures");
    let receipt = fixture::generate(directory, scale.clone(), false).unwrap();
    let mut database: Value = serde_json::from_reader(File::open(directory.join("database.json")).unwrap()).unwrap();
    let characters = database.as_object_mut().unwrap().remove("characters").unwrap();
    let presets = database.as_object_mut().unwrap().remove("botPresets").unwrap();
    database["botPresetsId"] = json!("synthetic-preset-0");
    database["loreBookDepth"] = json!(5);
    let staging = store.replace_begin().unwrap().staging_id;
    store.replace_put_root(&staging, &database).unwrap();
    store.replace_put_presets(&staging, presets.as_array().unwrap()).unwrap();
    for character in characters.as_array().unwrap() {
        store.replace_add_characters(&staging, std::slice::from_ref(character)).unwrap();
    }
    store.replace_commit(&staging, Some(store.revision().unwrap())).unwrap();
    for index in 0..scale.assets {
        let descriptor = fixture::asset_descriptor(scale.seed, index);
        let alias = server_sync::lww_tests::put_asset(store, &descriptor.logical_key,
            &fixture::asset_body(scale.seed, index));
        assert_eq!(alias.object_hash.as_deref(), Some(descriptor.payload_hash.as_str()));
    }
    assert_eq!(store.read_conversation("synthetic-character-0", "synthetic-conversation-0", None)
        .unwrap().unwrap().value["message"].as_array().unwrap().len(), 4096);
    receipt
}

fn bind(store: &mut PersistentStore, target: SyncTarget, target_id: &str, library: &str) {
    let state = store.lww_binding_state().unwrap();
    let inspection = store.register_lww_binding_inspection(state.target_authority, &target, target_id, library).unwrap();
    store.switch_lww_binding(&SwitchBindingRequest {
        header: server_sync::lww_tests::header(store), expected_selection_epoch: state.selection_epoch,
        target, inspection_id: Some(inspection),
    }).unwrap();
}

fn fixture_asset_inventory(store:&PersistentStore, scale:&FixtureScale) -> Vec<String> {
    let aliases = store.list_asset_aliases(None).unwrap().value;
    assert_eq!(aliases.len(), scale.assets as usize);
    let hashes = aliases.into_iter().map(|alias| {
        assert_eq!(alias.kind, "asset");
        alias.object_hash.expect("fixture asset must have a cataloged object")
    }).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(hashes.len(), scale.assets as usize);
    let mut catalog = std::collections::BTreeSet::new();
    let mut cursor = None;
    loop {
        let page = store.query_asset_object_catalog(4096, cursor.as_deref()).unwrap();
        catalog.extend(page.items.into_iter().map(|item| item.object_hash));
        cursor = page.next_cursor;
        if cursor.is_none() { break; }
    }
    assert!(hashes.is_subset(&catalog), "fixture aliases must reference actual catalog rows");
    hashes.into_iter().collect()
}

fn reset_work(asset_hashes:&[String]) {
    super::hash_work::reset_receive_intent_inputs();
    crate::asset_repository::body_io::reset_body_io();
    for hash in asset_hashes {
        crate::asset_repository::body_io::register_object_purpose(hash,
            crate::asset_repository::body_io::BodyPurpose::Asset);
    }
    crate::external_storage::lww_engine::cycle_keys::reset();
    lww::reset_work_metrics();
    super::hash_work::reset_hash_work();
    super::message_pages::reset_capture_work();
    server_sync::hash_metrics::reset_hash_metrics();
    lww_segment::reset_hash_bytes();
    lww_segment::reset_delegated_hash_work();
}

fn take_work(external: bool) -> NativeObservation {
    let mut result = NativeObservation::default();
    // The second tuple field is an alias of native_unit_envelope, not another domain.
    result.units_visited = lww::take_work_metrics().0;
    let native = super::hash_work::take_hash_work();
    for (key, work) in native.domains { result.insert_domain("native", key, work.calls, work.bytes); }
    for (key, count) in native.incomplete {
        result.incomplete.push(format!("native/{key}: {count}"));
    }
    let c = server_sync::hash_metrics::take_hash_metrics();
    if !external {
        for (key, work) in c.domains { result.insert_domain("C", &key, work.calls, work.bytes); }
        if c.incomplete { result.incomplete.push("C/delegated".into()); }
    }
    let direct_calls = lww_segment::take_hash_calls();
    let direct_bytes = lww_segment::take_hash_bytes();
    let delegated = lww_segment::take_delegated_hash_work();
    if external {
        result.insert_domain("X1", "content-digest", direct_calls, direct_bytes);
        for (key, (calls, bytes)) in delegated { result.insert_domain("X1", key, calls, bytes); }
    }
    let capture = super::message_pages::take_capture_work();
    result.messages_read = capture.work.messages_read as u64;
    result.message_bytes_read = capture.work.bytes_read;
    result.pages_written = capture.work.pages_written as u64;
    result.pages_reused = (capture.work.prefix_reused + capture.work.suffix_reused) as u64;
    if capture.failed_captures > 0 { result.incomplete.push("native/failed-capture".into()); }
    let bodies = crate::asset_repository::body_io::take_body_io();
    result.body_scope_complete = Some(bodies.complete() && bodies.thread == std::thread::current().id());
    result.body_scope_thread = Some(format!("{:?}", bodies.thread));
    result.body_stat_requests = bodies.stat_requests;
    result.body_batch_stat_requests = bodies.batch_stat_requests;
    result.body_presence_queries = bodies.presence_queries;
    result.body_asset_work = body_domain(bodies.asset_work());
    result.body_control_work = body_domain(bodies.control_work());
    result.body_owned_work = body_domain(bodies.domains.get("owned").cloned().unwrap_or_default());
    result.body_unknown_work = body_domain(bodies.unknown_work());
    result.body_unattributed_managed = body_domain(bodies.unattributed_managed.clone());
    result.body_scope_violations = bodies.scope_violations;
    result.asset_body_opens = Some(result.body_asset_work.open_attempts);
    result.asset_body_bytes_read = Some(result.body_asset_work.read_bytes);
    for (hash, object) in bodies.objects {
        result.body_objects.insert(hash, ObjectBodyDomain {
            purposes:object.purposes.into_iter().map(|purpose| format!("{purpose:?}")).collect(),
            work:body_domain(object.work),
        });
    }
    for (domain, work) in bodies.domains {
        result.body_domains.insert(domain.into(), body_domain(work));
    }
    let inputs=super::hash_work::take_receive_intent_inputs();
    let observed=result.hashes.get("native/native_receive_intent").cloned().unwrap_or_default();
    let mut evidence=invalid_diagnostics::ReceiveIntentEvidence {
        observed_calls:observed.calls,observed_bytes:observed.bytes,captured_calls:inputs.len() as u64,
        ..Default::default()};
    for input in inputs {
        evidence.captured_bytes=evidence.captured_bytes.checked_add(input.len() as u64).unwrap();
        let structure=(|| -> Result<invalid_diagnostics::JsonByteStructure,String> {
            let request:lww::StageReceive=serde_json::from_slice(&input).map_err(|e|e.to_string())?;
            let roundtrip=serde_json::to_vec(&request).map_err(|e|e.to_string())?;
            invalid_diagnostics::exact_json_structure(&input,&roundtrip)
        })();
        match structure {Ok(value)=>evidence.inputs.push(value),Err(error)=>evidence.errors.push(error)}
    }
    evidence.finish();
    result.receive_intents=Some(evidence);
    result
}

fn body_domain(work:crate::asset_repository::body_io::BodyWork) -> BodyDomain {
    BodyDomain {open_attempts:work.open_attempts, opens:work.opens, failed_opens:work.failed_opens,
        unknown_open_results:work.unknown_open_results, read_operations:work.read_operations,
        read_bytes:work.read_bytes, incomplete_reads:work.incomplete_reads,
        escaped_handles:work.escaped_handles, escaped_paths:work.escaped_paths,
        outstanding_readers:work.outstanding_readers}
}

fn require_routine_observation(observation:&NativeObservation, transport:&str,
    direction:Direction, change:SmallChange, iteration:u64) {
    if let Err(error) = observation.validate_routine_invariants() {
        eprintln!("{}", serde_json::to_string_pretty(&json!({
            "kind":"synthetic-native-observation-failure", "transport":transport,
            "direction":direction, "change":change, "iteration":iteration,
            "validation_thread":format!("{:?}",std::thread::current().id()),
            "error":error, "observation":observation,
        })).unwrap());
        panic!("invalid native sample: {error}");
    }
}

fn prepare_edit(store: &PersistentStore, change: SmallChange, direction: Direction, iteration: u64) -> (WorkingSetCommit, Value) {
    let root = store.read_root(None).unwrap().value;
    let mut commit = WorkingSetCommit { expected_revision:store.revision().unwrap(),
        binding_authority:Some(store.lww_binding_authority().unwrap()),
        request_id:Some(uuid::Uuid::new_v4().to_string()), ..Default::default() };
    let expected = match change {
        SmallChange::Setting => {
            let next = json!(root["loreBookDepth"].as_i64().unwrap() + 1);
            commit.unit_mutations = Some(vec![UnitMutation::Set {
                key:UnitKey::new(&["root", "loreBookDepth"]).unwrap(), value:next.clone() }]);
            next
        },
        SmallChange::Preset => {
            let next = json!(if root["botPresetsId"] == "synthetic-preset-0" {
                "synthetic-preset-1" } else { "synthetic-preset-0" });
            commit.unit_mutations = Some(vec![UnitMutation::Set {
                key:UnitKey::new(&["root", "botPresetsId"]).unwrap(), value:next.clone() }]);
            next
        },
        SmallChange::Append => {
            let conversation = store.read_conversation("synthetic-character-0", "synthetic-conversation-0", None)
                .unwrap().unwrap().value;
            let start = conversation["message"].as_array().unwrap().len() as i64;
            let message = json!({"role":"user", "data":"Fixed synthetic appended message",
                "chatId":format!("synthetic-append-{direction:?}-{iteration}")});
            commit.conversations = Some(vec![ConversationMutation::ReplaceRange {
                character_id:"synthetic-character-0".into(), conversation_id:"synthetic-conversation-0".into(),
                start, delete_count:0, messages:vec![message.clone()], conversation:None, configured_index:None }]);
            commit.messages_changed = Some(vec![MessageLocator {
                character_id:"synthetic-character-0".into(), conversation_id:"synthetic-conversation-0".into(), start:Some(start) }]);
            let mut expected_messages = conversation["message"].as_array().unwrap().clone();
            expected_messages.push(message);
            Value::Array(expected_messages)
        },
    };
    (commit, expected)
}

fn verify(store: &PersistentStore, change: SmallChange, expected: &Value) {
    match change {
        SmallChange::Setting => assert_eq!(&store.read_root(None).unwrap().value["loreBookDepth"], expected),
        SmallChange::Preset => assert_eq!(&store.read_root(None).unwrap().value["botPresetsId"], expected),
        SmallChange::Append => {
            let conversation = store.read_conversation("synthetic-character-0", "synthetic-conversation-0", None)
                .unwrap().unwrap().value;
            let messages = conversation["message"].as_array().unwrap();
            assert!(messages == expected.as_array().unwrap(), "append changed prior message content or fields");
            let unique = messages.iter().map(|message| message["chatId"].as_str().unwrap())
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(unique.len(), messages.len());
        },
    }
}

fn verify_bootstrap(a:&PersistentStore, b:&PersistentStore) {
    let mut local = a.read_root(None).unwrap().value;
    let mut remote = b.read_root(None).unwrap().value;
    // temperature is a derived preset field, excluded from shared root units.
    local.as_object_mut().unwrap().remove("temperature");
    remote.as_object_mut().unwrap().remove("temperature");
    assert_eq!(local, remote);
    assert_eq!(a.read_conversation("synthetic-character-0", "synthetic-conversation-0", None)
        .unwrap().unwrap().value, b.read_conversation("synthetic-character-0",
            "synthetic-conversation-0", None).unwrap().unwrap().value);
}

fn verify_settled(store:&PersistentStore, engine:&crate::external_storage::lww_engine::ExternalLwwEngine) {
    assert!(store.lww_read_outbox(store.lww_binding_authority().unwrap(), 1).unwrap().entries.is_empty());
    assert!(store.external_lww_pending(&engine.target_scope(), &store.lww_clock_state().unwrap().writer_id)
        .unwrap().is_none());
    let device = store.device_store().unwrap().connection();
    for query in ["SELECT COUNT(*) FROM lww_intents WHERE complete=0",
        "SELECT COUNT(*) FROM lww_receive WHERE finished=0",
        "SELECT COUNT(*) FROM lww_receive_rows WHERE status IN ('staged','held','deferred')"] {
        assert_eq!(device.query_row(query, [], |row| row.get::<_,i64>(0)).unwrap(), 0);
    }
}

pub(crate) struct ExternalDriver {
    pub fixture: CycleFixture,
    _generated: tempfile::TempDir,
    pub receipt: FixtureReceipt,
    asset_hashes: Vec<String>,
}

impl ExternalDriver {
    pub(crate) async fn prepare(scale: FixtureScale) -> Self {
        let mut fixture = CycleFixture::new();
        assert_ne!(fixture.a.lww_clock_state().unwrap().writer_id, fixture.b.lww_clock_state().unwrap().writer_id);
        bind(&mut fixture.a, SyncTarget::External(fixture.sender.connection_id.clone()),
            &fixture.sender.repository.connection_identity, &fixture.sender.library);
        bind(&mut fixture.b, SyncTarget::External(fixture.receiver.connection_id.clone()),
            &fixture.receiver.repository.connection_identity, &fixture.receiver.library);
        let generated = tempfile::tempdir().unwrap();
        let receipt = seed(&mut fixture.a, &generated.path().join("fixture"), scale);
        let asset_hashes = fixture_asset_inventory(&fixture.a, &receipt.scale);
        let authority = fixture.a.lww_binding_authority().unwrap();
        fixture.sender.publish(&mut fixture.a, authority, &[], &Cancellation::default()).await.unwrap();
        let authority = fixture.b.lww_binding_authority().unwrap();
        fixture.receiver.receive_and_apply(&mut fixture.b, authority, &[], &Cancellation::default()).await.unwrap();
        verify_settled(&fixture.a, &fixture.sender);
        verify_settled(&fixture.b, &fixture.receiver);
        let writer = fixture.a.lww_clock_state().unwrap().writer_id;
        let published_prefix = fixture.a.external_lww_next_sequence(&fixture.sender.target_scope(), &writer).unwrap() - 1;
        // Publication ACK advances the sender's own receive prefix durably.
        for store in [&fixture.a, &fixture.b] {
            let progress = store.lww_receive_progress(store.lww_binding_authority().unwrap()).unwrap();
            let own = progress.iter().find(|item| item.kind == "external" && item.writer_id.as_deref() == Some(writer.as_str())).unwrap();
            assert_eq!(own.cursor.0, published_prefix);
        }
        verify_bootstrap(&fixture.a, &fixture.b);
        Self {fixture, _generated:generated, receipt, asset_hashes}
    }

    pub(crate) async fn run(&mut self, direction:Direction, change:SmallChange, iteration:u64) -> NativeSample {
        let provider = self.fixture.provider.clone();
        let requests_before = (provider.read_count() + provider.upload_count() + provider.listing_count()) as u64;
        let bytes_before = provider.transferred_body_bytes();
        let head_reads_before = provider.read_attempts("head");
        let (source, destination, sender, receiver) = match direction {
            Direction::AtoB => (&mut self.fixture.a, &mut self.fixture.b, &mut self.fixture.sender, &mut self.fixture.receiver),
            Direction::BtoA => (&mut self.fixture.b, &mut self.fixture.a, &mut self.fixture.receiver, &mut self.fixture.sender),
        };
        let (commit, expected) = prepare_edit(source, change, direction, iteration);
        let source_database_bytes_before_change = source.storage_stats().unwrap().database_bytes;
        reset_work(&self.asset_hashes);
        let started = Instant::now();
        source.commit(&commit).unwrap();
        let durable_save_ms = started.elapsed().as_secs_f64() * 1000.0;
        sender.publish(source, source.lww_binding_authority().unwrap(), &[], &Cancellation::default()).await.unwrap();
        let publication_complete_ms = started.elapsed().as_secs_f64() * 1000.0;
        receiver.receive_and_apply(destination, destination.lww_binding_authority().unwrap(), &[], &Cancellation::default()).await.unwrap();
        let receiver_durable_complete_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut observation = take_work(true);
        let keys = crate::external_storage::lww_engine::cycle_keys::take();
        assert!(keys.complete(), "failed or unfinished external key observation");
        let selected_keys = key_set(keys.selected_keys);
        let emitted_keys = key_set(keys.emitted_keys);
        let affected_keys = key_set(keys.affected_keys);
        let external_key_scope = ExternalKeyScope {
            attempted_keys:key_set(keys.attempted_keys), held_keys:key_set(keys.held_keys),
            deferred_keys:key_set(keys.deferred_keys), segment_attempts:keys.segment_attempts,
            accepted_publications:keys.accepted_publications, receive_applies:keys.receive_applies,
            failed_operations:keys.failed_operations, pending_operations:keys.pending_operations,
        };
        observation.requests = (provider.read_count() + provider.upload_count() + provider.listing_count()) as u64 - requests_before;
        let bytes_after = provider.transferred_body_bytes();
        observation.uploaded_bytes = bytes_after.0 - bytes_before.0;
        observation.downloaded_bytes = bytes_after.1 - bytes_before.1;
        observation.shared_head_reads = Some((provider.read_attempts("head") - head_reads_before) as u64);
        require_routine_observation(&observation, "fake-provider", direction, change, iteration);
        verify(destination, change, &expected);
        assert!(source.lww_read_outbox(source.lww_binding_authority().unwrap(), 1).unwrap().entries.is_empty());
        NativeSample {transport:"native-fake-provider", direction, change, iteration, observation,
            fixture_scale:self.receipt.scale.clone(), generated_json_bytes:self.receipt.database_bytes,
            source_database_bytes_before_change,
            selected_keys:Some(selected_keys), emitted_keys:Some(emitted_keys), accepted_keys:None,
            affected_keys:Some(affected_keys), external_key_scope:Some(external_key_scope),
            durable_save_ms, publication_complete_ms, receiver_durable_complete_ms}
    }
}

pub(crate) struct ServerDriver {
    _server: LocalServerFixture,
    _directories: [tempfile::TempDir; 3],
    a: PersistentStore,
    b: PersistentStore,
    ca: LwwClient,
    cb: LwwClient,
    pub receipt: FixtureReceipt,
    asset_hashes: Vec<String>,
}

impl ServerDriver {
    pub(crate) fn prepare(scale:FixtureScale) -> Self {
        let server = LocalServerFixture::new();
        let (directory_a, mut a) = server_sync::lww_tests::local();
        let (directory_b, mut b) = server_sync::lww_tests::local();
        assert_ne!(a.lww_clock_state().unwrap().writer_id, b.lww_clock_state().unwrap().writer_id);
        let ca = server.client(&a);
        let cb = server.client(&b);
        let config = ca.client.config();
        assert_eq!(config.library_id, cb.client.config().library_id);
        assert_ne!(config.device_id, cb.client.config().device_id);
        for store in [&mut a, &mut b] {
            bind(store, SyncTarget::Server(server.endpoint.clone()), &server.endpoint, &config.library_id);
        }
        let generated = tempfile::tempdir().unwrap();
        let receipt = seed(&mut a, &generated.path().join("fixture"), scale);
        let asset_hashes = fixture_asset_inventory(&a, &receipt.scale);
        server_sync::lww_tests::drain_publications(&ca, &mut a, &[]).unwrap();
        server_sync::lww_tests::receive_available(&cb, &mut b, &[]).unwrap();
        server_sync::lww_tests::receive_available(&ca, &mut a, &[]).unwrap();
        assert!(a.lww_read_outbox(a.lww_binding_authority().unwrap(), 1).unwrap().entries.is_empty());
        verify_bootstrap(&a, &b);
        Self {_server:server, _directories:[directory_a,directory_b,generated], a,b,ca,cb,receipt,asset_hashes}
    }

    pub(crate) fn run(&mut self, direction:Direction, change:SmallChange, iteration:u64) -> NativeSample {
        let (source, destination, sender, receiver) = match direction {
            Direction::AtoB => (&mut self.a,&mut self.b,&self.ca,&self.cb),
            Direction::BtoA => (&mut self.b,&mut self.a,&self.cb,&self.ca),
        };
        let io = [sender.client.test_io.as_ref().unwrap(), receiver.client.test_io.as_ref().unwrap()];
        assert!(!Arc::ptr_eq(io[0], io[1]));
        for counter in io { counter.reset(); }
        let (commit, expected) = prepare_edit(source, change, direction, iteration);
        let source_database_bytes_before_change = source.storage_stats().unwrap().database_bytes;
        reset_work(&self.asset_hashes);
        let started = Instant::now();
        source.commit(&commit).unwrap();
        let durable_save_ms = started.elapsed().as_secs_f64() * 1000.0;
        let receipt = server_sync::lww_tests::publish_cycle(sender, source, &[]).unwrap().unwrap();
        let publication_complete_ms = started.elapsed().as_secs_f64() * 1000.0;
        let completion = server_sync::lww_tests::receive_cycle(receiver, destination, &[]).unwrap();
        assert!(completion.received_units > 0);
        let receiver_durable_complete_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut observation = take_work(false);
        // The native local-server LWW protocol has no shared mutable head API.
        observation.shared_head_reads = Some(0);
        for counter in io {
            let [requests,sent,received] = counter.snapshot();
            observation.requests += requests;
            observation.uploaded_bytes += sent;
            observation.downloaded_bytes += received;
        }
        require_routine_observation(&observation, "local-server", direction, change, iteration);
        // Read the actual submitted operation only after timing and collector take.
        let (body, intent):(Vec<u8>, String) = sender.log.0.query_row(
            "SELECT body,intent FROM publications WHERE id=?1", [&receipt.operation_id],
            |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
        let request: risunest_sync_wire::lww::PushRequest = serde_json::from_slice(&body).unwrap();
        let publication: server_sync::lww_client::Publication = serde_json::from_str(&intent).unwrap();
        assert_eq!(request.operation_id, receipt.operation_id);
        assert_eq!(publication.request, request);
        let selected_keys = key_set(publication.entries.into_iter().map(|entry| entry.key));
        // Submitted immutable body keys include dominated changes; accepted keys are winners.
        let emitted_keys = key_set(request.changes.into_iter().map(|change| change.key));
        assert_eq!(selected_keys, emitted_keys);
        let accepted_keys = key_set(receipt.accepted_keys);
        let affected_keys = key_set(completion.result.affected_keys);
        verify(destination, change, &expected);
        assert!(source.lww_read_outbox(source.lww_binding_authority().unwrap(), 1).unwrap().entries.is_empty());
        NativeSample {transport:"native-local-http", direction, change, iteration, observation,
            fixture_scale:self.receipt.scale.clone(), generated_json_bytes:self.receipt.database_bytes,
            source_database_bytes_before_change,
            selected_keys:Some(selected_keys), emitted_keys:Some(emitted_keys),
            accepted_keys:Some(accepted_keys), affected_keys:Some(affected_keys),
            external_key_scope:None,
            durable_save_ms, publication_complete_ms, receiver_durable_complete_ms}
    }
}

fn key_set(keys:impl IntoIterator<Item=UnitKey>) -> Vec<String> {
    keys.into_iter().map(|key| key.as_str().to_owned()).collect::<std::collections::BTreeSet<_>>()
        .into_iter().collect()
}

fn assert_same_key_sets(a:&NativeSample, b:&NativeSample) {
    for (name, small, larger) in [("selected", &a.selected_keys, &b.selected_keys),
        ("emitted", &a.emitted_keys, &b.emitted_keys), ("affected", &a.affected_keys, &b.affected_keys)] {
        assert!(small.is_some() && larger.is_some(), "unobserved {name} key set");
        assert_eq!(small, larger, "{name} key-set growth");
    }
    assert_eq!(a.accepted_keys, b.accepted_keys, "accepted key-set growth");
    if let (Some(small), Some(larger)) = (&a.external_key_scope, &b.external_key_scope) {
        assert_eq!(small.attempted_keys, larger.attempted_keys, "dispatch key-set growth");
        assert_eq!(small.held_keys, larger.held_keys, "held key-set growth");
        assert_eq!(small.deferred_keys, larger.deferred_keys, "deferred key-set growth");
    } else {
        assert_eq!(a.external_key_scope.is_some(), b.external_key_scope.is_some());
    }
}

fn measurement_gate() -> String {
    assert_eq!(std::env::var("RISUNEST_LWW_VERIFIED_WAVE2_HEAD").ok().as_deref(), Some("verified"),
        "root must release the measurement gate before writing smoke samples");
    let checkpoint = std::env::var("RISUNEST_LWW_MEASUREMENT_CHECKPOINT").expect("verified source checkpoint SHA");
    assert!(checkpoint.len() == 40 && checkpoint.bytes().all(|byte| byte.is_ascii_hexdigit()));
    checkpoint
}

fn paired_diagnostics(a:&NativeSample,b:&NativeSample) -> Value {
    let domains=a.observation.hashes.keys().chain(b.observation.hashes.keys())
        .collect::<std::collections::BTreeSet<_>>();
    let growth=domains.into_iter().filter_map(|domain| {
        let baseline=a.observation.hashes.get(domain).cloned().unwrap_or_default();
        let trial=b.observation.hashes.get(domain).cloned().unwrap_or_default();
        if trial.calls<=baseline.calls && trial.bytes<=baseline.bytes {return None;}
        Some(json!({"domain":domain,"baseline":baseline,"trial":trial}))
    }).collect::<Vec<_>>();
    let mut intent_deltas=Vec::new();
    if let (Some(baseline),Some(trial))=(&a.observation.receive_intents,&b.observation.receive_intents) {
        for (index,(left,right)) in baseline.inputs.iter().zip(&trial.inputs).enumerate() {
            intent_deltas.push(json!({"input_index":index,"baseline_bytes":left.input_bytes,
                "trial_bytes":right.input_bytes,"exclusive_field_deltas":invalid_diagnostics::byte_deltas(left,right)}));
        }
    }
    json!({"iteration":a.iteration,"change":a.change,"direction":a.direction,
        "all_positive_hash_domain_growth":growth,"receive_intent_deltas":intent_deltas,
        "baseline_receive_intents":a.observation.receive_intents,"trial_receive_intents":b.observation.receive_intents})
}

fn write_invalid_diagnostics(directory:&Path,by_scale:&[Vec<NativeSample>],reason:&str,index:Option<usize>) {
    std::fs::create_dir(directory).unwrap();
    let comparisons=if by_scale.len()==2 {
        by_scale[0].iter().zip(&by_scale[1]).map(|(a,b)|paired_diagnostics(a,b)).collect::<Vec<_>>()
    } else {Vec::new()};
    let output=std::fs::OpenOptions::new().write(true).create_new(true)
        .open(directory.join("INVALID-native-diagnostics.json")).unwrap();
    serde_json::to_writer_pretty(output,&json!({"schema":"risunest.lww-invalid-diagnostics/v1",
        "status":"INVALID","accepted":false,"failure_reason":reason,"failed_comparison_index":index,
        "checkpoint":std::env::var("RISUNEST_LWW_MEASUREMENT_CHECKPOINT").ok(),
        "field_semantics":"original JSON token spans, exclusive own bytes conserve each actual SHA input; typed roundtrip required",
        "attribution":"transport control is a candidate only; changes, values, keys and framing have no allowance",
        "comparisons":comparisons,"all_post_take_samples":by_scale})).unwrap();
}

fn validate_smoke_pairs(directory:&Path,by_scale:&[Vec<NativeSample>]) -> Result<(),String> {
    if by_scale.len()!=2 || by_scale[0].len()!=by_scale[1].len() {
        let reason="incomplete paired sample matrix";
        write_invalid_diagnostics(directory,by_scale,reason,None);
        return Err(reason.into());
    }
    for (index,(a,b)) in by_scale[0].iter().zip(&by_scale[1]).enumerate() {
        if [a,b].iter().any(|sample|!sample.observation.receive_intents.as_ref().is_some_and(|e|e.complete)) {
            let reason="incomplete exact receive-intent input provenance";
            write_invalid_diagnostics(directory,by_scale,reason,Some(index));
            return Err(reason.into());
        }
        let keys=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||assert_same_key_sets(a,b)));
        let result=if keys.is_err() {Err("exact key-set comparison failed".into())}
            else {native_observation::assert_frozen_growth(&a.observation,&b.observation,&[])};
        if let Err(error)=result {
            write_invalid_diagnostics(directory,by_scale,&error,Some(index));
            return Err(error);
        }
    }
    Ok(())
}

pub(crate) fn write_samples(directory:&Path, samples:&[NativeSample]) {
    let checkpoint = measurement_gate();
    assert!(!samples.is_empty());
    for sample in samples {
        sample.observation.validate_routine_invariants().unwrap();
        assert!(sample.observation.receive_intents.as_ref().is_some_and(|evidence|evidence.complete),
            "receive-intent input provenance must be complete before accepted output");
        assert!(sample.selected_keys.is_some() && sample.emitted_keys.is_some() && sample.affected_keys.is_some(),
            "key-set observation must be linked before smoke output");
        if let Some(keys) = &sample.external_key_scope {
            assert_eq!(keys.failed_operations, 0);
            assert_eq!(keys.pending_operations, 0);
        }
    }
    std::fs::create_dir(directory).unwrap();
    serde_json::to_writer_pretty(File::create(directory.join("native-smoke.json")).unwrap(),
        &json!({"schema":"risunest-lww-native-smoke/v1", "scope":"native-content-identity-sha-inputs",
            "verified_checkpoint":checkpoint, "timing_origin":"native commit start; cumulative milliseconds",
            "bootstrap_phase":"excluded; both endpoints settled before warmup or sample counters",
            "capture_work_scope":"partial native message capture work; SHA totals include separately observed full content identity inputs",
            "body_work_semantics":{
                "domains":"all-purpose actual CAS boundary totals, not summed again with object or role work",
                "asset":"actual asset-purpose object IO, including mixed asset/control objects",
                "control":"exclusively actual control-purpose object IO",
                "unknown":"unclassified canonical object IO; invalidates samples",
                "owned":"staging/import CAS body IO without per-object role coverage; any nonzero work invalidates these fixed routine samples",
                "inventory":"actual fixture asset aliases checked against persisted catalog during excluded setup, re-registered after every reset"},
            "key_semantics":{
                "local_http_emitted":"immutable body submitted to /push, including dominated submissions",
                "local_http_accepted":"winning server keys",
                "external_emitted":"authenticated publication changes after exact native ACK",
                "external_attempted":"keys at actual segment create dispatch",
                "affected":"actual native ApplyResult after durable completion"},
            "excluded":["server internal SHA", "MAC", "HKDF", "general hashing", "renderer paint", "scheduler empty polls"],
            "samples":samples})).unwrap();
}

#[test]
fn frozen_fixture_stages_real_native_rows_and_unique_message_ids() {
    let (directory, mut store) = server_sync::lww_tests::local();
    let generated = tempfile::tempdir().unwrap();
    let receipt = seed(&mut store, &generated.path().join("fixture"), FixtureScale::small());
    assert_eq!(receipt.scale.assets, 64);
    assert_eq!(receipt.scale.long_conversation_messages, 4096);
    let (commit, expected) = prepare_edit(&store, SmallChange::Append, Direction::AtoB, 0);
    store.commit(&commit).unwrap();
    verify(&store, SmallChange::Append, &expected);
    store.checkpoint(super::CheckpointMode::Truncate).unwrap();
    let mut reopened = PersistentStore::open(directory.path()).unwrap();
    verify(&reopened, SmallChange::Append, &expected);
    assert!(reopened.storage_stats().unwrap().database_bytes > 0);
    let mut prior = expected[0].clone();
    prior["data"] = json!("synthetic corrupted prefix");
    reopened.commit(&WorkingSetCommit {
        expected_revision:reopened.revision().unwrap(),
        conversations:Some(vec![ConversationMutation::ReplaceRange {
            character_id:"synthetic-character-0".into(), conversation_id:"synthetic-conversation-0".into(),
            start:0, delete_count:1, messages:vec![prior], conversation:None, configured_index:None }]),
        messages_changed:Some(vec![MessageLocator {
            character_id:"synthetic-character-0".into(), conversation_id:"synthetic-conversation-0".into(), start:Some(0) }]),
        ..Default::default()
    }).unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
        verify(&reopened, SmallChange::Append, &expected))).is_err(),
        "same last message and unique IDs must not hide a changed earlier prefix");
}

#[test]
fn external_fixture_bootstrap_correctness() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut driver = ExternalDriver::prepare(FixtureScale::small()).await;
        assert_eq!(driver.fixture.a.read_conversation("synthetic-character-0", "synthetic-conversation-0", None)
            .unwrap().unwrap().value, driver.fixture.b.read_conversation("synthetic-character-0",
                "synthetic-conversation-0", None).unwrap().unwrap().value);
        assert_eq!(driver.fixture.provider.read_attempts("head"), 0);
        let reads = driver.fixture.provider.read_count();
        let bytes = driver.fixture.provider.transferred_body_bytes();
        let authority = driver.fixture.a.lww_binding_authority().unwrap();
        assert_eq!(driver.fixture.sender.receive_and_apply(&mut driver.fixture.a, authority, &[],
            &Cancellation::default()).await.unwrap(), 0);
        assert_eq!(driver.fixture.provider.read_count(), reads);
        assert_eq!(driver.fixture.provider.transferred_body_bytes(), bytes);
        verify_settled(&driver.fixture.a, &driver.fixture.sender);
        verify_settled(&driver.fixture.b, &driver.fixture.receiver);
    });
}

#[test]
fn local_server_fixture_bootstrap_correctness() {
    let driver = ServerDriver::prepare(FixtureScale::small());
    verify_bootstrap(&driver.a, &driver.b);
}

#[test]
fn local_routine_observation_regression() {
    let mut driver = ServerDriver::prepare(FixtureScale::small());
    for change in [SmallChange::Setting, SmallChange::Preset, SmallChange::Append] {
        for direction in [Direction::AtoB, Direction::BtoA] {
            let sample = driver.run(direction, change, 0);
            sample.observation.validate_routine_invariants().unwrap();
            let evidence=sample.observation.receive_intents.as_ref().unwrap();
            assert!(evidence.complete);
            assert_eq!(evidence.observed_calls,evidence.captured_calls);
            assert_eq!(evidence.observed_bytes,evidence.captured_bytes);
            assert_eq!(evidence.inputs.iter().map(|input|input.input_bytes).sum::<u64>(),evidence.observed_bytes);
        }
    }
}

#[test]
fn actual_body_observer_binding_counts_reads_and_preserves_catalog_only_scope() {
    let directory = tempfile::tempdir().unwrap();
    let cas = crate::asset_repository::PayloadCas::new(directory.path()).unwrap();
    let object = cas.prepare_bytes(b"synthetic-body-data").unwrap();
    reset_work(&[]);
    assert_eq!(cas.stat_object(&object.content_hash).unwrap(), Some(19));
    let mut catalog = take_work(false);
    catalog.shared_head_reads = Some(0);
    assert_eq!(catalog.asset_body_opens, Some(0));
    assert_eq!(catalog.asset_body_bytes_read, Some(0));
    assert_eq!(catalog.body_stat_requests, 1);
    catalog.validate_routine_invariants().unwrap();

    reset_work(std::slice::from_ref(&object.content_hash));
    crate::asset_repository::body_io::register_object_purpose(&object.content_hash,
        crate::asset_repository::body_io::BodyPurpose::Control);
    assert_eq!(cas.read_object(&object.content_hash).unwrap().unwrap(), b"synthetic-body-data");
    let mut body = take_work(false);
    body.shared_head_reads = Some(0);
    assert_eq!(body.asset_body_opens, Some(1));
    assert_eq!(body.asset_body_bytes_read, Some(19));
    assert_eq!(body.body_domains["managed"].read_operations, 1);
    assert_eq!(body.body_asset_work.read_bytes, 19);
    assert_eq!(body.body_control_work, BodyDomain::default());
    assert_eq!(body.body_scope_complete, Some(true));
    assert!(body.validate_routine_invariants().is_err());
}

#[test]
fn actual_body_observer_distinguishes_control_and_unknown_reads() {
    use crate::asset_repository::body_io::{register_object_purpose, BodyPurpose};
    let directory = tempfile::tempdir().unwrap();
    let cas = crate::asset_repository::PayloadCas::new(directory.path()).unwrap();
    let object = cas.prepare_bytes(b"synthetic-control-data").unwrap();
    reset_work(&[]);
    register_object_purpose(&object.content_hash, BodyPurpose::Control);
    assert_eq!(cas.read_object(&object.content_hash).unwrap().unwrap(), b"synthetic-control-data");
    let mut control = take_work(false);
    control.shared_head_reads = Some(0);
    assert_eq!(control.body_control_work.open_attempts, 1);
    assert_eq!(control.body_control_work.read_bytes, 22);
    assert_eq!(control.body_objects[&object.content_hash].purposes, vec!["Control"]);
    assert_eq!(control.asset_body_opens, Some(0));
    control.validate_routine_invariants().unwrap();

    reset_work(&[]);
    assert_eq!(cas.read_object(&object.content_hash).unwrap().unwrap(), b"synthetic-control-data");
    let mut unknown = take_work(false);
    unknown.shared_head_reads = Some(0);
    assert_eq!(unknown.body_unknown_work.open_attempts, 1);
    assert_eq!(unknown.body_unknown_work.read_bytes, 22);
    assert_eq!(unknown.body_scope_complete, Some(false));
    assert!(unknown.validate_routine_invariants().is_err());
}

#[test]
fn actual_body_observer_binding_rejects_escaped_file_and_path() {
    let directory = tempfile::tempdir().unwrap();
    let cas = crate::asset_repository::PayloadCas::new(directory.path()).unwrap();
    let object = cas.prepare_bytes(b"synthetic-body-data").unwrap();
    reset_work(std::slice::from_ref(&object.content_hash));
    let file = cas.open_object(&object.content_hash).unwrap().unwrap();
    let mut escaped = take_work(false);
    escaped.shared_head_reads = Some(0);
    assert_eq!(escaped.body_domains["managed"].escaped_handles, 1);
    assert_eq!(escaped.body_scope_complete, Some(false));
    assert!(escaped.validate_routine_invariants().is_err());
    drop(file);
    reset_work(std::slice::from_ref(&object.content_hash));
    assert!(cas.object_path(&object.content_hash).unwrap().is_some());
    let mut path = take_work(false);
    path.shared_head_reads = Some(0);
    assert_eq!(path.asset_body_opens, Some(0));
    assert_eq!(path.body_domains["managed"].escaped_paths, 1);
    assert_eq!(path.body_scope_complete, Some(false));
    assert!(path.validate_routine_invariants().is_err());
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires root's verified Wave2 checkpoint and complete body observers"]
fn windows_fake_provider_smoke() {
    measurement_gate();
    let output = std::env::var_os("RISUNEST_LWW_NATIVE_OUTPUT").expect("separate synthetic output directory");
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut by_scale = Vec::new();
        for scale in frozen_small_scales() {
            let mut driver = ExternalDriver::prepare(scale).await;
            let mut samples = Vec::new();
            for iteration in 0..4 {
                for change in [SmallChange::Setting, SmallChange::Preset, SmallChange::Append] {
                    for direction in [Direction::AtoB, Direction::BtoA] {
                        let sample = driver.run(direction, change, iteration).await;
                        if iteration > 0 { samples.push(sample); }
                    }
                }
            }
            by_scale.push(samples);
        }
        validate_smoke_pairs(Path::new(&output),&by_scale).unwrap();
        write_samples(Path::new(&output), &by_scale.into_iter().flatten().collect::<Vec<_>>());
    });
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires root's verified Wave2 checkpoint and complete body observers"]
fn windows_local_server_smoke() {
    measurement_gate();
    let output = std::env::var_os("RISUNEST_LWW_NATIVE_OUTPUT").expect("separate synthetic output directory");
    let mut by_scale = Vec::new();
    for scale in frozen_small_scales() {
        let mut driver = ServerDriver::prepare(scale);
        let mut samples = Vec::new();
        for iteration in 0..4 {
            for change in [SmallChange::Setting, SmallChange::Preset, SmallChange::Append] {
                for direction in [Direction::AtoB, Direction::BtoA] {
                    let sample = driver.run(direction, change, iteration);
                    if iteration > 0 { samples.push(sample); }
                }
            }
        }
        by_scale.push(samples);
    }
    validate_smoke_pairs(Path::new(&output),&by_scale).unwrap();
    write_samples(Path::new(&output), &by_scale.into_iter().flatten().collect::<Vec<_>>());
}

#[test]
fn failed_pair_persists_all_positive_domains_without_accepted_output() {
    fn sample(bytes:u64) -> NativeSample {
        let mut observation=NativeObservation::default();
        observation.hashes.insert("native/native_receive_intent".into(),native_observation::HashDomain {calls:1,bytes});
        observation.hashes.insert("other/domain".into(),native_observation::HashDomain {calls:1,bytes});
        NativeSample {transport:"diagnostic-validator",direction:Direction::AtoB,change:SmallChange::Setting,
            iteration:1,fixture_scale:FixtureScale::small(),generated_json_bytes:0,source_database_bytes_before_change:0,
            selected_keys:Some(Vec::new()),emitted_keys:Some(Vec::new()),accepted_keys:Some(Vec::new()),
            affected_keys:Some(Vec::new()),external_key_scope:None,observation,
            durable_save_ms:0.0,publication_complete_ms:0.0,receiver_durable_complete_ms:0.0}
    }
    let directory=tempfile::tempdir().unwrap();
    let output=directory.path().join("invalid");
    let matrix=vec![vec![sample(10)],vec![sample(11)]];
    assert!(validate_smoke_pairs(&output,&matrix).is_err());
    assert!(!output.join("native-smoke.json").exists());
    let artifact:Value=serde_json::from_reader(File::open(output.join("INVALID-native-diagnostics.json")).unwrap()).unwrap();
    assert_eq!(artifact["status"],"INVALID");
    assert_eq!(artifact["accepted"],false);
    assert_eq!(artifact["comparisons"][0]["all_positive_hash_domain_growth"].as_array().unwrap().len(),2);
    assert_eq!(artifact["all_post_take_samples"].as_array().unwrap().len(),2);
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "explicit diagnostic only, always writes INVALID output"]
fn local_receive_intent_growth_diagnostic() {
    let output=std::env::var_os("RISUNEST_LWW_NATIVE_OUTPUT").expect("fresh separate diagnostic directory");
    let mut by_scale=Vec::new();
    for scale in frozen_small_scales() {
        let mut driver=ServerDriver::prepare(scale);
        let mut samples=Vec::new();
        for iteration in 0..4 {
            for change in [SmallChange::Setting,SmallChange::Preset,SmallChange::Append] {
                for direction in [Direction::AtoB,Direction::BtoA] {
                    samples.push(driver.run(direction,change,iteration));
                }
            }
        }
        by_scale.push(samples);
    }
    write_invalid_diagnostics(Path::new(&output),&by_scale,
        "diagnostic only: all executed warmup and recorded operations retained; no acceptance authorized",None);
}
