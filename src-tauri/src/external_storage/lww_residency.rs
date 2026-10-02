use super::{
    connection_store::ConnectionStore,
    contract::*,
    lww_segment::{self as segment, LargeBody},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Source {
    pub hash: String,
    pub library_id: String,
    pub connection_id: String,
    pub connection_root: PathBuf,
    pub protected_segment: String,
    pub body: LargeBody,
}
fn local(_: impl std::fmt::Display) -> ProviderError {
    segment::corrupt()
}
fn database(root: &Path, create: bool) -> Result<Option<Connection>> {
    let directory = root.join("external-sync");
    let path = directory.join("remote-bodies.sqlite");
    if !create && !path.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(&directory).map_err(local)?;
    if crate::trust_boundary::is_link_like(&std::fs::symlink_metadata(&directory).map_err(local)?) {
        return Err(segment::corrupt());
    }
    for suffix in ["", "-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{suffix}", path.display()));
        if path.exists() {
            crate::trust_boundary::open_regular_source(&path).map_err(local)?;
        }
    }
    let db = Connection::open(path).map_err(local)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000;
        CREATE TABLE IF NOT EXISTS sources(hash TEXT NOT NULL,library TEXT NOT NULL,source TEXT NOT NULL,PRIMARY KEY(hash,library));").map_err(local)?;
    Ok(Some(db))
}
pub(crate) fn register(root: &Path, source: &Source) -> Result<()> {
    risunest_sync_wire::validate_hash(&source.hash).map_err(local)?;
    let locator = source.body.locator.as_ref().ok_or_else(segment::corrupt)?;
    if source.protected_segment.is_empty()
        || locator.object.is_empty()
        || source.connection_id.is_empty()
    {
        return Err(segment::corrupt());
    }
    let db = database(root, true)?.ok_or_else(segment::corrupt)?;
    let previous: Option<String> = db
        .query_row(
            "SELECT source FROM sources WHERE hash=?1 AND library=?2",
            params![source.hash, source.library_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(local)?;
    if let Some(previous) = previous {
        let previous: Source = serde_json::from_str(&previous).map_err(local)?;
        if previous.body.plaintext_byte_length != source.body.plaintext_byte_length {
            return Err(segment::corrupt());
        }
    }
    db.execute("INSERT INTO sources(hash,library,source) VALUES(?1,?2,?3) ON CONFLICT(hash,library) DO UPDATE SET source=excluded.source",
        params![source.hash,source.library_id,serde_json::to_string(source).map_err(local)?]).map_err(local)?;
    Ok(())
}
pub(crate) fn source(root: &Path, hash: &str) -> Result<Option<Source>> {
    let Some(db) = database(root, false)? else {
        return Ok(None);
    };
    let value: Option<String> = db
        .query_row(
            "SELECT source FROM sources WHERE hash=?1 ORDER BY library LIMIT 1",
            [hash],
            |r| r.get(0),
        )
        .optional()
        .map_err(local)?;
    value
        .map(|value| serde_json::from_str(&value).map_err(local))
        .transpose()
}
pub(crate) fn stat(root: &Path, hash: &str) -> std::io::Result<Option<u64>> {
    source(root, hash)
        .map(|source| source.map(|source| source.body.plaintext_byte_length.0))
        .map_err(|_| std::io::Error::other("external-source-invalid"))
}
pub(crate) async fn fulfill(
    root: &Path,
    hash: &str,
    provider: &dyn Provider,
    repository: &RepositoryHandle,
    key: &[u8; 32],
    cancel: &Cancellation,
) -> Result<Option<std::fs::File>> {
    let cas = crate::asset_repository::PayloadCas::new(root).map_err(local)?;
    if let Some(file) = cas.open_object(hash).map_err(local)? {
        return Ok(Some(file));
    }
    let Some(bytes) = verified_body(root, hash, provider, repository, key, cancel).await? else {
        return Ok(None);
    };
    cancel.check()?;
    promote(root, hash, &bytes)
}
async fn verified_body(
    root: &Path,
    hash: &str,
    provider: &dyn Provider,
    repository: &RepositoryHandle,
    key: &[u8; 32],
    cancel: &Cancellation,
) -> Result<Option<Vec<u8>>> {
    let Some(source) = source(root, hash)? else {
        return Ok(None);
    };
    let locator = source.body.locator.as_ref().ok_or_else(segment::corrupt)?;
    locator.validate_for(repository)?;
    let sealed = super::lww_engine::read_bytes(provider, repository, locator, cancel).await?;
    if sealed.len() as u64 != source.body.byte_length.0
        || segment::digest(&sealed) != source.body.sha256
    {
        return Err(segment::corrupt());
    }
    let bytes = segment::open_body(&sealed, &source.library_id, &source.body.object_id, key)?;
    if bytes.len() as u64 != source.body.plaintext_byte_length.0 || segment::digest(&bytes) != hash
    {
        return Err(segment::corrupt());
    }
    Ok(Some(bytes))
}
fn promote(root: &Path, hash: &str, bytes: &[u8]) -> Result<Option<std::fs::File>> {
    let cas = crate::asset_repository::PayloadCas::new(root).map_err(local)?;
    let prepared = cas.prepare_bytes(bytes).map_err(local)?;
    if prepared.content_hash != hash {
        return Err(segment::corrupt());
    }
    crate::persistent_store::register_asset_objects_at_root(
        root,
        &[
            crate::persistent_store::asset_object_catalog::AssetObjectRegistration {
                object_hash: hash.into(),
                byte_size: bytes.len() as u64,
            },
        ],
        i64::try_from(super::runtime::now_ms()).map_err(local)?,
    )
    .map_err(local)?;
    cas.open_object(hash).map_err(local)
}
pub(crate) fn hydrate(
    root: &Path,
    hash: &str,
    check: &dyn Fn() -> crate::server_sync::Result<()>,
) -> crate::server_sync::Result<Option<std::fs::File>> {
    let cas = crate::asset_repository::PayloadCas::new(root)?;
    if let Some(file) = cas.open_object(hash)? {
        return Ok(Some(file));
    }
    if source(root, hash)
        .map_err(|_| crate::server_sync::SyncError::new("external-source-invalid", 409))?
        .is_none()
    {
        return Ok(None);
    };
    check()?;
    let bytes = tauri::async_runtime::block_on(fetch_missing(root, hash, &Cancellation::default()))
        .map_err(|error| {
            crate::server_sync::SyncError::new(
                match error.kind {
                    ErrorKind::ReauthRequired => "external-reauth-required",
                    _ => "external-body-unavailable",
                },
                502,
            )
        })?;
    check()?;
    let file = bytes
        .map(|bytes| promote(root, hash, &bytes))
        .transpose()
        .map_err(|_| crate::server_sync::SyncError::new("external-body-unavailable", 502))?
        .flatten();
    check()?;
    Ok(file)
}

pub(crate) async fn hydrate_missing(
    root: &Path,
    hash: &str,
    cancel: &Cancellation,
) -> Result<Option<std::fs::File>> {
    let cas = crate::asset_repository::PayloadCas::new(root).map_err(local)?;
    if let Some(file) = cas.open_object(hash).map_err(local)? {
        return Ok(Some(file));
    }
    let Some((provider, repository, key)) = remote_source(root, hash, cancel).await? else {
        return Ok(None);
    };
    fulfill(root, hash, provider.as_ref(), &repository, &key, cancel).await
}
async fn fetch_missing(root: &Path, hash: &str, cancel: &Cancellation) -> Result<Option<Vec<u8>>> {
    let Some((provider, repository, key)) = remote_source(root, hash, cancel).await? else {
        return Ok(None);
    };
    verified_body(root, hash, provider.as_ref(), &repository, &key, cancel).await
}
async fn remote_source(
    root: &Path,
    hash: &str,
    cancel: &Cancellation,
) -> Result<
    Option<(
        std::sync::Arc<dyn Provider>,
        RepositoryHandle,
        zeroize::Zeroizing<[u8; 32]>,
    )>,
> {
    let Some(source) = source(root, hash)? else {
        return Ok(None);
    };
    let stored = ConnectionStore::open(&source.connection_root)?.read(&source.connection_id)?;
    if stored.descriptor.repository_id != source.library_id {
        return Err(segment::corrupt());
    }
    let dependencies =
        super::connection::dependencies_for_config(&source.connection_root, &stored.config)?;
    let provider = super::connection::provider_for(&stored.config, dependencies)?;
    let (repository, _) = provider
        .open_repository(
            &stored.config,
            &SecretRef(stored.credential_ref.clone()),
            OpenMode::Existing,
            cancel,
        )
        .await?;
    let key = super::connection_commands::read_root_key(
        super::secrets::repository_key_vault(&source.connection_root).as_ref(),
        &stored.root_key_ref,
    )
    .await?;
    Ok(Some((provider, repository, key)))
}
