use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HashDomain {
    pub calls: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyDomain {
    pub open_attempts: u64,
    pub opens: u64,
    pub failed_opens: u64,
    pub unknown_open_results: u64,
    pub read_operations: u64,
    pub read_bytes: u64,
    pub incomplete_reads: u64,
    pub escaped_handles: u64,
    pub escaped_paths: u64,
    pub outstanding_readers: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectBodyDomain {
    pub purposes: Vec<String>,
    pub work: BodyDomain,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NativeObservation {
    pub units_visited: u64,
    pub hashes: BTreeMap<String, HashDomain>,
    pub incomplete: Vec<String>,
    pub requests: u64,
    pub uploaded_bytes: u64,
    pub downloaded_bytes: u64,
    pub messages_read: u64,
    pub message_bytes_read: u64,
    pub pages_written: u64,
    pub pages_reused: u64,
    pub asset_body_opens: Option<u64>,
    pub asset_body_bytes_read: Option<u64>,
    pub shared_head_reads: Option<u64>,
    pub body_scope_complete: Option<bool>,
    pub body_scope_thread: Option<String>,
    pub body_domains: BTreeMap<String, BodyDomain>,
    pub body_objects: BTreeMap<String, ObjectBodyDomain>,
    pub body_asset_work: BodyDomain,
    pub body_control_work: BodyDomain,
    pub body_owned_work: BodyDomain,
    pub body_unknown_work: BodyDomain,
    pub body_unattributed_managed: BodyDomain,
    pub body_scope_violations: u64,
    pub body_stat_requests: u64,
    pub body_batch_stat_requests: u64,
    pub body_presence_queries: u64,
    pub receive_intents:Option<super::invalid_diagnostics::ReceiveIntentEvidence>,
    pub commit_intents:Option<super::invalid_diagnostics::CommitIntentEvidence>,
}

impl NativeObservation {
    pub fn insert_domain(&mut self, owner: &str, domain: &str, calls: u64, bytes: u64) {
        let key = format!("{owner}/{domain}");
        assert!(self.hashes.insert(key, HashDomain { calls, bytes }).is_none(), "duplicate hash domain");
    }

    pub fn hash_totals(&self) -> Result<HashDomain, String> {
        if !self.incomplete.is_empty() {
            return Err(format!("incomplete SHA inputs: {:?}", self.incomplete));
        }
        self.hashes.values().try_fold(HashDomain::default(), |mut total, domain| {
            total.calls = total.calls.checked_add(domain.calls).ok_or("hash call overflow")?;
            total.bytes = total.bytes.checked_add(domain.bytes).ok_or("hash byte overflow")?;
            Ok(total)
        })
    }

    pub fn validate_routine_invariants(&self) -> Result<(), String> {
        self.hash_totals()?;
        if self.receive_intents.as_ref().is_some_and(|evidence|!evidence.complete) {
            return Err("incomplete exact receive-intent input provenance".into());
        }
        if self.commit_intents.as_ref().is_some_and(|evidence|!evidence.complete) {
            return Err("incomplete exact commit-intent input provenance".into());
        }
        if self.body_scope_complete != Some(true) || self.body_scope_violations != 0
            || self.body_unknown_work != BodyDomain::default()
            || self.body_unattributed_managed != BodyDomain::default()
            || self.body_owned_work != BodyDomain::default()
            || self.body_domains.get("owned").is_some_and(|work| work != &BodyDomain::default())
            || self.body_objects.values().any(|object| object.purposes.is_empty()
                && object.work != BodyDomain::default())
            || self.body_domains.values().chain(self.body_objects.values().map(|object| &object.work)).any(|domain|
            domain.unknown_open_results != 0 || domain.incomplete_reads != 0
                || domain.escaped_handles != 0 || domain.escaped_paths != 0
                || domain.outstanding_readers != 0) {
            return Err("incomplete or unobserved CAS body scope".into());
        }
        if self.asset_body_opens != Some(self.body_asset_work.open_attempts)
            || self.asset_body_bytes_read != Some(self.body_asset_work.read_bytes) {
            return Err("inconsistent asset-purpose body totals".into());
        }
        for (domain, work) in &self.hashes {
            if (domain.contains("binding") || domain.contains("catalog_proof") || domain.contains("generation_digest"))
                && (work.calls != 0 || work.bytes != 0)
            {
                return Err(format!("routine binding proof: {domain}"));
            }
        }
        for (name, value) in [("asset body opens", self.asset_body_opens),
            ("asset body bytes", self.asset_body_bytes_read), ("shared head reads", self.shared_head_reads)] {
            match value {
                Some(0) => {},
                Some(_) => return Err(format!("nonzero {name}")),
                None => return Err(format!("unobserved {name}")),
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct InputAttribution {
    pub input_index:usize,
    pub baseline_bytes:u64,
    pub trial_bytes:u64,
    pub baseline_leaf_bytes:u64,
    pub trial_leaf_bytes:u64,
    pub attributed_growth:u64,
    pub matching_nodes:u64,
    pub unchanged_protected_bytes:u64,
}

#[derive(Debug, Serialize)]
pub struct DomainAttribution {
    pub domain:&'static str,
    pub leaf:&'static str,
    pub calls:u64,
    pub baseline_bytes:u64,
    pub trial_bytes:u64,
    pub attributed_growth:u64,
    pub inputs:Vec<InputAttribution>,
}

#[derive(Debug, Serialize)]
pub struct FrozenGrowthProof {
    pub aggregate_control_growth:u64,
    pub domains:Vec<DomainAttribution>,
}

fn verify_input_collection(evidence:&super::invalid_diagnostics::ReceiveIntentEvidence,
    actual:&HashDomain) -> Result<(),String> {
    if !evidence.complete || !evidence.errors.is_empty()
        || evidence.observed_calls!=actual.calls || evidence.captured_calls!=actual.calls
        || evidence.inputs.len() as u64!=actual.calls
        || evidence.observed_bytes!=actual.bytes || evidence.captured_bytes!=actual.bytes {
        return Err("incomplete or inconsistent exact SHA input collection".into());
    }
    let mut bytes=0u64;
    for input in &evidence.inputs {
        bytes=bytes.checked_add(input.input_bytes).ok_or("input byte overflow")?;
        let own=input.nodes.values().try_fold(0u64,|total,node|total.checked_add(node.own_bytes).ok_or("partition overflow"))?;
        if own!=input.input_bytes || input.nodes.get("").map(|node|node.encoded_bytes)!=Some(input.input_bytes) {
            return Err("exact SHA input partition mismatch".into());
        }
        for (path,node) in &input.nodes {
            let children=input.nodes.iter().filter(|(child,_)| !child.is_empty()
                && child.rsplit_once('/').is_some_and(|(parent,_)|parent==path.as_str())).map(|(_,node)|node).collect::<Vec<_>>();
            let child_bytes=children.iter().try_fold(0u64,|sum,child|sum.checked_add(child.encoded_bytes).ok_or("child byte overflow"))?;
            if children.len() as u64!=node.children || node.own_bytes.checked_add(child_bytes)!=Some(node.encoded_bytes)
                || (!matches!(node.kind.as_str(),"array"|"object") && node.children!=0)
                || !matches!(node.kind.as_str(),"array"|"object"|"string"|"number"|"null"|"boolean") {
                return Err("invalid exact SHA node partition".into());
            }
            if !path.is_empty() && !input.nodes.contains_key(path.rsplit_once('/').ok_or("invalid node path")?.0) {
                return Err("orphan SHA input node".into());
            }
        }
    }
    if bytes!=actual.bytes {return Err("exact input lengths do not conserve SHA bytes".into());}
    Ok(())
}

fn prove_domain(a:&NativeObservation,b:&NativeObservation,domain:&'static str,leaf:&'static str,kind:&str,
    left:Option<&super::invalid_diagnostics::ReceiveIntentEvidence>,
    right:Option<&super::invalid_diagnostics::ReceiveIntentEvidence>) -> Result<DomainAttribution,String> {
    let smaller=a.hashes.get(domain).cloned().unwrap_or_default();
    let larger=b.hashes.get(domain).cloned().unwrap_or_default();
    let left=left.ok_or("missing exact baseline input collection")?;
    let right=right.ok_or("missing exact trial input collection")?;
    verify_input_collection(left,&smaller)?;
    verify_input_collection(right,&larger)?;
    if smaller.calls!=larger.calls {return Err("approved SHA domain input count changed".into());}
    let mut inputs=Vec::new();
    let mut growth=0u64;
    for (index,(a,b)) in left.inputs.iter().zip(&right.inputs).enumerate() {
        if !a.nodes.keys().eq(b.nodes.keys()) {return Err("SHA input structural node set changed".into());}
        let mut leaf_growth=0u64;
        let mut leaf_bytes=(0,0);
        for (path,left) in &a.nodes {
            let right=&b.nodes[path];
            if left.kind!=right.kind || left.children!=right.children {return Err("SHA input node shape changed".into());}
            if path.as_str()==leaf {
                if left.kind!=kind || left.children!=0 {return Err("approved control leaf type mismatch".into());}
                leaf_growth=right.own_bytes.checked_sub(left.own_bytes).ok_or("negative control growth cannot cancel work")?;
                leaf_bytes=(left.own_bytes,right.own_bytes);
            } else if left.own_bytes!=right.own_bytes {
                return Err(format!("protected or unknown SHA input contribution changed: {domain}:{path}"));
            }
        }
        if b.input_bytes.checked_sub(a.input_bytes)!=Some(leaf_growth) {return Err("SHA growth does not equal approved leaf growth".into());}
        growth=growth.checked_add(leaf_growth).ok_or("attribution overflow")?;
        inputs.push(InputAttribution {input_index:index,baseline_bytes:a.input_bytes,trial_bytes:b.input_bytes,
            baseline_leaf_bytes:leaf_bytes.0,trial_leaf_bytes:leaf_bytes.1,attributed_growth:leaf_growth,
            matching_nodes:a.nodes.len() as u64,unchanged_protected_bytes:a.input_bytes-leaf_bytes.0});
    }
    if larger.bytes.checked_sub(smaller.bytes)!=Some(growth) {return Err("domain SHA growth does not conserve approved attribution".into());}
    Ok(DomainAttribution {domain,leaf,calls:smaller.calls,baseline_bytes:smaller.bytes,
        trial_bytes:larger.bytes,attributed_growth:growth,inputs})
}

pub fn assert_frozen_growth(a:&NativeObservation,b:&NativeObservation) -> Result<FrozenGrowthProof,String> {
    a.validate_routine_invariants()?;
    b.validate_routine_invariants()?;
    if b.units_visited>a.units_visited || b.requests>a.requests {return Err("unit/request growth".into());}
    let mut proof=FrozenGrowthProof {aggregate_control_growth:0,domains:Vec::new()};
    for (domain,leaf,kind,left,right) in [
        ("native/native_receive_intent","/progress/cursor","string",a.receive_intents.as_ref(),b.receive_intents.as_ref()),
        ("native/native_intent","/commit/expectedRevision","number",a.commit_intents.as_ref(),b.commit_intents.as_ref())] {
        if a.hashes.contains_key(domain) || b.hashes.contains_key(domain) || left.is_some() || right.is_some() {
            let attribution=prove_domain(a,b,domain,leaf,kind,left,right)?;
            proof.aggregate_control_growth=proof.aggregate_control_growth.checked_add(attribution.attributed_growth).ok_or("growth overflow")?;
            proof.domains.push(attribution);
        }
    }
    for (key,larger) in &b.hashes {
        let smaller=a.hashes.get(key).cloned().unwrap_or_default();
        if larger.calls>smaller.calls {return Err(format!("hash call growth: {key}"));}
        if larger.bytes>smaller.bytes && !proof.domains.iter().any(|domain|domain.domain==key.as_str()) {
            return Err(format!("unattributed hash byte growth: {key}"));
        }
    }
    if proof.aggregate_control_growth>8192 {return Err("control metadata growth exceeds 8 KiB".into());}
    Ok(proof)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_and_unobserved_inputs_cannot_pass() {
        let mut sample = NativeObservation::default();
        assert!(sample.validate_routine_invariants().is_err());
        sample.asset_body_opens = Some(0);
        sample.asset_body_bytes_read = Some(0);
        sample.shared_head_reads = Some(0);
        sample.body_scope_complete = Some(true);
        assert!(sample.validate_routine_invariants().is_ok());
        sample.commit_intents=Some(Default::default());
        assert_eq!(sample.validate_routine_invariants().unwrap_err(),"incomplete exact commit-intent input provenance");
        sample.commit_intents=None;
        sample.incomplete.push("native/stream".into());
        assert!(sample.hash_totals().is_err());
    }
    #[test]
    fn role_coverage_and_reader_lifecycle_remain_fail_closed() {
        let mut sample = NativeObservation {asset_body_opens:Some(0), asset_body_bytes_read:Some(0),
            shared_head_reads:Some(0), body_scope_complete:Some(true), ..Default::default()};
        sample.body_control_work = BodyDomain {open_attempts:1, opens:1, read_operations:1,
            read_bytes:22, ..Default::default()};
        sample.body_domains.insert("managed".into(), sample.body_control_work.clone());
        sample.validate_routine_invariants().unwrap();
        sample.body_unknown_work.open_attempts = 1;
        assert!(sample.validate_routine_invariants().is_err());
        sample.body_unknown_work = BodyDomain::default();
        sample.body_domains.get_mut("managed").unwrap().outstanding_readers = 1;
        assert!(sample.validate_routine_invariants().is_err());
        sample.body_domains.get_mut("managed").unwrap().outstanding_readers = 0;
        sample.body_scope_violations = 1;
        assert!(sample.validate_routine_invariants().is_err());
        sample.body_scope_violations = 0;
        sample.body_owned_work.read_bytes = 1;
        assert!(sample.validate_routine_invariants().is_err());
    }

    #[test]
    fn growth_requires_precise_control_attribution_and_never_more_calls() {
        let mut a = NativeObservation { asset_body_opens:Some(0), asset_body_bytes_read:Some(0),
            shared_head_reads:Some(0), body_scope_complete:Some(true), ..Default::default() };
        a.insert_domain("C", "intent", 1, 100);
        let mut b = a.clone();
        b.hashes.get_mut("C/intent").unwrap().bytes += 10;
        assert!(assert_frozen_growth(&a, &b).is_err());
        b.hashes.get_mut("C/intent").unwrap().calls += 1;
        assert!(assert_frozen_growth(&a, &b).is_err());
    }

    fn observed(domain:&str,input:&[u8]) -> NativeObservation {
        let mut sample=NativeObservation {asset_body_opens:Some(0),asset_body_bytes_read:Some(0),
            shared_head_reads:Some(0),body_scope_complete:Some(true),..Default::default()};
        sample.hashes.insert(domain.into(),HashDomain {calls:1,bytes:input.len() as u64});
        let mut evidence=super::super::invalid_diagnostics::ReceiveIntentEvidence {
            observed_calls:1,captured_calls:1,observed_bytes:input.len() as u64,captured_bytes:input.len() as u64,
            inputs:vec![super::super::invalid_diagnostics::exact_json_structure(input,input).unwrap()],..Default::default()};
        evidence.finish();
        if domain=="native/native_intent" {sample.commit_intents=Some(evidence);}
        else {sample.receive_intents=Some(evidence);}
        sample
    }

    fn proof_pairs() -> Vec<(NativeObservation,NativeObservation)> {
        vec![(observed("native/native_receive_intent",br#"{"progress":{"cursor":"9"},"changes":[{"value":"same"}]}"#),
            observed("native/native_receive_intent",br#"{"progress":{"cursor":"10"},"changes":[{"value":"same"}]}"#)),
            (observed("native/native_intent",br#"{"commit":{"expectedRevision":9,"value":"same"},"aliases":[]}"#),
            observed("native/native_intent",br#"{"commit":{"expectedRevision":10,"value":"same"},"aliases":[]}"#))]
    }

    fn collection(sample:&mut NativeObservation) -> &mut super::super::invalid_diagnostics::ReceiveIntentEvidence {
        if sample.commit_intents.is_some() {sample.commit_intents.as_mut().unwrap()}
        else {sample.receive_intents.as_mut().unwrap()}
    }

    #[test]
    fn approved_exact_leaves_produce_conserved_per_input_attribution() {
        for (a,b) in proof_pairs() {
            let proof=assert_frozen_growth(&a,&b).unwrap();
            assert_eq!(proof.aggregate_control_growth,1);
            assert_eq!(proof.domains[0].inputs.len(),1);
            assert_eq!(proof.domains[0].inputs[0].attributed_growth,1);
            assert_eq!(proof.domains[0].trial_bytes-proof.domains[0].baseline_bytes,1);
        }
    }

    #[test]
    fn incomplete_missing_unpaired_and_extra_call_inputs_cannot_bypass_proof() {
        for (a,b) in proof_pairs() {
            let mut invalid=b.clone();collection(&mut invalid).complete=false;
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();collection(&mut invalid).errors.push("roundtrip failed".into());
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();invalid.receive_intents=None;invalid.commit_intents=None;
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();collection(&mut invalid).inputs.clear();
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();invalid.hashes.values_mut().next().unwrap().calls+=1;
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();
            for work in invalid.hashes.values_mut() {work.calls*=2;work.bytes*=2;}
            let evidence=collection(&mut invalid);
            evidence.inputs.push(evidence.inputs[0].clone());
            evidence.observed_calls*=2;evidence.captured_calls*=2;
            evidence.observed_bytes*=2;evidence.captured_bytes*=2;
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();collection(&mut invalid).captured_bytes+=1;
            assert!(assert_frozen_growth(&a,&invalid).is_err());
        }
    }

    #[test]
    fn node_set_kind_children_and_partition_tampering_cannot_bypass_proof() {
        for (a,b) in proof_pairs() {
            let mut invalid=b.clone();collection(&mut invalid).inputs[0].nodes.remove("");
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();collection(&mut invalid).inputs[0].nodes.get_mut("").unwrap().kind="array".into();
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();collection(&mut invalid).inputs[0].nodes.get_mut("").unwrap().children+=1;
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();collection(&mut invalid).inputs[0].nodes.get_mut("").unwrap().own_bytes+=1;
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();collection(&mut invalid).inputs[0].input_bytes+=1;
            assert!(assert_frozen_growth(&a,&invalid).is_err());
        }
    }

    #[test]
    fn protected_payload_negative_cancellation_unknown_fields_and_domains_reject() {
        for (domain,a,b) in [
            ("native/native_receive_intent",br#"{"progress":{"cursor":"9"},"changes":[{"value":"same"}]}"#.as_slice(),
                br#"{"progress":{"cursor":"10"},"changes":[{"value":"sam"}]}"#.as_slice()),
            ("native/native_intent",br#"{"commit":{"expectedRevision":9,"value":"same"},"aliases":[]}"#.as_slice(),
                br#"{"commit":{"expectedRevision":10,"value":"sam"},"aliases":[]}"#.as_slice())] {
            assert!(assert_frozen_growth(&observed(domain,a),&observed(domain,b)).is_err());
        }
        for (a,b) in proof_pairs() {
            assert!(assert_frozen_growth(&b,&a).is_err());
            let mut invalid=b.clone();invalid.insert_domain("other","unknown",1,1);
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();invalid.requests+=1;
            assert!(assert_frozen_growth(&a,&invalid).is_err());
            let mut invalid=b.clone();invalid.units_visited+=1;
            assert!(assert_frozen_growth(&a,&invalid).is_err());
        }
        let a=observed("native/native_receive_intent",br#"{"progress":{"cursor":"9"}}"#);
        let b=observed("native/native_receive_intent",br#"{"progress":{"cursor":"10"},"unknown":"added"}"#);
        assert!(assert_frozen_growth(&a,&b).is_err());
        for (domain,left,right) in [
            ("native/native_receive_intent",br#"{"progress":{"cursor":"9"},"changes":[{"value":"same"}]}"#.as_slice(),
                br#"{"progress":{"cursor":"10"},"changes":[{"value":"longer"}]}"#.as_slice()),
            ("native/native_intent",br#"{"commit":{"expectedRevision":9,"key":"same"},"aliases":[]}"#.as_slice(),
                br#"{"commit":{"expectedRevision":10,"key":"same"},"aliases":[{"key":"asset"}]}"#.as_slice()),
            ("native/native_receive_intent",br#"{"progress":{"cursor":"9"},"changes":[{"key":"same"}]}"#.as_slice(),
                br#"{"progress":{"cursor":"10"},"changes":[{"renamed":"same"}]}"#.as_slice()),
            ("native/native_intent",br#"{"commit":{"expectedRevision":9}}"#.as_slice(),
                br#" {"commit":{"expectedRevision":10}}"#.as_slice())] {
            assert!(assert_frozen_growth(&observed(domain,left),&observed(domain,right)).is_err());
        }
    }

    #[test]
    fn original_8192_byte_cap_is_aggregate_across_both_precise_domains() {
        fn repeat(sample:&mut NativeObservation,count:u64) {
            for domain in sample.hashes.values_mut() {domain.calls*=count;domain.bytes*=count;}
            let evidence=collection(sample);
            evidence.inputs=vec![evidence.inputs[0].clone();count as usize];
            evidence.observed_calls*=count;evidence.captured_calls*=count;
            evidence.observed_bytes*=count;evidence.captured_bytes*=count;
        }
        let mut pairs=proof_pairs();
        let (mut a,mut b)=pairs.remove(0);let (mut commit_a,mut commit_b)=pairs.remove(0);
        repeat(&mut a,4096);repeat(&mut b,4096);repeat(&mut commit_a,4096);repeat(&mut commit_b,4096);
        a.hashes.extend(commit_a.hashes);a.commit_intents=commit_a.commit_intents;
        b.hashes.extend(commit_b.hashes);b.commit_intents=commit_b.commit_intents;
        assert_eq!(assert_frozen_growth(&a,&b).unwrap().aggregate_control_growth,8192);
        let (mut commit_a,mut commit_b)=proof_pairs().remove(1);
        repeat(&mut commit_a,4097);repeat(&mut commit_b,4097);
        a.hashes.extend(commit_a.hashes);a.commit_intents=commit_a.commit_intents;
        b.hashes.extend(commit_b.hashes);b.commit_intents=commit_b.commit_intents;
        assert_eq!(assert_frozen_growth(&a,&b).unwrap_err(),"control metadata growth exceeds 8 KiB");
    }
}
