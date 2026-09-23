//! Stages a complete, authenticated external logical snapshot for activation.
//!
//! The transport owns download and decryption. This boundary accepts only local
//! regular files, revalidates their identities, and never performs network I/O.
use super::{
    active_generation, current_revision, record_apply as rows,
    server_sync_projection::{preserve_local_view, ServerPayload},
    PersistentStore,
    PreparedReplaceCommit, StoreError, StoreResult,
};
use crate::{
    asset_repository::{
        owner_manifest_codec::{decode_owner_manifest, encode_owner_manifest},
        PayloadCas,
    },
    logical_records::{
        decode_logical_record, decode_logical_record_key, decode_message_page,
        encode_logical_record_key, LogicalRecordEnvelope, LogicalRecordLocator,
    },
};
use crate::external_storage::content_store::{Body, ContentStore, ObjectSource};
use risunest_external_storage_format::format::{fingerprint, library_fingerprint_domain};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub(crate) struct ExternalSnapshotRecord {
    pub key: String,
    pub content_hash: String,
    pub byte_length: u64,
    pub source: ObjectSource,
}

#[derive(Clone, Debug)]
pub(crate) struct ExternalSnapshotObject {
    pub content_hash: String,
    pub byte_length: u64,
    pub source: ObjectSource,
}

pub(crate) struct ExternalSnapshotApplication<'a> {
    pub expected_revision: i64,
    pub staging_root: &'a Path,
    pub scope_id: &'a [u8; 32],
    pub fingerprint: &'a [u8; 32],
}

impl PreparedReplaceCommit {
    pub(crate) fn external_staging_id(&self) -> &str {
        &self.staging_id
    }
}

impl PersistentStore {
    pub(crate) fn external_active_generation(&self) -> StoreResult<String> {
        active_generation(&self.connection)
    }

    pub(crate) fn external_revision_at_root(root: &Path) -> StoreResult<i64> {
        let path = root.join("persistent").join(super::DATABASE_FILE);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || crate::trust_boundary::is_link_like(&metadata) {
            return invalid("Persistent database must be a regular file");
        }
        crate::trust_boundary::open_regular_source(&path)?;
        let connection = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        current_revision(&connection)
    }
}

impl PersistentStore {
    /// Prepare the library portion of a fully downloaded external snapshot.
    /// Device sections, when present in the repository scope, remain under the
    /// existing device-maintenance restore coordinator and are not read here.
    pub(crate) fn prepare_external_snapshot_application<R, O>(
        &mut self,
        application: &ExternalSnapshotApplication<'_>,
        records: R,
        objects: O,
    ) -> StoreResult<PreparedReplaceCommit>
    where
        R: IntoIterator<Item = StoreResult<ExternalSnapshotRecord>>,
        O: IntoIterator<Item = StoreResult<ExternalSnapshotObject>>,
    {
        validate_application(application)?;
        let actual_revision = self.revision()?;
        if application.expected_revision != actual_revision {
            return Err(StoreError::RevisionConflict {
                expected: application.expected_revision,
                actual: actual_revision,
            });
        }

        let root = verified_staging_root(application.staging_root)?;
        // Record bodies are asked for by identity, whether a local capture
        // named them or a receive decoded them into the job's own store. Only
        // a caller that offers neither leaves this directory absent.
        let content = ContentStore::open_existing(&root.join("external-storage"))?;
        let stage = self.replace_begin()?;
        let staged = (|| {
            let object_sizes =
                stage_objects(&self.repository_root, &root, content.as_ref(), objects)?;
            let record_hashes = stage_records(
                &mut self.connection,
                &self.repository_root,
                &stage.staging_id,
                application.expected_revision,
                &root,
                content.as_ref(),
                &object_sizes,
                records,
            )?;
            if fingerprint(application.scope_id, &record_hashes) != *application.fingerprint {
                return invalid("External snapshot logical fingerprint differs from its catalog");
            }

            self.prepare_replace_commit(&stage.staging_id, Some(application.expected_revision))
        })();

        match staged {
            Ok(prepared) => Ok(prepared),
            Err(error) => {
                if let Err(cleanup) = self.replace_abort(&stage.staging_id) {
                    return Err(StoreError::Store {
                        message: format!(
                            "{error}; failed to remove rejected external snapshot stage: {cleanup}"
                        ),
                    });
                }
                Err(error)
            }
        }
    }
}

