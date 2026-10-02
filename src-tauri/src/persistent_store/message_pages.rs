use super::{record_projection::codec_error, StoreError, StoreResult};
use risunest_external_storage_format::{
    logical_records::{decode_message_page, encoded_object, LOGICAL_MESSAGE_PAGE_SIZE},
    message_pages::{
        ManifestPage, MessageHash, MessageManifest, PageBoundary, MANIFEST_SCHEMA, MAX_PAGE_BYTES,
        MIN_PAGE_MESSAGES, PAGE_PREFIX, PAGE_SUFFIX,
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
        if {
            let hash_input = body;
            #[cfg(test)]
            crate::persistent_store::hash_work::observe("native_page_object_verify", hash_input.len());
            risunest_sync_wire::hash(hash_input)
        } != hash {
            return Err(codec_error("message object hash mismatch"));
        }
    }
    Ok(body)
}

pub(super) fn put_object(db: &Connection, hash: &str, body: &[u8]) -> StoreResult<()> {
    risunest_sync_wire::validate_hash(hash).map_err(codec_error)?;
    if {
        let hash_input = body;
        #[cfg(test)]
        crate::persistent_store::hash_work::observe("native_page_object_verify", hash_input.len());
        risunest_sync_wire::hash(hash_input)
    } != hash {
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
    let object = {
        let result = manifest.encode();
        #[cfg(test)]
        crate::persistent_store::hash_work::encoded_object("native_manifest_identity", &result);
        result
    }.map_err(codec_error)?;
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
        let (root, objects) = {
            let result = build_reference_tree(&hashes, false);
            #[cfg(test)]
            crate::persistent_store::hash_work::reference_creation(&result);
            result
        }.map_err(codec_error)?;
        descriptor.dependency_root = root;
        objects
    };
    descriptor.validate().map_err(codec_error)?;
    Ok((descriptor, objects))
}

fn unit_value(db: &Connection, manifest: &MessageManifest) -> StoreResult<UnitValue> {
    let object = {
        let result = manifest.encode();
        #[cfg(test)]
        crate::persistent_store::hash_work::encoded_object("native_manifest_identity", &result);
        result
    }.map_err(codec_error)?;
    put_object(db, &object.hash, &object.bytes)?;
    let (descriptor, references) = descriptor(manifest)?;
    for (hash, body) in references {
        put_object(db, &hash, &body)?;
    }
    let body = descriptor.bytes().map_err(codec_error)?;
    put_object(db, &{
        let hash_input = &body;
        #[cfg(test)]
        crate::persistent_store::hash_work::observe("native_descriptor_object_identity", hash_input.len());
        risunest_sync_wire::hash(hash_input)
    }, &body)?;
    {
        let result = UnitValue::object(descriptor);
        #[cfg(test)]
        crate::persistent_store::hash_work::descriptor_creation(&result);
        result
    }.map_err(codec_error)
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
    #[cfg(not(test))]
    {
        capture(tx, generation, character, conversation, edit)
    }
    #[cfg(test)]
    {
        let mut work = PageWork::default();
        let result = capture(tx, generation, character, conversation, edit, &mut work);
        record_capture_work(&work, result.is_ok());
        result
    }
}

#[cfg(test)]
#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct PageWork {
    pub messages_read: usize,
    pub hash_rows_read: usize,
    pub bytes_read: u64,
    pub pages_written: usize,
    pub prefix_reused: usize,
    pub suffix_reused: usize,
}

#[cfg(test)]
#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct CaptureWork {
    pub capture_calls: usize,
    pub successful_captures: usize,
    pub failed_captures: usize,
    pub work: PageWork,
}

#[cfg(test)]
std::thread_local! {
    static CAPTURE_WORK: std::cell::RefCell<CaptureWork> = std::cell::RefCell::new(CaptureWork::default());
}

#[cfg(test)]
pub(super) fn reset_capture_work() {
    CAPTURE_WORK.with(|slot| *slot.borrow_mut() = CaptureWork::default());
}

#[cfg(test)]
pub(super) fn take_capture_work() -> CaptureWork {
    CAPTURE_WORK.with(|slot| std::mem::take(&mut *slot.borrow_mut()))
}

#[cfg(test)]
fn record_capture_work(work: &PageWork, succeeded: bool) {
    CAPTURE_WORK.with(|slot| {
        let mut totals = slot.borrow_mut();
        totals.capture_calls += 1;
        totals.successful_captures += usize::from(succeeded);
        totals.failed_captures += usize::from(!succeeded);
        totals.work.messages_read += work.messages_read;
        totals.work.hash_rows_read += work.hash_rows_read;
        totals.work.bytes_read += work.bytes_read;
        totals.work.pages_written += work.pages_written;
        totals.work.prefix_reused += work.prefix_reused;
        totals.work.suffix_reused += work.suffix_reused;
    });
}

