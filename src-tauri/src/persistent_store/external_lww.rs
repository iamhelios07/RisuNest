use super::{
    lww::{AckEntry, Header, OutboxEntry},
    PersistentStore, StoreResult,
};
use risunest_sync_wire::stamp::DecimalU64;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE external_lww_sequences(target TEXT NOT NULL,writer TEXT NOT NULL,next_seq TEXT NOT NULL,PRIMARY KEY(target,writer));
CREATE TABLE external_lww_segments(target TEXT NOT NULL,writer TEXT NOT NULL,seq TEXT NOT NULL,authority TEXT NOT NULL,metadata TEXT NOT NULL,sealed BLOB NOT NULL,complete INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(target,writer,seq));
CREATE TABLE external_lww_seen(target TEXT NOT NULL,writer TEXT NOT NULL,seq TEXT NOT NULL,sha256 TEXT NOT NULL,PRIMARY KEY(target,writer,seq));
CREATE TABLE external_lww_objects(target TEXT NOT NULL,hash TEXT NOT NULL,body BLOB,PRIMARY KEY(target,hash));
CREATE TABLE external_lww_receives(request_id TEXT PRIMARY KEY,body TEXT NOT NULL);
CREATE TABLE external_lww_versions(target TEXT NOT NULL,key TEXT NOT NULL,stamp TEXT NOT NULL,identity TEXT NOT NULL,PRIMARY KEY(target,key,stamp));
"#;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UploadState {
    pub sealed_state: String,
    pub confirmed_offset: DecimalU64,
    pub expires_at_ms: Option<DecimalU64>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SealedBody {
    pub object_id: String,
    pub sha256: String,
    pub bytes: String,
    pub resume: Option<UploadState>,
    pub complete: bool,
    pub locator: Option<crate::external_storage::contract::RemoteLocator>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SealedPublication {
    pub target: String,
    pub writer: String,
    pub seq: DecimalU64,
    pub authority: DecimalU64,
    pub object_id: String,
    pub sha256: String,
    pub payload_sha256: String,
    pub payload: String,
    pub sealed: bool,
    pub entries: Vec<OutboxEntry>,
    pub bodies: Vec<SealedBody>,
    pub resume: Option<UploadState>,
    pub dispatched: bool,
    pub complete: bool,
}
impl SealedPublication {
    pub(crate) fn acknowledgements(&self) -> StoreResult<Vec<AckEntry>> {
        self.entries
            .iter()
            .map(|entry| {
                let identity = entry.value.identity();
                #[cfg(test)]
                crate::external_storage::lww_segment::observe_value_identity(
                    "publish-ack.descriptor",
                    "publish-ack.identity",
                    &entry.value,
                    identity.is_ok(),
                );
                Ok(AckEntry {
                    key: entry.key.clone(),
                    version: entry.version.clone(),
                    stamp: entry.stamp.clone(),
                    value_identity: identity.map_err(super::lww::error)?,
                })
            })
            .collect()
    }
}
impl PersistentStore {
    pub(crate) fn external_lww_unpublished_entries(
        &self,
        expected: DecimalU64,
    ) -> StoreResult<Vec<OutboxEntry>> {
        if self.lww_binding_authority()? != expected {
            return Err(super::lww::error("stale-binding-authority"));
        }
        let mut entries = Vec::new();
        for db in [&self.connection, self.device_store()?.connection()] {
            let mut statement =
                db.prepare("SELECT key,stamp,value,version FROM lww_outbox WHERE authority=?1")?;
            let rows = statement.query_map([expected.0.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            for row in rows {
                let (key, stamp, value, version) = row?;
                entries.push(OutboxEntry {
                    key: key.try_into().map_err(super::lww::error)?,
                    stamp: serde_json::from_str(&stamp)?,
                    value: serde_json::from_str(&value)?,
                    version,
                    target_authority: expected,
                });
            }
        }
        let mut statement = self
            .device_store()?
            .connection()
            .prepare("SELECT metadata FROM external_lww_segments")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            let publication: SealedPublication = serde_json::from_str(&row?)?;
            if publication.dispatched
                && publication.entries.iter().any(|published| {
                    entries.iter().any(|entry| {
                        published.key == entry.key
                            && published.version == entry.version
                            && published.stamp == entry.stamp
                            && published.value == entry.value
                    })
                })
            {
                return Err(super::lww::error("publication-unsettled"));
            }
        }
        Ok(entries)
    }
    pub(crate) fn external_lww_root(&self) -> std::path::PathBuf {
        self.repository_root.clone()
    }
    pub(crate) fn external_lww_abandon_unsent(
        &mut self,
        target: &str,
        writer: &str,
    ) -> StoreResult<()> {
        if let Some((pending, _)) = self.external_lww_pending(target, writer)? {
            if pending.dispatched {
                return Err(super::lww::error("publication-unsettled"));
            }
            self.device_store()?.connection().execute(
                "DELETE FROM external_lww_segments WHERE target=?1 AND writer=?2 AND seq=?3",
                params![target, writer, pending.seq.0.to_string()],
            )?;
        }
        Ok(())
    }
    pub(crate) fn external_lww_stable_receive(
        &self,
        request: super::lww::StageReceive,
    ) -> StoreResult<super::lww::StageReceive> {
        let body: Option<String> = self
            .device_store()?
            .connection()
            .query_row(
                "SELECT body FROM external_lww_receives WHERE request_id=?1",
                [&request.header.request_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(body) = body {
            return Ok(serde_json::from_str(&body)?);
        }
        self.device_store()?.connection().execute(
            "INSERT INTO external_lww_receives VALUES(?1,?2)",
            params![request.header.request_id, serde_json::to_string(&request)?],
        )?;
        Ok(request)
    }
    pub(crate) fn external_lww_verify_versions(
        &mut self,
        target: &str,
        changes: &[super::lww::Change],
    ) -> StoreResult<()> {
        let tx = self.device_store_mut()?.transaction()?;
        for change in changes {
            let stamp = serde_json::to_string(&change.stamp)?;
            let identity = change.value.identity();
            #[cfg(test)]
            crate::external_storage::lww_segment::observe_value_identity(
                "receive-history.descriptor",
                "receive-history.identity",
                &change.value,
                identity.is_ok(),
            );
            let identity = identity.map_err(super::lww::error)?;
            let old:Option<String>=tx.query_row("SELECT identity FROM external_lww_versions WHERE target=?1 AND key=?2 AND stamp=?3",params![target,change.key.as_str(),stamp],|r|r.get(0)).optional()?;
            if old.as_ref().is_some_and(|old| old != &identity) {
                return Err(super::lww::error("equal-stamp-integrity"));
            }
            tx.execute(
                "INSERT OR IGNORE INTO external_lww_versions VALUES(?1,?2,?3,?4)",
                params![target, change.key.as_str(), stamp, identity],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn external_lww_register_source(
        &self,
        source: &crate::external_storage::lww_residency::Source,
    ) -> StoreResult<()> {
        crate::external_storage::lww_residency::register(&self.repository_root, source)
            .map_err(super::lww::error)
    }
    pub(crate) fn external_lww_object_is_local(&self, hash: &str) -> StoreResult<bool> {
        Ok(
            crate::asset_repository::PayloadCas::new(&self.repository_root)?
                .open_object(hash)?
                .is_some(),
        )
    }
    pub(crate) fn external_lww_object_is_control(&self, hash: &str) -> StoreResult<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM message_page_objects WHERE hash=?1)",
            [hash],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn external_lww_has_object(&self, target: &str, hash: &str) -> StoreResult<bool> {
        Ok(self.device_store()?.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM external_lww_objects WHERE target=?1 AND hash=?2)",
            params![target, hash],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn external_lww_witness_object(&self, target: &str, hash: &str) -> StoreResult<()> {
        self.device_store()?.connection().execute(
            "INSERT OR IGNORE INTO external_lww_objects(target,hash) VALUES(?1,?2)",
            params![target, hash],
        )?;
        Ok(())
    }
    pub(crate) fn external_lww_next_sequence(
        &self,
        target: &str,
        writer: &str,
    ) -> StoreResult<u64> {
        let sequence: Option<String> = self
            .device_store()?
            .connection()
            .query_row(
                "SELECT next_seq FROM external_lww_sequences WHERE target=?1 AND writer=?2",
                params![target, writer],
                |r| r.get(0),
            )
            .optional()?;
        sequence
            .map(|value| {
                value
                    .try_into()
                    .map(|value: DecimalU64| value.0)
                    .map_err(super::lww::error)
            })
            .unwrap_or(Ok(1))
    }
    pub(crate) fn external_lww_pending(
        &self,
        target: &str,
        writer: &str,
    ) -> StoreResult<Option<(SealedPublication, Vec<u8>)>> {
        let seq = self.external_lww_next_sequence(target, writer)?;
        let found: Option<(String,Vec<u8>)> = self.device_store()?.connection().query_row(
            "SELECT metadata,sealed FROM external_lww_segments WHERE target=?1 AND writer=?2 AND seq=?3",
            params![target,writer,seq.to_string()], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        found
            .map(|(metadata, bytes)| Ok((serde_json::from_str(&metadata)?, bytes)))
            .transpose()
    }
    pub(crate) fn external_lww_persist(
        &mut self,
        publication: &SealedPublication,
        bytes: &[u8],
    ) -> StoreResult<()> {
        let tx = self.device_store_mut()?.transaction()?;
        let authority: String = tx.query_row(
            "SELECT binding_authority FROM lww_clock WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if authority != publication.authority.0.to_string() {
            return Err(super::lww::error("stale-binding-authority"));
        }
        let previous: Option<(Vec<u8>,String)> = tx.query_row("SELECT sealed,metadata FROM external_lww_segments WHERE target=?1 AND writer=?2 AND seq=?3",
            params![publication.target,publication.writer,publication.seq.0.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((sealed, metadata)) = previous {
            let previous: SealedPublication = serde_json::from_str(&metadata)?;
            if previous.sealed && sealed != bytes {
                return Err(super::lww::error("sealed-segment-integrity"));
            }
            if previous.entries != publication.entries {
                return Err(super::lww::error("captured-version-integrity"));
            }
            if previous.bodies.len() != publication.bodies.len()
                || previous
                    .bodies
                    .iter()
                    .zip(&publication.bodies)
                    .any(|(old, new)| {
                        old.object_id != new.object_id
                            || old.sha256 != new.sha256
                            || old.bytes != new.bytes
                    })
            {
                return Err(super::lww::error("sealed-body-integrity"));
            }
            if previous.sealed
                && (!publication.sealed
                    || previous.object_id != publication.object_id
                    || previous.sha256 != publication.sha256
                    || previous.payload != publication.payload
                    || previous.payload_sha256 != publication.payload_sha256)
            {
                return Err(super::lww::error("sealed-segment-integrity"));
            }
            if !previous.sealed
                && !publication.sealed
                && (previous.payload != publication.payload
                    || previous.payload_sha256 != publication.payload_sha256)
            {
                return Err(super::lww::error("captured-payload-integrity"));
            }
        }
        tx.execute("INSERT INTO external_lww_segments(target,writer,seq,authority,metadata,sealed,complete) VALUES(?1,?2,?3,?4,?5,?6,?7)
            ON CONFLICT(target,writer,seq) DO UPDATE SET metadata=excluded.metadata,sealed=excluded.sealed,complete=excluded.complete",
            params![publication.target,publication.writer,publication.seq.0.to_string(),publication.authority.0.to_string(),serde_json::to_string(publication)?,bytes,publication.complete])?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn external_lww_finish_publication(
        &mut self,
        publication: &SealedPublication,
        bytes: &[u8],
    ) -> StoreResult<()> {
        self.external_lww_persist(publication, bytes)?;
        self.lww_ack_outbox(
            &Header {
                binding_authority: publication.authority,
                request_id: format!(
                    "external-ack-{}-{}-{}-{}",
                    publication.target,
                    publication.authority.0,
                    publication.writer,
                    publication.seq.0
                ),
            },
            &publication.acknowledgements()?,
        )?;
        let next = publication
            .seq
            .0
            .checked_add(1)
            .ok_or_else(|| super::lww::error("sequence-overflow"))?;
        let tx = self.device_store_mut()?.transaction()?;
        let authority: String = tx.query_row(
            "SELECT binding_authority FROM lww_clock WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if authority != publication.authority.0.to_string() {
            return Err(super::lww::error("stale-binding-authority"));
        }
        tx.execute(
            "INSERT INTO external_lww_sequences(target,writer,next_seq) VALUES(?1,?2,?3)
            ON CONFLICT(target,writer) DO UPDATE SET next_seq=excluded.next_seq",
            params![publication.target, publication.writer, next.to_string()],
        )?;
        tx.execute("INSERT INTO lww_progress(authority,kind,writer_id,cursor) VALUES(?1,'external',?2,?3) ON CONFLICT(kind,writer_id) DO UPDATE SET cursor=excluded.cursor,authority=excluded.authority",params![authority,publication.writer,publication.seq.0.to_string()])?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn external_lww_verify_seen(
        &self,
        target: &str,
        writer: &str,
        seq: u64,
        hash: &str,
    ) -> StoreResult<bool> {
        let previous: Option<String> = self
            .device_store()?
            .connection()
            .query_row(
                "SELECT sha256 FROM external_lww_seen WHERE target=?1 AND writer=?2 AND seq=?3",
                params![target, writer, seq.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if previous.as_deref().is_some_and(|value| value != hash) {
            return Err(super::lww::error("writer-sequence-integrity"));
        }
        Ok(previous.is_some())
    }
    pub(crate) fn external_lww_matches_seen(
        &self,
        target: &str,
        writer: &str,
        seq: u64,
        hash: &str,
    ) -> StoreResult<bool> {
        Ok(self.device_store()?.connection().query_row("SELECT EXISTS(SELECT 1 FROM external_lww_seen WHERE target=?1 AND writer=?2 AND seq=?3 AND sha256=?4)",params![target,writer,seq.to_string(),hash],|r|r.get(0))?)
    }
    pub(crate) fn external_lww_record_seen(
        &self,
        target: &str,
        writer: &str,
        seq: u64,
        hash: &str,
    ) -> StoreResult<()> {
        self.external_lww_verify_seen(target, writer, seq, hash)?;
        self.device_store()?.connection().execute(
            "INSERT OR IGNORE INTO external_lww_seen(target,writer,seq,sha256) VALUES(?1,?2,?3,?4)",
            params![target, writer, seq.to_string(), hash],
        )?;
        Ok(())
    }
}
