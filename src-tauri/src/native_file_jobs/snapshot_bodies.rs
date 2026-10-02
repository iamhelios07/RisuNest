use super::{JobControl, JobPhase, JobState, JobProgress, NativeJobError};
use crate::{asset_repository::{PayloadCas, job_pins::{CasJobKind,CasObjectRole,CasReleaseOutcome,DurableCasJob}}, persistent_store::PersistentStore, server_sync::residency::AssetPolicy};
use rusqlite::{Connection,OpenFlags};
use serde::Serialize;
use std::{fs::File,io::{self,Read},path::{Path,PathBuf}};

pub(crate) struct BodyObject {pub hash:String,pub size:u64,pub owner:bool,pub cached:bool}
pub(crate) struct BodyPlan {
    pub stage_id:String,pub revision:i64,pub authority:String,pub protection_id:String,
    pub objects:Vec<BodyObject>,pub source:PathBuf,pub policy:AssetPolicy,
}

#[derive(Clone,Debug,Serialize,PartialEq,Eq)]
#[serde(rename_all="camelCase")]
pub(crate) struct BodyResult {
    pub stage_id:String,pub activated_revision:i64,pub binding_authority:String,
    pub policy:AssetPolicy,pub total:u64,pub locally_present:u64,pub remote_held:u64,pub unavailable:u64,
    pub all_bodies_local:bool,pub settled:bool,
}

fn error(value:impl std::fmt::Display)->NativeJobError {NativeJobError::new("snapshot-bodies-failed",value.to_string())}
fn sync_error(value:crate::server_sync::SyncError)->NativeJobError {NativeJobError::new(&value.code,value.cause.unwrap_or_else(||"Snapshot body source is unavailable".to_owned()))}

fn check(store:&PersistentStore,plan:&BodyPlan,job:&JobControl)->Result<(),NativeJobError> {
    if job.is_cancel_requested() {return Err(NativeJobError::new("cancelled","Snapshot body transfer was cancelled"));}
    if store.lww_binding_authority().map_err(error)?.0.to_string()!=plan.authority
        || store.server_asset_policy().map_err(sync_error)?!=plan.policy {
        return Err(NativeJobError::new("snapshot-body-authority-changed","Snapshot body transfer authority or policy changed"));
    }
    Ok(())
}

fn observe(store:&PersistentStore,plan:&BodyPlan)->Result<BodyResult,NativeJobError> {
    let cas=PayloadCas::new(store.repository_root()).map_err(error)?;
    let residency=crate::server_sync::residency::Residency::open(store.repository_root()).map_err(sync_error)?;
    let mut result=BodyResult {stage_id:plan.stage_id.clone(),activated_revision:plan.revision,binding_authority:plan.authority.clone(),policy:plan.policy,total:plan.objects.len() as u64,locally_present:0,remote_held:0,unavailable:0,all_bodies_local:false,settled:false};
    for object in &plan.objects {
        match cas.stat_object(&object.hash).map_err(error)? {
            Some(size) if size==object.size=>result.locally_present+=1,
            Some(_)=>return Err(NativeJobError::new("snapshot-body-size-differs","Snapshot body size differs from the frozen inventory")),
            None=>{
                let held=residency.object(&object.hash,None).map_err(sync_error)?.is_some_and(|proof|proof.size==object.size)
                    || crate::external_storage::lww_residency::stat(store.repository_root(),&object.hash).map_err(error)?==Some(object.size);
                if held {result.remote_held+=1;} else {result.unavailable+=1;}
            }
        }
    }
    result.all_bodies_local=result.locally_present==result.total;
    result.settled=result.all_bodies_local || (plan.policy==AssetPolicy::Remote && result.unavailable==0);
    Ok(result)
}

struct CheckedRead<'a,R:Read> {inner:R,store:&'a PersistentStore,plan:&'a BodyPlan,job:&'a JobControl}
impl<R:Read> Read for CheckedRead<'_,R> {
    fn read(&mut self,output:&mut[u8])->io::Result<usize> {
        check(self.store,self.plan,self.job).map_err(|error|io::Error::other(error.message))?;
        let read=self.inner.read(output)?;
        if read>0 {
            let mut progress=self.job.status.lock().map_err(|_|io::Error::other("Snapshot body progress mutex is poisoned"))?.progress;
            progress.completed_bytes=progress.completed_bytes.checked_add(read as u64).ok_or_else(||io::Error::other("Snapshot body read count overflow"))?;
            self.job.set_progress(progress).map_err(io::Error::other)?;
        }
        Ok(read)
    }
}

