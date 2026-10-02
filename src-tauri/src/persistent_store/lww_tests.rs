use super::*;

fn store() -> (tempfile::TempDir, PersistentStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = PersistentStore::open(dir.path()).unwrap();
    (dir, store)
}
fn mutation(key: &[&str], value: Value) -> UnitMutation {
    UnitMutation::Set {
        key: UnitKey::new(key).unwrap(),
        value,
    }
}
fn save(store: &mut PersistentStore, mutations: Vec<UnitMutation>) -> RevisionResult {
    store
        .commit(&WorkingSetCommit {
            expected_revision: store.revision().unwrap(),
            unit_mutations: Some(mutations),
            ..Default::default()
        })
        .unwrap()
}
fn stamp(time: u64) -> Stamp {
    Stamp {
        physical_ms: time.into(),
        logical: 0,
        writer_id: "00000000-0000-4000-8000-000000000001".into(),
    }
}
fn receive(
    store: &mut PersistentStore,
    id: &str,
    changes: Vec<Change>,
    generating: Vec<MessageLocator>,
) -> ApplyResult {
    let header = Header {
        binding_authority: store.lww_binding_authority().unwrap(),
        request_id: id.into(),
    };
    store
        .lww_stage_receive(&StageReceive {
            header: header.clone(),
            changes,
            progress: Progress {
                kind: "server".into(),
                cursor: 1.into(),
                writer_id: None,
            },
            admitted_time_upper_ms: u64::MAX.into(),
        })
        .unwrap();
    let result = store
        .lww_apply_receive(&ApplyReceive {
            header: header.clone(),
            generating,
        })
        .unwrap();
    store.lww_finish_receive(&header).unwrap();
    result
}
fn change(parts: &[&str], time: u64, value: Value) -> Change {
    Change {
        key: UnitKey::new(parts).unwrap(),
        stamp: stamp(time),
        value: inline(&value).unwrap(),
    }
}

