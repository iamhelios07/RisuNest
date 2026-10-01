use super::PersistentStore;
use crate::server_sync::{credentials::StoredConfig, Result, SyncError};
use rusqlite::{Connection, OptionalExtension};
impl PersistentStore {
    pub(crate) fn server_stored_config(&self) -> Result<Option<StoredConfig>> {
        let text: Option<String> = self
            .connection
            .query_row(
                "SELECT config FROM server_sync_state WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        text.map(|s| {
            serde_json::from_str(&s).map_err(|_| SyncError::new("invalid-local-server-config", 409))
        })
        .transpose()
    }
    pub(crate) fn server_repair_residency_access(&self) -> Result<()> {
        if let Some(config)=self.server_stored_config()? {
            if crate::server_sync::residency::Residency::exists(&self.repository_root) { crate::server_sync::residency::Residency::open(&self.repository_root)?.replace_access_config(&config)?; }
        } Ok(())
    }


}
pub(super) fn create_schema(db: &Connection) -> super::StoreResult<()> {
    db.execute_batch("CREATE TABLE server_sync_state(singleton INTEGER PRIMARY KEY CHECK(singleton=1),config TEXT NOT NULL);")?; Ok(())
}
pub(super) fn validate_schema(db: &Connection) -> super::StoreResult<()> {
    let actual: Option<String> = db.query_row("SELECT sql FROM sqlite_master WHERE type='table' AND name='server_sync_state'", [], |row| row.get(0)).optional()?;
    if actual.as_deref() != Some("CREATE TABLE server_sync_state(singleton INTEGER PRIMARY KEY CHECK(singleton=1),config TEXT NOT NULL)") { return Err(super::StoreError::Validation { message: "Server connection schema is incompatible".into() }); }
    Ok(())
}