/// A receive small enough to change the active generation in place: the records
/// the snapshot moved, the keys it no longer names, and the object lengths its
/// records refer to.
pub(crate) struct ExternalSnapshotDifference<'a> {
    pub expected_revision: i64,
    pub staging_root: &'a Path,
    pub records: &'a [ExternalSnapshotRecord],
    pub removed: &'a [String],
    pub object_sizes: &'a BTreeMap<String, u64>,
}

impl PersistentStore {
    /// Places the bodies a difference's records refer to and reports what every
    /// object in the snapshot is long, which is what resolving a record's
    /// dependencies needs. Writes into the content store rather than the
    /// library, so it belongs outside the apply transaction.
    pub(crate) fn stage_external_snapshot_objects<O>(
        &self,
        staging_root: &Path,
        objects: O,
    ) -> StoreResult<BTreeMap<String, u64>>
    where
        O: IntoIterator<Item = StoreResult<ExternalSnapshotObject>>,
    {
        let root = verified_staging_root(staging_root)?;
        let content = ContentStore::open_existing(&root.join("external-storage"))?;
        stage_objects(&self.repository_root, &root, content.as_ref(), objects)
    }

    /// Applies what the snapshot changed to the generation the library is
    /// already using, so the records it left alone are neither rewritten nor
    /// read. Everything the receive owes - rows, revision, the change index the
    /// other backup consumers read, the connection's base and its record map,
    /// and the job's completion - is one transaction.
    pub(crate) fn apply_external_snapshot_difference(
        &mut self,
        difference: &ExternalSnapshotDifference<'_>,
        job: &str,
    ) -> StoreResult<super::RevisionResult> {
        let arrived: Vec<(String, String)> = difference
            .records
            .iter()
            .map(|record| (record.key.clone(), record.content_hash.clone()))
            .collect();
        let root = verified_staging_root(difference.staging_root)?;
        let content = ContentStore::open_existing(&root.join("external-storage"))?;
        let cas = PayloadCas::new(&self.repository_root)?;
        let mut removals = Vec::with_capacity(difference.removed.len());
        for key in difference.removed {
            let locator = decode_logical_record_key(key)
                .map_err(|_| validation("External snapshot logical key is invalid"))?;
            if matches!(locator, LogicalRecordLocator::Root) {
                return invalid("External snapshot difference cannot remove the root record");
            }
            removals.push(locator);
        }
        let mut arrivals = Vec::with_capacity(difference.records.len());
        for record in difference.records {
            let locator = decode_logical_record_key(&record.key)
                .map_err(|_| validation("External snapshot logical key is invalid"))?;
            arrivals.push(locator);
        }

        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual_revision = current_revision(&tx)?;
        if actual_revision != difference.expected_revision {
            return Err(StoreError::RevisionConflict {
                expected: difference.expected_revision,
                actual: actual_revision,
            });
        }
        super::external_storage_state::begin_receive_activation(&tx, job)?;
        let generation = active_generation(&tx)?;
        let revision = actual_revision + 1;
        super::server_sync_outbox::begin_mutation(&tx, &generation, revision)?;
        super::content_change_index::begin_mutation(&tx, &generation, revision, "external")?;

        let local_root = tx
            .query_row(
                "SELECT value FROM root WHERE generation=?1",
                [&generation],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|value| serde_json::from_str(&value))
            .transpose()?;
        let mut touched = std::collections::BTreeSet::new();
        // A conversation whose character arrives in the same difference has to
        // follow it, exactly as a complete stage defers one behind its parent.
        let ordered = arrivals
            .iter()
            .enumerate()
            .filter(|(_, locator)| !matches!(locator, LogicalRecordLocator::Conversation { .. }))
            .chain(
                arrivals
                    .iter()
                    .enumerate()
                    .filter(|(_, locator)| {
                        matches!(locator, LogicalRecordLocator::Conversation { .. })
                    }),
            )
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        // A character is removed with everything under it, so its conversations
        // need no separate removal and the order between them does not matter.
        for locator in &removals {
            note_touched_character(&mut touched, locator);
            rows::apply_delete(&tx, &generation, locator)?;
        }
        for index in ordered {
            let locator = arrivals[index].clone();
            note_touched_character(&mut touched, &locator);
            apply_record(
                &tx,
                &cas,
                &generation,
                &generation,
                &root,
                content.as_ref(),
                difference.object_sizes,
                difference.records[index].clone(),
                locator,
                local_root.as_ref(),
            )?;
        }
        for character in touched {
            tx.execute("UPDATE characters SET conversation_count=(SELECT count(*) FROM conversations WHERE generation=?1 AND character_id=?2) WHERE generation=?1 AND character_id=?2",params![generation,character])?;
        }
        rows::validate_configured_index_uniqueness(&tx, &generation)?;
        let duplicate_plugin_ordinals: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM plugin_storage WHERE generation=?1 GROUP BY ordinal HAVING count(*)>1)",
            [&generation], |row| row.get(0),
        )?;
        if duplicate_plugin_ordinals {
            return invalid("External snapshot plugin records contain duplicate positions");
        }
        super::content_change_index::finish_mutation(&tx)?;
        super::server_sync_outbox::finish_mutation(&tx)?;
        super::commit::set_active(&tx, revision, &generation)?;
        // Read after the revision moves, so the base names what the library
        // now holds. The record map already describes the base this snapshot
        // follows, so only the keys that moved are written. The selected
        // adapter is deliberately not marked dirty: this content came from it.
        super::external_storage_state::finish_receive_activation(
            &tx,
            job,
            super::external_storage_state::BaseRecords::Moved {
                removed: difference.removed,
                arrived: &arrived,
            },
        )?;
        tx.commit()?;
        Ok(super::RevisionResult { revision })
    }
}

