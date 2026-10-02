#![cfg(test)]

#[path = "../../../benchmarks/lww-native/src/fixture.rs"]
mod fixture;
#[path = "../../../benchmarks/lww-native/src/native_observation.rs"]
mod native_observation;

use super::{lww::{self, MessageLocator, UnitMutation}, sync_selection::{SwitchBindingRequest, SyncTarget},
    ConversationMutation, PersistentStore, WorkingSetCommit};
use crate::{external_storage::{contract::Cancellation, lww_tests::CycleFixture, lww_segment},
    server_sync::{self, lww_client::LwwClient, lww_tests::LocalServerFixture}};
use fixture::{FixtureReceipt, FixtureScale};
use native_observation::NativeObservation;
use risunest_sync_wire::unit::UnitKey;
use serde::Serialize;
use serde_json::{json, Value};
use std::{fs::File, path::Path, sync::Arc, time::Instant};

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) enum SmallChange { Setting, Preset, Append }

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) enum Direction { AtoB, BtoA }

#[derive(Debug, Serialize)]
pub(crate) struct NativeSample {
    transport: &'static str,
    direction: Direction,
    change: SmallChange,
    iteration: u64,
    fixture_scale: FixtureScale,
    generated_json_bytes: u64,
    source_database_bytes_before_change: u64,
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

fn reset_work() {
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
    result
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
            message
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
            assert_eq!(messages.last().unwrap(), expected);
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

pub(crate) struct ExternalDriver {
    pub fixture: CycleFixture,
    _generated: tempfile::TempDir,
    pub receipt: FixtureReceipt,
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
        let authority = fixture.a.lww_binding_authority().unwrap();
        fixture.sender.publish(&mut fixture.a, authority, &[], &Cancellation::default()).await.unwrap();
        let authority = fixture.b.lww_binding_authority().unwrap();
        fixture.receiver.receive_and_apply(&mut fixture.b, authority, &[], &Cancellation::default()).await.unwrap();
        assert!(fixture.a.lww_read_outbox(fixture.a.lww_binding_authority().unwrap(), 1).unwrap().entries.is_empty());
        verify_bootstrap(&fixture.a, &fixture.b);
        Self {fixture, _generated:generated, receipt}
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
        reset_work();
        let started = Instant::now();
        source.commit(&commit).unwrap();
        let durable_save_ms = started.elapsed().as_secs_f64() * 1000.0;
        sender.publish(source, source.lww_binding_authority().unwrap(), &[], &Cancellation::default()).await.unwrap();
        let publication_complete_ms = started.elapsed().as_secs_f64() * 1000.0;
        receiver.receive_and_apply(destination, destination.lww_binding_authority().unwrap(), &[], &Cancellation::default()).await.unwrap();
        let receiver_durable_complete_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut observation = take_work(true);
        observation.requests = (provider.read_count() + provider.upload_count() + provider.listing_count()) as u64 - requests_before;
        let bytes_after = provider.transferred_body_bytes();
        observation.uploaded_bytes = bytes_after.0 - bytes_before.0;
        observation.downloaded_bytes = bytes_after.1 - bytes_before.1;
        observation.shared_head_reads = Some((provider.read_attempts("head") - head_reads_before) as u64);
        verify(destination, change, &expected);
        assert!(source.lww_read_outbox(source.lww_binding_authority().unwrap(), 1).unwrap().entries.is_empty());
        NativeSample {transport:"native-fake-provider", direction, change, iteration, observation,
            fixture_scale:self.receipt.scale.clone(), generated_json_bytes:self.receipt.database_bytes,
            source_database_bytes_before_change,
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
        server_sync::lww_tests::drain_publications(&ca, &mut a, &[]).unwrap();
        server_sync::lww_tests::receive_available(&cb, &mut b, &[]).unwrap();
        server_sync::lww_tests::receive_available(&ca, &mut a, &[]).unwrap();
        assert!(a.lww_read_outbox(a.lww_binding_authority().unwrap(), 1).unwrap().entries.is_empty());
        verify_bootstrap(&a, &b);
        Self {_server:server, _directories:[directory_a,directory_b,generated], a,b,ca,cb,receipt}
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
        reset_work();
        let started = Instant::now();
        source.commit(&commit).unwrap();
        let durable_save_ms = started.elapsed().as_secs_f64() * 1000.0;
        assert!(server_sync::lww_tests::publish_cycle(sender, source, &[]).unwrap().is_some());
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
        verify(destination, change, &expected);
        assert!(source.lww_read_outbox(source.lww_binding_authority().unwrap(), 1).unwrap().entries.is_empty());
        NativeSample {transport:"native-local-http", direction, change, iteration, observation,
            fixture_scale:self.receipt.scale.clone(), generated_json_bytes:self.receipt.database_bytes,
            source_database_bytes_before_change,
            durable_save_ms, publication_complete_ms, receiver_durable_complete_ms}
    }
}

fn measurement_gate() -> String {
    assert_eq!(std::env::var("RISUNEST_LWW_VERIFIED_WAVE2_HEAD").ok().as_deref(), Some("verified"),
        "root must release the measurement gate before writing smoke samples");
    let checkpoint = std::env::var("RISUNEST_LWW_MEASUREMENT_CHECKPOINT").expect("verified source checkpoint SHA");
    assert!(checkpoint.len() == 40 && checkpoint.bytes().all(|byte| byte.is_ascii_hexdigit()));
    checkpoint
}

pub(crate) fn write_samples(directory:&Path, samples:&[NativeSample]) {
    let checkpoint = measurement_gate();
    assert!(!samples.is_empty());
    for sample in samples { sample.observation.validate_routine_invariants().unwrap(); }
    std::fs::create_dir(directory).unwrap();
    serde_json::to_writer_pretty(File::create(directory.join("native-smoke.json")).unwrap(),
        &json!({"schema":"risunest-lww-native-smoke/v1", "scope":"native-content-identity-sha-inputs",
            "verified_checkpoint":checkpoint, "timing_origin":"native commit start; cumulative milliseconds",
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
    let reopened = PersistentStore::open(directory.path()).unwrap();
    verify(&reopened, SmallChange::Append, &expected);
    assert!(reopened.storage_stats().unwrap().database_bytes > 0);
}

#[test]
fn external_fixture_bootstrap_correctness() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let driver = ExternalDriver::prepare(FixtureScale::small()).await;
        assert_eq!(driver.fixture.a.read_conversation("synthetic-character-0", "synthetic-conversation-0", None)
            .unwrap().unwrap().value, driver.fixture.b.read_conversation("synthetic-character-0",
                "synthetic-conversation-0", None).unwrap().unwrap().value);
        assert_eq!(driver.fixture.provider.read_attempts("head"), 0);
    });
}

#[test]
fn local_server_fixture_bootstrap_correctness() {
    let driver = ServerDriver::prepare(FixtureScale::small());
    verify_bootstrap(&driver.a, &driver.b);
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
        for (a,b) in by_scale[0].iter().zip(&by_scale[1]) {
            native_observation::assert_frozen_growth(&a.observation, &b.observation, &[]).unwrap();
        }
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
    for (a,b) in by_scale[0].iter().zip(&by_scale[1]) {
        native_observation::assert_frozen_growth(&a.observation, &b.observation, &[]).unwrap();
    }
    write_samples(Path::new(&output), &by_scale.into_iter().flatten().collect::<Vec<_>>());
}