#[test]
fn unchanged_units_emit_nothing_and_a_commit_shares_one_stamp() {
    let (_dir, mut store) = store();
    save(
        &mut store,
        vec![
            mutation(&["root", "openAIKey"], serde_json::json!("synthetic")),
            mutation(&["root", "language"], serde_json::json!("en")),
        ],
    );
    let first = store.lww_read_outbox(0.into(), 10).unwrap();
    assert_eq!(first.entries.len(), 2);
    assert_eq!(first.entries[0].stamp, first.entries[1].stamp);
    store
        .lww_ack_outbox(
            &Header {
                binding_authority: 0.into(),
                request_id: "ack".into(),
            },
            &first
                .entries
                .iter()
                .map(|e| AckEntry {
                    key: e.key.clone(),
                    version: e.version.clone(),
                    stamp: e.stamp.clone(),
                    value_identity: e.value.identity().unwrap(),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    save(
        &mut store,
        vec![mutation(
            &["root", "openAIKey"],
            serde_json::json!("synthetic"),
        )],
    );
    assert!(store
        .lww_read_outbox(0.into(), 10)
        .unwrap()
        .entries
        .is_empty());
}
#[test]
fn later_local_edit_survives_exact_version_ack() {
    let (_dir, mut store) = store();
    save(
        &mut store,
        vec![mutation(&["root", "openAIKey"], serde_json::json!("a"))],
    );
    let old = store
        .lww_read_outbox(0.into(), 1)
        .unwrap()
        .entries
        .remove(0);
    save(
        &mut store,
        vec![mutation(&["root", "openAIKey"], serde_json::json!("b"))],
    );
    store
        .lww_ack_outbox(
            &Header {
                binding_authority: 0.into(),
                request_id: "ack".into(),
            },
            &[AckEntry {
                key: old.key,
                version: old.version,
                stamp: old.stamp,
                value_identity: old.value.identity().unwrap(),
            }],
        )
        .unwrap();
    assert_eq!(
        store.lww_read_outbox(0.into(), 10).unwrap().entries.len(),
        1
    );
}
#[test]
fn remote_lww_compares_pending_state_and_equal_stamp_integrity_is_rejected() {
    let (_dir, mut store) = store();
    save(
        &mut store,
        vec![mutation(&["root", "openAIKey"], serde_json::json!("local"))],
    );
    let old = store
        .lww_read_outbox(0.into(), 1)
        .unwrap()
        .entries
        .remove(0);
    receive(
        &mut store,
        "older",
        vec![change(
            &["root", "openAIKey"],
            old.stamp.physical_ms.0 - 1,
            serde_json::json!("old"),
        )],
        vec![],
    );
    assert_eq!(
        store.lww_read_outbox(0.into(), 10).unwrap().entries.len(),
        1
    );
    receive(
        &mut store,
        "newer",
        vec![change(
            &["root", "openAIKey"],
            old.stamp.physical_ms.0 + 1,
            serde_json::json!("new"),
        )],
        vec![],
    );
    assert!(store
        .lww_read_outbox(0.into(), 10)
        .unwrap()
        .entries
        .is_empty());
    assert_eq!(store.read_root(None).unwrap().value["openAIKey"], "new");
    let mut conflicting = change(
        &["root", "openAIKey"],
        old.stamp.physical_ms.0 + 1,
        serde_json::json!("bad"),
    );
    conflicting.stamp = stamp(old.stamp.physical_ms.0 + 1);
    assert!(store
        .lww_stage_receive(&StageReceive {
            header: Header {
                binding_authority: 0.into(),
                request_id: "bad".into()
            },
            changes: vec![conflicting],
            progress: Progress {
                kind: "server".into(),
                cursor: 2.into(),
                writer_id: None
            },
            admitted_time_upper_ms: u64::MAX.into()
        })
        .unwrap_err()
        .to_string()
        .contains("equal-stamp-integrity"));
}
#[test]
fn missing_parent_is_durable_and_hard_retirement_suppresses_future_children() {
    let (dir, mut store) = store();
    let result = receive(
        &mut store,
        "child",
        vec![change(
            &["character", "char", "name"],
            10,
            serde_json::json!("child"),
        )],
        vec![],
    );
    assert_eq!(result.held_keys.len(), 1);
    drop(store);
    let mut store = PersistentStore::open(dir.path()).unwrap();
    receive(
        &mut store,
        "parent",
        vec![change(
            &["exists", "character", "char"],
            9,
            serde_json::json!({"type":"character"}),
        )],
        vec![],
    );
    assert_eq!(
        store.read_character("char", None).unwrap().unwrap().value["name"],
        "child"
    );
    receive(
        &mut store,
        "delete",
        vec![Change {
            key: UnitKey::new(&["exists", "character", "char"]).unwrap(),
            stamp: stamp(20),
            value: UnitValue::Deleted,
        }],
        vec![],
    );
    receive(
        &mut store,
        "stale",
        vec![
            change(
                &["exists", "character", "char"],
                30,
                serde_json::json!({"type":"character"}),
            ),
            change(
                &["character", "char", "name"],
                31,
                serde_json::json!("revival"),
            ),
        ],
        vec![],
    );
    assert!(store.read_character("char", None).unwrap().is_none());
    assert!(is_retired(
        &store.connection,
        &unit_key(&["exists", "character", "char"]).unwrap()
    )
    .unwrap());
}
#[test]
fn unknown_units_are_opaque_and_never_recaptured() {
    let (_dir, mut store) = store();
    let unknown = change(&["future-unit", "x"], 1, serde_json::json!({"opaque":2}));
    receive(&mut store, "opaque", vec![unknown.clone()], vec![]);
    save(
        &mut store,
        vec![mutation(&["root", "language"], serde_json::json!("ko"))],
    );
    assert_eq!(
        read_unit(&store.connection, &unknown.key)
            .unwrap()
            .unwrap()
            .1,
        unknown.value
    );
    assert!(store
        .lww_read_outbox(0.into(), 10)
        .unwrap()
        .entries
        .iter()
        .all(|e| e.key != unknown.key));
}

#[test]
fn commit_intent_replays_once_after_restart() {
    let (dir, mut store) = store();
    let header = Header {
        binding_authority: 0.into(),
        request_id: "crash-local".into(),
    };
    let input = WorkingSetCommit {
        expected_revision: 0,
        unit_mutations: Some(vec![mutation(
            &["root", "language"],
            serde_json::json!("ko"),
        )]),
        ..Default::default()
    };
    let intent = Intent::Commit {
        commit: input.clone(),
        aliases: vec![],
    };
    let (issued, _) = store.reserve_intent(&header, &intent).unwrap();
    drop(store);
    let mut store = PersistentStore::open(dir.path()).unwrap();
    assert_eq!(store.read_root(None).unwrap().value["language"], "ko");
    let revision = store.revision().unwrap();
    store.lww_recover_intents().unwrap();
    assert_eq!(store.revision().unwrap(), revision);
    let entry = store
        .lww_read_outbox(0.into(), 10)
        .unwrap()
        .entries
        .remove(0);
    assert_eq!(entry.stamp, issued);
}

#[test]
fn receive_progress_moves_only_after_durable_apply_and_exact_finish() {
    let (dir, mut store) = store();
    let header = Header {
        binding_authority: 0.into(),
        request_id: "durable-receive".into(),
    };
    let staged = StageReceive {
        header: header.clone(),
        changes: vec![change(&["root", "language"], 10, serde_json::json!("ko"))],
        progress: Progress {
            kind: "external".into(),
            cursor: 9.into(),
            writer_id: Some(stamp(1).writer_id),
        },
        admitted_time_upper_ms: 100.into(),
    };
    store.lww_stage_receive(&staged).unwrap();
    assert!(store.lww_finish_receive(&header).is_err());
    assert!(store.lww_receive_progress(0.into()).unwrap().is_empty());
    store
        .lww_apply_receive(&ApplyReceive {
            header: header.clone(),
            generating: vec![],
        })
        .unwrap();
    drop(store);
    let mut store = PersistentStore::open(dir.path()).unwrap();
    assert_eq!(store.read_root(None).unwrap().value["language"], "ko");
    assert!(store.lww_receive_progress(0.into()).unwrap().is_empty());
    store.lww_finish_receive(&header).unwrap();
    store.lww_finish_receive(&header).unwrap();
    assert_eq!(
        store.lww_receive_progress(0.into()).unwrap()[0].cursor,
        9.into()
    );
}

#[test]
fn independent_fields_converge_in_both_delivery_orders() {
    let (_, mut a) = store();
    let (_, mut b) = store();
    let one = change(&["root", "language"], 10, serde_json::json!("ko"));
    let two = change(&["root", "username"], 20, serde_json::json!("synthetic"));
    receive(&mut a, "one", vec![one.clone()], vec![]);
    receive(&mut a, "two", vec![two.clone()], vec![]);
    receive(&mut b, "two", vec![two], vec![]);
    receive(&mut b, "one", vec![one], vec![]);
    assert_eq!(
        a.read_root(None).unwrap().value,
        b.read_root(None).unwrap().value
    );
}

fn create_conversation(store: &mut PersistentStore, char_id: &str, conv_id: &str) {
    save(
        store,
        vec![
            mutation(
                &["exists", "character", char_id],
                serde_json::json!({"type":"character"}),
            ),
            mutation(
                &["exists", "conversation", char_id, conv_id],
                serde_json::json!(true),
            ),
        ],
    );
}
fn copy_objects(source: &PersistentStore, target: &PersistentStore) {
    let mut statement = source
        .connection
        .prepare("SELECT hash,body FROM message_page_objects")
        .unwrap();
    for row in statement
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .unwrap()
    {
        let (hash, body) = row.unwrap();
        target.lww_put_object(&hash, &body).unwrap();
    }
}

#[test]
fn incremental_message_receive_staging_and_generation_deferral_reuse_after_restart() {
    use crate::persistent_store::hash_work::{reset_hash_work,take_hash_work,DomainWork};
    for generating in [false,true] {
        let (_,mut sender) = store(); let (directory,mut target) = store();
        create_conversation(&mut sender,"char","chat"); create_conversation(&mut target,"char","chat");
        let messages = (0..1024).map(|i| serde_json::json!({"chatId":format!("m-{i}"),"data":format!("synthetic-{i}")})).collect::<Vec<_>>();
        let replace = |store: &mut PersistentStore,start,delete_count,messages| {
            store.commit(&WorkingSetCommit { expected_revision:store.revision().unwrap(),
                conversations:Some(vec![super::super::ConversationMutation::ReplaceRange {
                    character_id:"char".into(),conversation_id:"chat".into(),start,delete_count,messages,
                    conversation:None,configured_index:None,
                }]),..Default::default() }).unwrap();
        };
        replace(&mut sender,0,0,messages.clone()); replace(&mut target,0,0,messages);
        let start = if generating { 500 } else { 1024 };
        let id = if generating { "m-500" } else { "appended" };
        replace(&mut sender,start,i64::from(generating),vec![serde_json::json!({"chatId":id,"data":"remote-update"})]);
        copy_objects(&sender,&target);
        let key = UnitKey::new(&["messages","char","chat"]).unwrap();
        let entry = sender.lww_read_outbox(0.into(),100).unwrap().entries.into_iter().find(|e| e.key==key).unwrap();
        let expected = entry.value.clone();
        let UnitValue::Object { descriptor,.. } = &expected else { unreachable!() };
        assert!(target.lww_verified_object_present(&descriptor.object_hash).unwrap());
        assert!(!target.lww_verified_object_present(&"00".repeat(32)).unwrap());
        let manifest = risunest_external_storage_format::message_pages::MessageManifest::decode(
            &target.lww_object_body(&descriptor.object_hash).unwrap().unwrap()).unwrap();
        let new_pages = manifest.pages.iter().filter(|page| !target.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM message_page_proofs WHERE hash=?1)",[&page.hash],|r|r.get::<_,bool>(0)).unwrap()).collect::<Vec<_>>();
        let message_calls = new_pages.iter().map(|p|p.message_count as u64).sum::<u64>();
        let message_bytes = new_pages.iter().map(|p|p.byte_length.0-(
            risunest_external_storage_format::message_pages::PAGE_PREFIX.len()+
            risunest_external_storage_format::message_pages::PAGE_SUFFIX.len()) as u64-(p.message_count as u64-1)).sum::<u64>();
        let change = Change { key:key.clone(),stamp:stamp(u64::MAX-1),value:entry.value };
        let header = Header { binding_authority:target.lww_binding_authority().unwrap(),request_id:format!("incremental-{generating}") };
        let original_rowids = target.connection.prepare("SELECT message_id,rowid FROM messages ORDER BY message_index").unwrap()
            .query_map([],|r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).unwrap()
            .collect::<Result<std::collections::BTreeMap<_,_>,_>>().unwrap();
        target.connection.execute_batch("CREATE TABLE page_receive_writes(kind TEXT);
            CREATE TRIGGER page_receive_insert AFTER INSERT ON messages BEGIN INSERT INTO page_receive_writes VALUES('insert'); END;
            CREATE TRIGGER page_receive_delete AFTER DELETE ON messages BEGIN INSERT INTO page_receive_writes VALUES('delete'); END;").unwrap();
        reset_hash_work();
        target.lww_stage_receive(&StageReceive { header:header.clone(),changes:vec![change.clone()],
            progress:Progress { kind:"server".into(),cursor:1.into(),writer_id:None },admitted_time_upper_ms:u64::MAX.into() }).unwrap();
        let admission = take_hash_work();
        assert_eq!(admission.domains["native_message_verify"],DomainWork { calls:message_calls,bytes:message_bytes });
        assert_eq!(admission.domains["native_page_decode_identity"],DomainWork {
            calls:new_pages.len() as u64,bytes:new_pages.iter().map(|p|p.byte_length.0).sum(),
        });
        assert!(message_calls<256,"fixed fixture must reuse unchanged pages");
        assert!(admission.incomplete.is_empty());
        drop(target); let mut target = PersistentStore::open(directory.path()).unwrap();
        reset_hash_work();
        let applied = target.lww_apply_receive(&ApplyReceive { header:header.clone(),generating:if generating {
            vec![MessageLocator { character_id:"char".into(),conversation_id:"chat".into(),start:None }]
        } else { vec![] } }).unwrap();
        let work = take_hash_work();
        for domain in ["native_message_verify","native_page_decode_identity","native_repage_message_verify","native_repage_page_identity"] {
            assert!(!work.domains.contains_key(domain),"restart apply must skip {domain}: {work:?}");
        }
        assert!(work.incomplete.is_empty());
        target.lww_finish_receive(&header).unwrap();
        if generating {
            assert_eq!(applied.deferred_keys,vec![key.clone()]);
            assert_eq!(target.connection.query_row("SELECT count(*) FROM page_receive_writes",[],|r|r.get::<_,i64>(0)).unwrap(),0);
            drop(target); target = PersistentStore::open(directory.path()).unwrap();
            reset_hash_work();
            let drained = target.lww_drain_deferred(&ApplyReceive { header:Header {
                binding_authority:target.lww_binding_authority().unwrap(),request_id:"incremental-drain".into(),
            },generating:vec![] }).unwrap();
            assert!(drained.affected_keys.contains(&key));
            let work = take_hash_work();
            for domain in ["native_message_verify","native_page_decode_identity","native_repage_message_verify","native_repage_page_identity"] {
                assert!(!work.domains.contains_key(domain),"deferred drain must skip {domain}: {work:?}");
            }
            assert!(work.incomplete.is_empty());
        } else { assert!(applied.affected_keys.contains(&key)); }
        let counts = |kind| target.connection.query_row("SELECT count(*) FROM page_receive_writes WHERE kind=?1",[kind],|r|r.get::<_,i64>(0)).unwrap();
        assert_eq!(counts("insert"),1); assert_eq!(counts("delete"),i64::from(generating));
        for (id,rowid) in target.connection.prepare("SELECT message_id,rowid FROM messages ORDER BY message_index").unwrap()
            .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap() {
            if id=="m-500" && generating { continue; }
            if let Some(old) = original_rowids.get(&id) { assert_eq!(*old,rowid,"unchanged {id}"); }
        }
        assert_eq!(read_unit(&target.connection,&key).unwrap(),Some((change.stamp,expected)));
        let generation = super::super::active_generation(&target.connection).unwrap();
        assert_eq!(target.connection.query_row("SELECT value FROM messages WHERE generation=?1 AND message_index=?2",params![generation,start],
            |r|r.get::<_,String>(0)).unwrap(),format!("{{\"chatId\":\"{id}\",\"data\":\"remote-update\"}}"));
    }
}

#[test]
fn every_generating_message_list_is_deferred_but_metadata_applies() {
    let (_, mut sender) = store();
    let (dir, mut target) = store();
    for conv in ["one", "two"] {
        create_conversation(&mut sender, "char", conv);
        create_conversation(&mut target, "char", conv);
    }
    let first = sender.lww_read_outbox(0.into(), 100).unwrap();
    sender
        .lww_ack_outbox(
            &Header {
                binding_authority: 0.into(),
                request_id: "clear".into(),
            },
            &first
                .entries
                .iter()
                .map(|e| AckEntry {
                    key: e.key.clone(),
                    version: e.version.clone(),
                    stamp: e.stamp.clone(),
                    value_identity: e.value.identity().unwrap(),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    for conv in ["one", "two"] {
        sender
            .commit(&WorkingSetCommit {
                expected_revision: sender.revision().unwrap(),
                conversations: Some(vec![super::super::ConversationMutation::ReplaceRange {
                    character_id: "char".into(),
                    conversation_id: conv.into(),
                    start: 99,
                    delete_count: 99,
                    messages: vec![serde_json::json!({"data":conv,"chatId":"kept"})],
                    conversation: None,
                    configured_index: None,
                }]),
                ..Default::default()
            })
            .unwrap();
    }
    copy_objects(&sender, &target);
    let generating = ["one", "two"]
        .map(|id| MessageLocator {
            character_id: "char".into(),
            conversation_id: id.into(),
            start: None,
        })
        .to_vec();
    let mut changes = sender
        .lww_read_outbox(0.into(), 100)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| Change {
            key: e.key,
            stamp: e.stamp,
            value: e.value,
        })
        .collect::<Vec<_>>();
    changes.push(change(
        &["conversation", "char", "one", "name"],
        u64::MAX - 1,
        serde_json::json!("remote-name"),
    ));
    let result = receive(&mut target, "generating", changes, generating.clone());
    assert_eq!(result.deferred_keys.len(), 2);
    assert!(target
        .lww_read_outbox_generating(0.into(), 100, &generating)
        .unwrap()
        .entries
        .iter()
        .all(|e| e.key.components()[0] != "messages"));
    drop(target);
    let mut target = PersistentStore::open(dir.path()).unwrap();
    let drained = target
        .lww_drain_deferred(&ApplyReceive {
            header: Header {
                binding_authority: 0.into(),
                request_id: "drain".into(),
            },
            generating: vec![],
        })
        .unwrap();
    assert_eq!(
        drained
            .affected_keys
            .iter()
            .filter(|key| key.components()[0] == "messages")
            .count(),
        2
    );
    let count: i64 = target
        .connection
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn order_preserves_folders_filters_retired_ids_and_appends_concurrent_additions() {
    let (_, mut store) = store();
    let changes = vec![
        change(
            &["exists", "character", "a"],
            1,
            serde_json::json!({"type":"character"}),
        ),
        change(
            &["exists", "character", "b"],
            2,
            serde_json::json!({"type":"character"}),
        ),
        change(
            &["exists", "character", "c"],
            3,
            serde_json::json!({"type":"character"}),
        ),
        change(
            &["order", "characters"],
            10,
            serde_json::json!([{"name":"folder","data":["b","missing","b"]},"a"]),
        ),
    ];
    receive(&mut store, "order", changes, vec![]);
    assert_eq!(
        store.read_root(None).unwrap().value["characterOrder"],
        serde_json::json!([{"name":"folder","data":["b"]},"a","c"])
    );
    receive(
        &mut store,
        "retire",
        vec![Change {
            key: unit_key(&["exists", "character", "b"]).unwrap(),
            stamp: stamp(11),
            value: UnitValue::Deleted,
        }],
        vec![],
    );
    assert_eq!(
        store.read_root(None).unwrap().value["characterOrder"][0]["data"],
        serde_json::json!([])
    );
}

#[test]
fn local_and_opaque_fields_persist_without_publication() {
    let (_, mut store) = store();
    create_conversation(&mut store, "char", "conv");
    save(
        &mut store,
        vec![
            mutation(&["character", "char", "chatPage"], serde_json::json!(7)),
            mutation(
                &["character", "char", "futureOwnField"],
                serde_json::json!({"x":1}),
            ),
            mutation(
                &["conversation", "char", "conv", "futureOwnField"],
                serde_json::json!(true),
            ),
        ],
    );
    let entries = store.lww_read_outbox(0.into(), 100).unwrap().entries;
    assert!(entries.iter().all(|e| !matches!(
        e.key.components().last().unwrap().as_str(),
        "chatPage" | "futureOwnField"
    )));
    assert_eq!(
        store.read_character("char", None).unwrap().unwrap().value["chatPage"],
        7
    );
}

#[test]
fn plugin_record_deletion_is_reusable_and_generic_record_deletion_is_permanent() {
    let (_, mut store) = store();
    receive(
        &mut store,
        "plugin",
        vec![change(
            &["record", "plugins", "synthetic"],
            1,
            serde_json::json!({"name":"synthetic","script":"a"}),
        )],
        vec![],
    );
    receive(
        &mut store,
        "plugin-delete",
        vec![Change {
            key: unit_key(&["record", "plugins", "synthetic"]).unwrap(),
            stamp: stamp(2),
            value: UnitValue::Deleted,
        }],
        vec![],
    );
    receive(
        &mut store,
        "plugin-reinstall",
        vec![change(
            &["record", "plugins", "synthetic"],
            3,
            serde_json::json!({"name":"synthetic","script":"b"}),
        )],
        vec![],
    );
    assert_eq!(
        store.read_root(None).unwrap().value["plugins"][0]["script"],
        "b"
    );
    receive(
        &mut store,
        "module",
        vec![
            change(&["exists", "modules", "module"], 1, serde_json::json!(true)),
            change(
                &["record", "modules", "module"],
                1,
                serde_json::json!({"id":"module"}),
            ),
        ],
        vec![],
    );
    receive(
        &mut store,
        "module-delete",
        vec![Change {
            key: unit_key(&["exists", "modules", "module"]).unwrap(),
            stamp: stamp(2),
            value: UnitValue::Deleted,
        }],
        vec![],
    );
    receive(
        &mut store,
        "module-stale",
        vec![change(
            &["record", "modules", "module"],
            3,
            serde_json::json!({"id":"module","name":"bad"}),
        )],
        vec![],
    );
    assert_eq!(
        store.read_root(None).unwrap().value["modules"],
        serde_json::json!([])
    );
}

#[test]
fn plugin_local_participation_filters_and_one_batch_shares_stamp() {
    use super::super::device_store::{plugin_values::PluginDeviceMutation, Section};
    let (_, mut store) = store();
    store
        .device_store_mut()
        .unwrap()
        .write_plugin_device_values(
            "orphan",
            &[
                PluginDeviceMutation::Set {
                    space: "string".into(),
                    key: "a".into(),
                    value: "1".into(),
                },
                PluginDeviceMutation::Set {
                    space: "json".into(),
                    key: "b".into(),
                    value: "null".into(),
                },
            ],
        )
        .unwrap();
    assert!(store
        .lww_read_outbox(0.into(), 100)
        .unwrap()
        .entries
        .is_empty());
    store
        .device_store_mut()
        .unwrap()
        .set_section_participating(Section::LocalPlugins, true)
        .unwrap();
    let out = store.lww_read_outbox(0.into(), 100).unwrap();
    assert_eq!(out.entries.len(), 2);
    assert_eq!(out.entries[0].stamp, out.entries[1].stamp);
    store
        .device_store_mut()
        .unwrap()
        .write_plugin_device_values(
            "orphan",
            &[PluginDeviceMutation::Set {
                space: "string".into(),
                key: "a".into(),
                value: "1".into(),
            }],
        )
        .unwrap();
    assert_eq!(
        store.lww_read_outbox(0.into(), 100).unwrap().entries[0].stamp,
        out.entries[0].stamp
    );
    assert!(store
        .device_store_mut()
        .unwrap()
        .set_section_participating(Section::Hypa, false)
        .is_err());
    store
        .device_store_mut()
        .unwrap()
        .set_section_participating(Section::LocalPlugins, false)
        .unwrap();
    assert!(store
        .lww_read_outbox(0.into(), 100)
        .unwrap()
        .entries
        .is_empty());
}

#[test]
fn unpublished_future_clock_repair_requires_native_proof_and_preserves_accepted_clock() {
    let (_, mut store) = store();
    let future = stamp(device_store::now_ms().unwrap() as u64 + 1_000_000);
    let accepted = stamp(1);
    store
        .device_store()
        .unwrap()
        .connection()
        .execute(
            "UPDATE lww_clock SET issued=?1,accepted=?2",
            params![
                serde_json::to_string(&future).unwrap(),
                serde_json::to_string(&accepted).unwrap()
            ],
        )
        .unwrap();
    save(
        &mut store,
        vec![mutation(&["root", "language"], serde_json::json!("ko"))],
    );
    let old = store.lww_read_outbox(0.into(), 100).unwrap().entries;
    let header = Header {
        binding_authority: 0.into(),
        request_id: "clock-repair".into(),
    };
    let corrected = DecimalU64(device_store::now_ms().unwrap() as u64);
    assert!(store
        .lww_retry_unpublished(&header, "missing", corrected)
        .is_err());
    store
        .lww_record_unpublished_proof(
            &header,
            "proof",
            &old.iter()
                .map(|e| AckEntry {
                    key: e.key.clone(),
                    version: e.version.clone(),
                    stamp: e.stamp.clone(),
                    value_identity: e.value.identity().unwrap(),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let before = store.lww_clock_state().unwrap().accepted;
    assert_eq!(before, Some(accepted));
    store
        .lww_retry_unpublished(&header, "proof", corrected)
        .unwrap();
    let repaired = store.lww_read_outbox(0.into(), 100).unwrap().entries;
    assert_eq!(repaired.len(), 1);
    assert!(repaired[0].stamp.physical_ms < old[0].stamp.physical_ms);
    assert_eq!(repaired[0].value, old[0].value);
    assert_eq!(store.lww_clock_state().unwrap().accepted, before);
    store
        .lww_retry_unpublished(&header, "proof", corrected)
        .unwrap();
    assert_eq!(
        store.lww_read_outbox(0.into(), 100).unwrap().entries,
        repaired
    );
}

fn clear_outbox(store: &mut PersistentStore) {
    let entries = store
        .lww_read_outbox(store.lww_binding_authority().unwrap(), 1000)
        .unwrap()
        .entries;
    store
        .lww_ack_outbox(
            &Header {
                binding_authority: store.lww_binding_authority().unwrap(),
                request_id: "clear-check".into(),
            },
            &entries
                .into_iter()
                .map(|entry| AckEntry {
                    key: entry.key,
                    stamp: entry.stamp,
                    value_identity: entry.value.identity().unwrap(),
                    version: entry.version,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
}
#[test]
fn same_batch_parent_creation_precedes_message_range_and_local_statics_never_publish() {
    let (_, mut store) = store();
    store
        .commit(&WorkingSetCommit {
            expected_revision: 0,
            unit_mutations: Some(vec![
                mutation(
                    &["exists", "character", "new"],
                    serde_json::json!({"type":"character"}),
                ),
                mutation(
                    &["exists", "conversation", "new", "chat"],
                    serde_json::json!(true),
                ),
                mutation(
                    &["conversation", "new", "chat", "name"],
                    serde_json::json!("created"),
                ),
            ]),
            conversations: Some(vec![super::super::ConversationMutation::ReplaceRange {
                character_id: "new".into(),
                conversation_id: "chat".into(),
                start: 0,
                delete_count: 0,
                messages: vec![serde_json::json!({"data":"synthetic","chatId":"kept"})],
                conversation: None,
                configured_index: None,
            }]),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        store
            .read_conversation("new", "chat", None)
            .unwrap()
            .unwrap()
            .value["message"][0]["chatId"],
        "kept"
    );
    save(
        &mut store,
        vec![mutation(
            &["character", "new", "statics"],
            serde_json::json!({"messages":[1],"value":2}),
        )],
    );
    clear_outbox(&mut store);
    save(
        &mut store,
        vec![
            mutation(
                &["character", "new", "statics"],
                serde_json::json!({"messages":[3],"value":2}),
            ),
            mutation(
                &["conversation", "new", "chat", "futureOwnField"],
                serde_json::json!("local"),
            ),
        ],
    );
    assert!(store
        .lww_read_outbox(0.into(), 100)
        .unwrap()
        .entries
        .is_empty());
    let value = store.read_character("new", None).unwrap().unwrap().value;
    assert_eq!(value["statics"]["messages"], serde_json::json!([3]));
    assert_eq!(
        store
            .read_conversation("new", "chat", None)
            .unwrap()
            .unwrap()
            .value["futureOwnField"],
        "local"
    );
}
#[test]
fn archive_publishes_one_state_unit_and_shared_payload_excludes_local_fields() {
    use std::io::Read;
    let (_, mut store) = store();
    create_conversation(&mut store, "char", "chat");
    save(
        &mut store,
        vec![
            mutation(
                &["character", "char", "name"],
                serde_json::json!("synthetic"),
            ),
            mutation(
                &["character", "char", "localFuture"],
                serde_json::json!("private"),
            ),
            mutation(
                &["character", "char", "statics"],
                serde_json::json!({"messages":["private"],"value":1}),
            ),
            mutation(
                &["conversation", "char", "chat", "localFuture"],
                serde_json::json!("private-chat"),
            ),
        ],
    );
    clear_outbox(&mut store);
    let revision = store.revision().unwrap();
    store.archive_character("char", revision, 10).unwrap();
    let out = store.lww_read_outbox(0.into(), 100).unwrap().entries;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].key.components(), ["archive", "char"]);
    let retired: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM lww_retired", [], |r| r.get(0))
        .unwrap();
    assert_eq!(retired, 0);
    let generation = active_generation(&store.connection).unwrap();
    let archived =
        super::super::archive::read_archived_object(&store.connection, &generation, "char")
            .unwrap()
            .unwrap();
    assert_ne!(archived.object_hash, archived.shared_object_hash);
    let body = store
        .lww_object_body(&archived.shared_object_hash)
        .unwrap()
        .unwrap();
    let mut decoded = String::new();
    flate2::read::GzDecoder::new(body.as_slice())
        .read_to_string(&mut decoded)
        .unwrap();
    let shared: Value = serde_json::from_str(&decoded).unwrap();
    assert!(shared["detail"].get("localFuture").is_none());
    assert!(shared["detail"]["statics"].get("messages").is_none());
    assert!(shared["conversations"][0]["detail"]
        .get("localFuture")
        .is_none());
    clear_outbox(&mut store);
    let revision = store.revision().unwrap();
    store.restore_character("char", revision).unwrap();
    let restored = store.read_character("char", None).unwrap().unwrap().value;
    assert_eq!(restored["localFuture"], "private");
    assert_eq!(
        restored["statics"]["messages"],
        serde_json::json!(["private"])
    );
    assert_eq!(
        store
            .read_conversation("char", "chat", None)
            .unwrap()
            .unwrap()
            .value["localFuture"],
        "private-chat"
    );
    assert_eq!(
        store.lww_read_outbox(0.into(), 100).unwrap().entries.len(),
        1
    );
}
#[test]
fn received_archive_retains_local_fields_and_holds_children_until_restore() {
    let (_, mut source) = store();
    let (dir, mut target) = store();
    create_conversation(&mut source, "char", "chat");
    create_conversation(&mut target, "char", "chat");
    save(
        &mut target,
        vec![mutation(
            &["character", "char", "privateOwn"],
            serde_json::json!("retained"),
        )],
    );
    clear_outbox(&mut source);
    let revision = source.revision().unwrap();
    source.archive_character("char", revision, 10).unwrap();
    copy_objects(&source, &target);
    let entry = source
        .lww_read_outbox(0.into(), 100)
        .unwrap()
        .entries
        .remove(0);
    let time = device_store::now_ms().unwrap() as u64 + 100_000;
    receive(
        &mut target,
        "remote-archive",
        vec![Change {
            key: entry.key.clone(),
            stamp: stamp(time),
            value: entry.value,
        }],
        vec![],
    );
    let result = receive(
        &mut target,
        "held-field",
        vec![change(
            &["character", "char", "name"],
            time + 1,
            serde_json::json!("remote-name"),
        )],
        vec![],
    );
    assert_eq!(result.held_keys.len(), 1);
    drop(target);
    let mut target = PersistentStore::open(dir.path()).unwrap();
    let result = receive(
        &mut target,
        "remote-restore",
        vec![Change {
            key: entry.key,
            stamp: stamp(time + 2),
            value: UnitValue::Deleted,
        }],
        vec![],
    );
    assert!(result
        .affected_keys
        .iter()
        .any(|k| k.components()[0] == "archive"));
    let value = target.read_character("char", None).unwrap().unwrap().value;
    assert_eq!(value["privateOwn"], "retained");
    assert_eq!(value["name"], "remote-name");
    assert_eq!(
        target
            .read_conversation("char", "chat", None)
            .unwrap()
            .unwrap()
            .value["id"],
        "chat"
    );
}
#[test]
fn plugin_local_received_while_disabled_activates_on_enable_without_publication() {
    let (_, mut store) = store();
    use device_store::Section;
    store
        .device_store_mut()
        .unwrap()
        .set_section_participating(Section::LocalPlugins, false)
        .unwrap();
    receive(
        &mut store,
        "opaque-plugin",
        vec![change(
            &["plugin-local", "orphan", "string", "key"],
            10,
            serde_json::json!("remote"),
        )],
        vec![],
    );
    assert!(store
        .device_store()
        .unwrap()
        .read_plugin_device_value("orphan", "string", "key")
        .unwrap()
        .is_none());
    store
        .device_store_mut()
        .unwrap()
        .set_section_participating(Section::LocalPlugins, true)
        .unwrap();
    assert_eq!(
        store
            .device_store()
            .unwrap()
            .read_plugin_device_value("orphan", "string", "key")
            .unwrap()
            .as_deref(),
        Some("remote")
    );
    assert!(store
        .lww_read_outbox(0.into(), 100)
        .unwrap()
        .entries
        .is_empty());
}

#[test]
fn archive_record_restore_keeps_both_bodies_but_publishes_filtered_body_only() {
    use crate::logical_records::{
        decode_logical_record, encode_logical_record_key, LogicalRecordEnvelope,
        LogicalRecordLocator,
    };
    let (_, mut store) = store();
    create_conversation(&mut store, "char", "chat");
    save(
        &mut store,
        vec![
            mutation(&["character", "char", "name"], serde_json::json!("before")),
            mutation(
                &["character", "char", "privateOwn"],
                serde_json::json!("local"),
            ),
        ],
    );
    let revision = store.revision().unwrap();
    store.archive_character("char", revision, 10).unwrap();
    let generation = active_generation(&store.connection).unwrap();
    let cas = crate::asset_repository::PayloadCas::new(&store.repository_root).unwrap();
    let locator = LogicalRecordLocator::Character {
        character_id: "char".into(),
    };
    let key = encode_logical_record_key(&locator).unwrap();
    let encoded = super::super::record_projection::reconstruct_record_with_owner_objects(
        &store.connection,
        &cas,
        &generation,
        &key,
        vec![],
        |_| Ok(()),
        &|hash| Ok(cas.stat_object(hash)?.unwrap()),
    )
    .unwrap();
    let envelope = decode_logical_record(&encoded).unwrap();
    let LogicalRecordEnvelope::ArchivedCharacter {
        archive_object_hash,
        shared_archive_object_hash,
        ..
    } = &envelope
    else {
        panic!("archive record")
    };
    assert_ne!(archive_object_hash, shared_archive_object_hash);
    assert!(envelope.dependency_hashes().contains(archive_object_hash));
    assert!(envelope
        .dependency_hashes()
        .contains(shared_archive_object_hash));
    let backup_units: BTreeMap<UnitKey, UnitValue> = {
        let mut q = store
            .connection
            .prepare("SELECT key,value FROM lww_units")
            .unwrap();
        q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .map(|row| {
                let (key, value) = row.unwrap();
                (
                    key.try_into().unwrap(),
                    serde_json::from_str(&value).unwrap(),
                )
            })
            .collect()
    };
    let revision = store.revision().unwrap();
    store.restore_character("char", revision).unwrap();
    save(
        &mut store,
        vec![mutation(
            &["character", "char", "name"],
            serde_json::json!("after"),
        )],
    );
    clear_outbox(&mut store);
    let staging = store.replace_begin().unwrap().staging_id;
    let root = store.read_root(None).unwrap().value;
    store.replace_put_root(&staging, &root).unwrap();
    store.replace_put_presets(&staging, &[]).unwrap();
    let tx = store.connection.transaction().unwrap();
    super::super::record_apply::apply_record_rows(&tx, &staging, &key, &locator, &envelope, 0)
        .unwrap();
    tx.commit().unwrap();
    let header = Header {
        binding_authority: 0.into(),
        request_id: "archive-backup-restore".into(),
    };
    let result = store
        .lww_commit_replacement_units(&header, &staging, Some(&backup_units))
        .unwrap();
    let out = store.lww_read_outbox(0.into(), 100).unwrap().entries;
    assert!(!out
        .iter()
        .any(|entry| entry.key.components()[0] == "exists"
            && matches!(entry.value, UnitValue::Deleted)));
    let archive = out
        .iter()
        .find(|entry| entry.key.components()[0] == "archive")
        .unwrap();
    let metadata = projection::archive_metadata(&store.connection, &archive.value).unwrap();
    assert_eq!(&metadata.object_hash, shared_archive_object_hash);
    assert_ne!(&metadata.object_hash, archive_object_hash);
    assert_eq!(
        store
            .lww_commit_replacement_units(&header, &staging, Some(&backup_units))
            .unwrap()
            .revision,
        result.revision
    );
    let mut altered = backup_units;
    altered.insert(
        unit_key(&["root", "language"]).unwrap(),
        inline(&serde_json::json!("altered")).unwrap(),
    );
    assert!(store
        .lww_commit_replacement_units(&header, &staging, Some(&altered))
        .is_err());
    let revision = store.revision().unwrap();
    store.restore_character("char", revision).unwrap();
    let value = store.read_character("char", None).unwrap().unwrap().value;
    assert_eq!(value["name"], "before");
    assert_eq!(value["privateOwn"], "local");
    assert!(store
        .read_conversation("char", "chat", None)
        .unwrap()
        .is_some());
}

fn detach_for_new_authority(store: &mut PersistentStore) -> DecimalU64 {
    let selection = super::super::sync_selection::read(&store.connection).unwrap();
    let binding = store.lww_binding_authority().unwrap();
    let change = super::super::sync_selection::BindingSelectionChange {
        expected_epoch: selection.epoch,
        new_epoch: Uuid::new_v4().to_string(),
        target: super::super::sync_selection::SyncTarget::None,
        library_id: None,
        target_id: None,
        inspection_id: None,
    };
    store
        .lww_switch_target(
            &Header {
                binding_authority: binding,
                request_id: Uuid::new_v4().to_string(),
            },
            &change,
        )
        .unwrap()
}
#[test]
fn default_binding_sends_nothing_then_changed_child_initializes_only_its_owners() {
    let (_, mut source) = store();
    let (_, mut target) = store();
    create_conversation(&mut source, "char", "chat");
    create_conversation(&mut source, "unrelated", "other");
    save(
        &mut source,
        vec![
            mutation(
                &["character", "char", "name"],
                serde_json::json!("baseline"),
            ),
            mutation(
                &["character", "char", "privateOwn"],
                serde_json::json!("private"),
            ),
            mutation(
                &["conversation", "char", "chat", "name"],
                serde_json::json!("before"),
            ),
            mutation(
                &["character", "unrelated", "name"],
                serde_json::json!("unrelated"),
            ),
        ],
    );
    let old_parent = read_unit(
        &source.connection,
        &unit_key(&["exists", "character", "char"]).unwrap(),
    )
    .unwrap()
    .unwrap();
    let old_name = read_unit(
        &source.connection,
        &unit_key(&["character", "char", "name"]).unwrap(),
    )
    .unwrap()
    .unwrap();
    let binding = detach_for_new_authority(&mut source);
    assert!(source
        .lww_read_outbox(binding, 100)
        .unwrap()
        .entries
        .is_empty());
    save(
        &mut source,
        vec![mutation(
            &["conversation", "char", "chat", "name"],
            serde_json::json!("changed"),
        )],
    );
    let out = source.lww_read_outbox(binding, 100).unwrap().entries;
    assert_eq!(
        out.iter()
            .find(|entry| entry.key.components() == ["exists", "character", "char"])
            .unwrap()
            .stamp,
        old_parent.0
    );
    assert_eq!(
        out.iter()
            .find(|entry| entry.key.components() == ["character", "char", "name"])
            .unwrap()
            .stamp,
        old_name.0
    );
    assert!(!out.iter().any(|entry| entry
        .key
        .components()
        .iter()
        .any(|component| component == "unrelated" || component == "privateOwn")));
    copy_objects(&source, &target);
    let result = receive(
        &mut target,
        "initialized-child",
        out.into_iter()
            .map(|entry| Change {
                key: entry.key,
                stamp: entry.stamp,
                value: entry.value,
            })
            .collect(),
        vec![],
    );
    assert!(result.held_keys.is_empty());
    assert_eq!(
        target.read_character("char", None).unwrap().unwrap().value["name"],
        "baseline"
    );
    assert_eq!(
        target
            .read_conversation("char", "chat", None)
            .unwrap()
            .unwrap()
            .value["name"],
        "changed"
    );
    clear_outbox(&mut source);
    save(
        &mut source,
        vec![mutation(
            &["conversation", "char", "chat", "name"],
            serde_json::json!("next"),
        )],
    );
    let out = source.lww_read_outbox(binding, 100).unwrap().entries;
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].key.components(),
        ["conversation", "char", "chat", "name"]
    );
}
#[test]
fn first_message_edit_initializes_default_parents_and_control_values_without_restamping() {
    let (_, mut source) = store();
    let (_, mut target) = store();
    create_conversation(&mut source, "char", "chat");
    save(
        &mut source,
        vec![mutation(
            &["conversation", "char", "chat", "name"],
            serde_json::json!("baseline"),
        )],
    );
    let parent = read_unit(
        &source.connection,
        &unit_key(&["exists", "conversation", "char", "chat"]).unwrap(),
    )
    .unwrap()
    .unwrap();
    let binding = detach_for_new_authority(&mut source);
    source
        .commit(&WorkingSetCommit {
            expected_revision: source.revision().unwrap(),
            conversations: Some(vec![super::super::ConversationMutation::ReplaceRange {
                character_id: "char".into(),
                conversation_id: "chat".into(),
                start: 0,
                delete_count: 0,
                messages: vec![serde_json::json!({"data":"synthetic","chatId":"kept"})],
                conversation: None,
                configured_index: None,
            }]),
            ..Default::default()
        })
        .unwrap();
    let out = source.lww_read_outbox(binding, 100).unwrap().entries;
    assert_eq!(
        out.iter()
            .find(|entry| entry.key.components() == ["exists", "conversation", "char", "chat"])
            .unwrap()
            .stamp,
        parent.0
    );
    copy_objects(&source, &target);
    let result = receive(
        &mut target,
        "first-message",
        out.into_iter()
            .map(|entry| Change {
                key: entry.key,
                stamp: entry.stamp,
                value: entry.value,
            })
            .collect(),
        vec![],
    );
    assert!(result.held_keys.is_empty());
    let conversation = target
        .read_conversation("char", "chat", None)
        .unwrap()
        .unwrap()
        .value;
    assert_eq!(conversation["name"], "baseline");
    assert_eq!(conversation["message"][0]["chatId"], "kept");
}
#[test]
fn explicit_nondefault_initialization_pages_existing_state_without_new_stamps() {
    let (_, mut source) = store();
    create_conversation(&mut source, "char", "chat");
    save(
        &mut source,
        vec![mutation(&["root", "language"], serde_json::json!("ko"))],
    );
    let binding = detach_for_new_authority(&mut source);
    let issued = source.lww_clock_state().unwrap().issued;
    let header = Header {
        binding_authority: binding,
        request_id: "explicit-init".into(),
    };
    let mut cursor = None;
    let mut exported = Vec::new();
    loop {
        let page = source
            .lww_queue_unit_state_page(&header, cursor.as_ref(), 1)
            .unwrap();
        exported.extend(page.entries);
        cursor = page.after_key;
        if !page.has_more {
            break;
        }
    }
    assert_eq!(source.lww_clock_state().unwrap().issued, issued);
    let out = source.lww_read_outbox(binding, 100).unwrap().entries;
    assert_eq!(out.len(), exported.len());
    for entry in out {
        let state = exported
            .iter()
            .find(|state| state.key == entry.key)
            .unwrap();
        assert_eq!(entry.stamp, state.stamp);
        assert_eq!(entry.value, state.value);
        assert_eq!(entry.version, state.version);
    }
    clear_outbox(&mut source);
    let page = source
        .lww_queue_unit_state_page(&header, None, 100)
        .unwrap();
    assert!(!page.entries.is_empty());
    assert!(source
        .lww_read_outbox(binding, 100)
        .unwrap()
        .entries
        .is_empty());
}

#[test]
fn future_clock_admission_blocks_transport_but_local_messages_keep_committing() {
    let (_, mut source) = store();
    let (_, mut target) = store();
    create_conversation(&mut source, "char", "chat");
    let future = stamp(device_store::now_ms().unwrap() as u64 + 1_000_000);
    source
        .device_store()
        .unwrap()
        .connection()
        .execute(
            "UPDATE lww_clock SET issued=?1",
            [serde_json::to_string(&future).unwrap()],
        )
        .unwrap();
    for (position, text) in [(0, "before"), (1, "after")] {
        source
            .commit(&WorkingSetCommit {
                expected_revision: source.revision().unwrap(),
                conversations: Some(vec![super::super::ConversationMutation::ReplaceRange {
                    character_id: "char".into(),
                    conversation_id: "chat".into(),
                    start: position,
                    delete_count: 0,
                    messages: vec![serde_json::json!({"data":text})],
                    conversation: None,
                    configured_index: None,
                }]),
                ..Default::default()
            })
            .unwrap();
        if position == 0 {
            copy_objects(&source, &target);
            let entries = source.lww_read_outbox(0.into(), 100).unwrap().entries;
            let revision = target.revision().unwrap();
            assert!(target
                .lww_stage_receive(&StageReceive {
                    header: Header {
                        binding_authority: 0.into(),
                        request_id: "future-denied".into()
                    },
                    changes: entries
                        .into_iter()
                        .map(|entry| Change {
                            key: entry.key,
                            stamp: entry.stamp,
                            value: entry.value
                        })
                        .collect(),
                    progress: Progress {
                        kind: "external".into(),
                        cursor: 1.into(),
                        writer_id: None
                    },
                    admitted_time_upper_ms: DecimalU64(device_store::now_ms().unwrap() as u64),
                })
                .is_err());
            assert_eq!(target.revision().unwrap(), revision);
        }
    }
    let conversation = source
        .read_conversation("char", "chat", None)
        .unwrap()
        .unwrap()
        .value;
    assert_eq!(conversation["message"].as_array().unwrap().len(), 2);
    assert_eq!(conversation["message"][1]["data"], "after");
    assert!(source
        .lww_read_outbox(0.into(), 100)
        .unwrap()
        .entries
        .iter()
        .any(|entry| entry.key.components()[0] == "messages"
            && entry.stamp.physical_ms.0 >= future.physical_ms.0));
}

fn new_device_stage(store: &mut PersistentStore) -> (Header, String, Vec<Change>) {
    use super::super::sync_selection::SyncTarget;
    let target = SyncTarget::Server("fresh-connection".into());
    let state = store.lww_binding_state().unwrap();
    let inspection = store
        .register_lww_binding_inspection(state.target_authority, &target, "target", "library")
        .unwrap();
    let changes = vec![change(&["root", "language"], 10, serde_json::json!("en"))];
    let request_id = Uuid::new_v4().to_string();
    let header = Header { binding_authority:state.target_authority,request_id:request_id.clone() };
    let staging = store.lww_stage_binding_units(&header,&inspection,&changes,10.into()).unwrap().staging_id;
    (
        Header {
            binding_authority: state.target_authority,
            request_id,
        },
        staging,
        changes,
    )
}
#[test]
fn explicit_new_device_replaces_before_identity_reset_and_completed_retry_preserves_later_edits() {
    use super::super::device_store::plugin_values::PluginDeviceMutation;
    let (_, mut store) = store();
    save(
        &mut store,
        vec![mutation(&["root", "language"], serde_json::json!("old"))],
    );
    let (header, staging, _) = new_device_stage(&mut store);
    let before = store.lww_clock_state().unwrap();
    let accepted = stamp(500);
    let db = store.device_store().unwrap().connection();
    db.execute(
        "UPDATE lww_clock SET accepted=?1",
        [serde_json::to_string(&accepted).unwrap()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO lww_unpublished_proofs VALUES('old-publication',?1,'[]')",
        [header.binding_authority.0.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO lww_progress VALUES('external','','9',?1)",
        [header.binding_authority.0.to_string()],
    )
    .unwrap();
    store
        .device_store_mut()
        .unwrap()
        .write_setting("hub", &serde_json::json!("local-setting"))
        .unwrap();
    store
        .device_store_mut()
        .unwrap()
        .write_plugin_device_values(
            "orphan",
            &[
                PluginDeviceMutation::Set {
                    space: "json".into(),
                    key: "old".into(),
                    value: "null".into(),
                },
                PluginDeviceMutation::Set {
                    space: "string".into(),
                    key: "old".into(),
                    value: "old".into(),
                },
            ],
        )
        .unwrap();
    let preparation = store.prepare_lww_new_device(&header, &staging).unwrap();
    assert_eq!(
        store.prepare_lww_new_device(&header, &staging).unwrap(),
        preparation
    );
    assert!(store
        .lww_replace_target_as_new_device(&header, &staging, &preparation.authorization_id)
        .is_err());
    assert_eq!(store.lww_clock_state().unwrap().writer_id, before.writer_id);
    store
        .authorize_lww_new_device(&preparation.authorization_id)
        .unwrap();
    let result = store
        .lww_replace_target_as_new_device(&header, &staging, &preparation.authorization_id)
        .unwrap();
    assert_ne!(result.writer_id, before.writer_id);
    assert_eq!(result.writer_id, preparation.writer_id);
    assert_eq!(result.binding_authority.0, header.binding_authority.0 + 1);
    let after = store.lww_clock_state().unwrap();
    assert_eq!(after.writer_id, result.writer_id);
    assert_eq!(after.accepted, Some(accepted));
    assert!(after.issued.is_none());
    assert_eq!(store.read_root(None).unwrap().value["language"], "en");
    assert_eq!(
        store.lww_binding_state().unwrap().target_authority,
        result.binding_authority
    );
    assert_eq!(
        store.lww_binding_state().unwrap().library_id,
        Some("library".into())
    );
    assert_eq!(
        store.device_store().unwrap().read_setting("hub").unwrap(),
        Some(serde_json::json!("local-setting"))
    );
    assert_eq!(
        store
            .device_store()
            .unwrap()
            .connection()
            .query_row("SELECT count(*) FROM plugin_device_storage", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .device_store()
            .unwrap()
            .connection()
            .query_row(
                "SELECT count(*) FROM lww_unpublished_proofs WHERE proof_id='old-publication'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(store
        .lww_receive_progress(result.binding_authority)
        .unwrap()
        .is_empty());
    assert!(store.lww_read_outbox(header.binding_authority, 10).is_err());
    store
        .device_store_mut()
        .unwrap()
        .write_plugin_device_values(
            "orphan",
            &[PluginDeviceMutation::Set {
                space: "string".into(),
                key: "later".into(),
                value: "kept".into(),
            }],
        )
        .unwrap();
    save(
        &mut store,
        vec![mutation(&["root", "language"], serde_json::json!("later"))],
    );
    let current = store.lww_clock_state().unwrap();
    let revision = store.revision().unwrap();
    assert_eq!(
        store
            .lww_replace_target_as_new_device(&header, &staging, &preparation.authorization_id)
            .unwrap(),
        result
    );
    assert_eq!(store.revision().unwrap(), revision);
    assert_eq!(store.lww_clock_state().unwrap().issued, current.issued);
    assert_eq!(store.read_root(None).unwrap().value["language"], "later");
    assert_eq!(
        store
            .device_store()
            .unwrap()
            .connection()
            .query_row(
                "SELECT value FROM plugin_device_storage WHERE key='later'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "kept"
    );
    assert!(store
        .lww_replace_target_as_new_device(&header, "different-stage", &preparation.authorization_id)
        .is_err());
}
#[test]
fn explicit_new_device_recovery_finishes_after_library_activation_without_exposing_old_edits() {
    let (directory, mut store) = store();
    save(
        &mut store,
        vec![mutation(&["root", "language"], serde_json::json!("old"))],
    );
    let (header, staging, changes) = new_device_stage(&mut store);
    let preparation = store.prepare_lww_new_device(&header, &staging).unwrap();
    store
        .authorize_lww_new_device(&preparation.authorization_id)
        .unwrap();
    let state = store.lww_clock_state().unwrap();
    let writer_id = preparation.writer_id.clone();
    let selection_change=serde_json::from_str::<super::super::sync_selection::BindingSelectionChange>(&store.device_store().unwrap().connection().query_row("SELECT selection_change FROM lww_new_device_authorizations WHERE authorization_id=?1",[&preparation.authorization_id],|r|r.get::<_,String>(0)).unwrap()).unwrap();
    let new_authority = DecimalU64(header.binding_authority.0 + 1);
    let intent = Intent::NewDevice {
        authorization_id: preparation.authorization_id.clone(),
        selection_change: selection_change.clone(),
        staging_id: staging.clone(),
        changes: changes.clone(),
        old_writer_id: state.writer_id.clone(),
        writer_id: writer_id.clone(),
        new_authority,
    };
    let body = serde_json::to_string(&intent).unwrap();
    let digest = risunest_sync_wire::hash(body.as_bytes());
    let stamp = state.issued.unwrap();
    store
        .device_store()
        .unwrap()
        .connection()
        .execute(
            "INSERT INTO lww_intents VALUES(?1,?2,?3,?4,?5,0)",
            params![
                header.request_id,
                header.binding_authority.0.to_string(),
                serde_json::to_string(&stamp).unwrap(),
                body,
                digest
            ],
        )
        .unwrap();
    let activation = commit::replace_commit_lww(
        &mut store.connection,
        &staging,
        &header,
        &stamp,
        &digest,
        &[],
        true,
        &changes,
        Some(new_authority),
        Some(&selection_change),
    )
    .unwrap();
    assert_eq!(store.lww_clock_state().unwrap().writer_id, state.writer_id);
    drop(store);
    let mut store = PersistentStore::open(directory.path()).unwrap();
    let result = store
        .lww_replace_target_as_new_device(&header, &staging, &preparation.authorization_id)
        .unwrap();
    assert_eq!(result.revision, activation.revision);
    assert_eq!(result.writer_id, writer_id);
    assert_eq!(result.binding_authority, new_authority);
    assert_eq!(store.lww_clock_state().unwrap().writer_id, writer_id);
    assert_eq!(store.read_root(None).unwrap().value["language"], "en");
    assert!(store
        .lww_read_outbox(new_authority, 100)
        .unwrap()
        .entries
        .is_empty());
}
#[test]
fn explicit_new_device_rejects_mutated_or_unattested_stage_without_identity_change() {
    let (_, mut store) = store();
    let stage = store.replace_begin().unwrap().staging_id;
    store
        .replace_put_root(&stage, &serde_json::json!({}))
        .unwrap();
    let before = store.lww_clock_state().unwrap();
    assert!(store
        .lww_replace_target_as_new_device(
            &Header {
                binding_authority: before.binding_authority,
                request_id: "unattested".into()
            },
            &stage,
            "missing"
        )
        .is_err());
    assert_eq!(store.lww_clock_state().unwrap().writer_id, before.writer_id);
    let (header, staging, _) = new_device_stage(&mut store);
    let preparation = store.prepare_lww_new_device(&header, &staging).unwrap();
    store
        .authorize_lww_new_device(&preparation.authorization_id)
        .unwrap();
    store
        .replace_put_root(&staging, &serde_json::json!({"language":"changed"}))
        .unwrap();
    let before = store.lww_clock_state().unwrap();
    assert!(store
        .lww_replace_target_as_new_device(&header, &staging, &preparation.authorization_id)
        .is_err());
    let after = store.lww_clock_state().unwrap();
    assert_eq!(after.writer_id, before.writer_id);
    assert_eq!(after.binding_authority, before.binding_authority);
    assert_eq!(after.issued, before.issued);
}

#[test]
fn explicit_new_device_empty_target_activates_empty_stage_without_republishing_local_units() {
    use super::super::sync_selection::SyncTarget;
    let (_, mut store) = store();
    save(
        &mut store,
        vec![mutation(&["root", "language"], serde_json::json!("local"))],
    );
    let state = store.lww_binding_state().unwrap();
    let target = SyncTarget::Server("empty-target".into());
    let inspection = store
        .register_lww_binding_inspection(state.target_authority, &target, "target", "empty-library")
        .unwrap();
    let header = Header {
        binding_authority: state.target_authority,
        request_id: Uuid::new_v4().to_string(),
    };
    let staging = store.lww_stage_binding_units(&header,&inspection,&[],0.into()).unwrap().staging_id;
    let preparation = store.prepare_lww_new_device(&header, &staging).unwrap();
    assert_eq!(
        store.lww_binding_state().unwrap().target_authority,
        state.target_authority
    );
    assert_eq!(store.read_root(None).unwrap().value["language"], "local");
    store
        .authorize_lww_new_device(&preparation.authorization_id)
        .unwrap();
    let result = store
        .lww_replace_target_as_new_device(&header, &staging, &preparation.authorization_id)
        .unwrap();
    assert_eq!(result.writer_id, preparation.writer_id);
    assert!(store
        .read_root(None)
        .unwrap()
        .value
        .get("language")
        .is_none());
    assert_eq!(store.lww_binding_state().unwrap().target, target);
    assert!(store
        .lww_read_outbox(result.binding_authority, 100)
        .unwrap()
        .entries
        .is_empty());
    assert!(store
        .lww_read_unit_state(result.binding_authority, None, 100)
        .unwrap()
        .entries
        .is_empty());
}