fn note_touched_character(
    touched: &mut std::collections::BTreeSet<String>,
    locator: &LogicalRecordLocator,
) {
    if let LogicalRecordLocator::Character { character_id }
    | LogicalRecordLocator::Conversation { character_id, .. } = locator
    {
        touched.insert(character_id.clone());
    }
}

fn validate_application(application: &ExternalSnapshotApplication<'_>) -> StoreResult<()> {
    if *application.scope_id != library_fingerprint_domain() {
        return invalid("External snapshot fingerprint domain differs");
    }
    Ok(())
}

fn verified_staging_root(root: &Path) -> StoreResult<PathBuf> {
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || crate::trust_boundary::is_link_like(&metadata) {
        return invalid("External snapshot staging root must be a real directory");
    }
    Ok(fs::canonicalize(root)?)
}

/// One snapshot input. A downloaded asset is a file confined to the staging
/// directory it arrived in; a record body is asked for by identity, wherever
/// the content store beside it put it.
fn open_snapshot_input(
    root: &Path,
    content: Option<&ContentStore>,
    source: &ObjectSource,
    expected_size: u64,
) -> StoreResult<Body> {
    let path = match source {
        ObjectSource::Captured(digest) => {
            let content = content
                .ok_or_else(|| validation("External snapshot capture store is unavailable"))?;
            if content.stat(digest)? != Some(expected_size) {
                return invalid("External snapshot input size differs from its catalog");
            }
            return Ok(content.open_body(digest)?);
        }
        ObjectSource::File(path) => path,
        // The library already holds this body; it is confirmed in place rather
        // than read through here.
        ObjectSource::Library(_) => {
            return invalid("External snapshot library body is not an input");
        }
    };
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || crate::trust_boundary::is_link_like(&metadata) {
        return invalid("External snapshot input must be a regular file");
    }
    let canonical = fs::canonicalize(path)?;
    if canonical == root || !canonical.starts_with(root) {
        return invalid("External snapshot input escaped its staging directory");
    }
    let file = crate::trust_boundary::open_regular_source(&canonical)?;
    if file.metadata()?.len() != expected_size {
        return invalid("External snapshot input size differs from its catalog");
    }
    Ok(Body::File(file))
}