#[cfg(test)]
pub(super) fn capture_with_work(
    tx: &Transaction<'_>,
    generation: &str,
    character: &str,
    conversation: &str,
    edit: Option<MessageEdit>,
) -> StoreResult<(UnitValue, PageWork)> {
    let mut work = PageWork::default();
    let value = capture(tx, generation, character, conversation, edit, &mut work)?;
    Ok((value, work))
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
        Some(body) => unit_value(tx, &{
            let result = MessageManifest::decode(&body);
            #[cfg(test)]
            crate::persistent_store::hash_work::decoded("native_manifest_decode_identity", &body, &result);
            result
        }.map_err(codec_error)?),
        None => capture_manifest(tx, generation, character, conversation, None),
    }
}

fn capture(
    tx: &Transaction<'_>,
    generation: &str,
    character: &str,
    conversation: &str,
    edit: Option<MessageEdit>,
    #[cfg(test)] work: &mut PageWork,
) -> StoreResult<UnitValue> {
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
        return Ok(UnitValue::Deleted);
    };
    if count < 0 {
        return Err(codec_error("negative conversation message count"));
    }
    let count_usize = usize::try_from(count).map_err(codec_error)?;
    let previous = load_pages(tx, generation, character, conversation)?;
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
    #[cfg(test)]
    {
        work.prefix_reused = prefix;
    }
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
    #[cfg(test)]
    {
        work.hash_rows_read += usize::from(next.is_some());
    }
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
            if {
                #[cfg(test)]
                crate::persistent_store::hash_work::observe("native_message_verify", body.len());
                MessageHash::from_bytes(&body)
            } != hash {
                return Err(codec_error("cached message hash mismatch"));
            }
            if position > start {
                bytes.push(b',');
            }
            bytes.extend(&body);
            #[cfg(test)]
            {
                work.messages_read += 1;
            }
            #[cfg(test)]
            {
                work.bytes_read += body.len() as u64;
            }
            position += 1;
            next = next_hash(tx, &mut rows, generation, character, conversation)?;
            #[cfg(test)]
            {
                work.hash_rows_read += usize::from(next.is_some());
            }
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
        let object = {
            let result = encoded_object(bytes);
            #[cfg(test)]
            crate::persistent_store::hash_work::encoded_object("native_page_identity", &result);
            result
        }.map_err(codec_error)?;
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
                            #[cfg(test)]
                            {
                                work.suffix_reused += 1;
                            }
                        }
                        position = count_usize;
                        break;
                    }
                }
            }
        }
        put_object(tx, &object.hash, &object.bytes)?;
        pages.push(boundary);
        #[cfg(test)]
        {
            work.pages_written += 1;
        }
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
    unit_value(tx, &manifest)
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
    let cached = {
        #[cfg(test)]
        crate::persistent_store::hash_work::observe("native_message_verify", bytes.len());
        MessageHash::from_bytes(&bytes)
    };
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
            {
                let result = manifest.encode();
                #[cfg(test)]
                crate::persistent_store::hash_work::encoded_object("native_manifest_identity", &result);
                result
            }.map_err(codec_error)?.bytes
        ],
    )?;
    Ok(())
}

