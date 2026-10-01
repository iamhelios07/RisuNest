use std::fmt;

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Json(serde_json::Error),
    Validation { message: String },
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for StoreError {}
impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
pub type StoreResult<T> = Result<T, StoreError>;
mod record_projection {
    pub fn codec_error(error: impl std::fmt::Display) -> super::StoreError {
        super::StoreError::Validation {
            message: error.to_string(),
        }
    }
}
#[path = "../../../src-tauri/src/persistent_store/message_pages.rs"]
mod message_pages;

#[cfg(test)]
mod tests {
    use super::*;
    use message_pages::*;
    use risunest_external_storage_format::message_pages::{MessageHash, MessageManifest};
    use risunest_sync_wire::{payload_value, unit::UnitValue};
    use rusqlite::{params, Connection};
    use serde_json::{json, Value};

    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE messages(generation TEXT,character_id TEXT,conversation_id TEXT,message_index INTEGER,
            message_id TEXT,value TEXT,canonical_hash TEXT NOT NULL DEFAULT '',canonical_size INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(generation,character_id,conversation_id,message_index));
            CREATE TABLE conversations(generation TEXT,character_id TEXT,conversation_id TEXT,message_count INTEGER,
            PRIMARY KEY(generation,character_id,conversation_id));
            INSERT INTO conversations VALUES('g','c','chat',0);").unwrap();
        db.execute_batch(SCHEMA).unwrap();
        db
    }
    fn insert(db: &Connection, start: usize, values: &[Value]) {
        for (offset, value) in values.iter().enumerate() {
            let body = payload_value::encode(value).unwrap();
            let hash = MessageHash::from_bytes(&body);
            db.execute(
                "INSERT INTO messages VALUES('g','c','chat',?1,?2,?3,?4,?5)",
                params![
                    (start + offset) as i64,
                    value.get("chatId").and_then(Value::as_str),
                    String::from_utf8(body).unwrap(),
                    hash.hash,
                    hash.byte_length as i64
                ],
            )
            .unwrap();
        }
        db.execute(
            "UPDATE conversations SET message_count=(SELECT count(*) FROM messages)",
            [],
        )
        .unwrap();
    }
    fn synthetic() -> Vec<Value> {
        (0..1024)
            .map(|i| json!({"chatId":format!("id-{i}"),"data":format!("synthetic-{i}")}))
            .collect()
    }
    fn manifest(db: &Connection, value: &UnitValue) -> MessageManifest {
        let UnitValue::Object { descriptor, .. } = value else {
            panic!("object expected")
        };
        MessageManifest::decode(&object_body(db, &descriptor.object_hash).unwrap().unwrap())
            .unwrap()
    }

    #[test]
    fn persisted_restart_range_updates_match_full_capture_and_preserve_message_ids() {
        for (name, start, delete, values) in [
            (
                "append",
                1024,
                0,
                vec![json!({"chatId":"appended","data":"new"})],
            ),
            (
                "edit",
                500,
                1,
                vec![json!({"chatId":"id-500","data":"edited"})],
            ),
            (
                "insert",
                500,
                0,
                vec![json!({"chatId":"inserted","data":"new"})],
            ),
            ("delete", 500, 1, vec![]),
        ] {
            let mut db = fixture();
            insert(&db, 0, &synthetic());
            let tx = db.transaction().unwrap();
            capture_manifest(&tx, "g", "c", "chat", None).unwrap();
            tx.commit().unwrap();
            // Only database state survives this boundary, no retained in-memory pager.
            let tx = db.transaction().unwrap();
            let before = current_manifest(&tx, "g", "c", "chat").unwrap();
            tx.execute(
                "DELETE FROM messages WHERE message_index>=?1 AND message_index<?2",
                params![start as i64, (start + delete) as i64],
            )
            .unwrap();
            let delta = values.len() as i64 - delete as i64;
            if delta != 0 {
                tx.execute("UPDATE messages SET message_index=-(message_index+?1)-1 WHERE message_index>=?2",params![delta,(start+delete) as i64]).unwrap();
                tx.execute(
                    "UPDATE messages SET message_index=-message_index-1 WHERE message_index<0",
                    [],
                )
                .unwrap();
            }
            insert(&tx, start, &values);
            let (updated, work) = capture_with_work(
                &tx,
                "g",
                "c",
                "chat",
                Some(MessageEdit {
                    start: start as i64,
                    delete_count: delete as i64,
                    insert_count: values.len() as i64,
                }),
            )
            .unwrap();
            let updated_manifest = manifest(&tx, &updated);
            let full = capture_manifest(&tx, "g", "c", "chat", None).unwrap();
            assert_eq!(updated, full, "{name}");
            assert_ne!(before, updated, "{name}");
            assert!(work.messages_read < 256, "{name}: {work:?}");
            assert!(
                work.hash_rows_read <= work.messages_read + 1,
                "{name}: {work:?}"
            );
            let mut recipient = fixture();
            for hash in updated_manifest
                .pages
                .iter()
                .map(|p| p.hash.as_str())
                .chain(match &updated {
                    UnitValue::Object { descriptor, .. } => vec![descriptor.object_hash.as_str()],
                    _ => unreachable!(),
                })
            {
                put_object(&recipient, hash, &object_body(&tx, hash).unwrap().unwrap()).unwrap();
            }
            let remote = recipient.transaction().unwrap();
            apply_manifest(&remote, "g", "c", "chat", &updated).unwrap();
            validate_manifest(&remote, &updated).unwrap();
            let ids = remote
                .prepare("SELECT message_id FROM messages ORDER BY message_index")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(ids.len(), updated_manifest.message_count.0 as usize);
            if name == "edit" {
                assert_eq!(ids[500], "id-500");
            }
            println!("native-{name}: {work:?}");
        }
    }

    #[test]
    fn remote_missing_body_or_tampered_manifest_changes_nothing() {
        let mut source = fixture();
        insert(&source, 0, &synthetic()[..24]);
        let tx = source.transaction().unwrap();
        let value = capture_manifest(&tx, "g", "c", "chat", None).unwrap();
        let mut target = fixture();
        insert(&target, 0, &[json!({"chatId":"kept","data":"local"})]);
        let remote = target.transaction().unwrap();
        assert!(apply_manifest(&remote, "g", "c", "chat", &value).is_err());
        assert!(validate_manifest(&remote, &value).is_err());
        let UnitValue::Object { descriptor, .. } = &value else {
            unreachable!()
        };
        put_object(
            &remote,
            &descriptor.object_hash,
            &require_body(&tx, &descriptor.object_hash),
        )
        .unwrap();
        assert!(apply_manifest(&remote, "g", "c", "chat", &value).is_err());
        assert_eq!(
            remote
                .query_row("SELECT message_id FROM messages", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "kept"
        );
        assert!(put_object(&remote, &"00".repeat(32), b"wrong").is_err());
        let values = synthetic();
        let mut forged = manifest(&tx, &value);
        forged.pages.clear();
        for chunk in values[..24].chunks(12) {
            let page =
                risunest_external_storage_format::logical_records::encode_message_page(chunk)
                    .unwrap();
            put_object(&remote, &page.hash, &page.bytes).unwrap();
            forged.pages.push(
                risunest_external_storage_format::message_pages::ManifestPage {
                    hash: page.hash,
                    message_count: chunk.len() as u32,
                    byte_length: risunest_sync_wire::stamp::DecimalU64(page.size),
                },
            );
        }
        let object = forged.encode().unwrap();
        put_object(&remote, &object.hash, &object.bytes).unwrap();
        let mut descriptor = risunest_sync_wire::descriptor::RecordDescriptor::content(object.hash);
        descriptor.dependencies = forged.pages.iter().map(|p| p.hash.clone()).collect();
        descriptor.dependencies.sort();
        descriptor.dependencies.dedup();
        let forged = UnitValue::object(descriptor).unwrap();
        assert!(validate_manifest(&remote, &forged).is_err());
        assert!(apply_manifest(&remote, "g", "c", "chat", &forged).is_err());
        assert_eq!(
            remote
                .query_row("SELECT message_id FROM messages", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "kept"
        );
    }
    fn require_body(db: &Connection, hash: &str) -> Vec<u8> {
        object_body(db, hash).unwrap().unwrap()
    }

    #[test]
    fn empty_manifest_and_upstream_uncached_messages_are_supported() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        let empty = capture_manifest(&tx, "g", "c", "chat", None).unwrap();
        assert_eq!(manifest(&tx, &empty).message_count.0, 0);
        tx.execute("INSERT INTO messages(generation,character_id,conversation_id,message_index,message_id,value)
            VALUES('g','c','chat',0,'upstream','{\"chatId\":\"upstream\",\"data\":0.1}')",[]).unwrap();
        tx.execute("UPDATE conversations SET message_count=1", [])
            .unwrap();
        capture_manifest(&tx, "g", "c", "chat", None).unwrap();
        let hash: String = tx
            .query_row("SELECT canonical_hash FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn edits_on_size_cut_restart_the_preceding_page_and_rollback_is_atomic() {
        let mut db = fixture();
        let values = vec![
            json!({"chatId":"first","data":"small"}),
            json!({"chatId":"huge","data":"x".repeat(256*1024)}),
            json!({"chatId":"last","data":"small"}),
        ];
        insert(&db, 0, &values);
        let tx = db.transaction().unwrap();
        let old = capture_manifest(&tx, "g", "c", "chat", None).unwrap();
        tx.commit().unwrap();
        {
            let tx = db.transaction().unwrap();
            tx.execute("DELETE FROM messages WHERE message_index=1", [])
                .unwrap();
            insert(&tx, 1, &[json!({"chatId":"huge","data":"edited"})]);
            let (updated, work) = capture_with_work(
                &tx,
                "g",
                "c",
                "chat",
                Some(MessageEdit {
                    start: 1,
                    delete_count: 1,
                    insert_count: 1,
                }),
            )
            .unwrap();
            assert_ne!(old, updated);
            assert_eq!(work.prefix_reused, 0);
            assert_eq!(manifest(&tx, &updated).pages.len(), 1);
            assert_eq!(
                capture_manifest(&tx, "g", "c", "chat", None).unwrap(),
                updated
            );
            tx.rollback().unwrap();
        }
        let tx = db.transaction().unwrap();
        assert_eq!(current_manifest(&tx, "g", "c", "chat").unwrap(), old);
        assert_eq!(capture_manifest(&tx, "g", "c", "chat", None).unwrap(), old);
    }
}
