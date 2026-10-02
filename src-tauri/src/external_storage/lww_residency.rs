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
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct PackedSource {
    pub hash:String,
    pub byte_length:u64,
    pub library_id:String,
    pub connection_id:String,
    pub connection_root:PathBuf,
    pub protected_snapshot:String,
    pub catalog:risunest_external_storage_format::snapshot::StoredObject,
    pub chunks:Vec<risunest_external_storage_format::snapshot::StoredChunk>,
    pub packs:Vec<risunest_external_storage_format::snapshot::StoredObject>,
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(tag="kind",rename_all="kebab-case",deny_unknown_fields)]
pub(crate) enum FrozenBodySource {
    Standalone(Source),
    Packed(PackedSource),
}
impl FrozenBodySource {
    pub(crate) fn byte_length(&self)->u64 {
        match self {
            Self::Standalone(source)=>source.body.plaintext_byte_length.0,
            Self::Packed(source)=>source.byte_length,
        }
    }
}
pub(crate) fn freeze_remote_body(root:&Path,hash:&str)->Result<Option<FrozenBodySource>> {
    if let Some(source)=source(root,hash)? {return Ok(Some(FrozenBodySource::Standalone(source)));}
    Ok(packed_source(root,hash)?.map(FrozenBodySource::Packed))
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
        CREATE TABLE IF NOT EXISTS sources(hash TEXT NOT NULL,library TEXT NOT NULL,source TEXT NOT NULL,PRIMARY KEY(hash,library));
        CREATE TABLE IF NOT EXISTS packed_sources(hash TEXT NOT NULL,library TEXT NOT NULL,source TEXT NOT NULL,PRIMARY KEY(hash,library));").map_err(local)?;
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
        .and_then(|source| if let Some(source)=source {Ok(Some(source.body.plaintext_byte_length.0))} else {packed_source(root,hash).map(|s|s.map(|s|s.byte_length))})
        .map_err(|_| std::io::Error::other("external-source-invalid"))
}
pub(crate) fn validate_packed_source(source:&PackedSource,repository:&RepositoryHandle) -> Result<()> {
    risunest_sync_wire::validate_hash(&source.hash).map_err(local)?;
    source.catalog.validate().map_err(local)?;
    super::packaging::RemoteObject::from_stored(&source.catalog,repository)?;
    if source.catalog.header.role!=risunest_external_storage_format::snapshot::ObjectRole::Catalog
        || source.protected_snapshot.is_empty() || source.connection_id.is_empty()
        || (source.chunks.is_empty() && source.byte_length!=0) { return Err(segment::corrupt()); }
    let mut length=0u64;
    let packs=source.packs.iter().map(|p|(p.header.object_id.as_str(),p)).collect::<std::collections::BTreeMap<_,_>>();
    if packs.len()!=source.packs.len() {return Err(segment::corrupt());}
    for chunk in &source.chunks {
        let pack=packs.get(chunk.pack_id.as_str()).ok_or_else(segment::corrupt)?;
        super::packaging::RemoteObject::from_stored(pack,repository)?;
        if pack.header.role!=risunest_external_storage_format::snapshot::ObjectRole::Pack
            || pack.header.repository_id!=source.catalog.header.repository_id
            || chunk.offset.checked_add(chunk.stored_length).is_none_or(|end|end>pack.plaintext_length) { return Err(segment::corrupt()); }
        length=length.checked_add(chunk.plaintext_length).ok_or_else(segment::corrupt)?;
    }
    if length!=source.byte_length { return Err(segment::corrupt()); }
    Ok(())
}
pub(crate) fn register_packed(root:&Path,source:&PackedSource,repository:&RepositoryHandle) -> Result<()> {
    validate_packed_source(source,repository)?;
    register_verified_packed(root,source)
}
pub(super) fn register_verified_packed(root:&Path,source:&PackedSource) -> Result<()> {
    let db=database(root,true)?.ok_or_else(segment::corrupt)?;
    db.execute("INSERT INTO packed_sources VALUES(?1,?2,?3) ON CONFLICT(hash,library) DO UPDATE SET source=excluded.source",params![source.hash,source.library_id,serde_json::to_string(source).map_err(local)?]).map_err(local)?;
    Ok(())
}
pub(crate) fn packed_source(root:&Path,hash:&str) -> Result<Option<PackedSource>> {
    let Some(db)=database(root,false)? else {return Ok(None)};
    let value:Option<String>=db.query_row("SELECT source FROM packed_sources WHERE hash=?1 ORDER BY library LIMIT 1",[hash],|r|r.get(0)).optional().map_err(local)?;
    value.map(|v|serde_json::from_str(&v).map_err(local)).transpose()
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
        let Some(source)=packed_source(root,hash)? else {return Ok(None)};
        let staging=tempfile::tempdir().map_err(local)?;
        return super::snapshot_restore::download_packed_body(hash,source.byte_length,source.chunks,source.packs,staging.path(),key,provider,repository,cancel).await.map(Some);
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
pub(crate) fn hydrate_registered(root:&Path,hash:&str,check:&dyn Fn()->crate::server_sync::Result<()>)->crate::server_sync::Result<bool> {
    check()?;
    let cas=crate::asset_repository::PayloadCas::new(root)?;
    if cas.stat_object(hash)?.is_some() {return Ok(true);}
    if stat(root,hash)?.is_none() {return Ok(false);}
    let scratch=tempfile::tempdir()?;
    let Some(mut body)=tauri::async_runtime::block_on(spool_verified_remote_body(root,hash,scratch.path(),&Cancellation::default())).map_err(|_|crate::server_sync::SyncError::new("external-body-unavailable",502))? else {return Ok(false)};
    check()?;
    let prepared=cas.prepare_reader(body.as_file_mut())?;
    if prepared.content_hash!=hash {return Err(crate::server_sync::SyncError::new("external-body-unavailable",502));}
    crate::persistent_store::register_asset_objects_at_root(root,&[crate::persistent_store::asset_object_catalog::AssetObjectRegistration{object_hash:hash.into(),byte_size:prepared.byte_size}],i64::try_from(super::runtime::now_ms()).map_err(|_|crate::server_sync::SyncError::new("external-body-unavailable",502))?).map_err(|_|crate::server_sync::SyncError::new("external-body-unavailable",502))?;
    check()?;Ok(true)
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
    if stat(root, hash)?.is_none()
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
pub(crate) async fn spool_verified_remote_body(root:&Path,hash:&str,destination:&Path,cancel:&Cancellation)->Result<Option<tempfile::NamedTempFile>> {
    let Some(source)=freeze_remote_body(root,hash)? else {return Ok(None)};
    spool_frozen_remote_body(&source,destination,cancel).await.map(Some)
}
pub(crate) async fn spool_frozen_remote_body(source:&FrozenBodySource,destination:&Path,cancel:&Cancellation)->Result<tempfile::NamedTempFile> {
    let (hash,connection_root,connection_id,library_id)=match source {
        FrozenBodySource::Standalone(source)=>(&source.hash,&source.connection_root,&source.connection_id,&source.library_id),
        FrozenBodySource::Packed(source)=>(&source.hash,&source.connection_root,&source.connection_id,&source.library_id),
    };
    let (provider,repository,key)=open_source_connection(connection_root,connection_id,library_id,cancel).await?;
    let stored=ConnectionStore::open(connection_root)?.read(connection_id)?;
    if let FrozenBodySource::Packed(source)=source {validate_packed_source(source,&repository)?;}
    risunest_sync_wire::validate_hash(hash).map_err(local)?;
    let connection_root=connection_root.clone();
    let connection_id=connection_id.clone();
    let source=source.clone();
    let writer=uuid::Uuid::new_v4().to_string();
    let job=uuid::Uuid::new_v4().to_string();
    let protection=super::leases::LeaseContext{root:&connection_root,connection_id:&connection_id,writer_id:&writer,descriptor:&stored.descriptor,root_key:&key,provider:provider.as_ref(),repository:&repository,clock:super::leases::system_clock(),protection_supported:stored.capabilities.lease_operations,ledger:None};
    let owner=match super::leases::admit_shared_work(&protection,&job,cancel).await? {
        super::leases::Admission::Admitted(owner)=>owner,
        super::leases::Admission::Yield{reason}=>return Err(super::leases::yield_error(reason)),
        super::leases::Admission::UnsupportedProtection=>return Err(ProviderError::new(ErrorKind::Unsupported)),
    };
    owner.run(&protection,cancel,async {
        std::fs::create_dir_all(destination).map_err(local)?;
        let mut output=tempfile::NamedTempFile::new_in(destination).map_err(local)?;
        let scratch=tempfile::tempdir_in(destination).map_err(local)?;
        if let FrozenBodySource::Standalone(source)=&source {
            let locator=source.body.locator.as_ref().ok_or_else(segment::corrupt)?;
            locator.validate_for(&repository)?;
            let cipher=scratch.path().join("ciphertext");
            let mut sink=super::transfer::SpoolSink::create(&cipher,source.body.byte_length.0)?;
            let receipt=match provider.read_object(&repository,locator,None,&mut sink,cancel).await? {
                ReadReceipt::Body(receipt)=>receipt,
                ReadReceipt::NotModified(_)=>return Err(segment::corrupt()),
            };
            if receipt.locator!=*locator || receipt.byte_length!=source.body.byte_length.0 || !sink.is_verified() {return Err(segment::corrupt());}
            let _verified=super::transfer::SpoolSource::verified(&cipher,source.body.byte_length.0,&source.body.sha256)?;
            let mut input=crate::trust_boundary::open_regular_source(&cipher).map_err(local)?;
            segment::open_body_stream(&mut input,&mut output,&source.library_id,&source.body.object_id,&key,source.body.plaintext_byte_length.0)?;
            if !super::snapshot_restore::verify_body_file(output.path(),source.body.plaintext_byte_length.0,hash)? {return Err(segment::corrupt());}
        } else if let FrozenBodySource::Packed(source)=&source {
            let path=super::snapshot_restore::download_packed_body_file(hash,source.byte_length,source.chunks.clone(),source.packs.clone(),scratch.path(),&key,provider.as_ref(),&repository,cancel).await?;
            let mut input=crate::trust_boundary::open_regular_source(&path).map_err(local)?;
            std::io::copy(&mut input,&mut output).map_err(local)?;
        }
        output.as_file().sync_all().map_err(local)?;
        std::io::Seek::seek(output.as_file_mut(),std::io::SeekFrom::Start(0)).map_err(local)?;
        if let Some(reason)=owner.recheck(&protection,cancel).await? {return Err(super::leases::yield_error(reason));}
        Ok(output)
    }).await
}
pub(crate) async fn read_verified_remote_body(root:&Path,hash:&str,cancel:&Cancellation)->Result<Option<Vec<u8>>> {
    let temporary=tempfile::tempdir().map_err(local)?;
    let Some(source)=freeze_remote_body(root,hash)? else {return Ok(None)};
    let file=spool_frozen_remote_body(&source,temporary.path(),cancel).await?;
    read_frozen_body_spool(&source,&file,cancel).map(Some)
}
pub(crate) fn read_frozen_body_spool(source:&FrozenBodySource,file:&tempfile::NamedTempFile,cancel:&Cancellation)->Result<Vec<u8>> {
    use std::io::Read;
    cancel.check()?;
    let hash=match source {FrozenBodySource::Standalone(source)=>&source.hash,FrozenBodySource::Packed(source)=>&source.hash};
    risunest_sync_wire::validate_hash(hash).map_err(local)?;
    #[cfg(test)]
    let _scope=crate::asset_repository::body_io::object_scope(hash);
    let input=crate::trust_boundary::open_regular_source(file.path());
    #[cfg(test)]
    crate::asset_repository::body_io::open_result("managed",&input);
    let input=input.map_err(local)?;
    if input.metadata().map_err(local)?.len()!=source.byte_length() {return Err(segment::corrupt());}
    #[cfg(test)]
    let mut input=crate::asset_repository::body_io::TrackedBodyFile::new(input,hash);
    #[cfg(not(test))]
    let mut input=input;
    let mut bytes=Vec::new();
    bytes.try_reserve_exact(usize::try_from(source.byte_length()).map_err(local)?).map_err(local)?;
    let mut chunk=[0u8;64*1024];
    loop {
        cancel.check()?;
        let count=input.read(&mut chunk).map_err(local)?;
        if count==0 {break;}
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() as u64>source.byte_length() {return Err(segment::corrupt());}
    }
    if bytes.len() as u64!=source.byte_length() {return Err(segment::corrupt());}
    cancel.check()?;
    Ok(bytes)
}
async fn fetch_missing(root:&Path,hash:&str,cancel:&Cancellation)->Result<Option<Vec<u8>>> {read_verified_remote_body(root,hash,cancel).await}
#[cfg(test)]
mod frozen_spool_tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn frozen_spool_read_observes_actual_asset_open_and_bytes() {
        let bytes=b"synthetic captured remote body";
        let hash=risunest_sync_wire::hash(bytes);
        let source=FrozenBodySource::Standalone(Source {
            hash:hash.clone(),library_id:"synthetic-library".into(),connection_id:"synthetic-connection".into(),
            connection_root:PathBuf::new(),protected_segment:"synthetic-segment".into(),
            body:LargeBody {object_id:"synthetic-body".into(),sha256:hash.clone(),byte_length:(bytes.len() as u64).into(),plaintext_byte_length:(bytes.len() as u64).into(),locator:None},
        });
        let mut file=tempfile::NamedTempFile::new().unwrap();file.write_all(bytes).unwrap();
        crate::asset_repository::body_io::reset_body_io();
        crate::asset_repository::body_io::register_object_purpose(&hash,crate::asset_repository::body_io::BodyPurpose::Asset);
        assert_eq!(read_frozen_body_spool(&source,&file,&Cancellation::default()).unwrap(),bytes);
        let observed=crate::asset_repository::body_io::take_body_io();
        assert!(observed.complete());
        assert_eq!(observed.asset_work().opens,1);
        assert_eq!(observed.asset_work().read_bytes,bytes.len() as u64);
    }
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
    let (connection_root,connection_id,library_id)=if let Some(source)=source(root,hash)? {(source.connection_root,source.connection_id,source.library_id)} else if let Some(source)=packed_source(root,hash)? {(source.connection_root,source.connection_id,source.library_id)} else {return Ok(None)};
    open_source_connection(&connection_root,&connection_id,&library_id,cancel).await.map(Some)
}
async fn open_source_connection(connection_root:&Path,connection_id:&str,library_id:&str,cancel:&Cancellation)->Result<(std::sync::Arc<dyn Provider>,RepositoryHandle,zeroize::Zeroizing<[u8;32]>)> {
    let stored = ConnectionStore::open(connection_root)?.read(connection_id)?;
    if stored.descriptor.repository_id != library_id {
        return Err(segment::corrupt());
    }
    let dependencies =
        super::connection::dependencies_for_config(&connection_root, &stored.config)?;
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
        super::secrets::repository_key_vault(&connection_root).as_ref(),
        &stored.root_key_ref,
    )
    .await?;
    Ok((provider, repository, key))
}