fn validate_hash(hash: &str, subject: &str) -> StoreResult<[u8; 32]> {
    if !crate::trust_boundary::is_lower_hex_256(hash) {
        return invalid(format!(
            "{subject} hash must be 64 lowercase hexadecimal characters"
        ));
    }
    let decoded = hex::decode(hash).map_err(|_| validation("External snapshot hash is invalid"))?;
    decoded
        .try_into()
        .map_err(|_| validation("External snapshot hash is invalid"))
}

fn stage_objects<O>(
    repository_root: &Path,
    staging_root: &Path,
    content: Option<&ContentStore>,
    objects: O,
) -> StoreResult<BTreeMap<String, u64>>
where
    O: IntoIterator<Item = StoreResult<ExternalSnapshotObject>>,
{
    let cas = PayloadCas::new(repository_root)?;
    let mut sizes = BTreeMap::new();
    for object in objects {
        let object = object?;
        validate_hash(&object.content_hash, "External snapshot object")?;
        if sizes
            .insert(object.content_hash.clone(), object.byte_length)
            .is_some()
        {
            return invalid("External snapshot contains a duplicate object hash");
        }
        if let ObjectSource::Library(digest) = &object.source {
            // The repository already holds this body. Confirming that it is
            // there under the identity and length the catalog names is the
            // whole of the work; reading it back to write it again is not.
            if digest != &object.content_hash {
                return invalid("External snapshot library body is not the object it names");
            }
            let path = cas
                .object_path(&object.content_hash)?
                .ok_or_else(|| validation("External snapshot library body is missing"))?;
            let file = crate::trust_boundary::open_regular_source(&path)?;
            if file.metadata()?.len() != object.byte_length {
                return invalid("External snapshot input size differs from its catalog");
            }
            continue;
        }
        let mut source =
            open_snapshot_input(staging_root, content, &object.source, object.byte_length)?;
        let prepared =
            cas.prepare_reader_expected(&mut source, &object.content_hash, object.byte_length)?;
        if prepared.content_hash != object.content_hash || prepared.byte_size != object.byte_length
        {
            return invalid("External snapshot object identity changed while staging");
        }
    }
    Ok(sizes)
}

#[allow(clippy::too_many_arguments)]
fn stage_records<R>(
    connection: &mut rusqlite::Connection,
    repository_root: &Path,
    generation: &str,
    expected_revision: i64,
    staging_root: &Path,
    content: Option<&ContentStore>,
    objects: &BTreeMap<String, u64>,
    records: R,
) -> StoreResult<BTreeMap<String, [u8; 32]>>
where
    R: IntoIterator<Item = StoreResult<ExternalSnapshotRecord>>,
{
    let cas = PayloadCas::new(repository_root)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let actual_revision = current_revision(&transaction)?;
    if actual_revision != expected_revision {
        return Err(StoreError::RevisionConflict {
            expected: expected_revision,
            actual: actual_revision,
        });
    }
    let active = active_generation(&transaction)?;
    let local_root = transaction
        .query_row(
            "SELECT value FROM root WHERE generation=?1",
            [&active],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|value| serde_json::from_str(&value))
        .transpose()?;

    let mut hashes = BTreeMap::new();
    let mut deferred_conversations = Vec::new();
    let mut saw_root = false;
    for record in records {
        let record = record?;
        let hash = validate_hash(&record.content_hash, "External logical record")?;
        if hashes.insert(record.key.clone(), hash).is_some() {
            return invalid("External snapshot contains a duplicate logical key");
        }
        let locator = decode_logical_record_key(&record.key)
            .map_err(|_| validation("External snapshot logical key is invalid"))?;
        if matches!(locator, LogicalRecordLocator::Root) {
            saw_root = true;
        }
        if matches!(locator, LogicalRecordLocator::Conversation { .. }) {
            deferred_conversations.push((record, locator));
        } else {
            apply_record(
                &transaction,
                &cas,
                &active,
                generation,
                staging_root,
                content,
                objects,
                record,
                locator,
                local_root.as_ref(),
            )?;
        }
    }
    if !saw_root {
        return invalid("External snapshot does not contain its root logical record");
    }
    for (record, locator) in deferred_conversations {
        apply_record(
            &transaction,
            &cas,
            &active,
            generation,
            staging_root,
            content,
            objects,
            record,
            locator,
            local_root.as_ref(),
        )?;
    }

    transaction.execute(
        "UPDATE characters SET conversation_count=(
            SELECT count(*) FROM conversations
            WHERE conversations.generation=?1
              AND conversations.character_id=characters.character_id
         ) WHERE generation=?1",
        [generation],
    )?;
    rows::validate_configured_index_uniqueness(&transaction, generation)?;
    let duplicate_plugin_ordinals: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM plugin_storage WHERE generation=?1
            GROUP BY ordinal HAVING count(*)>1
         )",
        [generation],
        |row| row.get(0),
    )?;
    if duplicate_plugin_ordinals {
        return invalid("External snapshot plugin records contain duplicate positions");
    }
    transaction.commit()?;
    Ok(hashes)
}

