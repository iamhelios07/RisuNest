//! Bounded BLOB operations on a caller-owned SQLite connection.
//!
//! Small immutable objects cost one file publication and one directory sync
//! each when they live in a filesystem store. This crate puts their bodies in
//! a table the owner already keeps, so a group of them becomes durable in one
//! transaction instead of one publication per object.
//!
//! It owns no threads, locks, scheduler, encryption or collection policy. The
//! caller supplies the connection or transaction, decides which objects are
//! small enough to belong here, and keeps its own large-object path.

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub enum StoreError {
    /// A stored body no longer matches the identity it is filed under, or a
    /// caller offered different bytes for an existing identity.
    Corrupt,
    /// The identity is not 64 lowercase hexadecimal characters.
    InvalidIdentity,
    /// The body is larger than the caller said it would accept.
    TooLarge,
    Database(rusqlite::Error),
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Corrupt => out.write_str("small object body does not match its identity"),
            Self::InvalidIdentity => out.write_str("small object identity is not a content hash"),
            Self::TooLarge => out.write_str("small object body exceeds the requested limit"),
            Self::Database(error) => write!(out, "{error}"),
        }
    }
}

impl std::error::Error for StoreError {}

pub type Result<T> = std::result::Result<T, StoreError>;

pub const TABLE: &str = "small_objects";

/// Lowercase hexadecimal SHA-256, the identity both owners already use.
pub fn identity(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn check_identity(hash: &str) -> Result<()> {
    if hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(StoreError::InvalidIdentity)
    }
}

/// Create the table if the owner's database does not have it yet. The owner
/// keeps its own schema version; this table carries no version of its own
/// because its shape is fully described by the content identity it is keyed by.
pub fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {TABLE}(hash TEXT PRIMARY KEY, body BLOB NOT NULL);"
    ))?;
    Ok(())
}

/// Write a group of bodies in the caller's transaction, reusing one prepared
/// statement. An identity already present keeps its stored body; offering
/// different bytes under that identity is corruption, never an update.
pub fn insert_batch(tx: &Transaction<'_>, objects: &[(&str, &[u8])]) -> Result<()> {
    let mut existing = tx.prepare(&format!("SELECT body FROM {TABLE} WHERE hash=?1"))?;
    let mut insert = tx.prepare(&format!("INSERT INTO {TABLE}(hash,body) VALUES(?1,?2)"))?;
    for (hash, bytes) in objects {
        check_identity(hash)?;
        if identity(bytes) != *hash {
            return Err(StoreError::Corrupt);
        }
        let stored: Option<Vec<u8>> = existing.query_row([hash], |r| r.get(0)).optional()?;
        match stored {
            Some(stored) if stored == *bytes => continue,
            Some(_) => return Err(StoreError::Corrupt),
            None => {
                insert.execute(params![hash, bytes])?;
            }
        }
    }
    Ok(())
}

/// The stored body length, or `None` when this store does not hold it. The
/// caller uses that answer to decide which of its stores owns the object; it
/// must never infer the location from a size threshold.
pub fn size(db: &Connection, hash: &str) -> Result<Option<u64>> {
    check_identity(hash)?;
    let size: Option<i64> = db
        .query_row(
            &format!("SELECT length(body) FROM {TABLE} WHERE hash=?1"),
            [hash],
            |r| r.get(0),
        )
        .optional()?;
    Ok(size.map(|size| size.max(0) as u64))
}

/// Read a body, refusing one larger than the caller accepts before returning
/// it and verifying that what was stored still matches its identity.
pub fn read(db: &Connection, hash: &str, limit: usize) -> Result<Option<Vec<u8>>> {
    check_identity(hash)?;
    let Some(size) = size(db, hash)? else {
        return Ok(None);
    };
    if size > limit as u64 {
        return Err(StoreError::TooLarge);
    }
    let bytes: Vec<u8> = db.query_row(
        &format!("SELECT body FROM {TABLE} WHERE hash=?1"),
        [hash],
        |r| r.get(0),
    )?;
    if identity(&bytes) != hash {
        return Err(StoreError::Corrupt);
    }
    Ok(Some(bytes))
}

