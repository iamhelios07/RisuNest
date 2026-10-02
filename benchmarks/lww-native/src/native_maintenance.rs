use super::*;

#[derive(Serialize)]
pub(crate) struct RawCompaction {
    pub status:&'static str,
    pub fixture_for_scenario:measurement::Scenario,
    pub iteration:u32,
    pub warmup:bool,
    pub native_publication_complete_ms:Option<f64>,
    pub snapshot:Option<Value>,
    pub protection_run_completed:bool,
    pub native_observation:NativeObservation,
    pub source_observation:Option<Value>,
    pub error:Option<String>,
}

/// Snapshot creation is excluded setup for listing/restore cases, and background work for overlap.
pub(crate) async fn compact_native_raw(
    engine:&crate::external_storage::lww_engine::ExternalLwwEngine,
    provider:&Arc<crate::external_storage::fake::FakeProvider>,
    directory:&Path,job_id:&str,writer:&str,
    capabilities:&crate::external_storage::capabilities::Capabilities,
    protection:Option<(&crate::external_storage::leases::LeaseOwner,&crate::external_storage::leases::LeaseContext<'_>)>,
    asset_hashes:&[String],scenario:measurement::Scenario,iteration:u32,
) ->Result<RawCompaction,String> {
    if !matches!(scenario,measurement::Scenario::ConsolidatedSnapshot|measurement::Scenario::IncomparableSnapshots
        |measurement::Scenario::DuringCompaction) || iteration>=6 {
        return Err("compaction slot is outside the frozen one-plus-five policy".into());
    }
    if Arc::as_ptr(&engine.provider) as *const ()!=Arc::as_ptr(provider) as *const () {
        return Err("compaction IO observer belongs to a different provider".into());
    }
    let requests_before=provider.read_count()+provider.upload_count()+provider.listing_count();
    let bytes_before=provider.transferred_body_bytes();
    reset_work(asset_hashes);
    crate::external_storage::worker_observation::begin();
    let started=Instant::now();
    let cancel=Cancellation::default();
    let outcome=if let Some((owner,context))=protection {
        owner.run(context,&cancel,async {
            if let Some(reason)=owner.recheck(context,&cancel).await? {
                return Err(crate::external_storage::leases::yield_error(reason));
            }
            engine.compact_published(directory,job_id,writer,capabilities,&cancel,Some((owner,context))).await
        }).await
    } else {
        engine.compact_published(directory,job_id,writer,capabilities,&cancel,None).await
    };
    let elapsed_ms=started.elapsed().as_secs_f64()*1000.0;
    let mut observation=take_work(true);
    for worker in crate::external_storage::worker_observation::take() {
        let mut incomplete=worker.hashes.incomplete.into_iter().map(|(domain,count)|format!("{domain}:{count}")).collect::<Vec<_>>();
        if !worker.completed {incomplete.push("actual external worker did not complete".into());}
        let receipt=native_observation::NativeWorkerHashReceipt {thread:worker.thread,
            domains:worker.hashes.domains.into_iter().map(|(name,work)|(name.into(),native_observation::HashDomain {
                calls:work.calls,bytes:work.bytes})).collect(),incomplete};
        if let Err(reason)=observation.attach_worker_hash_receipt(receipt) {observation.incomplete.push(reason);}
    }
    let bytes_after=provider.transferred_body_bytes();
    observation.requests=(provider.read_count()+provider.upload_count()+provider.listing_count()-requests_before) as u64;
    observation.uploaded_bytes=bytes_after.0-bytes_before.0;
    observation.downloaded_bytes=bytes_after.1-bytes_before.1;
    observation.incomplete.push("external compaction format/pack/source SHA coverage is not yet frozen".into());
    let (snapshot,error)=match outcome {
        Ok(completed)=>(Some(json!({"snapshotId":completed.snapshot_id,"repositoryId":completed.repository_id,
            "stateFingerprint":completed.fingerprint,"libraryFingerprint":completed.library_fingerprint,
            "logicalRevision":completed.logical_revision,"referencedObjectCount":completed.referenced_objects.len(),
            "sectionCount":completed.sections.len()})),None),
        Err(error)=>(None,Some(format!("{error:?}"))),
    };
    Ok(RawCompaction {status:"INVALID",fixture_for_scenario:scenario,iteration,warmup:iteration==0,
        native_publication_complete_ms:snapshot.as_ref().map(|_|elapsed_ms),
        protection_run_completed:protection.is_some() && snapshot.is_some(),snapshot,native_observation:observation,
        source_observation:None,error})
}