#[allow(clippy::too_many_arguments)]
fn apply_record(
    transaction: &Transaction<'_>,
    cas: &PayloadCas,
    active: &str,
    generation: &str,
    staging_root: &Path,
    content: Option<&ContentStore>,
    objects: &BTreeMap<String, u64>,
    record: ExternalSnapshotRecord,
    locator: LogicalRecordLocator,
    local_root: Option<&serde_json::Value>,
) -> StoreResult<()> {
    if encode_logical_record_key(&locator)
        .map_err(|_| validation("External snapshot logical key is invalid"))?
        != record.key
    {
        return invalid("External snapshot logical key is not canonical");
    }
    let bytes = read_verified_record(staging_root, content, &record)?;
    let mut envelope = decode_logical_record(&bytes)
        .map_err(|_| validation("External snapshot logical record is invalid"))?;
    rows::validate_locator_envelope(&locator, &envelope)?;

    let local_character = if let LogicalRecordLocator::Character { character_id } = &locator {
        transaction
            .query_row(
                "SELECT detail FROM characters WHERE generation=?1 AND character_id=?2",
                params![active, character_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|value| serde_json::from_str(&value))
            .transpose()?
    } else {
        None
    };
    let mut payload = ServerPayload {
        record: envelope,
        messages: None,
        derived_objects: BTreeMap::new(),
    };
    preserve_local_view(&mut payload, local_character.as_ref(), local_root);
    envelope = payload.record;

    let messages = resolve_dependencies(cas, objects, &mut envelope)?;
    rows::apply_materialized_record(
        transaction,
        generation,
        locator,
        envelope,
        messages.as_deref(),
    )?;
    Ok(())
}

fn read_verified_record(
    root: &Path,
    content: Option<&ContentStore>,
    record: &ExternalSnapshotRecord,
) -> StoreResult<Vec<u8>> {
    let mut source = open_snapshot_input(root, content, &record.source, record.byte_length)?;
    let capacity = usize::try_from(record.byte_length)
        .map_err(|_| validation("External logical record is too large for this platform"))?;
    let mut bytes = Vec::with_capacity(capacity.min(1024 * 1024));
    source
        .by_ref()
        .take(record.byte_length.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != record.byte_length
        || hex::encode(Sha256::digest(&bytes)) != record.content_hash
    {
        return invalid("External logical record differs from its catalog hash and size");
    }
    if source.len()? != record.byte_length {
        return invalid("External logical record changed while staging");
    }
    Ok(bytes)
}

fn resolve_dependencies(
    cas: &PayloadCas,
    objects: &BTreeMap<String, u64>,
    envelope: &mut LogicalRecordEnvelope,
) -> StoreResult<Option<Vec<serde_json::Value>>> {
    match envelope {
        LogicalRecordEnvelope::Root { value, owner_heads } => {
            let resolved = resolve_owner_heads(cas, objects, owner_heads)?;
            rows::rehydrate_root_owners(value, &resolved)?;
            Ok(None)
        }
        LogicalRecordEnvelope::Character {
            detail,
            owner_heads,
            ..
        } => {
            let character_id = detail
                .get("chaId")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| validation("External character ID is missing"))?
                .to_owned();
            let resolved = resolve_owner_heads(cas, objects, owner_heads)?;
            rows::rehydrate_character_owner(detail, &character_id, &resolved)?;
            Ok(None)
        }
        LogicalRecordEnvelope::ArchivedCharacter {
            archive_object_hash,
            archive_object_size,
            asset_hashes,
            owner_heads,
            ..
        } => {
            require_object(
                cas,
                objects,
                archive_object_hash,
                Some(*archive_object_size),
            )?;
            for hash in asset_hashes {
                require_object(cas, objects, hash, None)?;
            }
            resolve_owner_heads(cas, objects, owner_heads)?;
            Ok(None)
        }
        LogicalRecordEnvelope::Conversation {
            message_page_hashes,
            ..
        } => {
            let mut messages = Vec::new();
            for hash in message_page_hashes.iter() {
                require_object(cas, objects, hash, None)?;
                let bytes = cas
                    .read_object(hash)?
                    .ok_or_else(|| validation("External message page is missing from local CAS"))?;
                let mut page = decode_message_page(&bytes)
                    .map_err(|_| validation("External message page is invalid"))?;
                messages.append(&mut page);
            }
            Ok(Some(messages))
        }
        LogicalRecordEnvelope::Asset {
            object_hash, size, ..
        }
        | LogicalRecordEnvelope::Inlay {
            object_hash, size, ..
        } => {
            let hash = object_hash
                .as_deref()
                .ok_or_else(|| validation("External alias is missing its complete payload"))?;
            require_object(cas, objects, hash, Some(*size))?;
            Ok(None)
        }
        // The store no longer holds cold payloads; the shared record format still carries the variant.
        LogicalRecordEnvelope::Cold { .. } => {
            Err(validation("External cold records are unsupported"))
        }
        LogicalRecordEnvelope::Preset { .. } | LogicalRecordEnvelope::Plugin { .. } => Ok(None),
    }
}

fn resolve_owner_heads(
    cas: &PayloadCas,
    objects: &BTreeMap<String, u64>,
    heads: &[crate::logical_records::LogicalOwnerHead],
) -> StoreResult<Vec<rows::ResolvedOwnerHead>> {
    let mut resolved = Vec::with_capacity(heads.len());
    for head in heads {
        let tuples = if let Some(hash) = &head.manifest_hash {
            require_object(cas, objects, hash, None)?;
            let bytes = cas
                .read_object(hash)?
                .ok_or_else(|| validation("External owner manifest is missing from local CAS"))?;
            let entries = decode_owner_manifest(&bytes)
                .map_err(|_| validation("External owner manifest is invalid"))?;
            if entries.len() as u64 != head.entry_count
                || encode_owner_manifest(&entries)
                    .map_err(|_| validation("External owner manifest is invalid"))?
                    != bytes
            {
                return invalid("External owner manifest differs from its logical head");
            }
            let mut tuples = Vec::with_capacity(entries.len());
            for entry in entries {
                if let Some(hash) = entry.payload_hash {
                    require_object(cas, objects, &hex::encode(hash), None)?;
                }
                tuples.push(serde_json::Value::Array(
                    entry
                        .tuple
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect(),
                ));
            }
            Some(tuples)
        } else {
            None
        };
        resolved.push(rows::ResolvedOwnerHead {
            head: head.clone(),
            tuples,
        });
    }
    Ok(resolved)
}

fn require_object(
    cas: &PayloadCas,
    objects: &BTreeMap<String, u64>,
    hash: &str,
    expected_size: Option<u64>,
) -> StoreResult<()> {
    let catalog_size = objects
        .get(hash)
        .copied()
        .ok_or_else(|| validation("External snapshot has an incomplete payload reference"))?;
    if expected_size.is_some_and(|expected| expected != catalog_size)
        || cas.stat_object(hash)? != Some(catalog_size)
    {
        return invalid("External snapshot payload size differs from its logical reference");
    }
    Ok(())
}

fn validation(message: impl Into<String>) -> StoreError {
    StoreError::Validation {
        message: message.into(),
    }
}

fn invalid<T>(message: impl Into<String>) -> StoreResult<T> {
    Err(validation(message))
}

#[cfg(test)]
#[path = "tests/external_apply_tests.rs"]
mod tests;