fn validated_manifest(
    db: &Connection,
    value: &UnitValue,
) -> StoreResult<(MessageManifest, Vec<Value>, Vec<Vec<u8>>, Vec<PageBoundary>)> {
    {
        let result = value.validate();
        #[cfg(test)]
        crate::persistent_store::hash_work::validation(&value);
        result
    }.map_err(codec_error)?;
    let UnitValue::Object {
        descriptor: received,
        ..
    } = value
    else {
        return Err(codec_error("messages require an object manifest"));
    };
    let body = require_object(db, &received.object_hash)?;
    let manifest = {
        let result = MessageManifest::decode(&body);
        #[cfg(test)]
        crate::persistent_store::hash_work::decoded("native_manifest_decode_identity", &body, &result);
        result
    }.map_err(codec_error)?;
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
        let decoded = {
            let result = decode_message_page(&body);
            #[cfg(test)]
            crate::persistent_store::hash_work::decoded("native_page_decode_identity", &body, &result);
            result
        }.map_err(codec_error)?;
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
        .map(|b| {
            #[cfg(test)]
            crate::persistent_store::hash_work::observe("native_message_verify", b.len());
            MessageHash::from_bytes(b)
        })
        .collect::<Vec<_>>();
    let rebuilt = {
        let result = risunest_external_storage_format::message_pages::repage::<StoreError>(
            &Default::default(),
            hashes,
            0..0,
            messages.len(),
            |i| {
                #[cfg(test)]
                crate::persistent_store::hash_work::observe("native_repage_message_verify", bodies[i].len());
                Ok(bodies[i].clone())
            },
        );
        #[cfg(test)]
        match &result {
            Ok(rebuilt) => {
                for object in &rebuilt.objects {
                    crate::persistent_store::hash_work::observe("native_repage_page_identity", object.bytes.len());
                }
            },
            Err(_) => crate::persistent_store::hash_work::incomplete("native_repage_page_identity"),
        }
        result
    }?;
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
        let hash = {
            #[cfg(test)]
            crate::persistent_store::hash_work::observe("native_message_verify", body.len());
            MessageHash::from_bytes(&body)
        };
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

#[cfg(test)]
mod capture_work_tests {
    use super::*;

    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE messages(generation TEXT,character_id TEXT,conversation_id TEXT,message_index INTEGER,
            message_id TEXT,value TEXT,canonical_hash TEXT NOT NULL DEFAULT '',canonical_size INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(generation,character_id,conversation_id,message_index));
            CREATE TABLE conversations(generation TEXT,character_id TEXT,conversation_id TEXT,message_count INTEGER,
            PRIMARY KEY(generation,character_id,conversation_id));
            INSERT INTO conversations VALUES('g','c','chat',2);
            INSERT INTO messages VALUES('g','c','chat',0,'first','{\"chatId\":\"first\"}','','0');
            INSERT INTO messages VALUES('g','c','chat',1,'second','{\"chatId\":\"second\"}','','0');").unwrap();
        db.execute_batch(SCHEMA).unwrap();
        db
    }

    #[test]
    fn hash_work_counts_fallback_reverification_pages_and_manifest_passes() {
        use crate::persistent_store::hash_work::{reset_hash_work, take_hash_work, DomainWork};
        let mut db = fixture(); let tx = db.transaction().unwrap();
        let bodies = [b"{\"chatId\":\"first\"}".to_vec(), b"{\"chatId\":\"second\"}".to_vec()];
        let message_bytes = bodies.iter().map(|body| body.len() as u64).sum::<u64>();
        let page_bytes = PAGE_PREFIX.len() as u64 + message_bytes + 1 + PAGE_SUFFIX.len() as u64;
        reset_hash_work();
        let value = capture_manifest(&tx, "g", "c", "chat", None).unwrap();
        let manifest_bytes: Vec<u8> = tx.query_row("SELECT body FROM message_page_manifests", [], |row| row.get(0)).unwrap();
        let work = take_hash_work();
        assert_eq!(work.domains["native_message_verify"], DomainWork { calls: 4, bytes: 2 * message_bytes });
        assert_eq!(work.domains["native_page_identity"], DomainWork { calls: 1, bytes: page_bytes });
        assert_eq!(work.domains["native_manifest_identity"], DomainWork { calls: 3, bytes: 3 * manifest_bytes.len() as u64 });
        assert!(work.incomplete.is_empty());
        reset_hash_work();
        validate_manifest(&tx, &value).unwrap();
        let work = take_hash_work();
        assert_eq!(work.domains["native_manifest_decode_identity"], DomainWork { calls: 1, bytes: manifest_bytes.len() as u64 });
        assert_eq!(work.domains["native_page_decode_identity"], DomainWork { calls: 1, bytes: page_bytes });
        assert_eq!(work.domains["native_repage_message_verify"], DomainWork { calls: 2, bytes: message_bytes });
        assert_eq!(work.domains["native_repage_page_identity"], DomainWork { calls: 1, bytes: page_bytes });
        assert!(work.incomplete.is_empty());
    }

    #[test]
    fn verified_object_put_counts_both_passes_and_failed_hash_exactly() {
        use crate::persistent_store::hash_work::{reset_hash_work, take_hash_work, DomainWork};
        let db = fixture(); let body = b"synthetic control"; let hash = risunest_sync_wire::hash(body);
        reset_hash_work();
        put_object(&db, &hash, body).unwrap(); object_body(&db, &hash).unwrap();
        assert!(put_object(&db, &hash, b"wrong").is_err());
        let work = take_hash_work();
        assert_eq!(work.domains["native_page_object_verify"], DomainWork { calls: 4, bytes: (3 * body.len() + 5) as u64 });
        assert!(work.incomplete.is_empty());
    }

    #[test]
    fn failed_delegated_decode_marks_unknown_partial_work_incomplete() {
        use crate::persistent_store::hash_work::{reset_hash_work, take_hash_work};
        let db = fixture();
        let body = b"{ \"schema\":\"risunest.message-manifest/v1\",\"messageCount\":\"0\",\"pages\":[]}";
        let hash = risunest_sync_wire::hash(body); put_object(&db, &hash, body).unwrap();
        let value = UnitValue::object(RecordDescriptor::content(hash)).unwrap();
        reset_hash_work();
        assert!(validate_manifest(&db, &value).is_err());
        let work = take_hash_work();
        assert_eq!(work.incomplete["native_manifest_decode_identity"], 1);
        assert!(!work.domains.contains_key("native_manifest_decode_identity"));
        assert_eq!(work.domains["native_page_object_verify"].bytes, body.len() as u64);
    }

    #[test]
    fn reference_creation_counts_returned_pages_without_rehashing() {
        use crate::persistent_store::hash_work::{reset_hash_work, take_hash_work, DomainWork};
        let manifest = MessageManifest { schema: MANIFEST_SCHEMA.into(), message_count: 65.into(), pages: (0..65).map(|index| ManifestPage {
            hash: format!("{index:064x}"), message_count: 1, byte_length: 80.into()
        }).collect() };
        reset_hash_work();
        let (_, objects) = descriptor(&manifest).unwrap();
        let work = take_hash_work();
        assert_eq!(work.domains["native_reference_create"], DomainWork { calls: objects.len() as u64, bytes: objects.iter().map(|(_, body)| body.len() as u64).sum() });
        assert!(!work.domains.contains_key("native_reference_relation_boundary"));
        assert!(work.incomplete.is_empty());
    }

    #[test]
    fn captures_are_counted_once_and_take_clears_the_actual_work() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        capture_manifest(&tx, "g", "c", "chat", None).unwrap();
        reset_capture_work();
        current_manifest(&tx, "g", "c", "chat").unwrap();
        assert_eq!(take_capture_work(), CaptureWork::default());
        let value = serde_json::json!({"chatId":"third"});
        let body = payload_value::encode(&value).unwrap();
        let hash = MessageHash::from_bytes(&body);
        tx.execute(
            "INSERT INTO messages VALUES('g','c','chat',2,'third',?1,?2,?3)",
            params![
                String::from_utf8(body).unwrap(),
                hash.hash,
                hash.byte_length as i64
            ],
        )
        .unwrap();
        tx.execute("UPDATE conversations SET message_count=3", [])
            .unwrap();
        capture_manifest(
            &tx,
            "g",
            "c",
            "chat",
            Some(MessageEdit {
                start: 2,
                delete_count: 0,
                insert_count: 1,
            }),
        )
        .unwrap();
        let totals = take_capture_work();
        assert_eq!(totals.capture_calls, 1);
        assert_eq!(totals.successful_captures, 1);
        assert_eq!(totals.failed_captures, 0);
        assert_eq!(totals.work.messages_read, 3);
        assert_eq!(totals.work.hash_rows_read, 3);
        assert_eq!(
            totals.work.bytes_read,
            b"{\"chatId\":\"first\"}".len() as u64
                + b"{\"chatId\":\"second\"}".len() as u64
                + b"{\"chatId\":\"third\"}".len() as u64
        );
        assert_eq!(totals.work.pages_written, 1);
        assert_eq!(totals.work.prefix_reused + totals.work.suffix_reused, 0);
        assert_eq!(take_capture_work(), CaptureWork::default());
        assert!(capture_manifest(
            &tx,
            "g",
            "c",
            "chat",
            Some(MessageEdit {
                start: 4,
                delete_count: 0,
                insert_count: 1
            })
        )
        .is_err());
        let totals = take_capture_work();
        assert_eq!(totals.capture_calls, 1);
        assert_eq!(totals.successful_captures, 0);
        assert_eq!(totals.failed_captures, 1);
        assert_eq!(totals.work, PageWork::default());
    }

    #[test]
    fn accumulator_is_thread_local_and_explicit_probes_are_not_double_counted() {
        reset_capture_work();
        record_capture_work(
            &PageWork {
                messages_read: 7,
                bytes_read: 70,
                ..Default::default()
            },
            true,
        );
        std::thread::spawn(|| {
            assert_eq!(take_capture_work(), CaptureWork::default());
            record_capture_work(
                &PageWork {
                    messages_read: 2,
                    bytes_read: 20,
                    ..Default::default()
                },
                false,
            );
            let totals = take_capture_work();
            assert_eq!(totals.capture_calls, 1);
            assert_eq!(totals.failed_captures, 1);
            assert_eq!(totals.work.messages_read, 2);
        })
        .join()
        .unwrap();
        let totals = take_capture_work();
        assert_eq!(totals.capture_calls, 1);
        assert_eq!(totals.successful_captures, 1);
        assert_eq!(totals.work.messages_read, 7);
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        let (_, work) = capture_with_work(&tx, "g", "c", "chat", None).unwrap();
        assert_eq!(work.messages_read, 2);
        assert_eq!(take_capture_work(), CaptureWork::default());
    }
}
