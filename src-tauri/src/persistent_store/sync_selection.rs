//! Local operational authority. Never exported as library content or connection QR.
use super::{active_generation, current_revision, StoreError, StoreResult};
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE library_sync_selection(singleton INTEGER PRIMARY KEY CHECK(singleton=1),target TEXT NOT NULL CHECK(target IN ('none','server','external')),connection_id TEXT,selection_epoch TEXT NOT NULL,paused INTEGER NOT NULL CHECK(paused IN (0,1)),decision_required INTEGER NOT NULL CHECK(decision_required IN (0,1)),CHECK((target='none' AND connection_id IS NULL) OR (target!='none' AND length(connection_id)>0)));
CREATE TABLE local_library_identity(singleton INTEGER PRIMARY KEY CHECK(singleton=1),store_id TEXT NOT NULL,library_epoch TEXT NOT NULL);
CREATE TABLE lww_binding_identity(singleton INTEGER PRIMARY KEY CHECK(singleton=1),library_id TEXT,target_id TEXT);
CREATE TABLE lww_binding_inspections(inspection_id TEXT PRIMARY KEY,target TEXT NOT NULL,target_id TEXT NOT NULL,library_id TEXT NOT NULL,source_authority TEXT NOT NULL,source_epoch TEXT NOT NULL);
CREATE TABLE lww_binding_stages(staging_id TEXT PRIMARY KEY,inspection_id TEXT NOT NULL,receive_id TEXT NOT NULL UNIQUE,changes TEXT NOT NULL,activation_epoch TEXT);
"#;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureIdentity {
    pub store_id: String,
    pub library_epoch: String,
    pub generation: String,
    pub selection_epoch: String,
    // Decimal strings are used by the UI bridge; SQLite uses signed 64-bit integers.
    pub revision: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "connectionId", rename_all = "lowercase")]
pub(crate) enum SyncTarget {
    None,
    Server(String),
    External(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub target: SyncTarget,
    pub epoch: String,
    pub paused: bool,
    pub decision_required: bool,
}

fn invalid(message: &str) -> StoreError {
    StoreError::Validation {
        message: message.into(),
    }
}

pub(super) fn create_schema(db: &Connection) -> StoreResult<()> {
    db.execute_batch(SCHEMA)?;
    db.execute(
        "INSERT INTO library_sync_selection VALUES(1,'none',NULL,?1,0,0)",
        [uuid::Uuid::new_v4().to_string()],
    )?;
    db.execute(
        "INSERT INTO local_library_identity VALUES(1,?1,?2)",
        params![
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string()
        ],
    )?;
    db.execute("INSERT INTO lww_binding_identity VALUES(1,NULL,NULL)", [])?;
    Ok(())
}

pub(crate) fn read(db: &Connection) -> StoreResult<Selection> {
    let (kind,id,epoch,paused,decision):(String,Option<String>,String,bool,bool)=db.query_row("SELECT target,connection_id,selection_epoch,paused,decision_required FROM library_sync_selection WHERE singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    let target = match (kind.as_str(), id) {
        ("none", None) => SyncTarget::None,
        ("server", Some(id)) => SyncTarget::Server(id),
        ("external", Some(id)) => SyncTarget::External(id),
        _ => return Err(invalid("Invalid sync selection")),
    };
    Ok(Selection {
        target,
        epoch,
        paused,
        decision_required: decision,
    })
}

pub(crate) fn identity(db: &Connection) -> StoreResult<CaptureIdentity> {
    let (store_id, library_epoch) = db.query_row(
        "SELECT store_id,library_epoch FROM local_library_identity WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(CaptureIdentity {
        store_id,
        library_epoch,
        generation: active_generation(db)?,
        selection_epoch: read(db)?.epoch,
        revision: current_revision(db)?,
    })
}

