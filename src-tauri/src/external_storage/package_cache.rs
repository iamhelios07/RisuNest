//! What a publication already knows about the remote objects it can name.
//!
//! The cache is rebuildable: every row restates something an authenticated
//! remote document already says, so losing it costs work rather than
//! correctness. It is not a second authority and never decides on its own that
//! a remote object still exists.
use super::{
    contract::{RepositoryHandle, Result},
    contract::ObjectRole,
    packaging::{corrupt, transient, EntryPlan, RemoteObject},
};
use risunest_external_storage_format::snapshot as wire;
use rusqlite::{params, Connection, OptionalExtension};
use std::{fs, path::Path};

pub(super) fn kind_name(kind: wire::CatalogKind) -> &'static str {
    match kind {
        wire::CatalogKind::Records => "records",
        wire::CatalogKind::Assets => "assets",
        wire::CatalogKind::Section => "section",
    }
}

pub(super) struct PackageCache {
    db: Connection,
}
impl PackageCache {
    pub(super) fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(transient)?;
        if crate::trust_boundary::is_link_like(&fs::symlink_metadata(root).map_err(transient)?) {
            return Err(corrupt("package cache is a link"));
        }
        let db_path = root.join("snapshot-cache.sqlite");
        if db_path.exists()
            && crate::trust_boundary::is_link_like(
                &fs::symlink_metadata(&db_path).map_err(transient)?,
            )
        {
            return Err(corrupt("package cache database is a link"));
        }
        let db = Connection::open(db_path).map_err(transient)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS remote_objects(
               repository_id TEXT NOT NULL, connection_identity TEXT NOT NULL,
               object_id TEXT NOT NULL, plaintext_sha256 TEXT NOT NULL,
               value TEXT NOT NULL,
               PRIMARY KEY(repository_id,connection_identity,object_id));
             CREATE TABLE IF NOT EXISTS entries(
               repository_id TEXT NOT NULL, connection_identity TEXT NOT NULL,
               catalog_kind TEXT NOT NULL, entry_key TEXT NOT NULL,
               content_sha256 TEXT NOT NULL, byte_length INTEGER NOT NULL,
               value TEXT NOT NULL, source TEXT NOT NULL,
               PRIMARY KEY(repository_id,connection_identity,catalog_kind,entry_key));
             CREATE TABLE IF NOT EXISTS catalogs(
               repository_id TEXT NOT NULL, connection_identity TEXT NOT NULL,
               catalog_kind TEXT NOT NULL, fingerprint TEXT NOT NULL,
               value TEXT NOT NULL,
               PRIMARY KEY(repository_id,connection_identity,catalog_kind,fingerprint));",
        )
        .map_err(transient)?;
        // Rows without the publication they belong to can never be admitted,
        // and everything here is rebuildable from authenticated metadata, so
        // the table starts again rather than carrying a column it has to guess.
        let tagged: bool = db
            .prepare("SELECT 1 FROM pragma_table_info('entries') WHERE name='source'")
            .map_err(transient)?
            .exists([])
            .map_err(transient)?;
        if !tagged {
            db.execute_batch(
                "DROP TABLE entries;
                 CREATE TABLE entries(
                   repository_id TEXT NOT NULL, connection_identity TEXT NOT NULL,
                   catalog_kind TEXT NOT NULL, entry_key TEXT NOT NULL,
                   content_sha256 TEXT NOT NULL, byte_length INTEGER NOT NULL,
                   value TEXT NOT NULL, source TEXT NOT NULL,
                   PRIMARY KEY(repository_id,connection_identity,catalog_kind,entry_key));",
            )
            .map_err(transient)?;
        }
        Ok(Self { db })
    }
    pub(super) fn object(
        &self,
        format_repository_id: &str,
        repository: &RepositoryHandle,
        id: &str,
        plaintext: &str,
    ) -> Result<Option<RemoteObject>> {
        let encoded: Option<String> = self.db.query_row(
            "SELECT value FROM remote_objects WHERE repository_id=?1 AND connection_identity=?2 AND object_id=?3 AND plaintext_sha256=?4",
            params![format_repository_id,repository.connection_identity,id,plaintext], |row| row.get(0),
        ).optional().map_err(transient)?;
        let Some(encoded) = encoded else {
            return Ok(None);
        };
        let value: RemoteObject = serde_json::from_str(&encoded).map_err(corrupt)?;
        value.stored(repository)?;
        if value.repository_id != format_repository_id || value.object_id != id
            || value.plaintext_sha256 != plaintext
        {
            return Err(corrupt("cached object identity"));
        }
        Ok(Some(value))
    }
    pub(super) fn forget_object(
        &self,
        format_repository_id: &str,
        repository: &RepositoryHandle,
        object_id: &str,
    ) -> Result<()> {
        let transaction = self.db.unchecked_transaction().map_err(transient)?;
        transaction.execute(
            "DELETE FROM remote_objects WHERE repository_id=?1 AND connection_identity=?2 AND object_id=?3",
            params![format_repository_id, repository.connection_identity, object_id],
        ).map_err(transient)?;
        transaction.execute(
            "DELETE FROM entries WHERE repository_id=?1 AND connection_identity=?2
             AND EXISTS(SELECT 1 FROM json_each(entries.value,'$.packs') AS pack
               WHERE json_extract(pack.value,'$.objectId')=?3)",
            params![format_repository_id, repository.connection_identity, object_id],
        ).map_err(transient)?;
        // A changed child can change every parent hash. These roots are only
        // an optimization; their source entries and sealed uploads stay intact.
        transaction.execute(
            "DELETE FROM catalogs WHERE repository_id=?1 AND connection_identity=?2",
            params![format_repository_id, repository.connection_identity],
        ).map_err(transient)?;
        transaction.commit().map_err(transient)
    }

    pub(super) fn put_object(&self, repository: &RepositoryHandle, value: &RemoteObject) -> Result<()> {
        value.stored(repository)?;
        self.db.execute(
            "INSERT INTO remote_objects VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(repository_id,connection_identity,object_id) DO UPDATE SET plaintext_sha256=excluded.plaintext_sha256,value=excluded.value",
            params![value.repository_id,repository.connection_identity,value.object_id,value.plaintext_sha256,serde_json::to_string(value).map_err(corrupt)?],
        ).map_err(transient)?;
        Ok(())
    }
    pub(super) fn entry(
        &self,
        format_repository_id: &str,
        repository: &RepositoryHandle,
        kind: wire::CatalogKind,
        key: &str,
        digest: &str,
        length: u64,
    ) -> Result<Option<(EntryPlan, String)>> {
        let row: Option<(String, String)> = self.db.query_row(
            "SELECT value,source FROM entries WHERE repository_id=?1 AND connection_identity=?2 AND catalog_kind=?3 AND entry_key=?4 AND content_sha256=?5 AND byte_length=?6",
            params![format_repository_id,repository.connection_identity,kind_name(kind),key,digest,i64::try_from(length).map_err(corrupt)?],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(transient)?;
        let Some((encoded, source)) = row else {
            return Ok(None);
        };
        let value: EntryPlan = serde_json::from_str(&encoded).map_err(corrupt)?;
        value.validate(format_repository_id, repository)?;
        Ok(Some((value, source)))
    }
    pub(super) fn put_entries(
        &self,
        format_repository_id: &str,
        repository: &RepositoryHandle,
        kind: wire::CatalogKind,
        values: &[EntryPlan],
    ) -> Result<()> {
        let transaction = self.db.unchecked_transaction().map_err(transient)?;
        for value in values {
            value.validate(format_repository_id, repository)?;
            transaction.execute(
                "INSERT INTO entries VALUES(?1,?2,?3,?4,?5,?6,?7,'')
                 ON CONFLICT(repository_id,connection_identity,catalog_kind,entry_key) DO UPDATE SET content_sha256=excluded.content_sha256,byte_length=excluded.byte_length,value=excluded.value,source=''",
                params![format_repository_id,repository.connection_identity,kind_name(kind),value.key,value.content_sha256,i64::try_from(value.byte_length).map_err(corrupt)?,serde_json::to_string(value).map_err(corrupt)?],
            ).map_err(transient)?;
        }
        transaction.commit().map_err(transient)?;
        Ok(())
    }

    /// The publication a row belongs to is only known once its catalog root
    /// exists, so every entry the new catalog carries is tagged then, reused
    /// ones included. A row left untagged by a stopped publication names no
    /// root and is never admitted.
    pub(super) fn tag_entries(
        &self,
        format_repository_id: &str,
        repository: &RepositoryHandle,
        kind: wire::CatalogKind,
        keys: &[String],
        source: &str,
    ) -> Result<()> {
        if source.is_empty() {
            return Err(corrupt("untagged catalog root"));
        }
        let transaction = self.db.unchecked_transaction().map_err(transient)?;
        {
            let mut statement = transaction.prepare(
                "UPDATE entries SET source=?5 WHERE repository_id=?1 AND connection_identity=?2 AND catalog_kind=?3 AND entry_key=?4",
            ).map_err(transient)?;
            for key in keys {
                statement.execute(params![
                    format_repository_id, repository.connection_identity, kind_name(kind), key, source,
                ]).map_err(transient)?;
            }
        }
        transaction.commit().map_err(transient)?;
        Ok(())
    }
    pub(super) fn catalog(
        &self,
        format_repository_id: &str,
        repository: &RepositoryHandle,
        kind: wire::CatalogKind,
        fingerprint: &str,
    ) -> Result<Option<RemoteObject>> {
        let encoded: Option<String> = self.db.query_row(
            "SELECT value FROM catalogs WHERE repository_id=?1 AND connection_identity=?2 AND catalog_kind=?3 AND fingerprint=?4",
            params![format_repository_id,repository.connection_identity,kind_name(kind),fingerprint], |row| row.get(0),
        ).optional().map_err(transient)?;
        let Some(encoded) = encoded else {
            return Ok(None);
        };
        let value: RemoteObject = serde_json::from_str(&encoded).map_err(corrupt)?;
        value.stored(repository)?;
        if value.role != ObjectRole::Catalog || value.repository_id != format_repository_id {
            return Err(corrupt("cached catalog role"));
        }
        Ok(Some(value))
    }
    pub(super) fn put_catalog(
        &self,
        repository: &RepositoryHandle,
        kind: wire::CatalogKind,
        fingerprint: &str,
        value: &RemoteObject,
    ) -> Result<()> {
        value.stored(repository)?;
        self.db
            .execute(
                "INSERT OR REPLACE INTO catalogs VALUES(?1,?2,?3,?4,?5)",
                params![
                    value.repository_id,
                    repository.connection_identity,
                    kind_name(kind),
                    fingerprint,
                    serde_json::to_string(value).map_err(corrupt)?
                ],
            )
            .map_err(transient)?;
        Ok(())
    }
}
/// The existing upload inventory bounds collection ownership. Reading it never
/// discovers foreign packs or treats historical receipts as current existence.
pub(crate) fn forget_remote_object(root: &Path, repository: &RepositoryHandle, object: &RemoteObject) -> Result<()> {
    PackageCache::open(root)?.forget_object(&object.repository_id, repository, &object.object_id)
}