/// Identities and sizes in identity order, starting after the given one. The
/// bodies stay in the database, so a caller enumerating its whole store for
/// accounting or collection does not materialize them.
pub fn page(db: &Connection, after: &str, limit: usize) -> Result<Vec<(String, u64)>> {
    let mut statement = db.prepare(&format!(
        "SELECT hash,length(body) FROM {TABLE} WHERE hash>?1 ORDER BY hash LIMIT ?2"
    ))?;
    let rows = statement
        .query_map(params![after, limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Remove a group of bodies in the caller's transaction. The caller owns the
/// decision that they are unreferenced; this returns how many rows it removed.
pub fn delete_batch(tx: &Transaction<'_>, hashes: &[&str]) -> Result<usize> {
    let mut delete = tx.prepare(&format!("DELETE FROM {TABLE} WHERE hash=?1"))?;
    let mut removed = 0;
    for hash in hashes {
        check_identity(hash)?;
        removed += delete.execute([hash])?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(path: &std::path::Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .unwrap();
        initialize(&db).unwrap();
        db
    }

    fn body(index: usize) -> Vec<u8> {
        format!("synthetic small object {index:08}").into_bytes()
    }

    #[test]
    fn a_group_survives_reopening_and_an_abandoned_group_leaves_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("objects.sqlite");
        let mut db = open(&path);
        let committed = (0..8).map(body).collect::<Vec<_>>();
        let tx = db.transaction().unwrap();
        insert_batch(
            &tx,
            &committed
                .iter()
                .map(|bytes| (identity(bytes), bytes.as_slice()))
                .collect::<Vec<_>>()
                .iter()
                .map(|(hash, bytes)| (hash.as_str(), *bytes))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        tx.commit().unwrap();
        let abandoned = body(100);
        let tx = db.transaction().unwrap();
        insert_batch(&tx, &[(identity(&abandoned).as_str(), &abandoned)]).unwrap();
        drop(tx);
        drop(db);

        let db = open(&path);
        for bytes in &committed {
            assert_eq!(
                read(&db, &identity(bytes), bytes.len()).unwrap().as_ref(),
                Some(bytes)
            );
        }
        assert!(size(&db, &identity(&abandoned)).unwrap().is_none());
    }

    #[test]
    fn an_existing_identity_is_reused_and_different_bytes_under_it_are_corruption() {
        let directory = tempfile::tempdir().unwrap();
        let mut db = open(&directory.path().join("objects.sqlite"));
        let bytes = body(1);
        let hash = identity(&bytes);
        for _ in 0..3 {
            let tx = db.transaction().unwrap();
            insert_batch(&tx, &[(hash.as_str(), &bytes)]).unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(size(&db, &hash).unwrap(), Some(bytes.len() as u64));
        let tx = db.transaction().unwrap();
        assert!(matches!(
            insert_batch(&tx, &[(hash.as_str(), b"different bytes")]),
            Err(StoreError::Corrupt)
        ));
        drop(tx);
        // A body filed under someone else's identity never enters the store.
        let tx = db.transaction().unwrap();
        assert!(matches!(
            insert_batch(&tx, &[(identity(&body(2)).as_str(), &bytes)]),
            Err(StoreError::Corrupt)
        ));
        drop(tx);
        assert!(matches!(
            size(&db, "not a content hash"),
            Err(StoreError::InvalidIdentity)
        ));
    }

    #[test]
    fn a_read_refuses_a_body_above_its_limit_and_reports_a_damaged_one() {
        let directory = tempfile::tempdir().unwrap();
        let mut db = open(&directory.path().join("objects.sqlite"));
        let bytes = body(7);
        let hash = identity(&bytes);
        let tx = db.transaction().unwrap();
        insert_batch(&tx, &[(hash.as_str(), &bytes)]).unwrap();
        tx.commit().unwrap();
        assert!(matches!(
            read(&db, &hash, bytes.len() - 1),
            Err(StoreError::TooLarge)
        ));
        db.execute(
            &format!("UPDATE {TABLE} SET body=?1 WHERE hash=?2"),
            params![b"damaged".to_vec(), hash],
        )
        .unwrap();
        assert!(matches!(read(&db, &hash, 1024), Err(StoreError::Corrupt)));
    }

    #[test]
    fn enumeration_is_ordered_and_pages_and_deletion_removes_exactly_its_group() {
        let directory = tempfile::tempdir().unwrap();
        let mut db = open(&directory.path().join("objects.sqlite"));
        let bodies = (0..20).map(body).collect::<Vec<_>>();
        let hashes = bodies.iter().map(|b| identity(b)).collect::<Vec<_>>();
        let tx = db.transaction().unwrap();
        insert_batch(
            &tx,
            &hashes
                .iter()
                .zip(&bodies)
                .map(|(hash, bytes)| (hash.as_str(), bytes.as_slice()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        tx.commit().unwrap();

        let mut seen = Vec::new();
        let mut after = String::new();
        loop {
            let group = page(&db, &after, 7).unwrap();
            if group.is_empty() {
                break;
            }
            assert!(group.len() <= 7);
            after = group.last().unwrap().0.clone();
            seen.extend(group);
        }
        let mut expected = hashes.clone();
        expected.sort();
        assert_eq!(
            seen.iter().map(|(hash, _)| hash.clone()).collect::<Vec<_>>(),
            expected
        );
        assert!(seen.iter().all(|(hash, size)| {
            *size == bodies[hashes.iter().position(|h| h == hash).unwrap()].len() as u64
        }));

        let removed = hashes[..5].iter().map(String::as_str).collect::<Vec<_>>();
        let tx = db.transaction().unwrap();
        assert_eq!(delete_batch(&tx, &removed).unwrap(), 5);
        // Deleting what is already gone is not an error and removes nothing.
        assert_eq!(delete_batch(&tx, &removed).unwrap(), 0);
        tx.commit().unwrap();
        assert_eq!(page(&db, "", 100).unwrap().len(), 15);
        for hash in &hashes[5..] {
            assert!(size(&db, hash).unwrap().is_some());
        }
    }
}