/// Selection changes require the existing exclusive replacement reservation/permit.
/// The caller must settle server operations before entering this local transaction.
pub(crate) fn select(
    tx: &Transaction<'_>,
    expected_epoch: &str,
    target: &SyncTarget,
) -> StoreResult<Selection> {
    if read(tx)?.epoch != expected_epoch {
        return Err(invalid("Sync selection changed"));
    }
    require_no_pending_publication(tx)?;
    let (kind, id) = match target {
        SyncTarget::None => ("none", None),
        SyncTarget::Server(id) => ("server", Some(id)),
        SyncTarget::External(id) => ("external", Some(id)),
    };
    if id.is_some_and(|id| id.is_empty()) {
        return Err(invalid("Missing sync connection identity"));
    }
    tx.execute("UPDATE library_sync_selection SET target=?1,connection_id=?2,selection_epoch=?3,paused=0,decision_required=0 WHERE singleton=1",params![kind,id,uuid::Uuid::new_v4().to_string()])?;
    tx.execute(
        "UPDATE external_storage_jobs SET phase='stale' WHERE role IN ('sync','restore') AND phase IN ('preparing','ready')",
        [],
    )?;
    read(tx)
}

pub(crate) fn set_paused(tx: &Transaction<'_>, expected_epoch: &str, paused: bool) -> StoreResult<Selection> {
    let current = read(tx)?;
    if current.epoch != expected_epoch || !matches!(current.target, SyncTarget::External(_)) {
        return Err(invalid("Sync selection changed"));
    }
    tx.execute("UPDATE library_sync_selection SET paused=?1 WHERE singleton=1", [paused])?;
    read(tx)
}

pub(crate) fn require_no_pending_publication(db: &Connection) -> StoreResult<()> {
    let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM external_storage_jobs WHERE phase='applying')",[],|r|r.get(0))?;
    if pending {
        return Err(invalid(
            "Unsettled sync operation blocks library replacement",
        ));
    }
    Ok(())
}

pub(crate) fn require_publish(
    db: &Connection,
    capture: &CaptureIdentity,
    connection: &str,
) -> StoreResult<()> {
    require_publish_with_pause(db, capture, connection, false)
}

/// Exit drain may flush the selected paused target without changing the
/// user's durable pause intent. Native code validates the live exit session
/// immediately before every call to this override.
pub(crate) fn require_publish_exit_drain(
    db: &Connection,
    capture: &CaptureIdentity,
    connection: &str,
) -> StoreResult<()> {
    require_publish_with_pause(db, capture, connection, true)
}

fn require_publish_with_pause(
    db: &Connection,
    capture: &CaptureIdentity,
    connection: &str,
    allow_paused: bool,
) -> StoreResult<()> {
    let current = identity(db)?;
    let selection = read(db)?;
    if current.store_id != capture.store_id
        || current.library_epoch != capture.library_epoch
        || current.generation != capture.generation
        || current.selection_epoch != capture.selection_epoch
        || capture.revision > current.revision
        || capture.revision < 0
    {
        return Err(invalid("Stale external capture"));
    }
    if selection.target != SyncTarget::External(connection.into())
        || (selection.paused && !allow_paused)
        || selection.decision_required
    {
        return Err(invalid("External sync target is not active"));
    }
    Ok(())
}

pub(super) fn replaced(tx: &Transaction<'_>) -> StoreResult<()> {
    require_no_pending_publication(tx)?;
    tx.execute(
        "UPDATE local_library_identity SET library_epoch=?1 WHERE singleton=1",
        [uuid::Uuid::new_v4().to_string()],
    )?;
    tx.execute(
        "UPDATE library_sync_selection SET decision_required=1 WHERE target='external'",
        [],
    )?;
    tx.execute(
        "UPDATE external_storage_jobs SET phase='stale' WHERE role!='backup' AND phase IN ('preparing','ready')",
        [],
    )?;
    Ok(())
}

pub(crate) fn require_server(db: &Connection) -> StoreResult<()> {
    let selected = read(db)?;
    if !matches!(selected.target, SyncTarget::Server(_)) || selected.paused {
        return Err(invalid("Server sync target is not active"));
    }
    Ok(())
}

