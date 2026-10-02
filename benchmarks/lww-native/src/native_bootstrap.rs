use super::*;

#[derive(Serialize)]
pub(crate) struct RawServerBootstrap {
    pub status:&'static str,
    pub scenario:measurement::Scenario,
    pub direction:final_runner::Direction,
    pub iteration:u32,
    pub warmup:bool,
    pub policy:BootstrapPolicy,
    pub writer_before:String,
    pub writer_after:Option<String>,
    pub usable_database_ms:Option<f64>,
    pub all_bodies_local_ms:Option<f64>,
    pub activation_revision:Option<i64>,
    pub final_revision:Option<i64>,
    pub activation_request:Option<Value>,
    pub body_barrier_authority:Option<String>,
    pub native_observation:NativeObservation,
    pub source_end_and_shutdown:Option<Value>,
    pub asset_sha_and_publication_ledger:Option<Value>,
    pub missing_observers:Vec<&'static str>,
    pub error:Option<String>,
}
#[derive(Serialize)]
pub(crate) struct BootstrapPolicy {
    pub warmups:u32,
    pub recorded_repetitions:u32,
    pub percentile_resolution:&'static str,
    pub timing_scope:&'static str,
}

/// Candidate configuration and verified producer inventories are excluded setup.
/// The caller must supply a fresh unbound destination with its independent writer.
pub(crate) fn server_bootstrap_native_raw(destination:&mut PersistentStore,
    io:Arc<server_sync::client::TestIoCounters>,asset_hashes:&[String],selected_character_id:Option<&str>,
    direction:final_runner::Direction,iteration:u32)->Result<RawServerBootstrap,String> {
    if iteration>=6 {return Err("bootstrap slot exceeds frozen one-plus-five policy".into());}
    if destination.lww_binding_state().map_err(|e|e.to_string())?.target
        !=crate::persistent_store::sync_selection::SyncTarget::None {
        return Err("bootstrap destination is already bound".into());
    }
    let writer_before=destination.lww_clock_state().map_err(|e|e.to_string())?.writer_id;
    io.reset();reset_work(asset_hashes);
    let started=Instant::now();
    let mut usable_database_ms=None;
    let mut activation_revision=None;
    let mut body_barrier_authority=None;
    let mut all_bodies_local_ms=None;
    let mut completion=None;
    let outcome=(||->Result<(),String> {
        let bound=server_sync::first_binding_cycle(destination,io.clone(),|receipt| {
            usable_database_ms=Some(started.elapsed().as_secs_f64()*1000.0);
            activation_revision=Some(receipt.revision);
        }).map_err(|e|format!("{e:?}"))?;
        body_barrier_authority=Some(bound.state.target_authority.0.to_string());
        completion=Some(bound);
        server_sync::hydrate_binding_bodies(destination,io.clone(),selected_character_id,||{})
            .map_err(|e|format!("{e:?}"))?;
        all_bodies_local_ms=Some(started.elapsed().as_secs_f64()*1000.0);
        Ok(())
    })();
    let mut observation=take_work(false);
    let [requests,sent,received]=io.snapshot();observation.requests=requests;
    observation.uploaded_bytes=sent;observation.downloaded_bytes=received;
    let mut record=RawServerBootstrap {status:"INVALID",scenario:measurement::Scenario::FullBootstrap,
        direction,iteration,warmup:iteration==0,policy:BootstrapPolicy {warmups:1,recorded_repetitions:5,
            percentile_resolution:"N=5 nearest-rank p95 and p99 share the largest sample; population tail is unresolved",
            timing_scope:"native durable replacement callback and awaited Full body barrier; renderer adoption/paint unobserved"},
        writer_before,writer_after:None,usable_database_ms,all_bodies_local_ms,activation_revision,
        final_revision:None,activation_request:None,body_barrier_authority,native_observation:observation,
        source_end_and_shutdown:None,asset_sha_and_publication_ledger:None,
        missing_observers:vec!["verified producer source roles and complete source End plus final Shutdown"],error:outcome.err()};
    if let Err(reason)=record.native_observation.validate_costly_native_coverage() {
        record.missing_observers.push("complete caller and worker native SHA plus body publication ledger");
        record.error=Some(record.error.take().map_or(reason.clone(),|prior|format!("{prior}; {reason}")));
    } else {
        record.asset_sha_and_publication_ledger=Some(json!({
            "domains":record.native_observation.body_domains,"objects":record.native_observation.body_objects,
            "assetWork":record.native_observation.body_asset_work,"controlWork":record.native_observation.body_control_work,
            "nativeShaSubsetSemantics":"per-object body_sha is a subset of native caller/worker SHA totals, never added twice",
            "publicationByteSemantics":"successful link/rename identity size, not copied bytes",
        }));
    }
    let checks=verify_post_scope(&record.native_observation,||->Result<(),String> {
        let writer=destination.lww_clock_state().map_err(|e|e.to_string())?.writer_id;
        record.writer_after=Some(writer.clone());
        if writer!=record.writer_before {return Err("bootstrap changed destination writer".into());}
        let current=destination.revision().map_err(|e|e.to_string())?;
        record.final_revision=Some(current);
        if let Some(bound)=&completion {
            let request=&bound.activation_request;
            record.activation_request=Some(json!({
                "header":request.header,
                "expectedSelectionEpoch":request.expected_selection_epoch,
                "stagingId":request.staging_id,"receiveId":request.receive_id,
                "targetId":request.target_id,"libraryId":request.library_id,
            }));
            if record.activation_revision!=Some(bound.activation.revision) {return Err("actual activation callback receipt differs".into());}
            let replay=destination.replace_lww_binding(&bound.activation_request).map_err(|e|e.to_string())?;
            if replay.revision!=bound.activation.revision || destination.revision().map_err(|e|e.to_string())?!=current {
                return Err("durable activation receipt replay changed final state".into());
            }
        }
        if record.all_bodies_local_ms.is_some() {
            let cas=crate::asset_repository::PayloadCas::new(destination.repository_root()).map_err(|e|e.to_string())?;
            for hash in asset_hashes {
                if cas.stat_object(hash).map_err(|e|e.to_string())?.is_none() {return Err("awaited body barrier left required Asset absent".into());}
            }
        }
        Ok(())
    });
    if let Err(failed)=checks {
        record.error=Some(record.error.take().map_or(failed.reason.clone(),|prior|format!("{prior}; {}",failed.reason)));
    }
    Ok(record)
}
