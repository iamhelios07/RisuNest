use super::{contract::*, lww_checkpoint::{self, Checkpoint, PublishedCatalog}, lww_engine::{ExternalLwwEngine, read_bytes}, lww_segment as segment, packaging::RemoteObject};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use risunest_external_storage_format::{crypto::derive_key, snapshot as wire};
use crate::persistent_store::{content_capture::ContentCaptureSink, external_capture::CapturedSnapshot, sync_selection::CaptureIdentity};
use std::{collections::{BTreeMap,BTreeSet}, io::Cursor, path::Path};

#[cfg(test)]
pub(crate) struct CompactionBarrier { pub reached:tokio::sync::Notify, pub resume:tokio::sync::Notify }
#[cfg(test)]
impl CompactionBarrier {pub(crate) fn new()->std::sync::Arc<Self> {std::sync::Arc::new(Self{reached:tokio::sync::Notify::new(),resume:tokio::sync::Notify::new()})}}
#[cfg(test)]
static BARRIERS:std::sync::LazyLock<std::sync::Mutex<BTreeMap<String,std::sync::Arc<CompactionBarrier>>>>=std::sync::LazyLock::new(||std::sync::Mutex::new(BTreeMap::new()));
#[cfg(test)]
pub(crate) fn install_compaction_barrier(job_id:&str,barrier:std::sync::Arc<CompactionBarrier>) {assert!(BARRIERS.lock().unwrap().insert(job_id.into(),barrier).is_none());}
#[cfg(test)]
async fn pause_compaction(job_id:&str) {
    let barrier=BARRIERS.lock().unwrap().remove(job_id);
    if let Some(barrier)=barrier {barrier.reached.notify_one();barrier.resume.notified().await;}
}
pub(crate) struct PublishedState {
    pub catalog: PublishedCatalog,
    pub controls: BTreeMap<String,Vec<u8>>,
    pub bodies: BTreeMap<String,Vec<u8>>,
    pub standalone: BTreeMap<String,segment::LargeBody>,
    pub standalone_roots:BTreeMap<String,String>,
    pub asset_catalogs: Vec<wire::StoredObject>,
    pub snapshots: Vec<(RemoteObject,Checkpoint)>,
    pub segments: Vec<(ObjectReceipt,segment::Segment)>,
}
impl PublishedState {
    pub(crate) fn require_complete(&self)->Result<()> {
        if self.segments.iter().any(|(_,segment)|self.catalog.coverage.get(&segment.writer_id).copied().unwrap_or(risunest_sync_wire::stamp::DecimalU64(0))<segment.seq) {
            return Err(ProviderError::new(ErrorKind::PreconditionFailed));
        }
        Ok(())
    }
}fn error(_: impl std::fmt::Display) -> ProviderError { segment::corrupt() }
impl ExternalLwwEngine {
    pub(crate) async fn stage_published_objects(&self,store:&mut crate::persistent_store::PersistentStore,state:&mut PublishedState,directory:&Path,cancel:&Cancellation)->Result<()> {
        let root=store.external_lww_root();
        let cas=crate::asset_repository::PayloadCas::new(&root).map_err(error)?;
        for (hash,bytes) in &state.controls { store.lww_put_object(hash,bytes).map_err(error)?; }
        for (hash,bytes) in &state.bodies { if cas.stat_object(hash).map_err(error)?.is_none() {store.lww_put_managed_object(hash,bytes).map_err(error)?;} }
        for (hash,body) in &state.standalone {
            if cas.stat_object(hash).map_err(error)?.is_none() {
                super::lww_residency::register(&root,&super::lww_residency::Source {hash:hash.clone(),library_id:self.library.clone(),connection_id:self.connection_id.clone(),connection_root:self.connection_root.clone(),protected_segment:state.standalone_roots.get(hash).cloned().ok_or_else(segment::corrupt)?,body:body.clone()})?;
            }
        }
        let mut seen=BTreeSet::new();
        let retained=lww_checkpoint::retained(&state.snapshots.iter().map(|(_,snapshot)|snapshot.clone()).collect::<Vec<_>>())?;
        for (snapshot,document) in &state.snapshots {
            if !retained.contains(&document.snapshot_id) {continue;}
            let mut catalogs=document.asset_catalogs.clone(); catalogs.push(document.library.asset_catalog.clone());
            for catalog in catalogs {
                if !seen.insert(catalog.header.object_id.clone()) {continue;}
                let object=RemoteObject::from_stored(&catalog,&self.repository)?;
                let (entries,packs,_)=super::snapshot_restore::read_catalog(&object,wire::CatalogKind::Assets,&self.root_key,directory,self.provider.as_ref(),&self.repository,cancel).await?;
                for entry in entries {
                    let hash=hex::encode(entry.content_sha256);
                    if cas.stat_object(&hash).map_err(error)?.is_some() {continue;}
                    let selected=entry.chunks.iter().map(|c|c.pack_id.as_str()).collect::<BTreeSet<_>>();
                    let packs=packs.iter().filter(|(id,_)|selected.contains(id.as_str())).map(|(_,pack)|pack.stored(&self.repository)).collect::<Result<Vec<_>>>()?;
                    super::lww_residency::register_packed(&root,&super::lww_residency::PackedSource {hash,byte_length:entry.byte_length,library_id:self.library.clone(),connection_id:self.connection_id.clone(),connection_root:self.connection_root.clone(),protected_snapshot:snapshot.object_id.clone(),catalog:catalog.clone(),chunks:entry.chunks,packs},&self.repository)?;
                }
            }
        }
        Ok(())
    }
    pub(crate) async fn snapshot_listing(&self,cancel:&Cancellation) -> Result<Vec<ObjectReceipt>> {
        let mut output=Vec::new(); let mut cursor=None; let mut visited=BTreeSet::new();
        loop {
            let page=self.provider.list_objects(&self.repository,Collection::Snapshots,cursor.as_deref(),100,cancel).await?;
            for receipt in page.objects {if self.classified_checkpoint(&receipt,cancel).await?.is_some() {output.push(receipt);}}
            match page.next_cursor { Some(next) => { if !visited.insert(next.clone()) { return Err(segment::corrupt()); } cursor=Some(next); }, None=>break }
        }
        Ok(output)
    }
    pub(crate) async fn checkpoint(&self,receipt:&ObjectReceipt,cancel:&Cancellation)->Result<(RemoteObject,Checkpoint)> {self.classified_checkpoint(receipt,cancel).await?.ok_or_else(segment::corrupt)}
    async fn classified_checkpoint(&self, receipt:&ObjectReceipt,cancel:&Cancellation) -> Result<Option<(RemoteObject,Checkpoint)>> {
        receipt.locator.validate_for(&self.repository)?;
        let bytes=read_bytes(self.provider.as_ref(),&self.repository,&receipt.locator,cancel).await?;
        let key=derive_key(&self.root_key,&self.repository.repository_id,"metadata").map_err(error)?;
        let mut plaintext=Vec::new();
        let header=wire::open_envelope(&mut Cursor::new(&bytes),&mut plaintext,&key,wire::MAX_METADATA_BYTES as u64).map_err(error)?;
        if header.repository_id!=self.repository.repository_id || receipt.byte_length!=bytes.len() as u64 || !receipt.complete {return Err(segment::corrupt());}
        if header.object_id.starts_with("snapshot-") && matches!(header.role,wire::ObjectRole::SyncState|wire::ObjectRole::BackupBundle) {
            let backup=super::control::SnapshotView::read(&plaintext,header.role,&self.repository.repository_id)?;
            if header.object_id!=format!("snapshot-{}",backup.snapshot_id) {return Err(segment::corrupt());}
            return Ok(None);
        }
        let document=Checkpoint::decode(&plaintext)?;
        for body in document.standalone_bodies.values() {body.locator.as_ref().ok_or_else(segment::corrupt)?.validate_for(&self.repository)?;}
        if header.repository_id!=self.repository.repository_id || header.role!=wire::ObjectRole::SyncState
            || header.object_id!=document.snapshot_id || document.repository_id!=self.repository.repository_id || document.library_id!=self.library
            || receipt.byte_length!=bytes.len() as u64 || !receipt.complete { return Err(segment::corrupt()); }
        Ok(Some((RemoteObject { repository_id:header.repository_id,object_id:header.object_id,role:ObjectRole::Snapshot,receipt:receipt.clone(),ciphertext_sha256:segment::digest(&bytes),plaintext_length:plaintext.len() as u64,plaintext_sha256:segment::digest(&plaintext) },document)))
    }
    pub(crate) async fn published_state(&self,directory:&Path,cancel:&Cancellation) -> Result<PublishedState> {
        std::fs::create_dir_all(directory).map_err(error)?;
        let mut catalog=PublishedCatalog::create(&directory.join("published.sqlite"))?;
        let mut snapshots=Vec::new();
        for receipt in self.snapshot_listing(cancel).await? { snapshots.push(self.checkpoint(&receipt,cancel).await?); }
        let retained=lww_checkpoint::retained(&snapshots.iter().map(|(_,s)|s.clone()).collect::<Vec<_>>())?;
        let mut controls=BTreeMap::new(); let mut bodies=BTreeMap::new(); let mut standalone=BTreeMap::new(); let mut standalone_roots=BTreeMap::new(); let mut asset_catalogs=Vec::new();
        for (root,snapshot) in &snapshots {
            if !retained.contains(&snapshot.snapshot_id) { continue; }
            let data=RemoteObject::from_stored(&snapshot.library.record_catalog,&self.repository)?;
            let stage=directory.join(&snapshot.snapshot_id);
            let (records,objects)=super::snapshot_restore::download_checkpoint_data(&data,&stage,&self.root_key,self.provider.as_ref(),&self.repository,cancel).await?;
            let mut proof=PublishedCatalog::create(&stage.join("proof.sqlite"))?;
            let content=super::content_store::ContentStore::open(&stage.join("external-storage")).map_err(error)?;
            for record in records {
                let body=content.read_all(&record.content_hash).map_err(error)?;
                let change:crate::persistent_store::lww::Change=risunest_sync_wire::canonical::decode(&body,body.len()).map_err(error)?;
                if record.key!=change.key.as_str() { return Err(segment::corrupt()); }
                proof.merge(&change)?; catalog.merge(&change)?;
            }
            if proof.identity()?!=snapshot.state_identity { return Err(segment::corrupt()); }
            for object in objects {
                let bytes=match object.source { super::content_store::ObjectSource::Captured(hash)=>content.read_all(&hash).map_err(error)?, super::content_store::ObjectSource::File(path)=>std::fs::read(path).map_err(error)?, _=>return Err(segment::corrupt()) };
                if segment::digest(&bytes)!=object.content_hash { return Err(segment::corrupt()); }
                controls.insert(object.content_hash,bytes);
            }
            catalog.include_prefixes(&snapshot.covered_prefixes);
            asset_catalogs.push(snapshot.library.asset_catalog.clone());
            asset_catalogs.extend(snapshot.asset_catalogs.clone());
            for (hash,source) in &snapshot.standalone_bodies { standalone.insert(hash.clone(),source.clone()); standalone_roots.insert(hash.clone(),root.object_id.clone()); }
        }
        let mut groups:BTreeMap<(String,u64),Vec<ObjectReceipt>>=BTreeMap::new();
        for receipt in self.listing(cancel).await? {
            let name=receipt.locator.object.rsplit('/').next().ok_or_else(segment::corrupt)?;
            let (writer,seq,_)=parse_segment_object_id(name)?;
            groups.entry((writer.into(),seq)).or_default().push(receipt);
        }
        let mut segments=Vec::new();
        let mut history=PublishedCatalog::create(&directory.join("history.sqlite"))?;
        for change in catalog.changes()? {history.merge(&change)?;}
        let upper=self.admitted_upper()?.checked_add(300_000).ok_or_else(segment::corrupt)?;
        for ((writer,seq),variants) in groups {
            let mut identity=None; let mut winner=None;
            for receipt in variants {
                let name=receipt.locator.object.rsplit('/').next().ok_or_else(segment::corrupt)?;
                let (_,_,hash)=parse_segment_object_id(name)?;
                if identity.as_deref().is_some_and(|old|old!=hash) { return Err(segment::corrupt()); }
                let bytes=read_bytes(self.provider.as_ref(),&self.repository,&receipt.locator,cancel).await?;
                if segment::digest(&bytes)!=hash { return Err(segment::corrupt()); }
                let payload=segment::open(&bytes,&self.library,&writer,seq,&self.root_key)?;
                for change in &payload.changes { if change.stamp.physical_ms.0>upper { return Err(ProviderError::new(ErrorKind::ClockSkew)); } history.merge(change)?; }
                identity=Some(hash.to_owned()); winner=Some((receipt,payload));
            }
            if let Some((receipt,payload))=winner {
                if catalog.include_segment(&payload)? {
                    for (hash,bytes) in &payload.message_pages { controls.insert(hash.clone(),URL_SAFE_NO_PAD.decode(bytes).map_err(error)?); }
                    for (hash,bytes) in &payload.small_bodies {
                        let bytes=URL_SAFE_NO_PAD.decode(bytes).map_err(error)?;
                        if segment::digest(&bytes)!=*hash { return Err(segment::corrupt()); }
                        bodies.insert(hash.clone(),bytes);
                    }
                    for (hash,source) in &payload.large_bodies { standalone.insert(hash.clone(),source.clone()); standalone_roots.insert(hash.clone(),super::contract::segment_object_id(&payload.writer_id,payload.seq.0,&identity.clone().ok_or_else(segment::corrupt)?)?); }
                }
                segments.push((receipt,payload));
            }
        }
        Ok(PublishedState {catalog,controls,bodies,standalone,standalone_roots,asset_catalogs,snapshots,segments})
    }
    pub(crate) async fn compact_published(&self,directory:&Path,job_id:&str,writer:&str,capabilities:&super::capabilities::Capabilities,cancel:&Cancellation,protection:Option<(&super::leases::LeaseOwner,&super::leases::LeaseContext<'_>)>) -> Result<super::packaging::CompletedSnapshot> {
        let state=self.published_state(&directory.join("inputs"),cancel).await?;
        if let Some((owner,context))=protection {if let Some(reason)=owner.recheck(context,cancel).await? {return Err(super::leases::yield_error(reason));}}
        #[cfg(test)] pause_compaction(job_id).await;
        cancel.check()?;
        let identity=CaptureIdentity {store_id:writer.into(),library_epoch:self.library.clone(),generation:job_id.into(),selection_epoch:job_id.into(),revision:0};
        let external=directory.join("external-storage");
        let mut capture=super::capture::CaptureCatalog::create(&directory.join("capture"),&external,None).map_err(error)?;
        capture.begin(&identity,None).map_err(error)?;
        let changes=state.catalog.changes()?;
        for change in &changes {
            let bytes=risunest_sync_wire::canonical::encode(&serde_json::to_value(change).map_err(error)?).map_err(error)?;
            capture.record(change.key.as_str(),&bytes).map_err(error)?;
        }
        if let Some(first)=changes.first() {
            for (hash,bytes) in state.controls { capture.object(&hash,&bytes).map_err(error)?; capture.reference(first.key.as_str(),&hash,bytes.len() as u64).map_err(error)?; }
            let cas=crate::asset_repository::PayloadCas::new(directory).map_err(error)?;
            for (hash,bytes) in state.bodies {
                let prepared=cas.prepare_bytes(&bytes).map_err(error)?;
                if prepared.content_hash!=hash { return Err(segment::corrupt()); }
                capture.reference(first.key.as_str(),&hash,bytes.len() as u64).map_err(error)?;
            }
        }
        capture.finish().map_err(error)?;
        let fingerprint=capture.content_fingerprint(&risunest_external_storage_format::format::library_fingerprint_domain()).map_err(error)?;
        let metadata=super::packaging::SnapshotMetadata { snapshot_id:job_id.into(),repository_id:self.repository.repository_id.clone(),library_id:self.library.clone(),author_device_id:writer.into(),created_at_ms:self.admitted_upper()?,logical_revision:0,parent_snapshot_id:None,content_fingerprint:fingerprint,
            purpose:super::packaging::SnapshotPurpose::LwwCheckpoint { covered_prefixes:state.catalog.coverage.clone(),state_identity:state.catalog.identity()?,asset_catalogs:state.asset_catalogs,standalone_bodies:state.standalone } };
        let captured=CapturedSnapshot{id:job_id.into(),identity:identity.clone(),catalog:capture,projected_records:changes.len(),shared:true};
        let mut journal=super::journal::TransferJournal::open(&directory.join("journal"),super::journal::JobIdentity{job_id:job_id.into(),connection_id:self.connection_id.clone(),repository_id:self.repository.repository_id.clone(),capture_id:job_id.into(),capture:identity}).map_err(error)?;
        let mut limits=super::packaging::PackageLimits::from_capabilities(capabilities)?;
        limits.target_plaintext_bytes=32*1024*1024;
        super::packaging::package_and_upload_protected(captured,Vec::new(),directory,&directory.join("cache"),metadata,&self.root_key,limits,None,&mut journal,self.provider.as_ref(),&self.repository,&super::phase_progress::PhaseProgress::silent(),cancel,protection).await
    }
}
