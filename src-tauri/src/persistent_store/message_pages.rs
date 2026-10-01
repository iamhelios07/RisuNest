use super::{record_projection::codec_error, StoreError, StoreResult};
use risunest_external_storage_format::{
    logical_records::{decode_message_page, encoded_object, LOGICAL_MESSAGE_PAGE_SIZE},
    message_pages::{
        ManifestPage, MessageHash, MessageManifest, PageBoundary, PageWork, MANIFEST_SCHEMA,
        MAX_PAGE_BYTES, MIN_PAGE_MESSAGES, PAGE_PREFIX, PAGE_SUFFIX,
    },
};
use risunest_sync_wire::{
    descriptor::{build_reference_tree, inline_references, RecordDescriptor},
    payload_value,
    stamp::DecimalU64,
    unit::UnitValue,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;

pub(super) const SCHEMA: &str = "
CREATE TABLE message_page_indexes (
    generation TEXT NOT NULL, character_id TEXT NOT NULL, conversation_id TEXT NOT NULL,
    page_start INTEGER NOT NULL, message_count INTEGER NOT NULL,
    hash TEXT NOT NULL, byte_length INTEGER NOT NULL,
    PRIMARY KEY (generation,character_id,conversation_id,page_start)
);
CREATE TABLE message_page_manifests (
    generation TEXT NOT NULL, character_id TEXT NOT NULL, conversation_id TEXT NOT NULL,
    body BLOB NOT NULL, PRIMARY KEY(generation,character_id,conversation_id)
);
CREATE TABLE message_page_objects (hash TEXT PRIMARY KEY, body BLOB NOT NULL);
";

#[derive(Clone, Copy, Debug)]
pub(super) struct MessageEdit {
    pub start: i64,
    pub delete_count: i64,
    pub insert_count: i64,
}

pub(super) fn object_body(db: &Connection, hash: &str) -> StoreResult<Option<Vec<u8>>> {
    let body: Option<Vec<u8>> = db
        .query_row(
            "SELECT body FROM message_page_objects WHERE hash=?1",
            [hash],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(body) = &body {
        if risunest_sync_wire::hash(body) != hash {
            return Err(codec_error("message object hash mismatch"));
        }
    }
    Ok(body)
}

pub(super) fn put_object(db: &Connection, hash: &str, body: &[u8]) -> StoreResult<()> {
    risunest_sync_wire::validate_hash(hash).map_err(codec_error)?;
    if risunest_sync_wire::hash(body) != hash {
        return Err(codec_error("message object hash mismatch"));
    }
    db.execute(
        "INSERT OR IGNORE INTO message_page_objects(hash,body) VALUES(?1,?2)",
        params![hash, body],
    )?;
    if object_body(db, hash)?.as_deref() != Some(body) {
        return Err(codec_error("message object identity collision"));
    }
    Ok(())
}

fn require_object(db: &Connection, hash: &str) -> StoreResult<Vec<u8>> {
    object_body(db, hash)?.ok_or_else(|| codec_error("message object body is missing"))
}

fn descriptor(
    manifest: &MessageManifest,
) -> StoreResult<(RecordDescriptor, Vec<(String, Vec<u8>)>)> {
    let object = manifest.encode().map_err(codec_error)?;
    let mut descriptor = RecordDescriptor::content(object.hash);
    let mut hashes = manifest
        .pages
        .iter()
        .map(|p| p.hash.clone())
        .collect::<Vec<_>>();
    hashes.sort();
    hashes.dedup();
    let objects = if inline_references(&hashes).map_err(codec_error)? {
        descriptor.dependencies = hashes;
        Vec::new()
    } else {
        let (root, objects) = build_reference_tree(&hashes, false).map_err(codec_error)?;
        descriptor.dependency_root = root;
        objects
    };
    descriptor.validate().map_err(codec_error)?;
    Ok((descriptor, objects))
}

fn unit_value(db: &Connection, manifest: &MessageManifest) -> StoreResult<UnitValue> {
    let object = manifest.encode().map_err(codec_error)?;
    put_object(db, &object.hash, &object.bytes)?;
    let (descriptor, references) = descriptor(manifest)?;
    for (hash, body) in references {
        put_object(db, &hash, &body)?;
    }
    let body = descriptor.bytes().map_err(codec_error)?;
    put_object(db, &risunest_sync_wire::hash(&body), &body)?;
    UnitValue::object(descriptor).map_err(codec_error)
}

pub(super) fn load_pages(
    db: &Connection,
    generation: &str,
    character: &str,
    conversation: &str,
) -> StoreResult<Vec<PageBoundary>> {
    let mut statement = db.prepare(
        "SELECT page_start,message_count,hash,byte_length FROM message_page_indexes
        WHERE generation=?1 AND character_id=?2 AND conversation_id=?3 ORDER BY page_start",
    )?;
    let rows = statement.query_map(params![generation, character, conversation], |r| {
        Ok(PageBoundary {
            start: sql_usize(r.get::<_, i64>(0)?, 0)?,
            page: ManifestPage {
                message_count: r.get::<_, u32>(1)?,
                hash: r.get(2)?,
                byte_length: DecimalU64(sql_u64(r.get::<_, i64>(3)?, 3)?),
            },
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub(super) fn capture_manifest(
    tx: &Transaction<'_>,
    generation: &str,
    character: &str,
    conversation: &str,
    edit: Option<MessageEdit>,
) -> StoreResult<UnitValue> {
    capture_with_work(tx, generation, character, conversation, edit).map(|(value, _)| value)
}

pub(super) fn current_manifest(
    tx: &Transaction<'_>,
    generation: &str,
    character: &str,
    conversation: &str,
) -> StoreResult<UnitValue> {
    let body: Option<Vec<u8>> = tx
        .query_row(
            "SELECT body FROM message_page_manifests WHERE generation=?1 AND character_id=?2
        AND conversation_id=?3",
            params![generation, character, conversation],
            |r| r.get(0),
        )
        .optional()?;
    match body {
        Some(body) => unit_value(tx, &MessageManifest::decode(&body).map_err(codec_error)?),
        None => capture_manifest(tx, generation, character, conversation, None),
    }
}

pub(super) fn capture_with_work(
    tx: &Transaction<'_>,
    generation: &str,
    character: &str,
    conversation: &str,
    edit: Option<MessageEdit>,
) -> StoreResult<(UnitValue, PageWork)> {
    let count: Option<i64> = tx
        .query_row(
            "SELECT message_count FROM conversations WHERE generation=?1 AND character_id=?2
        AND conversation_id=?3",
            params![generation, character, conversation],
            |r| r.get(0),
        )
        .optional()?;
    let Some(count) = count else {
        tx.execute("DELETE FROM message_page_indexes WHERE generation=?1 AND character_id=?2 AND conversation_id=?3",params![generation,character,conversation])?;
        tx.execute("DELETE FROM message_page_manifests WHERE generation=?1 AND character_id=?2 AND conversation_id=?3",params![generation,character,conversation])?;
        return Ok((UnitValue::Deleted, PageWork::default()));
    };
    if count < 0 {
        return Err(codec_error("negative conversation message count"));
    }
    let count_usize = usize::try_from(count).map_err(codec_error)?;
    let previous = load_pages(tx, generation, character, conversation)?;
    let mut work = PageWork::default();
    let edit = edit.filter(|_| !previous.is_empty());
    if let Some(edit) = edit {
        let old_count = previous
            .iter()
            .try_fold(0i64, |count, p| {
                count.checked_add(p.page.message_count as i64)
            })
            .ok_or_else(|| codec_error("message page count overflow"))?;
        if edit.start < 0
            || edit.delete_count < 0
            || edit.insert_count < 0
            || edit.start > old_count
            || edit.delete_count > old_count - edit.start
            || Some(count)
                != old_count
                    .checked_sub(edit.delete_count)
                    .and_then(|v| v.checked_add(edit.insert_count))
        {
            return Err(codec_error("invalid message page edit range"));
        }
    }
    let prefix = edit.map_or(0, |e| {
        previous
            .partition_point(|p| p.start < e.start as usize)
            .saturating_sub(1)
    });
    let mut pages = previous[..prefix].to_vec();
    work.prefix_reused = prefix;
    let mut position = if edit.is_some() {
        previous.get(prefix).map_or(0, |p| p.start)
    } else {
        0
    };
    let mut statement = tx.prepare("SELECT message_index,canonical_hash,canonical_size FROM messages
        WHERE generation=?1 AND character_id=?2 AND conversation_id=?3 AND message_index>=?4 ORDER BY message_index")?;
    let mut rows = statement.query(params![
        generation,
        character,
        conversation,
        position as i64
    ])?;
    let mut next = next_hash(tx, &mut rows, generation, character, conversation)?;
    work.hash_rows_read += usize::from(next.is_some());
    while let Some((_, _)) = &next {
        let start = position;
        let mut bytes = PAGE_PREFIX.to_vec();
        loop {
            let (index, hash) = next
                .take()
                .ok_or_else(|| codec_error("message page index is incomplete"))?;
            if index != position {
                return Err(codec_error("noncontiguous conversation messages"));
            }
            let body: String = tx.query_row(
                "SELECT value FROM messages WHERE generation=?1 AND character_id=?2
                AND conversation_id=?3 AND message_index=?4",
                params![generation, character, conversation, position as i64],
                |r| r.get(0),
            )?;
            let body = payload_value::canonicalize(body.as_bytes()).map_err(codec_error)?;
            if MessageHash::from_bytes(&body) != hash {
                return Err(codec_error("cached message hash mismatch"));
            }
            if position > start {
                bytes.push(b',');
            }
            bytes.extend(&body);
            work.messages_read += 1;
            work.bytes_read += body.len() as u64;
            position += 1;
            next = next_hash(tx, &mut rows, generation, character, conversation)?;
            work.hash_rows_read += usize::from(next.is_some());
            let page_count = position - start;
            if page_count == LOGICAL_MESSAGE_PAGE_SIZE
                || (page_count >= MIN_PAGE_MESSAGES && hash.boundary().map_err(codec_error)?)
                || bytes.len() + PAGE_SUFFIX.len() > MAX_PAGE_BYTES
                || next.is_none()
                || next.as_ref().is_some_and(|(_, h)| {
                    bytes.len() as u64 + 1 + h.byte_length + PAGE_SUFFIX.len() as u64
                        > MAX_PAGE_BYTES as u64
                })
            {
                break;
            }
        }
        bytes.extend(PAGE_SUFFIX);
        let object = encoded_object(bytes).map_err(codec_error)?;
        let boundary = PageBoundary {
            start,
            page: ManifestPage {
                hash: object.hash.clone(),
                message_count: (position - start) as u32,
                byte_length: DecimalU64(object.size),
            },
        };
        if let Some(edit) = edit {
            let end = (edit.start + edit.insert_count) as usize;
            if start >= end {
                let old_start = (start as i64 - edit.insert_count + edit.delete_count) as usize;
                if let Ok(old_page) = previous.binary_search_by_key(&old_start, |p| p.start) {
                    if boundary.page == previous[old_page].page {
                        for old in &previous[old_page..] {
                            let mut page = old.clone();
                            page.start = (page.start as i64 + edit.insert_count - edit.delete_count)
                                as usize;
                            pages.push(page);
                            work.suffix_reused += 1;
                        }
                        position = count_usize;
                        break;
                    }
                }
            }
        }
        put_object(tx, &object.hash, &object.bytes)?;
        pages.push(boundary);
        work.pages_written += 1;
    }
    if position != count_usize {
        return Err(codec_error("conversation message count mismatch"));
    }
    let manifest = MessageManifest {
        schema: MANIFEST_SCHEMA.into(),
        message_count: DecimalU64(count as u64),
        pages: pages.iter().map(|p| p.page.clone()).collect(),
    };
    save_index(tx, generation, character, conversation, &pages, &manifest)?;
    Ok((unit_value(tx, &manifest)?, work))
}

fn next_hash(
    tx: &Transaction<'_>,
    rows: &mut rusqlite::Rows<'_>,
    generation: &str,
    character: &str,
    conversation: &str,
) -> StoreResult<Option<(usize, MessageHash)>> {
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let index: i64 = row.get(0)?;
    let hash: String = row.get(1)?;
    let size = sql_u64(row.get::<_, i64>(2)?, 2)?;
    if index < 0 {
        return Err(codec_error("negative message index"));
    }
    if !hash.is_empty() {
        return Ok(Some((
            index as usize,
            MessageHash {
                hash,
                byte_length: size,
            },
        )));
    }
    // Upstream import staging may have inserted messages without a computed cache.
    let body: String = tx.query_row(
        "SELECT value FROM messages WHERE generation=?1 AND character_id=?2
        AND conversation_id=?3 AND message_index=?4",
        params![generation, character, conversation, index],
        |r| r.get(0),
    )?;
    let bytes = payload_value::canonicalize(body.as_bytes()).map_err(codec_error)?;
    let cached = MessageHash::from_bytes(&bytes);
    tx.execute("UPDATE messages SET canonical_hash=?5,canonical_size=?6 WHERE generation=?1 AND character_id=?2
        AND conversation_id=?3 AND message_index=?4",params![generation,character,conversation,index,cached.hash,sql_i64(cached.byte_length)?])?;
    Ok(Some((index as usize, cached)))
}

fn save_index(
    tx: &Transaction<'_>,
    generation: &str,
    character: &str,
    conversation: &str,
    pages: &[PageBoundary],
    manifest: &MessageManifest,
) -> StoreResult<()> {
    manifest.validate().map_err(codec_error)?;
    let old = load_pages(tx, generation, character, conversation)?;
    for page in &old {
        if pages
            .binary_search_by_key(&page.start, |p| p.start)
            .is_err()
        {
            tx.execute("DELETE FROM message_page_indexes WHERE generation=?1 AND character_id=?2 AND conversation_id=?3 AND page_start=?4",
                params![generation,character,conversation,page.start as i64])?;
        }
    }
    for page in pages {
        if old
            .binary_search_by_key(&page.start, |p| p.start)
            .is_ok_and(|i| old[i] == *page)
        {
            continue;
        }
        tx.execute(
            "INSERT OR REPLACE INTO message_page_indexes VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                generation,
                character,
                conversation,
                page.start as i64,
                page.page.message_count,
                page.page.hash,
                sql_i64(page.page.byte_length.0)?
            ],
        )?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO message_page_manifests VALUES(?1,?2,?3,?4)",
        params![
            generation,
            character,
            conversation,
            manifest.encode().map_err(codec_error)?.bytes
        ],
    )?;
    Ok(())
}

fn validated_manifest(
    db: &Connection,
    value: &UnitValue,
) -> StoreResult<(MessageManifest, Vec<Value>, Vec<Vec<u8>>, Vec<PageBoundary>)> {
    value.validate().map_err(codec_error)?;
    let UnitValue::Object {
        descriptor: received,
        ..
    } = value
    else {
        return Err(codec_error("messages require an object manifest"));
    };
    let body = require_object(db, &received.object_hash)?;
    let manifest = MessageManifest::decode(&body).map_err(codec_error)?;
    let (expected, references) = descriptor(&manifest)?;
    if received != &expected {
        return Err(codec_error("message manifest dependencies mismatch"));
    }
    for (hash, body) in references {
        if require_object(db, &hash)? != body {
            return Err(codec_error("message reference tree mismatch"));
        }
    }
    let mut messages = Vec::new();
    let mut pages = Vec::new();
    for page in &manifest.pages {
        let body = require_object(db, &page.hash)?;
        let decoded = decode_message_page(&body).map_err(codec_error)?;
        if decoded.len() != page.message_count as usize || body.len() as u64 != page.byte_length.0 {
            return Err(codec_error("message page manifest length mismatch"));
        }
        pages.push(PageBoundary {
            start: messages.len(),
            page: page.clone(),
        });
        messages.extend(decoded);
    }
    let bodies = messages
        .iter()
        .map(|v| payload_value::encode(v).map_err(codec_error))
        .collect::<StoreResult<Vec<_>>>()?;
    let hashes = bodies
        .iter()
        .map(|b| MessageHash::from_bytes(b))
        .collect::<Vec<_>>();
    let rebuilt = risunest_external_storage_format::message_pages::repage::<StoreError>(
        &Default::default(),
        hashes,
        0..0,
        messages.len(),
        |i| Ok(bodies[i].clone()),
    )?;
    if rebuilt.index.manifest() != manifest {
        return Err(codec_error("message page boundaries are not deterministic"));
    }
    Ok((manifest, messages, bodies, pages))
}

pub(super) fn validate_manifest(db: &Connection, value: &UnitValue) -> StoreResult<()> {
    validated_manifest(db, value).map(|_| ())
}

pub(super) fn apply_manifest(
    tx: &Transaction<'_>,
    generation: &str,
    character: &str,
    conversation: &str,
    value: &UnitValue,
) -> StoreResult<()> {
    let (manifest, messages, bodies, pages) = validated_manifest(tx, value)?;
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM conversations WHERE generation=?1 AND character_id=?2 AND conversation_id=?3)",
        params![generation,character,conversation],|r|r.get(0))?;
    if !exists {
        return Err(codec_error("message manifest parent is missing"));
    }
    tx.execute(
        "DELETE FROM messages WHERE generation=?1 AND character_id=?2 AND conversation_id=?3",
        params![generation, character, conversation],
    )?;
    for (index, (value, body)) in messages.iter().zip(bodies).enumerate() {
        let hash = MessageHash::from_bytes(&body);
        tx.execute("INSERT INTO messages(generation,character_id,conversation_id,message_index,message_id,value,canonical_hash,canonical_size)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![generation,character,conversation,index as i64,
                value.get("chatId").and_then(Value::as_str),String::from_utf8(body).map_err(codec_error)?,hash.hash,sql_i64(hash.byte_length)?])?;
    }
    tx.execute("UPDATE conversations SET message_count=?4 WHERE generation=?1 AND character_id=?2 AND conversation_id=?3",
        params![generation,character,conversation,sql_i64(manifest.message_count.0)?])?;
    save_index(tx, generation, character, conversation, &pages, &manifest)
}

impl From<risunest_external_storage_format::logical_records::LogicalRecordError> for StoreError {
    fn from(error: risunest_external_storage_format::logical_records::LogicalRecordError) -> Self {
        codec_error(error)
    }
}

fn sql_i64(value: u64) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| codec_error("message size exceeds database integer range"))
}
fn sql_u64(value: i64, index: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn sql_usize(value: i64, index: usize) -> rusqlite::Result<usize> {
    usize::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
