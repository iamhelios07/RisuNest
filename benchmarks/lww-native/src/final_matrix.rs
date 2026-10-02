use super::*;
use std::{collections::{BTreeMap,BTreeSet},path::PathBuf};
use measurement::Scenario;
use final_runner::Direction as FinalDirection;
use sha2::Digest;

fn environment(name:&str)->Result<String,String> {
    std::env::var(name).map_err(|_|format!("{name} is required"))
}
fn write(directory:&Path,name:&str,value:&Value)->Result<(),String> {
    std::fs::create_dir_all(directory).map_err(|e|e.to_string())?;
    let file=std::fs::OpenOptions::new().create_new(true).write(true).open(directory.join(name)).map_err(|e|e.to_string())?;
    serde_json::to_writer_pretty(file,value).map_err(|e|e.to_string())
}
fn load(path:&Path,requirement:scale_certificate::ScaleRequirement,output:&Path)->Result<(PersistentStore,scale_certificate::ScaleCertificate,tempfile::TempDir),String> {
    let value:Value=serde_json::from_reader(File::open(path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if value["setup_only"]!=true {return Err("input is not a native construction receipt".into());}
    let root=PathBuf::from(value["target"]["root"].as_str().ok_or("construction root missing")?);
    if output.starts_with(&root) || root.starts_with(output) {return Err("measurement output overlaps the preserved certified source".into());}
    let certificate:scale_certificate::ScaleCertificate=serde_json::from_value(value["target"]["fixture"]["certificate"].clone())
        .map_err(|e|e.to_string())?;
    if certificate.requirement!=requirement {return Err("construction receipt has the wrong physical tier".into());}
    certificate.validate()?;
    if value["schema"]!="risunest.lww-native-construction/v1" {return Err("input is not the owned native constructor receipt".into());}
    let original_writer=value["target"]["writer_id"].as_str().ok_or("construction writer is absent")?;
    let database=root.join("persistent/persistent.sqlite");let device=root.join("persistent/device.sqlite");
    for database in [&database,&device] {
        let wal=PathBuf::from(format!("{}-wal",database.display()));
        if wal.exists() && std::fs::metadata(&wal).map_err(|e|e.to_string())?.len()!=0 {
            return Err("owned constructor has an active WAL; no fixture fork was made".into());
        }
    }
    let readonly=rusqlite::Connection::open_with_flags(&database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e|e.to_string())?;
    let target:String=readonly.query_row("SELECT target FROM library_sync_selection WHERE singleton=1",[],|row|row.get(0)).map_err(|e|e.to_string())?;
    if target!="none" {return Err("owned constructor is already bound".into());}
    for table in ["lww_publications","lww_receive_rows","lww_binding_switch_requests","lww_binding_stages"] {
        let count=readonly.query_row(&format!("SELECT count(*) FROM {table}"),[],|row|sql_u64(row,0)).map_err(|e|e.to_string())?;
        if count!=0 {return Err(format!("owned constructor contains transport state in {table}"));}
    }
    let device_readonly=rusqlite::Connection::open_with_flags(&device,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e|e.to_string())?;
    let actual_writer:String=device_readonly.query_row("SELECT writer_id FROM device_meta WHERE singleton=1",[],|row|row.get(0)).map_err(|e|e.to_string())?;
    if actual_writer!=original_writer {return Err("owned constructor writer changed since its receipt".into());}
    for query in ["SELECT count(*) FROM lww_progress","SELECT count(*) FROM external_lww_segments",
        "SELECT count(*) FROM external_lww_seen","SELECT count(*) FROM lww_receive",
        "SELECT count(*) FROM lww_intents WHERE complete=0"] {
        if device_readonly.query_row(query,[],|row|sql_u64(row,0)).map_err(|e|e.to_string())?!=0 {
            return Err("owned constructor has transport progress or pending operations".into());
        }
    }
    drop(device_readonly);drop(readonly);
    let fork=tempfile::tempdir().map_err(|e|e.to_string())?;
    std::fs::create_dir_all(fork.path().join("persistent")).map_err(|e|e.to_string())?;
    std::fs::copy(&database,fork.path().join("persistent/persistent.sqlite")).map_err(|e|e.to_string())?;
    for asset in &certificate.assets {
        let source=root.join("assets/objects").join(&asset.payload_hash[..2]).join(&asset.payload_hash[2..]);
        let destination=fork.path().join("assets/objects").join(&asset.payload_hash[..2]).join(&asset.payload_hash[2..]);
        std::fs::create_dir_all(destination.parent().ok_or("canonical Asset parent absent")?).map_err(|e|e.to_string())?;
        std::fs::copy(source,destination).map_err(|e|e.to_string())?;
    }
    let mut store=PersistentStore::open(fork.path()).map_err(|e|e.to_string())?;
    let fork_writer=store.lww_clock_state().map_err(|e|e.to_string())?.writer_id;
    if fork_writer==original_writer {return Err("physical fixture fork did not initialize an independent writer".into());}
    if store.lww_binding_state().map_err(|e|e.to_string())?.target!=SyncTarget::None {
        return Err("final matrix requires a fresh unbound certified producer".into());
    }
    let plan=native_shards::ShardPlan::new(1,requirement.minimums().1);
    let actual=collect_native_scale_certificate(&mut store,&plan,requirement)?;
    if actual!=certificate {return Err("actual persisted producer no longer matches its construction certificate".into());}
    write(output,"excluded-certified-fixture-fork.json",&json!({"phase":"excluded-physical-fixture-fork","originalRoot":root,
        "forkRoot":fork.path(),"originalWriter":original_writer,"forkWriter":fork_writer,"physicalCertificate":actual,
        "scope":"checkpointed library SQLite and canonical CAS copied; device/config/jobs not copied; original library unit stamps and unbound outbox retained"}))?;
    Ok((store,certificate,fork))
}
fn inventory(store:&PersistentStore)->Result<Vec<String>,String> {
    let aliases=store.list_asset_aliases(None).map_err(|e|e.to_string())?.value;
    let hashes=aliases.into_iter().map(|alias|alias.object_hash.ok_or("fixture alias is hashless".to_owned()))
        .collect::<Result<BTreeSet<_>,_>>()?;
    if hashes.is_empty() {return Err("actual Asset inventory is empty".into());}
    Ok(hashes.into_iter().collect())
}
fn preinstall(source:&PersistentStore,destination:&PersistentStore,hashes:&[String],all:bool,missing_all:bool)->Result<Value,String> {
    let source_cas=crate::asset_repository::PayloadCas::new(source.repository_root()).map_err(|e|e.to_string())?;
    let destination_cas=crate::asset_repository::PayloadCas::new(destination.repository_root()).map_err(|e|e.to_string())?;
    let mut present=BTreeMap::new();let mut missing=BTreeMap::new();
    for (index,hash) in hashes.iter().enumerate() {
        let size=source_cas.stat_object(hash).map_err(|e|e.to_string())?.ok_or("certified source Asset is absent")?;
        if !missing_all && (all || index%97!=0) {
            let body=source_cas.read_object(hash).map_err(|e|e.to_string())?.ok_or("setup Asset body missing")?;
            let prepared=destination_cas.prepare_bytes(&body).map_err(|e|e.to_string())?;
            if prepared.content_hash!=*hash || prepared.byte_size!=size {return Err("setup Asset identity differs".into());}
            present.insert(hash.clone(),size);
        } else {missing.insert(hash.clone(),size);}
    }
    Ok(json!({"phase":"excluded-actual-destination-body-setup","alreadyPresent":present,"missing":missing}))
}
fn source(run:&str)->Result<source_process::SourceProcess,String> {
    source_process::SourceProcess::start(&PathBuf::from(environment("LWW_SOURCE_TEST_EXE")?),
        &environment("LWW_SOURCE_FINGERPRINT")?,&environment("LWW_SOURCE_BINARY_SHA")?,run)
}

pub(super) fn run()->Result<(),String> {
    let result=run_inner();
    if let Err(reason)=&result {
        if let Ok(output)=environment("RISUNEST_LWW_OUTPUT_DIRECTORY") {
            let output=PathBuf::from(output);
            if !output.join("INVALID-native-matrix-failure.json").exists() {
                write(&output,"INVALID-native-matrix-failure.json",&json!({"status":"INVALID","reason":reason,
                    "rendererAdoption":null,"sourceFileReads":null}))?;
            }
        }
    }
    result
}

fn run_inner()->Result<(),String> {
    if environment("RISUNEST_LWW_FINAL_STAGE")?!="authorized-synthetic" {return Err("final-stage gate is closed".into());}
    let checkpoint=environment("RISUNEST_LWW_FINAL_CHECKPOINT")?;
    if checkpoint.len()!=40 || !checkpoint.bytes().all(|b|b.is_ascii_hexdigit()) {return Err("verified checkpoint is required".into());}
    let directory=PathBuf::from(environment("RISUNEST_LWW_OUTPUT_DIRECTORY")?);
    let transport=environment("RISUNEST_LWW_FINAL_TRANSPORT")?;
    let selected=match std::env::var("RISUNEST_LWW_FINAL_SCENARIO") {
        Ok(value)=>Some(serde_json::from_value::<Scenario>(json!(value)).map_err(|e|e.to_string())?),
        Err(std::env::VarError::NotPresent)=>None,Err(e)=>return Err(e.to_string()),
    };
    let route=std::env::var("RISUNEST_LWW_FINAL_ROUTE").unwrap_or_else(|_|"server-sync-first-binding".into());
    let executable=std::env::current_exe().map_err(|e|e.to_string())?;
    let binary=hex::encode(sha2::Sha256::digest(std::fs::read(&executable).map_err(|e|e.to_string())?));
    let mut tiers=Vec::new();
    for (name,variable,requirement) in [("target","RISUNEST_LWW_TARGET_RECEIPT",scale_certificate::ScaleRequirement::Target),
        ("above-target","RISUNEST_LWW_ABOVE_RECEIPT",scale_certificate::ScaleRequirement::AboveTarget)] {
        let (store,certificate,fork)=load(&PathBuf::from(environment(variable)?),requirement,&directory.join(name))?;
        if directory.starts_with(store.repository_root()) || store.repository_root().starts_with(&directory) {
            return Err("measurement output and native data roots must be separate".into());
        }
        tiers.push((name,store,certificate,fork));
    }
    scale_certificate::validate_above_target(&tiers[0].2,&tiers[1].2)?;
    write(&directory,"native-matrix-plan.json",&json!({"checkpoint":checkpoint,"binary_sha256":binary,
        "transport":transport,"selectedScenario":selected,"route":route,"routineWarmups":final_runner::WARMUPS,
        "routineRepetitions":final_runner::REPETITIONS,"costlyWarmups":costly_runner::WARMUPS,
        "costlyRepetitions":costly_runner::REPETITIONS,"costlyPercentileResolution":"N5 nearest-rank p95/p99 equal the maximum",
        "nativeTimingOnly":true,"rendererAdoption":null,"certificates":tiers.iter().map(|t|&t.2).collect::<Vec<_>>()}))?;
    let result=(||->Result<(),String> {
        let mut reports=Vec::new();
        let mut selected_samples=Vec::new();
        let mut resume_samples=Vec::new();
        let mut costly_reports=Vec::new();
        for (name,mut store,certificate,_fork) in tiers {
            let output=directory.join(name);std::fs::create_dir_all(&output).map_err(|e|e.to_string())?;
            let hashes=inventory(&store)?;
            if transport=="local-server" {
                let server=LocalServerFixture::new();
                let sender=server.client(&store);let config=sender.client.config();
                bind(&mut store,SyncTarget::Server(server.endpoint.clone()),&server.endpoint,&config.library_id);
                server_sync::lww_tests::drain_publications(&sender,&mut store,&[]).map_err(|e|format!("{e:?}"))?;
                if selected.is_some_and(|scenario|costly_runner::SCENARIOS.contains(&scenario)) {
                    costly_server(&output,&mut store,server,sender,&certificate,&hashes,selected.unwrap(),&route)?;
                    costly_reports.push(serde_json::from_reader::<_,Value>(File::open(output.join("costly-native-results.json")).map_err(|e|e.to_string())?)
                        .map_err(|e|e.to_string())?);
                    continue;
                }
                let (_peer_directory,mut peer)=server_sync::lww_tests::local();
                server.prepare_binding_candidate(&peer);
                let io=Arc::new(server_sync::client::TestIoCounters::default());
                server_sync::first_binding_cycle(&mut peer,io.clone(),|_|{}).map_err(|e|format!("{e:?}"))?;
                server_sync::hydrate_binding_bodies(&peer,io.clone(),Some("synthetic-character-0"),||{}).map_err(|e|format!("{e:?}"))?;
                let receiver=LocalServerFixture::reopen_client(&peer,io).map_err(|e|format!("{e:?}"))?;
                server_sync::lww_tests::receive_available(&sender,&mut store,&[]).map_err(|e|format!("{e:?}"))?;
                server_sync::lww_tests::receive_available(&receiver,&mut peer,&[]).map_err(|e|format!("{e:?}"))?;
                if selected==Some(Scenario::OrdinaryResume) {
                    let samples=resume_server(store,peer,sender,receiver,&hashes,&output)?;
                    resume_samples.push((certificate,samples));continue;
                }
                let collection=if let Some(scenario)=selected {
                    if !final_runner::AVAILABLE.contains(&scenario) {return Err("scenario requires the actual external renderer endpoint".into());}
                    collect_selected(scenario,|direction,iteration,warmup| {
                        let (a,b,ca,cb)=match direction {FinalDirection::AtoB=>(&mut store,&mut peer,&sender,&receiver),
                            FinalDirection::BtoA=>(&mut peer,&mut store,&receiver,&sender)};
                        final_server_cycle(a,b,ca,cb,&hashes,scenario,direction,iteration,warmup)
                    })
                } else {collect_final_local(&mut store,&mut peer,&sender,&receiver,&hashes)};
                write(&output,"native-raw-collection.json",&serde_json::to_value(&collection).map_err(|e|e.to_string())?)?;
                if selected.is_none() {reports.push(final_runner::report(&checkpoint,&binary,&transport,certificate,&collection)?);}
                else {selected_samples.push(collection);}
            } else if transport=="fake-provider" {
                let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e|e.to_string())?;
                let (collection,resume)=runtime.block_on(external_tier(&output,store,&hashes,selected,&route))?;
                if let Some(samples)=resume {resume_samples.push((certificate,samples));continue;}
                if selected.is_some_and(|scenario|costly_runner::SCENARIOS.contains(&scenario)) {
                    costly_reports.push(serde_json::from_reader::<_,Value>(File::open(output.join("costly-native-results.json")).map_err(|e|e.to_string())?)
                        .map_err(|e|e.to_string())?);continue;
                }
                write(&output,"native-raw-collection.json",&serde_json::to_value(&collection).map_err(|e|e.to_string())?)?;
                if selected.is_none() {reports.push(final_runner::report(&checkpoint,&binary,&transport,certificate,&collection)?);}
                else {selected_samples.push(collection);}
            } else {return Err("transport must be local-server or fake-provider".into());}
        }
        if reports.len()==2 {return final_runner::write_comparison(&directory,&reports[0],&reports[1]);}
        if resume_samples.len()==2 {
            let proofs=final_runner::compare_resume(&resume_samples[0].0,&resume_samples[1].0,&resume_samples[0].1,&resume_samples[1].1)?;
            return write(&directory,"native-final-resume-pair.json",&json!({"status":"native-only-valid","proofs":proofs,
                "samples":[&resume_samples[0].1,&resume_samples[1].1],"rendererAdoption":null}));
        }
        if selected_samples.len()==2 {
            let a=&selected_samples[0];let b=&selected_samples[1];
            if !a.failures.is_empty() || !b.failures.is_empty() || a.samples.len()!=b.samples.len() {
                return Err("selected native collection contains failed matrix slots".into());
            }
            let mut proofs=Vec::new();
            for (a,b) in a.samples.iter().zip(&b.samples) {
                a.validate()?;b.validate()?;
                if (a.scenario,a.direction,a.iteration,a.warmup,&a.selected_keys,&a.emitted_keys,&a.affected_keys,&a.dispatch_keys,&a.accepted_winner_keys)
                    !=(b.scenario,b.direction,b.iteration,b.warmup,&b.selected_keys,&b.emitted_keys,&b.affected_keys,&b.dispatch_keys,&b.accepted_winner_keys) {
                    return Err("selected tier pair changed exact actual key sets".into());
                }
                proofs.push(native_observation::assert_frozen_growth(&a.observation,&b.observation)?);
            }
            let mut timings=Vec::new();
            for (tier,collection) in [("target",a),("above-target",b)] {
                for direction in [FinalDirection::AtoB,FinalDirection::BtoA] {
                    let recorded=collection.samples.iter().filter(|sample|sample.direction==direction && !sample.warmup).collect::<Vec<_>>();
                    if recorded.len()!=final_runner::REPETITIONS as usize {return Err("selected routine recorded slot count differs from policy".into());}
                    timings.push(json!({"tier":tier,"direction":direction,"recordedCount":recorded.len(),
                        "nativeDurableSaveMs":measurement::percentiles(recorded.iter().map(|sample|sample.timings.durable_save_ms).collect())?,
                        "nativePublicationCompleteMs":measurement::percentiles(recorded.iter().map(|sample|sample.timings.publication_complete_ms).collect())?,
                        "nativeReceiverDurableAckMs":measurement::percentiles(recorded.iter().map(|sample|sample.timings.receiver_durable_ack_ms).collect())?}));
                }
            }
            return write(&directory,"native-final-selected-pair.json",&json!({"status":"native-only-valid","proofs":proofs,"timings":timings,"rendererAdoption":null}));
        }
        if costly_reports.len()==2 {
            let valid=costly_reports.iter().all(|report|report["status"]=="native-only-valid");
            write(&directory,"native-costly-pair.json",&json!({"status":if valid {"native-only-valid"}else{"INVALID"},
                "tiers":costly_reports,"checkpoint":checkpoint,"binary_sha256":binary,"rendererAdoption":null}))?;
            return if valid {Ok(())}else{Err("costly native matrix contains actual failed or unobserved native completion slots".into())};
        }
        Err("selected native endpoint requires actual external renderer orchestration".into())
    })();
    let after=hex::encode(sha2::Sha256::digest(std::fs::read(&executable).map_err(|e|e.to_string())?));
    write(&directory,"native-matrix-result.json",&json!({"status":if result.is_ok() && binary==after {"native-only-valid"}else{"INVALID"},
        "checkpoint":checkpoint,"binaryShaBefore":binary,"binaryShaAfter":after,"error":result.as_ref().err(),"rendererAdoption":null}))?;
    if binary!=after {return Err("executed native binary changed during the matrix".into());}
    result
}

fn collect_selected(scenario:Scenario,mut cycle:impl FnMut(FinalDirection,u32,bool)->Result<final_runner::RoutineSample,final_runner::CycleFailure>)
    ->final_runner::CollectedSamples {
    let mut collected=final_runner::CollectedSamples {samples:Vec::new(),failures:Vec::new()};
    for direction in [FinalDirection::AtoB,FinalDirection::BtoA] {for iteration in 0..final_runner::WARMUPS+final_runner::REPETITIONS {
        match cycle(direction,iteration,iteration<final_runner::WARMUPS) {
            Ok(sample)=>collected.samples.push(sample),
            Err(failure)=>collected.failures.push(final_runner::FailedSlot {scenario,direction,iteration,failure}),
        }
    }}collected
}

fn resume_server(a:PersistentStore,b:PersistentStore,ca:LwwClient,cb:LwwClient,hashes:&[String],output:&Path)
    ->Result<Vec<final_runner::NativeResumeSample>,String> {
    let mut samples=Vec::new();let mut pair=Some((a,b,ca,cb));
    for direction in [FinalDirection::AtoB,FinalDirection::BtoA] {for iteration in 0..final_runner::WARMUPS+final_runner::REPETITIONS {
        let (mut a,mut b,ca,cb)=pair.take().ok_or("resume endpoint pair absent")?;
        let outcome=match direction {
            FinalDirection::AtoB=>final_server_resume_endpoint(&mut a,&ca,b,cb,hashes,direction,iteration,iteration<final_runner::WARMUPS)
                .map(|(b,cb,sample)|((a,b,ca,cb),sample)),
            FinalDirection::BtoA=>final_server_resume_endpoint(&mut b,&cb,a,ca,hashes,direction,iteration,iteration<final_runner::WARMUPS)
                .map(|(a,ca,sample)|((a,b,ca,cb),sample)),
        };
        match outcome {
            Ok((next,sample))=>{write(output,&format!("resume-{}-{iteration}.json",if direction==FinalDirection::AtoB {"a-b"}else{"b-a"}),
                &serde_json::to_value(&sample).map_err(|e|e.to_string())?)?;
                pair=Some(next);samples.push(sample);
            },
            Err(failed)=>{write(output,"INVALID-resume.json",&json!({"direction":direction,"iteration":iteration,"failure":failed}))?;
                return Err("actual reopened recipient failed, raw scope retained".into());},
        }
    }}final_runner::summarize_resume(&samples)?;Ok(samples)
}

fn costly_server(output:&Path,producer:&mut PersistentStore,server:LocalServerFixture,sender:LwwClient,
    certificate:&scale_certificate::ScaleCertificate,hashes:&[String],scenario:Scenario,route:&str)->Result<(),String> {
    use crate::native_file_jobs::{NativeFileJobState,JobRegistry,JobKind,portable,OpenedJobSource};
    let setup=tempfile::tempdir().map_err(|e|e.to_string())?;
    let delta=if scenario==Scenario::DuringAssetTransfer {Some(prepare_large_asset_delta(producer,certificate,&sender)?)}else{None};
    let hashes=if delta.is_some() {inventory(producer)?}else{hashes.to_vec()};
    if let Some(delta)=&delta {write(output,"excluded-large-asset-delta.json",delta)?;}
    let portable_path=if route=="device-file-backup" && scenario!=Scenario::DuringBackup {
        let registry=JobRegistry::default();let revision=producer.revision().map_err(|e|e.to_string())?;
        let job=registry.create_with_context(JobKind::ExportPortableBackup,Some(revision),vec![])?;
        let summary=portable::export_portable(None,revision,setup.path(),&setup.path().join("handoffs"),
            producer.open_native_job_store().map_err(|e|e.to_string())?,&job,None,"synthetic-final-matrix")
            .map_err(|e|format!("{}: {}",e.code,e.message))?;
        Some(PathBuf::from(summary.handoff_path.ok_or("actual backup handoff absent")?))
    } else {None};
    let snapshot=if route=="native-snapshot" {Some(producer.snapshot_create("synthetic-final-matrix").map_err(|e|e.to_string())?)}else{None};
    let mut summaries=Vec::new();
    for slot in costly_runner::slots().into_iter().filter(|slot|slot.scenario==scenario) {
        let label=format!("{:?}-{}",slot.direction,slot.iteration);
        let (_destination_root,mut destination)=server_sync::lww_tests::local();
        if producer.lww_clock_state().map_err(|e|e.to_string())?.writer_id==destination.lww_clock_state().map_err(|e|e.to_string())?.writer_id {
            return Err("costly destination reused the producer writer".into());
        }
        let present=if let Some(delta)=&delta {
            let hash=delta["payloadHash"].as_str().ok_or("actual physical barrier Asset absent")?;
            let preexisting=hashes.iter().filter(|candidate|candidate.as_str()!=hash).cloned().collect::<Vec<_>>();
            let mut receipt=preinstall(producer,&destination,&preexisting,true,false)?;
            receipt["missing"]=json!({hash:delta["byteSize"]});receipt
        } else {preinstall(producer,&destination,&hashes,scenario==Scenario::RestoreAllPresent,scenario==Scenario::FullBootstrap)?};
        write(output,&format!("costly-{label}-setup.json"),&json!({"baseCertificate":certificate,"bodyPartition":present,
            "destinationWriter":destination.lww_clock_state().map_err(|e|e.to_string())?.writer_id}))?;
        let jobs=Arc::new(NativeFileJobState::initialize(destination.repository_root().join("native-file-jobs")));
        let outcome=(||->Result<Value,String> {
            match route {
                "device-file-backup"=>{
                    if scenario==Scenario::DuringBackup {
                        server.prepare_binding_candidate(&destination);
                        let io=Arc::new(server_sync::client::TestIoCounters::default());
                        server_sync::first_binding_cycle(&mut destination,io.clone(),|_|{}).map_err(|e|format!("{e:?}"))?;
                        server_sync::hydrate_binding_bodies(&destination,io.clone(),Some("synthetic-character-0"),||{}).map_err(|e|format!("{e:?}"))?;
                        let receiver=LocalServerFixture::reopen_client(&destination,io).map_err(|e|format!("{e:?}"))?;
                        let sender=LocalServerFixture::reopen_client(producer,Arc::new(server_sync::client::TestIoCounters::default())).map_err(|e|format!("{e:?}"))?;
                        return final_backup_overlap_costly_slot(&slot,producer.open_native_job_store().map_err(|e|e.to_string())?,
                            &setup.path().join(format!("backup-{label}")),&setup.path().join("handoffs"),
                            producer.open_native_job_store().map_err(|e|e.to_string())?,destination,sender,receiver,hashes.clone());
                    }
                    let device=Arc::new(crate::device_backup::DeviceBackupState::initialize(destination.repository_root().join("device-backup")));
                    let source=portable_path.as_ref().map(|path| {
                        let file=File::open(path).map_err(|e|e.to_string())?;
                        let total_bytes=file.metadata().map_err(|e|e.to_string())?.len();
                        Ok::<_,String>(OpenedJobSource {file,total_bytes,custody:None})
                    }).transpose()?;
                    final_device_file_costly_slot(&slot,destination,source,setup.path().join(format!("restore-{label}")),
                        &setup.path().join("handoffs"),jobs,Arc::new(crate::persistent_store::commands::PersistentStoreState::default()),
                        device,
                        hashes.clone(),|_,_|{})
                },
                "native-snapshot"=>{
                    server.prepare_binding_candidate(&destination);
                    server_sync::first_binding_cycle(&mut destination,Arc::new(server_sync::client::TestIoCounters::default()),|_|{})
                        .map_err(|e|format!("{e:?}"))?;
                    std::fs::create_dir_all(&destination.snapshots_dir).map_err(|e|e.to_string())?;
                    std::fs::copy(producer.snapshots_dir.join("snapshots.sqlite"),destination.snapshots_dir.join("snapshots.sqlite"))
                        .map_err(|e|e.to_string())?;
                    final_snapshot_costly_slot(&slot,&mut destination,&snapshot.as_ref().ok_or("snapshot producer absent")?.id,
                        &uuid::Uuid::new_v4().to_string(),&jobs,setup.path(),&hashes,|_|Ok(()))
                },
                "server-sync-first-binding"=>{
                    let mut child=source(&format!("final-{label}"))?;
                    let sender=native_bootstrap::source_client(&mut child,producer,"synthetic-final-producer",&hex::encode(sha2::Sha256::digest(label.as_bytes())),false)?;
                    let config=sender.client.config();
                    bind(producer,SyncTarget::Server(config.endpoint.clone()),&config.endpoint,&config.library_id);
                    reset_work(&hashes);
                    let published=server_sync::lww_tests::drain_publications(&sender,producer,&[]);
                    let observed=take_work(false);published.map_err(|e|format!("{e:?}"))?;
                    let roles=observed.body_objects.iter().map(|(hash,object)|(hash.clone(),object.purposes.iter().cloned().collect()))
                        .collect::<BTreeMap<String,BTreeSet<String>>>();
                    let candidate=native_bootstrap::source_client(&mut child,&destination,"synthetic-final-destination",
                        &hex::encode(sha2::Sha256::digest(format!("destination-{label}").as_bytes())),true)?;
                    if scenario==Scenario::DuringAssetTransfer {
                        let hash=delta.as_ref().ok_or("excluded large Asset delta absent")?["payloadHash"].as_str().ok_or("large Asset delta hash absent")?;
                        let mut roles=roles;roles.entry(hash.into()).or_default().insert("Asset".into());
                        server_sync::first_binding_cycle(&mut destination,candidate.client.test_io.as_ref().ok_or("candidate counters absent")?.clone(),|_|{})
                            .map_err(|e|format!("{e:?}"))?;
                        let receiver=LocalServerFixture::reopen_client(&destination,Arc::new(server_sync::client::TestIoCounters::default())).map_err(|e|format!("{e:?}"))?;
                        final_server_transfer_costly_slot(&slot,child,&mut destination,&receiver,producer,&sender,&roles,hash,Some("synthetic-character-0"))
                    } else {final_server_bootstrap_costly_slot(&slot,child,&mut destination,
                        candidate.client.test_io.ok_or("candidate counters absent")?,&roles,Some("synthetic-character-0"))}
                },
                _=>Err("selected restore route does not use the local-server producer".into()),
            }
        })();
        let raw=match outcome {Ok(raw)=>raw,Err(reason)=>json!({"status":"INVALID","slot":slot,"route":route,"error":reason})};
        write(output,&format!("costly-{label}-raw.json"),&raw)?;
        let result=server_native_result(&raw,&slot,&present);
        let summary=match result {Ok(summary)=>summary,Err(error)=>json!({"status":"INVALID","slot":slot,"error":error,"rendererAdoption":null})};
        write(output,&format!("costly-{label}-native-result.json"),&summary)?;summaries.push(summary);
    }
    write(output,"costly-native-results.json",&costly_summary(&summaries))?;
    Ok(())
}

async fn external_tier(output:&Path,store:PersistentStore,hashes:&[String],selected:Option<Scenario>,route:&str)
    ->Result<(final_runner::CollectedSamples,Option<Vec<final_runner::NativeResumeSample>>),String> {
    let mut fixture=CycleFixture::new();
    fixture.a=store;fixture.sender.connection_root=fixture.a.repository_root().to_owned();
    bind(&mut fixture.a,SyncTarget::External(fixture.sender.connection_id.clone()),&fixture.sender.repository.connection_identity,&fixture.sender.library);
    bind(&mut fixture.b,SyncTarget::External(fixture.receiver.connection_id.clone()),&fixture.receiver.repository.connection_identity,&fixture.receiver.library);
    let cancel=Cancellation::default();
    let authority_a=fixture.a.lww_binding_authority().map_err(|e|e.to_string())?;
    let authority_b=fixture.b.lww_binding_authority().map_err(|e|e.to_string())?;
    fixture.sender.publish(&mut fixture.a,authority_a.clone(),&[],&cancel).await.map_err(|e|e.to_string())?;
    fixture.receiver.receive_and_apply(&mut fixture.b,authority_b,&[],&cancel).await.map_err(|e|e.to_string())?;
    fixture.sender.receive_and_apply(&mut fixture.a,authority_a,&[],&cancel).await.map_err(|e|e.to_string())?;
    verify_bootstrap(&fixture.a,&fixture.b);verify_settled(&fixture.a,&fixture.sender);verify_settled(&fixture.b,&fixture.receiver);
    if selected==Some(Scenario::OrdinaryResume) {
        let CycleFixture {directory_a,directory_b,a,b,provider,sender,receiver}=fixture;
        let _directories=(directory_a,directory_b);
        let mut pair=Some((a,b,sender,receiver));let mut samples=Vec::new();
        for direction in [FinalDirection::AtoB,FinalDirection::BtoA] {for iteration in 0..final_runner::WARMUPS+final_runner::REPETITIONS {
            let (mut a,mut b,mut sender,mut receiver)=pair.take().ok_or("resume endpoint pair absent")?;
            let outcome=match direction {
                FinalDirection::AtoB=>final_external_resume_endpoint(&mut a,&mut sender,b,receiver,&provider,hashes,direction,iteration,iteration<final_runner::WARMUPS).await
                    .map(|(b,receiver,sample)|((a,b,sender,receiver),sample)),
                FinalDirection::BtoA=>final_external_resume_endpoint(&mut b,&mut receiver,a,sender,&provider,hashes,direction,iteration,iteration<final_runner::WARMUPS).await
                    .map(|(a,sender,sample)|((a,b,sender,receiver),sample)),
            };
            match outcome {
                Ok((next,sample))=>{write(output,&format!("resume-{:?}-{iteration}.json",direction),&serde_json::to_value(&sample).map_err(|e|e.to_string())?)?;
                    pair=Some(next);samples.push(sample);},
                Err(failed)=>{write(output,"INVALID-resume.json",&json!({"direction":direction,"iteration":iteration,"failure":failed}))?;
                    return Err("actual external reopened recipient failed, raw scope retained".into());},
            }
        }}
        write(output,"native-resume-summary.json",&json!({"samples":samples,"summaries":final_runner::summarize_resume(&samples)?}))?;
        return Ok((final_runner::CollectedSamples {samples:Vec::new(),failures:Vec::new()},Some(samples)));
    }
    if selected.is_some_and(|scenario|costly_runner::SCENARIOS.contains(&scenario)) {
        costly_external(output,&mut fixture,hashes,selected.unwrap(),route).await?;
        return Ok((final_runner::CollectedSamples {samples:Vec::new(),failures:Vec::new()},None));
    }
    if let Some(scenario)=selected {
        if !final_runner::AVAILABLE.contains(&scenario) {return Err("selected scenario requires actual resume or renderer orchestration".into());}
        let mut collection=final_runner::CollectedSamples {samples:Vec::new(),failures:Vec::new()};
        for direction in [FinalDirection::AtoB,FinalDirection::BtoA] {for iteration in 0..final_runner::WARMUPS+final_runner::REPETITIONS {
            let (a,b,sender,receiver)=match direction {FinalDirection::AtoB=>(&mut fixture.a,&mut fixture.b,&mut fixture.sender,&mut fixture.receiver),
                FinalDirection::BtoA=>(&mut fixture.b,&mut fixture.a,&mut fixture.receiver,&mut fixture.sender)};
            match final_external_cycle(a,b,sender,receiver,&fixture.provider,hashes,scenario,direction,iteration,iteration<final_runner::WARMUPS).await {
                Ok(sample)=>collection.samples.push(sample),
                Err(failure)=>collection.failures.push(final_runner::FailedSlot {scenario,direction,iteration,failure}),
            }
        }}Ok((collection,None))
    } else {Ok((final_runner::collect_async(&mut FinalExternalAdapter {fixture:&mut fixture,asset_hashes:hashes}).await,None))}
}


async fn connected(fixture:&CycleFixture)->Result<Arc<crate::external_storage::connection_commands::ConnectedRepository>,String> {
    use crate::external_storage::{contract::{Provider,ConnectionConfig,SecretRef,OpenMode,RemoteLocator},connection_store::StoredConnection};
    let config=ConnectionConfig {provider:"synthetic".into(),profile:None,endpoint:"https://synthetic.invalid".into(),
        account_id:"fixture".into(),location:BTreeMap::new(),oauth_profile:None};
    let (handle,capabilities)=fixture.provider.open_repository(&config,&SecretRef("fixture".into()),OpenMode::Existing,&Cancellation::default())
        .await.map_err(|e|format!("{e:?}"))?;
    if handle.repository_id!=fixture.sender.repository.repository_id || handle.connection_identity!=fixture.sender.repository.connection_identity {
        return Err("actual reopened provider repository differs from the publisher".into());
    }
    let dependencies=crate::external_storage::fake::loopback_dependencies(crate::external_storage::fake::MemoryVault::default(),
        crate::external_storage::runtime::now_ms()).dependencies;
    Ok(Arc::new(crate::external_storage::connection_commands::ConnectedRepository {
        stored:StoredConnection {id:fixture.sender.connection_id.clone(),config,descriptor:fixture.sender.descriptor.clone(),
            descriptor_locator:RemoteLocator {connection_identity:handle.connection_identity.clone(),collection:None,object:"descriptor".into()},
            provider_repository_id:handle.repository_id.clone(),credential_ref:"fixture".into(),root_key_ref:"fixture-key".into(),
            recovery_key_ref:"fixture-recovery".into(),retention_policy:None,capabilities,
            created_at_ms:crate::external_storage::runtime::now_ms(),verified_at_ms:crate::external_storage::runtime::now_ms(),
            last_sync_at_ms:None,last_backup_at_ms:None},provider:fixture.provider.clone(),handle,dependencies,
        root_key:zeroize::Zeroizing::new(*fixture.sender.root_key)}))
}

async fn prepare_remote_backup(store:&mut PersistentStore,connected:&crate::external_storage::connection_commands::ConnectedRepository,
    directory:&Path)->Result<crate::external_storage::packaging::CompletedSnapshot,String> {
    use crate::external_storage::{packaging,sections,journal::{TransferJournal,JobIdentity},phase_progress::PhaseProgress};
    let cancel=Cancellation::default();
    let mut worker=store.open_native_job_store().map_err(|e|e.to_string())?;
    let connection=connected.stored.id.clone();let spool=directory.join("sections");let worker_cancel=cancel.clone();
    let capture=tokio::task::spawn_blocking(move ||->Result<_,String> {
        let probe=crate::local_backup::NeverCancelled;
        let hydration=worker.hydrate_external_capture_dependencies(&connection,&probe).map_err(|e|e.to_string())?;
        let (lease,rows)=worker.lww_acquire_backup_capture(worker.revision().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        let sections=match sections::capture_prepared_backup_sections(&rows,&spool,&worker_cancel) {
            Ok(sections)=>sections,Err(error)=>{let _=worker.release_revision(&lease.lease);return Err(format!("{error:?}"));}
        };
        worker.capture_external_library_from_lease_with_sections(&connection,&hydration,&lease.lease,sections,&probe)
            .map_err(|e| {let _=worker.release_revision(&lease.lease);e.to_string()})
    }).await.map_err(|e|e.to_string())??;
    let identity=capture.identity.clone();let snapshot_id=uuid::Uuid::new_v4().to_string();
    let fingerprint=capture.catalog.content_fingerprint(&risunest_external_storage_format::format::library_fingerprint_domain())
        .map_err(|e|e.to_string())?;
    let original_units=capture.catalog.original_backup_units().map_err(|e|e.to_string())?;
    let sections=capture.catalog.backup_sections().map_err(|e|e.to_string())?;
    let mut journal=TransferJournal::open(directory,JobIdentity {job_id:snapshot_id.clone(),connection_id:connected.stored.id.clone(),
        repository_id:connected.handle.repository_id.clone(),capture_id:capture.id.clone(),capture:identity.clone()}).map_err(|e|format!("{e:?}"))?;
    let metadata=packaging::SnapshotMetadata {snapshot_id:snapshot_id.clone(),repository_id:connected.stored.descriptor.repository_id.clone(),
        library_id:identity.library_epoch.clone(),author_device_id:identity.store_id.clone(),created_at_ms:crate::external_storage::runtime::now_ms(),
        logical_revision:identity.revision.try_into().map_err(|_|"capture revision is negative")?,parent_snapshot_id:None,content_fingerprint:fingerprint,
        purpose:packaging::SnapshotPurpose::BackupBundle {source:risunest_external_storage_format::control::BundleSource::Device {writer_id:identity.store_id},
            remote_generation:None,original_units}};
    let cache=directory.join("cache");
    let completed=packaging::package_and_upload(capture,sections,store.repository_root(),&cache,metadata,&connected.root_key,
        packaging::PackageLimits::from_capabilities(&connected.stored.capabilities).map_err(|e|format!("{e:?}"))?,None,&mut journal,
        connected.provider.as_ref(),&connected.handle,&PhaseProgress::silent(),&cancel).await.map_err(|e|format!("{e:?}"))?;
    match packaging::verify_publication(&completed,store.repository_root(),&cache,&mut journal,&connected.root_key,
        connected.provider.as_ref(),&connected.handle,&cancel).await.map_err(|e|format!("{e:?}"))? {
        packaging::PublicationReadiness::Verified=>Ok(completed),
        packaging::PublicationReadiness::Repackage=>Err("actual excluded backup publication requires repackage".into()),
    }
}

async fn costly_external(output:&Path,fixture:&mut CycleFixture,hashes:&[String],scenario:Scenario,route:&str)->Result<(),String> {
    use crate::external_storage::{leases,lease_ledger::LocalLeaseLedger,job_store::{JobStore,DurableJob,StartJobRequest}};
    let setup=tempfile::tempdir().map_err(|e|e.to_string())?;
    let connected=connected(fixture).await?;
    let backup=if route=="remote-backup" {Some(prepare_remote_backup(&mut fixture.a,&connected,&setup.path().join("remote-backup")).await?)}else{None};
    if scenario==Scenario::ConsolidatedSnapshot {
        fixture.sender.compact_published(&setup.path().join("consolidated"),&uuid::Uuid::new_v4().to_string(),
            &fixture.a.lww_clock_state().map_err(|e|e.to_string())?.writer_id,&fixture.sender.capabilities,&Cancellation::default(),None)
            .await.map_err(|e|format!("{e:?}"))?;
    }
    if scenario==Scenario::IncomparableSnapshots {
        // The second writer contributes a real publication, not a fabricated checkpoint.
        server_sync::lww_tests::save(&mut fixture.b,&["root","loreBookDepth"],json!(7));
        let authority=fixture.b.lww_binding_authority().map_err(|e|e.to_string())?;
        fixture.receiver.publish(&mut fixture.b,authority,&[],&Cancellation::default()).await.map_err(|e|format!("{e:?}"))?;
        let writers=[fixture.a.lww_clock_state().map_err(|e|e.to_string())?.writer_id,fixture.b.lww_clock_state().map_err(|e|e.to_string())?.writer_id];
        let paths=[setup.path().join("incomparable-a"),setup.path().join("incomparable-b")];
        let ids=[uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string()];
        let raw=native_maintenance::produce_incomparable_native(&fixture.sender,&fixture.provider,[&paths[0],&paths[1]],
            [&ids[0],&ids[1]],[&writers[0],&writers[1]],&fixture.sender.capabilities).await?;
        write(output,"excluded-incomparable-producer.json",&raw)?;
    }
    let source_ids=authenticated_source_ids(fixture,backup.as_ref(),hashes,&setup.path().join("source-inventory")).await?;
    write(output,"excluded-authenticated-source-body-ids.json",&json!({"logicalAssetsByPhysicalObject":source_ids,
        "inventoryOrigin":"actual verified catalogs and standalone descriptors; no body bytes fetched for this inventory"}))?;
    let mut summaries=Vec::new();
    for slot in costly_runner::slots().into_iter().filter(|slot|slot.scenario==scenario) {
        let label=format!("{:?}-{}",slot.direction,slot.iteration);
        let (_destination_directory,mut destination)=server_sync::lww_tests::local();
        let partition=preinstall(&fixture.a,&destination,hashes,scenario==Scenario::RestoreAllPresent,
            matches!(scenario,Scenario::FullBootstrap|Scenario::ConsolidatedSnapshot|Scenario::IncomparableSnapshots))?;
        write(output,&format!("costly-{label}-setup.json"),&partition)?;
        let reads_before=source_ids.keys().map(|id|(id.clone(),fixture.provider.read_attempts(id))).collect::<BTreeMap<_,_>>();
        let raw=if scenario==Scenario::DuringBackup {
            use crate::external_storage::contract::{Provider,SecretRef,OpenMode};
            let (source,peer,source_engine,peer_engine)=match slot.direction {
                FinalDirection::AtoB=>(&fixture.a,&fixture.b,&fixture.sender,&fixture.receiver),
                FinalDirection::BtoA=>(&fixture.b,&fixture.a,&fixture.receiver,&fixture.sender),
            };
            let backup=source.open_native_job_store().map_err(|e|e.to_string())?;
            let foreground=source.open_native_job_store().map_err(|e|e.to_string())?;
            let peer=peer.open_native_job_store().map_err(|e|e.to_string())?;
            let mut engines=Vec::new();
            for engine in [source_engine,peer_engine] {
                let (handle,capabilities)=fixture.provider.open_repository(&connected.stored.config,&SecretRef("fixture".into()),
                    OpenMode::Existing,&Cancellation::default()).await.map_err(|e|format!("{e:?}"))?;
                if handle.repository_id!=engine.repository.repository_id || handle.connection_identity!=engine.repository.connection_identity {
                    return Err("actual backup foreground repository reopen changed identity".into());
                }
                engines.push(crate::external_storage::lww_engine::ExternalLwwEngine {provider:engine.provider.clone(),repository:handle,
                    library:engine.library.clone(),root_key:zeroize::Zeroizing::new(*engine.root_key),admission:engine.admission.clone(),
                    connection_id:engine.connection_id.clone(),connection_root:engine.connection_root.clone(),
                    capabilities,descriptor:engine.descriptor.clone()});
            }
            let receiver=engines.pop().ok_or("actual foreground receiver absent")?;
            let sender=engines.pop().ok_or("actual foreground sender absent")?;
            let provider=fixture.provider.clone();let assets=hashes.to_vec();
            let direction=slot.direction;let iteration=slot.iteration;let warmup=slot.warmup;
            let owned=setup.path().join(format!("backup-{label}"));let handoffs=setup.path().join(format!("handoffs-{label}"));
            tokio::task::spawn_blocking(move || {
                let inputs=Arc::new(std::sync::Mutex::new(Some((foreground,peer,sender,receiver,provider,assets.clone()))));
                let result=Arc::new(std::sync::Mutex::new(None));let callback_result=result.clone();
                let mut raw=native_maintenance::portable_backup_raw(backup,&owned,&handoffs,&assets,iteration,move |lease,revision| {
                    let observed=match inputs.lock().unwrap().take() {
                        Some((mut foreground,mut peer,mut sender,mut receiver,provider,assets))=> {
                            std::thread::spawn(move || {
                                let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build()
                                    .map_err(|error|final_runner::CycleFailure {reason:error.to_string(),observation:None})?;
                                runtime.block_on(final_external_cycle(&mut foreground,&mut peer,&mut sender,&mut receiver,
                                    &provider,&assets,Scenario::SettingEdit,direction,iteration,warmup))
                            }).join().unwrap_or_else(|_|Err(final_runner::CycleFailure {reason:"backup foreground worker panicked".into(),observation:None}))
                        },
                        None=>Err(final_runner::CycleFailure {reason:"backup capture callback repeated".into(),observation:None}),
                    };
                    *callback_result.lock().unwrap()=Some((lease.to_owned(),revision,observed));
                })?;
                match result.lock().unwrap().take() {
                    Some((lease,revision,observed))=>{
                        raw["foregroundCaptureLease"]=json!({"lease":lease,"revision":revision,"workerJoinedWhileLeaseHeld":true});
                        raw["foregroundObservation"]=match observed {Ok(sample)=>serde_json::to_value(sample).map_err(|e|e.to_string())?,
                            Err(failure)=>json!({"status":"INVALID","reason":failure.reason,"nativeObservation":failure.observation})};
                    },
                    None=>raw["foregroundObservation"]=json!({"status":"INVALID","reason":"actual backup capture callback was not observed"}),
                }
                raw["direction"]=serde_json::to_value(direction).map_err(|e|e.to_string())?;
                Ok::<Value,String>(raw)
            }).await.map_err(|e|e.to_string())?
        } else if route=="serverless-bootstrap" {
            final_external_bootstrap_costly_slot(&slot,&mut destination,&fixture.sender,connected.clone(),&fixture.provider,hashes,Some("synthetic-character-0")).await
        } else if route=="remote-backup" {
            let request:StartJobRequest=serde_json::from_value(json!({"connectionId":connected.stored.id,"kind":"restore",
                "snapshotId":backup.as_ref().ok_or("actual full backup producer absent")?.snapshot_id,
                "restoreAreas":["library","referencedAssets","hypa","local-plugins","local-settings"]})).map_err(|e|e.to_string())?;
            let job=DurableJob::new(request,false,crate::external_storage::runtime::now_ms(),destination.external_identity().map_err(|e|e.to_string())?);
            JobStore::open(destination.repository_root()).map_err(|e|format!("{e:?}"))?.put(&job).map_err(|e|format!("{e:?}"))?;
            let jobs=crate::native_file_jobs::NativeFileJobState::initialize(destination.repository_root().join("native-file-jobs"));
            let permit=jobs.admission.file(true)?;
            let outcome=final_remote_backup_costly_slot(&slot,&mut destination,&crate::persistent_store::commands::PersistentStoreState::default(),
                &connected,&job,&fixture.provider,permit,hashes,Some("synthetic-character-0".into())).await;
            outcome.map(|(raw,guard)| {drop(guard);raw})
        } else if scenario==Scenario::DuringCompaction {
            let writer=fixture.a.lww_clock_state().map_err(|e|e.to_string())?.writer_id;
            let ledger=Arc::new(LocalLeaseLedger::open(setup.path()).map_err(|e|format!("{e:?}"))?);
            let job_id=uuid::Uuid::new_v4().to_string();
            let compactor=crate::external_storage::lww_engine::ExternalLwwEngine {provider:fixture.sender.provider.clone(),repository:crate::external_storage::fake::repository(),
                library:fixture.sender.library.clone(),root_key:zeroize::Zeroizing::new(*fixture.sender.root_key),admission:fixture.sender.admission.clone(),
                connection_id:fixture.sender.connection_id.clone(),connection_root:fixture.sender.connection_root.clone(),
                capabilities:fixture.sender.capabilities.clone(),descriptor:fixture.sender.descriptor.clone()};
            let context=leases::LeaseContext {root:&compactor.connection_root,connection_id:&compactor.connection_id,writer_id:&writer,
                descriptor:&compactor.descriptor,root_key:&compactor.root_key,provider:compactor.provider.as_ref(),repository:&compactor.repository,
                clock:leases::system_clock(),protection_supported:compactor.capabilities.lease_operations,ledger:Some(ledger.clone())};
            let owner=match leases::admit(&context,&job_id,risunest_external_storage_format::control::LeaseKind::Work,&Cancellation::default())
                .await.map_err(|e|format!("{e:?}"))? {leases::Admission::Admitted(owner)=>owner,_=>return Err("actual compaction protection not admitted".into())};
            final_compaction_overlap_costly_slot(&slot,compactor,fixture.provider.clone(),setup.path().join(&label),job_id,writer,owner,ledger,
                &mut fixture.a,&mut fixture.b,&mut fixture.sender,&mut fixture.receiver,hashes).await
        } else {Err("selected external scenario/route has no actual native owner endpoint".into())};
        let raw=match raw {Ok(raw)=>raw,Err(error)=>json!({"status":"INVALID","slot":slot,"route":route,"error":error})};
        write(output,&format!("costly-{label}-raw.json"),&raw)?;
        let reads=source_ids.keys().map(|id|Ok((id.clone(),fixture.provider.read_attempts(id).checked_sub(reads_before[id])
            .ok_or("actual source per-object read counter regressed")? as u64))).collect::<Result<BTreeMap<_,_>,String>>()?;
        let reporting=if scenario==Scenario::DuringBackup {server_native_result(&raw,&slot,&partition)}else{external_native_result(&raw,&slot,&partition,&reads)};
        let summary=match reporting {Ok(summary)=>summary,Err(error)=>json!({"status":"INVALID","slot":slot,"error":error,
            "actualSourceObjectReadAttempts":reads,"sourceFileReads":null,"decodedAssetSourceReads":null,"rendererAdoption":null})};
        write(output,&format!("costly-{label}-native-result.json"),&summary)?;summaries.push(summary);
    }
    write(output,"costly-native-results.json",&costly_summary(&summaries))?;
    Ok(())
}

async fn authenticated_source_ids(fixture:&CycleFixture,backup:Option<&crate::external_storage::packaging::CompletedSnapshot>,
    hashes:&[String],directory:&Path)->Result<BTreeMap<String,BTreeSet<String>>,String> {
    use crate::external_storage::{lww_residency,packaging::RemoteObject};
    let (_metadata_root,mut metadata)=server_sync::lww_tests::local();
    let cancel=Cancellation::default();
    if let Some(backup)=backup {
        let catalog=backup.asset_catalog.stored(&fixture.sender.repository).map_err(|e|format!("{e:?}"))?;
        crate::external_storage::snapshot_restore::admit_asset_catalogs(std::slice::from_ref(&catalog),&mut metadata,&fixture.sender.library,
            &backup.snapshot_id,&fixture.sender.connection_id,&fixture.sender.connection_root,&fixture.sender.root_key,
            fixture.sender.provider.as_ref(),&fixture.sender.repository,&cancel).await.map_err(|e|format!("{e:?}"))?;
    } else {
        let mut state=fixture.sender.published_state(directory,&cancel).await.map_err(|e|format!("{e:?}"))?;
        fixture.sender.stage_published_objects(&mut metadata,&mut state,directory,&cancel).await.map_err(|e|format!("{e:?}"))?;
    }
    let mut ids=BTreeMap::<String,BTreeSet<String>>::new();
    for hash in hashes {
        if let Some(source)=lww_residency::packed_source(metadata.repository_root(),hash).map_err(|e|format!("{e:?}"))? {
            if source.packs.is_empty() {return Err("authenticated logical Asset has no physical source pack".into());}
            for pack in source.packs {
                let physical=RemoteObject::from_stored(&pack,&fixture.sender.repository).map_err(|e|format!("{e:?}"))?;
                ids.entry(physical.receipt.locator.object).or_default().insert(hash.clone());
            }
        } else if let Some(source)=lww_residency::source(metadata.repository_root(),hash).map_err(|e|format!("{e:?}"))? {
            let locator=source.body.locator.ok_or("authenticated standalone source locator missing")?;
            locator.validate_for(&fixture.sender.repository).map_err(|e|format!("{e:?}"))?;
            ids.entry(locator.object).or_default().insert(hash.clone());
        } else {return Err("current logical Asset lacks an authenticated source body plan".into());}
    }
    Ok(ids)
}

fn external_native_result(raw:&Value,slot:&costly_runner::CostlySlot,partition:&Value,reads:&BTreeMap<String,u64>)->Result<Value,String> {
    if raw["error"].as_str().is_some() || raw["errors"].as_array().is_some_and(|errors|!errors.is_empty()) {
        return Err("actual native operation reported an error; strict raw record preserved".into());
    }
    let observation=raw.get("nativeObservation").or_else(||raw.pointer("/background/native_observation"))
        .ok_or("actual native counter scope missing")?;
    let observation:NativeObservation=serde_json::from_value(observation.clone()).map_err(|e|e.to_string())?;
    observation.validate_costly_native_coverage()?;
    if slot.scenario==Scenario::RestoreAllPresent {
        if reads.values().any(|count|*count!=0) || observation.body_asset_work!=BodyDomain::default() {
            return Err("all-present observed a real source body request or destination Asset IO/SHA/publication".into());
        }
    }
    if slot.scenario==Scenario::RestoreMissing {
        let missing=partition["missing"].as_object().ok_or("actual destination missing partition absent")?;
        if observation.body_objects.iter().any(|(hash,work)|work.purposes.contains(&"Asset".into())
            && (work.work!=BodyDomain::default() || work.owned_work!=BodyDomain::default()) && !missing.contains_key(hash)) {
            return Err("missing restore performed actual destination work on an already-present Asset".into());
        }
    }
    let (activation,completion)=if slot.scenario==Scenario::DuringCompaction {
        if raw["actualProtectedInputBarrierReached"]!=true || raw.pointer("/background/protection_run_completed")!=Some(&json!(true))
            || raw.pointer("/background/snapshot").is_none_or(Value::is_null) {return Err("actual protected compaction publication receipt missing".into());}
        if raw.pointer("/background/temporary_costs").is_none_or(Value::is_null)
            || raw.pointer("/background/actual_visits").and_then(Value::as_u64).is_none() {
            return Err("actual compactor temporary catalog/disk or visit observation is unavailable".into());
        }
        (None,raw.pointer("/background/native_publication_complete_ms").and_then(Value::as_f64))
    } else {
        if raw["activationRevision"].is_null() && raw.pointer("/activationReceipt/receivedRevision").is_none() {
            return Err("actual durable activation receipt is missing".into());
        }
        (raw["nativeActivationMs"].as_f64(),raw["nativeHydrationConsumerReturnedMs"].as_f64().or_else(||raw["nativeBodyPhaseCompleteMs"].as_f64()))
    };
    if completion.is_none_or(|value|!value.is_finite() || value<0.0) {return Err("actual awaited production completion is unobserved".into());}
    Ok(json!({"status":"native-only-valid","slot":slot,"nativeActivationMs":activation,"nativeCompletionMs":completion,
        "nativeObservation":observation,"actualSourceObjectReadAttempts":reads,
        "sourceFileReads":null,"decodedAssetSourceReads":null,"rendererAdoption":null,
        "compactionCosts":raw.get("background").map(|background|json!({"temporaryCosts":background.get("temporary_costs"),
            "actualVisits":background.get("actual_visits"),"actualUncachedProviderBodyReadAttempts":reads})),
        "scopeClarification":"raw strict full-source result remains INVALID; this report validates existing native counters, awaited completion and authenticated physical provider body-request IDs only"}))
}

fn server_native_result(raw:&Value,slot:&costly_runner::CostlySlot,partition:&Value)->Result<Value,String> {
    if raw["error"].as_str().is_some() || raw["errors"].as_array().is_some_and(|errors|!errors.is_empty())
        || raw.pointer("/nativeWorkerRaw/errors").and_then(Value::as_array).is_some_and(|errors|!errors.is_empty()) {
        return Err("actual native operation failed; all raw failure receipts retained".into());
    }
    let native=raw.get("nativeObservation").or_else(||raw.get("native_observation"))
        .or_else(||raw.pointer("/nativeWorkerRaw/nativeObservation")).ok_or("actual native observation missing")?;
    let observation:NativeObservation=serde_json::from_value(native.clone()).map_err(|e|e.to_string())?;
    observation.validate_costly_native_coverage()?;
    if let Some(parent)=raw.get("nativeParentObservation") {
        serde_json::from_value::<NativeObservation>(parent.clone()).map_err(|e|e.to_string())?.validate_costly_native_coverage()?;
    }
    let all_present=slot.scenario==Scenario::RestoreAllPresent;
    if all_present && observation.body_asset_work!=BodyDomain::default() {return Err("all-present observed actual destination Asset body work".into());}
    if slot.scenario==Scenario::RestoreMissing {
        let missing=partition["missing"].as_object().ok_or("actual missing partition absent")?;
        if observation.body_objects.iter().any(|(hash,work)|work.purposes.contains(&"Asset".into())
            && (work.work!=BodyDomain::default() || work.owned_work!=BodyDomain::default()) && !missing.contains_key(hash)) {
            return Err("missing restore touched an already-present destination Asset".into());
        }
    }
    if let Some(source)=raw.get("source_end_and_shutdown") {
        let settled:source_receipt::SettledSource=serde_json::from_value(source.clone()).map_err(|e|e.to_string())?;
        settled.validate()?;
        if all_present && settled.end.observation.purposes.get("Asset").is_some_and(|work|!work.asset_zero()) {
            return Err("all-present actual source reader touched Asset bodies".into());
        }
    } else {
        let source=&raw["sourceObservation"];
        if source["complete"]!=true {return Err("actual portable or snapshot source scope incomplete".into());}
        if all_present {
            for work in source["objects"].as_object().ok_or("actual archive object observations absent")?.values() {
                if ["opens","reads","bytes","hashedBytes"].iter().any(|field|work[*field].as_u64()!=Some(0)) {
                    return Err("all-present actual archive Asset reader/hash work is nonzero".into());
                }
            }
            for work in source["cachedSqlReads"].as_object().ok_or("actual SQL source observations absent")?.values() {
                if ["reads","bytes","failures","failedAdoptions"].iter().any(|field|work[*field].as_u64()!=Some(0)) {
                    return Err("all-present actual SQL Asset reads are nonzero".into());
                }
            }
        }
    }
    let (activation,completion)=if raw["route"]=="device-file-backup" {
        if raw["nativeAdoptionConfirmed"]!=true || raw["workerJoined"]!=true {return Err("actual portable native ACK or worker completion missing".into());}
        (raw["nativeActivationStatusObservedMs"].as_f64(),raw.pointer("/nativeWorkerRaw/nativeBodiesCompleteMs").and_then(Value::as_f64))
    } else if slot.scenario==Scenario::DuringBackup {
        let foreground:final_runner::RoutineSample=serde_json::from_value(raw["foregroundObservation"].clone()).map_err(|e|e.to_string())?;
        foreground.validate()?;
        let captures=raw.pointer("/sourceObservation/captures").and_then(Value::as_array).ok_or("actual coherent capture milestones absent")?;
        if !captures.iter().any(|capture|capture["lease"]==raw["foregroundCaptureLease"]["lease"] && capture["revision"]==raw["foregroundCaptureLease"]["revision"])
            || raw["foregroundCaptureLease"]["workerJoinedWhileLeaseHeld"]!=true {return Err("foreground did not complete under the actual coherent capture lease".into());}
        (None,raw["nativeBackupCompleteMs"].as_f64())
    } else if slot.scenario==Scenario::DuringAssetTransfer {
        if raw["source_reached"].is_null() || raw["source_release"].is_null() || raw["foreground_publication_ms"].as_f64().is_none() {
            return Err("actual source-read overlap or foreground publication missing".into());
        }
        (None,raw["native_bodies_settled_ms"].as_f64())
    } else {
        (raw["usable_database_ms"].as_f64().or_else(||raw["nativeActivationMs"].as_f64()),
            raw["all_bodies_local_ms"].as_f64().or_else(||raw["nativeBodyPhaseCompleteMs"].as_f64()))
    };
    if completion.is_none_or(|value|!value.is_finite() || value<0.0) {return Err("actual awaited native completion is unobserved".into());}
    Ok(json!({"status":"native-only-valid","slot":slot,"nativeActivationMs":activation,"nativeCompletionMs":completion,
        "nativeObservation":observation,"nativeParentObservation":raw.get("nativeParentObservation"),
        "sourceObservation":raw.get("sourceObservation").or_else(||raw.get("source_end_and_shutdown")),"rendererAdoption":null,
        "scopeClarification":"strict raw status remains unchanged; actual native ACK/completion and existing source scopes validated separately from renderer adoption"}))
}

fn costly_summary(records:&[Value])->Value {
    let mut timings=Vec::new();
    for direction in [FinalDirection::AtoB,FinalDirection::BtoA] {
        let direction_value=serde_json::to_value(direction).unwrap();
        let recorded=records.iter().filter(|record|record["slot"]["direction"]==direction_value && record["slot"]["warmup"]==false).collect::<Vec<_>>();
        let complete=recorded.len()==costly_runner::REPETITIONS as usize && recorded.iter().all(|record|record["status"]=="native-only-valid");
        let values=recorded.iter().filter_map(|record|record["nativeCompletionMs"].as_f64()).collect::<Vec<_>>();
        let percentiles=if complete {measurement::percentiles(values).ok()}else{None};
        let activations=recorded.iter().filter_map(|record|record["nativeActivationMs"].as_f64()).collect::<Vec<_>>();
        let activation_percentiles=if complete && activations.len()==costly_runner::REPETITIONS as usize {measurement::percentiles(activations).ok()}else{None};
        timings.push(json!({"direction":direction,"nativeCompletionMs":percentiles,"nativeActivationMs":activation_percentiles,"recordedCount":recorded.len()}));
    }
    let valid=records.len()==2*(costly_runner::WARMUPS+costly_runner::REPETITIONS) as usize
        && records.iter().all(|record|record["status"]=="native-only-valid");
    json!({"status":if valid {"native-only-valid"}else{"INVALID"},"records":records,"timings":timings,
        "percentileResolution":"N5 nearest-rank p95/p99 equal maximum; population tail unmeasured",
        "networkProfile":"actual loopback TCP or in-memory FakeProvider, no injected latency/bandwidth limit",
        "rendererAdoption":null,"scope":"actual native completion and observed SHA/IO/provider requests; raw strict validation status is preserved"})
}