pub(super) fn restored_copy(tx: &Transaction<'_>) -> StoreResult<()> {
    tx.execute(
        "UPDATE local_library_identity SET store_id=?1,library_epoch=?2 WHERE singleton=1",
        params![
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string()
        ],
    )?;
    tx.execute("UPDATE library_sync_selection SET selection_epoch=?1,decision_required=1 WHERE singleton=1",[uuid::Uuid::new_v4().to_string()])?;
    tx.execute("UPDATE external_storage_jobs SET phase='stale' WHERE phase NOT IN ('complete','cancelled')",[])?;
    tx.execute("UPDATE content_change_consumers SET rebuild_required=1", [])?;
    tx.execute("DELETE FROM content_change_context", [])?;
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BindingContent {
    pub library: serde_json::Value,
    pub managed_alias_count: u64,
    pub ordinary_plugin_value_count: u64,
    pub hypa_value_count: u64,
    pub plugin_local_value_count: u64,
}

impl super::PersistentStore {
    pub(crate) fn lww_binding_content(&self) -> StoreResult<BindingContent> {
        let generation = active_generation(&self.connection)?;
        let ordinary = self.connection.query_row("SELECT COUNT(*) FROM plugin_storage WHERE generation=?1", [&generation], |row| row.get::<_, i64>(0))?;
        let aliases = self.connection.query_row("SELECT COUNT(*) FROM asset_aliases WHERE generation=?1 AND object_hash IS NOT NULL", [&generation], |row| row.get::<_, i64>(0))?;
        let device = self.device_store()?;
        let plugin_local = device.connection().query_row("SELECT COUNT(*) FROM plugin_device_storage WHERE tombstone=0", [], |row| row.get::<_, i64>(0))?;
        Ok(BindingContent {
            library: self.materialize(None)?,
            managed_alias_count: aliases as u64,
            ordinary_plugin_value_count: ordinary as u64,
            hypa_value_count: device.hypa_embedding_usage()?.0,
            plugin_local_value_count: plugin_local as u64,
        })
    }
}

#[tauri::command]
pub(crate) fn pds_lww_binding_content(state: tauri::State<'_, super::commands::PersistentStoreState>) -> StoreResult<BindingContent> {
    super::commands::with_store(state, |store| store.lww_binding_content())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BindingSelectionChange {
    pub expected_epoch: String,
    pub new_epoch: String,
    pub target: SyncTarget,
    pub library_id: Option<String>,
    pub target_id: Option<String>,
    pub inspection_id: Option<String>,
}

pub(crate) fn apply_binding_selection(tx: &Transaction<'_>, change: &BindingSelectionChange) -> StoreResult<Selection> {
    let current = read(tx)?;
    if current.epoch == change.new_epoch {
        let (library, target): (Option<String>, Option<String>) = tx.query_row("SELECT library_id,target_id FROM lww_binding_identity WHERE singleton=1", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        if current.target != change.target || library != change.library_id || target != change.target_id {
            return Err(invalid("Binding switch identity changed"));
        }
        return Ok(current);
    }
    if current.epoch != change.expected_epoch || change.new_epoch.is_empty() || change.new_epoch == change.expected_epoch {
        return Err(invalid("Sync selection changed"));
    }
    let (kind, connection) = match &change.target {
        SyncTarget::None => ("none", None),
        SyncTarget::Server(id) => ("server", Some(id)),
        SyncTarget::External(id) => ("external", Some(id)),
    };
    if connection.is_some_and(|id| id.is_empty()) || (connection.is_some() && (change.library_id.as_deref().is_none_or(str::is_empty) || change.target_id.as_deref().is_none_or(str::is_empty))) {
        return Err(invalid("Missing binding identity"));
    }
    tx.execute("UPDATE library_sync_selection SET target=?1,connection_id=?2,selection_epoch=?3,paused=0,decision_required=0 WHERE singleton=1", params![kind, connection, change.new_epoch])?;
    if !matches!(change.target, SyncTarget::None) {
        tx.execute("UPDATE lww_binding_identity SET library_id=?1,target_id=?2 WHERE singleton=1", params![change.library_id, change.target_id])?;
    }
    if let Some(inspection) = &change.inspection_id {
        tx.execute("UPDATE lww_binding_stages SET activation_epoch=?1 WHERE inspection_id=?2", params![change.new_epoch, inspection])?;
    }
    tx.execute("UPDATE external_storage_jobs SET phase='stale' WHERE role IN ('sync','restore') AND phase IN ('preparing','ready')", [])?;
    read(tx)
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        create_schema(&db).unwrap();
        db.execute_batch("CREATE TABLE external_storage_jobs(role TEXT,phase TEXT); INSERT INTO external_storage_jobs VALUES('backup','ready'),('sync','ready'),('restore','preparing');").unwrap();
        db
    }
    fn change(db: &Connection) -> BindingSelectionChange {
        BindingSelectionChange { expected_epoch: read(db).unwrap().epoch, new_epoch: "new".into(), target: SyncTarget::Server("connection".into()), library_id: Some("library".into()), target_id: Some("target".into()), inspection_id: None }
    }
    #[test]
    fn binding_content_counts_orphan_values_in_both_namespaces_when_participation_is_off() {
        use super::super::device_store::plugin_values::PluginDeviceMutation;
        let dir = tempfile::tempdir().unwrap();
        let mut store = super::super::PersistentStore::open(dir.path()).unwrap();
        let device = store.device_store_mut().unwrap();
        device.write_plugin_device_values("orphan", &[
            PluginDeviceMutation::Set { space: "json".into(), key: "empty-json".into(), value: "null".into() },
            PluginDeviceMutation::Set { space: "string".into(), key: "empty-string".into(), value: "".into() },
        ]).unwrap();
        let participation: bool = device.connection().query_row("SELECT participating FROM device_sections WHERE section='local-plugins'", [], |r| r.get(0)).unwrap();
        assert!(!participation);
        let content = store.lww_binding_content().unwrap();
        assert_eq!(content.plugin_local_value_count, 2);
        assert_eq!(content.hypa_value_count, 0);
        assert_eq!(content.ordinary_plugin_value_count, 0);
    }
    #[test]
    fn binding_selection_replays_exactly_and_preserves_backup_jobs() {
        let mut db = database(); let change = change(&db);
        let tx = db.transaction().unwrap();
        let result = apply_binding_selection(&tx, &change).unwrap();
        assert_eq!(apply_binding_selection(&tx, &change).unwrap(), result);
        let backup: String = tx.query_row("SELECT phase FROM external_storage_jobs WHERE role='backup'", [], |r| r.get(0)).unwrap();
        assert_eq!(backup, "ready");
        let stale: i64 = tx.query_row("SELECT COUNT(*) FROM external_storage_jobs WHERE phase='stale'", [], |r| r.get(0)).unwrap();
        assert_eq!(stale, 2);
    }
    #[test]
    fn binding_selection_rejects_stale_and_altered_replays() {
        let mut db = database(); let mut change = change(&db);
        let tx = db.transaction().unwrap();
        apply_binding_selection(&tx, &change).unwrap();
        change.library_id = Some("other".into());
        assert!(apply_binding_selection(&tx, &change).is_err());
        change.new_epoch = "another".into();
        assert!(apply_binding_selection(&tx, &change).is_err());
    }
    #[test]
    fn binding_selection_rejects_missing_identity_without_mutation() {
        let mut db = database(); let mut change = change(&db); change.target_id = None;
        let tx = db.transaction().unwrap();
        assert!(apply_binding_selection(&tx, &change).is_err());
        assert_eq!(read(&tx).unwrap().epoch, change.expected_epoch);
    }
}