pub(crate) fn known_remote_objects(
    root: &Path,
    format_repository_id: &str,
    repository: &RepositoryHandle,
) -> Result<Vec<RemoteObject>> {
    let path = root.join("snapshot-cache.sqlite");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(transient(error)),
        Ok(_) => {}
    }
    if crate::trust_boundary::is_link_like(&fs::symlink_metadata(root).map_err(transient)?) {
        return Err(corrupt("package cache root is a link"));
    }
    crate::trust_boundary::open_regular_source(&path).map_err(transient)?;
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(transient)?;
    let mut query = db.prepare(
        "SELECT object_id,plaintext_sha256,value FROM remote_objects
         WHERE repository_id=?1 AND connection_identity=?2 ORDER BY object_id",
    ).map_err(transient)?;
    let rows = query.query_map(params![format_repository_id, repository.connection_identity], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    }).map_err(transient)?;
    let mut objects = Vec::new();
    for row in rows {
        let (id, digest, encoded) = row.map_err(transient)?;
        if encoded.len() > 128 * 1024 { return Err(corrupt("inventory object exceeds limit")); }
        let object: RemoteObject = serde_json::from_str(&encoded).map_err(corrupt)?;
        object.stored(repository)?;
        if object.repository_id != format_repository_id || object.object_id != id
            || object.plaintext_sha256 != digest
        {
            return Err(corrupt("inventory object identity differs"));
        }
        objects.push(object);
    }
    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_storage::{contract::ErrorKind, fake};

    /// Nothing here is authoritative, so a cache without the publication its
    /// rows belong to starts again instead of guessing one.
    #[test]
    fn entries_without_their_publication_are_discarded_on_open() {
        let root = tempfile::tempdir().unwrap();
        let repository = fake::repository();
        {
            let db = Connection::open(root.path().join("snapshot-cache.sqlite")).unwrap();
            db.execute_batch(
                "CREATE TABLE entries(
                   repository_id TEXT NOT NULL, connection_identity TEXT NOT NULL,
                   catalog_kind TEXT NOT NULL, entry_key TEXT NOT NULL,
                   content_sha256 TEXT NOT NULL, byte_length INTEGER NOT NULL,
                   value TEXT NOT NULL,
                   PRIMARY KEY(repository_id,connection_identity,catalog_kind,entry_key));
                 INSERT INTO entries VALUES('format-repository','synthetic-identity','assets','a','b',1,'{}');",
            ).unwrap();
        }
        let cache = PackageCache::open(root.path()).unwrap();
        let held: i64 = cache.db.query_row("SELECT count(*) FROM entries", [], |row| row.get(0)).unwrap();
        assert_eq!(held, 0);
        assert!(cache
            .entry("format-repository", &repository, wire::CatalogKind::Assets, "a", &"b".repeat(64), 1)
            .unwrap()
            .is_none());
        assert_eq!(
            cache
                .tag_entries("format-repository", &repository, wire::CatalogKind::Assets, &[], "")
                .unwrap_err()
                .kind,
            ErrorKind::Corrupt,
        );
    }
}