pub(crate) fn run(store:&mut PersistentStore,plan:&BodyPlan,job:&JobControl,scratch:&Path)->Result<BodyResult,NativeJobError> {
    job.start(JobPhase::CopyingMissingBodies).map_err(error)?;
    let initial=observe(store,plan)?;
    job.set_snapshot_bodies(initial.clone()).map_err(error)?;
    check(store,plan,job)?;
    let cas=PayloadCas::new(store.repository_root()).map_err(error)?;
    let mut missing=Vec::new();
    for object in &plan.objects {
        if !store.portable_object_present(&object.hash,object.size).map_err(error)? {missing.push(object);}
    }
    let missing_bytes=missing.iter().try_fold(0u64,|total,object|total.checked_add(object.size)).ok_or_else(||error("Snapshot missing body size overflow"))?;
    job.set_progress(JobProgress {total_bytes:Some(missing_bytes),total_items:Some(missing.len() as u64),..Default::default()}).map_err(error)?;
    let mut pins=match DurableCasJob::open(store.repository_root(),&plan.protection_id) {
        Ok(pins)=>pins,
        Err(failure) if failure.kind()==io::ErrorKind::NotFound=>DurableCasJob::begin(store.repository_root(),&plan.protection_id,CasJobKind::LocalBackupRestore,super::portable::now()).map_err(error)?,
        Err(failure)=>return Err(error(failure)),
    };
    let outcome=(|| {
        if initial.settled {
            pins.seal(store,super::portable::now()).map_err(error)?;
            pins.release(CasReleaseOutcome::Committed).map_err(error)?;
            store.snapshot_restore_bodies_completed(&plan.stage_id).map_err(error)?;
            return Ok(initial);
        }
        for object in &missing {
            check(store,plan,job)?;
            let role=if object.owner {CasObjectRole::OwnerManifest} else {CasObjectRole::DirectObject};
            if store.portable_object_present(&object.hash,object.size).map_err(error)? {
                pins.pin_existing(&cas,&object.hash,object.size,role).map_err(error)?;
                continue;
            }
            if plan.policy==AssetPolicy::Remote {continue;}
            if object.cached {
                let file=File::open(&plan.source).map_err(error)?;
                let identity=crate::asset_repository::exact_file_identity(&file).map_err(error)?;
                let source=Connection::open_with_flags(&plan.source,OpenFlags::SQLITE_OPEN_READ_ONLY|OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(error)?;
                source.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;").map_err(error)?;
                let (row,size):(i64,i64)=source.query_row("SELECT rowid,length(body) FROM message_page_objects WHERE hash=?1",[&object.hash],|row|Ok((row.get(0)?,row.get(1)?))).map_err(error)?;
                if u64::try_from(size).ok()!=Some(object.size) {return Err(error("Snapshot cached payload size differs"));}
                let blob=source.blob_open("main","message_page_objects","body",row,true).map_err(error)?;
                let mut input=CheckedRead {inner:blob,store:&*store,plan,job};
                pins.prepare_reader_expected(&cas,&mut input,&object.hash,object.size,role).map_err(error)?;
                if crate::asset_repository::exact_file_identity(&file).map_err(error)?!=identity {return Err(error("Snapshot cached source identity changed"));}
            } else {
                let check_source=||check(store,plan,job).map_err(|error|crate::server_sync::SyncError::new(error.code,409));
                if let Some(body)=crate::server_sync::residency::open_transient_server_with_check(store.repository_root(),scratch,&object.hash,&check_source).map_err(sync_error)? {
                    let mut input=CheckedRead {inner:body,store:&*store,plan,job};
                    pins.prepare_reader_expected(&cas,&mut input,&object.hash,object.size,role).map_err(error)?;
                } else {
                    let cancellation=crate::external_storage::contract::Cancellation::with_external_flag(job.cancellation_flag());
                    let body=tauri::async_runtime::block_on(crate::external_storage::lww_residency::spool_verified_remote_body(store.repository_root(),&object.hash,scratch,&cancellation)).map_err(error)?;
                    if let Some(mut body)=body {
                        let mut input=CheckedRead {inner:body.as_file_mut(),store:&*store,plan,job};
                        pins.prepare_reader_expected(&cas,&mut input,&object.hash,object.size,role).map_err(error)?;
                    }
                }
            }
            let mut progress=job.status.lock().map_err(|_|error("Snapshot body progress mutex is poisoned"))?.progress;
            progress.completed_items+=1;
            job.set_progress(progress).map_err(error)?;
        }
        check(store,plan,job)?;
        let result=observe(store,plan)?;
        job.set_snapshot_bodies(result.clone()).map_err(error)?;
        if !result.settled {return Err(NativeJobError::new("snapshot-bodies-incomplete","Snapshot library is restored, but some bodies remain unavailable"));}
        pins.seal(store,super::portable::now()).map_err(error)?;
        pins.release(CasReleaseOutcome::Committed).map_err(error)?;
        store.snapshot_restore_bodies_completed(&plan.stage_id).map_err(error)?;
        Ok(result)
    })();
    if outcome.is_err() {if let Ok(result)=observe(store,plan) {let _=job.set_snapshot_bodies(result);}}
    outcome
}

pub(crate) fn finish(job:&JobControl,outcome:Result<BodyResult,NativeJobError>)->Result<(),String> {
    match outcome {
        Ok(result)=>{
            job.set_snapshot_bodies(result)?;
            let mut status=job.status.lock().map_err(|_|"native job mutex poisoned".to_owned())?;
            status.state=JobState::Succeeded;
            status.phase=JobPhase::Complete;
            drop(status);
            job.mark_terminal()
        }
        Err(_) if job.is_cancel_requested()=>job.finish_cancelled(),
        Err(failure)=>job.finish_failure(&failure.code,&failure.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_sync::lww_tests::{local,put_asset};

    fn activated(store:&mut PersistentStore)->BodyPlan {
        let snapshot=store.snapshot_create("synthetic-body-phase").unwrap();
        let stage=store.snapshot_restore_stage(&snapshot.id,&uuid::Uuid::new_v4().to_string()).unwrap();
        let authority=store.lww_binding_authority().unwrap();
        let receipt=store.snapshot_restore_activate(&stage.staging_id,store.revision().unwrap(),authority.clone()).unwrap();
        store.snapshot_restore_body_plan(&stage.staging_id,receipt.revision,&authority.0.to_string()).unwrap()
    }

    fn job(plan:&BodyPlan)->std::sync::Arc<JobControl> {
        let job=super::super::JobRegistry::default().create_internal(super::super::JobKind::SnapshotBodies,Some(plan.revision),vec![],false).unwrap();
        {
            let mut status=job.status.lock().unwrap();
            status.snapshot_staging_id=Some(plan.stage_id.clone());
            status.activation_revision=Some(plan.revision);
            status.activation_authority=Some(plan.authority.clone());
        }
        job
    }

    #[test]
    fn snapshot_bodies_all_present_has_zero_asset_work_and_keeps_later_edit_receipt() {
        let (root,mut store)=local();
        put_asset(&mut store,"assets/synthetic.png",b"synthetic snapshot body");
        let plan=activated(&mut store);
        put_asset(&mut store,"assets/later.png",b"later foreground body");
        let foreground_revision=store.revision().unwrap();
        let retried=store.snapshot_restore_body_plan(&plan.stage_id,plan.revision,&plan.authority).unwrap();
        assert_eq!(retried.protection_id,plan.protection_id);
        crate::asset_repository::body_io::reset_body_io();
        let result=run(&mut store,&plan,&job(&plan),root.path()).unwrap();
        let io=crate::asset_repository::body_io::take_body_io();
        let work=io.asset_work();
        assert_eq!(work.opens,0);
        assert_eq!(work.read_bytes,0);
        assert_eq!(work.staging_written_bytes,0);
        assert_eq!(work.publication_attempts,0);
        assert!(result.all_bodies_local && result.settled);
        assert_eq!(result.locally_present,1);
        assert_eq!(store.revision().unwrap(),foreground_revision);
        assert!(!plan.source.exists());
    }

    #[test]
    fn snapshot_bodies_unavailable_and_cancel_keep_committed_revision_and_body_only_retry() {
        let (root,mut store)=local();
        let alias=put_asset(&mut store,"assets/synthetic.png",b"synthetic snapshot body");
        let plan=activated(&mut store);
        let cas=PayloadCas::new(store.repository_root()).unwrap();
        std::fs::remove_file(cas.object_path(alias.object_hash.as_ref().unwrap()).unwrap().unwrap()).unwrap();
        let failed=job(&plan);
        assert!(run(&mut store,&plan,&failed,root.path()).is_err());
        let status=failed.status.lock().unwrap();
        assert_eq!(status.snapshot_bodies.as_ref().unwrap().unavailable,1);
        drop(status);
        assert!(plan.source.exists());
        assert_eq!(store.revision().unwrap(),plan.revision);
        let retry=store.snapshot_restore_body_plan(&plan.stage_id,plan.revision,&plan.authority).unwrap();
        assert_eq!(retry.protection_id,plan.protection_id);
        let cancelled=job(&retry);
        cancelled.cancel_requested.store(true,std::sync::atomic::Ordering::Release);
        assert_eq!(run(&mut store,&retry,&cancelled,root.path()).unwrap_err().code,"cancelled");
        assert_eq!(store.revision().unwrap(),plan.revision);
        assert!(plan.source.exists());
        cas.prepare_bytes(b"synthetic snapshot body").unwrap();
        assert!(run(&mut store,&retry,&job(&retry),root.path()).unwrap().all_bodies_local);
        assert_eq!(store.revision().unwrap(),plan.revision);
    }
}
